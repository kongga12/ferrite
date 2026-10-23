//! egov 언어 서버(LSP, 표준 입출력). 비동기 런타임 없이 메시지를 하나씩 처리한다.
//! 시작할 때 프로젝트의 매퍼를 모아 색인하고, 열린 문서는 편집 중인 내용으로 덮어쓴다.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use ferrite_core::text_file;
use ferrite_lsp::position::{PositionEncoding, to_lsp};
use ferrite_lsp::transport::{read_message, write_message};
use ferrite_lsp::uri::{path_to_uri, uri_to_path};
use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Range, Uri};
use ropey::Rope;
use serde_json::{Value, json};

use crate::mapper::{self, Index, Severity};

/// 색인할 때 건너뛰는 폴더(빌드 결과물·도구 폴더). 빌드 결과물에는 같은 매퍼의 사본이 있다.
const SKIP_DIRS: [&str; 10] = [
    ".git",
    ".gradle",
    ".idea",
    ".settings",
    ".vscode",
    "bin",
    "build",
    "node_modules",
    "out",
    "target",
];

/// 표준 입출력으로 LSP 서버를 돌린다. `exit` 알림이나 입력이 끝나면 돌아온다.
pub fn run_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run(&mut stdin.lock(), &mut stdout.lock())
}

pub fn run(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<()> {
    let mut server = Server::default();
    while let Some(body) = read_message(input)? {
        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if method == "exit" {
            break;
        }
        let replies = match message.get("id") {
            Some(id) => vec![server.request(id.clone(), method, &params)],
            None => server.notification(method, &params),
        };
        for reply in replies {
            write_message(output, &serde_json::to_vec(&reply)?)?;
        }
        output.flush()?;
    }
    Ok(())
}

struct Document {
    rope: Rope,
}

struct Server {
    root: Option<PathBuf>,
    encoding: PositionEncoding,
    index: Index,
    /// 열린 문서(편집 중인 내용).
    open: HashMap<PathBuf, Document>,
    /// 진단을 보낸 문서. 더 이상 매퍼가 아니게 되면 빈 진단으로 지운다.
    published: HashSet<PathBuf>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            root: None,
            encoding: PositionEncoding::Utf16,
            index: Index::default(),
            open: HashMap::new(),
            published: HashSet::new(),
        }
    }
}

impl Server {
    fn request(&mut self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "initialize" => {
                self.root = initialize_root(params);
                // UTF-8을 고를 수 있으면 고른다(한글 위치 계산이 바이트 그대로라 단순하다).
                let offered = params
                    .pointer("/capabilities/general/positionEncodings")
                    .and_then(Value::as_array)
                    .is_some_and(|list| list.iter().any(|e| e == "utf-8"));
                if offered {
                    self.encoding = PositionEncoding::Utf8;
                }
                let encoding = match self.encoding {
                    PositionEncoding::Utf8 => "utf-8",
                    PositionEncoding::Utf16 => "utf-16",
                };
                json!({ "jsonrpc": "2.0", "id": id, "result": {
                    "capabilities": {
                        "positionEncoding": encoding,
                        "textDocumentSync": { "openClose": true, "change": 1 },
                    },
                    "serverInfo": { "name": "ferrite-egov", "version": env!("CARGO_PKG_VERSION") },
                } })
            }
            "shutdown" => json!({ "jsonrpc": "2.0", "id": id, "result": null }),
            _ => json!({ "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("지원하지 않는 요청: {method}") } }),
        }
    }

    fn notification(&mut self, method: &str, params: &Value) -> Vec<Value> {
        match method {
            "initialized" => {
                if let Some(root) = self.root.clone() {
                    self.index_project(&root);
                }
                Vec::new()
            }
            "textDocument/didOpen" => {
                let text = params.pointer("/textDocument/text").and_then(Value::as_str);
                self.update(document_path(params), text)
            }
            // 전체 내용 동기화만 쓴다(textDocumentSync.change = Full).
            "textDocument/didChange" => {
                let text = params
                    .pointer("/contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text"))
                    .and_then(Value::as_str);
                self.update(document_path(params), text)
            }
            "textDocument/didClose" => {
                let Some(path) = document_path(params) else {
                    return Vec::new();
                };
                self.open.remove(&path);
                // 닫으면 디스크 내용이 다시 기준이다.
                match read_mapper(&path) {
                    Some(mapper) => self.index.mappers.insert(path.clone(), mapper),
                    None => self.index.mappers.remove(&path),
                };
                let mut replies = self.publish_all();
                if self.published.remove(&path) {
                    replies.extend(publish(&path, Vec::new()));
                }
                replies
            }
            _ => Vec::new(),
        }
    }

    /// 열린 문서의 내용을 바꾸고 열린 매퍼 모두를 다시 진단한다(다른 파일을 참조할 수 있어서).
    fn update(&mut self, path: Option<PathBuf>, text: Option<&str>) -> Vec<Value> {
        let (Some(path), Some(text)) = (path, text) else {
            return Vec::new();
        };
        match mapper::parse(text) {
            Some(mapper) => {
                self.index.mappers.insert(path.clone(), mapper);
            }
            None => {
                self.index.mappers.remove(&path);
            }
        }
        self.open.insert(
            path,
            Document {
                rope: Rope::from_str(text),
            },
        );
        self.publish_all()
    }

    fn publish_all(&mut self) -> Vec<Value> {
        let mut replies = Vec::new();
        for (path, document) in &self.open {
            let Some(mapper) = self.index.mappers.get(path) else {
                if self.published.remove(path) {
                    replies.extend(publish(path, Vec::new()));
                }
                continue;
            };
            let diagnostics = mapper::diagnose(mapper, &self.index)
                .into_iter()
                .map(|finding| {
                    let rope = &document.rope;
                    let to = |byte: usize| {
                        to_lsp(
                            rope,
                            rope.byte_to_char(byte.min(rope.len_bytes())),
                            self.encoding,
                        )
                    };
                    Diagnostic {
                        range: Range::new(to(finding.range.start), to(finding.range.end)),
                        severity: Some(match finding.severity {
                            Severity::Error => DiagnosticSeverity::ERROR,
                            Severity::Warning => DiagnosticSeverity::WARNING,
                        }),
                        code: Some(NumberOrString::String(finding.code.into())),
                        source: Some("ferrite-egov".into()),
                        message: finding.message,
                        ..Default::default()
                    }
                })
                .collect();
            self.published.insert(path.clone());
            replies.extend(publish(path, diagnostics));
        }
        replies
    }

    /// 프로젝트 폴더의 매퍼를 모두 읽어 색인한다. 열린 문서는 편집 중인 내용을 그대로 둔다.
    fn index_project(&mut self, root: &Path) {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if !SKIP_DIRS.iter().any(|s| name == *s) {
                        stack.push(path);
                    }
                } else if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
                    && !self.open.contains_key(&path)
                    && let Some(mapper) = read_mapper(&path)
                {
                    self.index.mappers.insert(path, mapper);
                }
            }
        }
    }
}

/// 디스크의 매퍼를 읽는다. 예전 egov 프로젝트의 EUC-KR(CP949) 파일도 읽는다.
fn read_mapper(path: &Path) -> Option<mapper::Mapper> {
    let bytes = text_file::read(path).ok()?;
    // 대부분의 XML은 매퍼가 아니므로 파싱 전에 거른다.
    let looks = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if !looks(b"<mapper") && !looks(b"<sqlMap") {
        return None;
    }
    mapper::parse(&text_file::decode(&bytes).ok()?.text)
}

fn initialize_root(params: &Value) -> Option<PathBuf> {
    let uri = params
        .pointer("/workspaceFolders/0/uri")
        .or_else(|| params.get("rootUri"))
        .and_then(Value::as_str)?;
    uri_to_path(&uri.parse::<Uri>().ok()?)
}

fn document_path(params: &Value) -> Option<PathBuf> {
    let uri = params
        .pointer("/textDocument/uri")
        .and_then(Value::as_str)?;
    uri_to_path(&uri.parse::<Uri>().ok()?)
}

fn publish(path: &Path, diagnostics: Vec<Diagnostic>) -> Option<Value> {
    let uri = path_to_uri(path).ok()?;
    Some(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": diagnostics },
    }))
}
