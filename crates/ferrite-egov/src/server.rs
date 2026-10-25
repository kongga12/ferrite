//! egov 언어 서버(LSP, 표준 입출력). 비동기 런타임 없이 메시지를 하나씩 처리한다.
//! 시작할 때 프로젝트의 매퍼와 Java 인터페이스를 모아 색인하고, 열린 문서는 편집 중인 내용으로
//! 덮어쓴다.
//!
//! 위치는 UTF-16으로 주고받는다. 이 서버는 다른 서버(jdt.ls, lemminx)와 함께 한 문서를 맡아
//! 결과가 합쳐지므로, 그 서버들과 같은 단위를 써야 위치가 어긋나지 않는다.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use ferrite_core::text_file;
use ferrite_lsp::position::{PositionEncoding, from_lsp, to_lsp};
use ferrite_lsp::transport::{read_message, write_message};
use ferrite_lsp::uri::{path_to_uri, uri_to_path};
use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Uri};
use ropey::Rope;
use serde_json::{Value, json};

use crate::java::{self, JavaFile, Target, TypeDecl};
use crate::jsp;
use crate::mapper::{self, Aliases, Context, Dialect, Finding, Index, Kind, Severity};

const ENCODING: PositionEncoding = PositionEncoding::Utf16;
/// 클래스 이름 자동완성에서 한 번에 돌려줄 최대 개수. 넘으면 목록이 미완성이라고 알린다.
const MAX_CLASS_ITEMS: usize = 200;
/// 필드가 아니라 이름으로 값을 꺼내는 타입(`#{아무이름}`). 속성을 완성할 수 없다.
const MAP_TYPES: [&str; 6] = [
    "map",
    "hashmap",
    "java.util.map",
    "java.util.hashmap",
    "egovmap",
    "egovframework.rte.psl.dataaccess.util.egovmap",
];
/// MyBatis 기본 별칭 중 자주 쓰는 것(클래스 이름 자동완성에 함께 보인다).
const BUILTIN_ALIASES: [&str; 10] = [
    "string",
    "int",
    "long",
    "double",
    "boolean",
    "date",
    "map",
    "hashmap",
    "list",
    "arraylist",
];

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

fn is_java(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("java"))
}

fn is_jsp(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["jsp", "jspf", "tag"]
            .iter()
            .any(|j| e.eq_ignore_ascii_case(j))
    })
}

#[derive(Default)]
struct Server {
    root: Option<PathBuf>,
    index: Index,
    /// Java 파일의 타입(Mapper 인터페이스, VO 클래스).
    java: HashMap<PathBuf, JavaFile>,
    /// XML 파일의 타입 별칭(iBatis sqlMap, MyBatis 설정).
    aliases: HashMap<PathBuf, Aliases>,
    /// 열린 문서(편집 중인 내용).
    open: HashMap<PathBuf, Rope>,
    /// 진단을 보낸 문서. 더 이상 대상이 아니게 되면 빈 진단으로 지운다.
    published: HashSet<PathBuf>,
}

impl Server {
    fn request(&mut self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "initialize" => {
                self.root = initialize_root(params);
                json!({ "jsonrpc": "2.0", "id": id, "result": {
                    "capabilities": {
                        "positionEncoding": "utf-16",
                        "textDocumentSync": { "openClose": true, "change": 1 },
                        "definitionProvider": true,
                        "completionProvider": { "triggerCharacters": ["{", "#", "\"", "."] },
                    },
                    "serverInfo": { "name": "ferrite-egov", "version": env!("CARGO_PKG_VERSION") },
                } })
            }
            "textDocument/definition" => {
                json!({ "jsonrpc": "2.0", "id": id, "result": self.definition(params) })
            }
            "textDocument/completion" => {
                json!({ "jsonrpc": "2.0", "id": id, "result": self.completion(params) })
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
                let text = read_text(&path);
                self.reindex(&path, text.as_deref());
                let mut replies = self.publish_all();
                if self.published.remove(&path) {
                    replies.extend(publish(&path, Vec::new()));
                }
                replies
            }
            _ => Vec::new(),
        }
    }

    /// 열린 문서의 내용을 바꾸고 열린 문서 모두를 다시 진단한다(서로 참조할 수 있어서).
    fn update(&mut self, path: Option<PathBuf>, text: Option<&str>) -> Vec<Value> {
        let (Some(path), Some(text)) = (path, text) else {
            return Vec::new();
        };
        self.reindex(&path, Some(text));
        self.open.insert(path, Rope::from_str(text));
        self.publish_all()
    }

    /// 파일 하나의 색인을 이 내용으로 바꾼다(None이면 뺀다).
    fn reindex(&mut self, path: &Path, text: Option<&str>) {
        if is_jsp(path) {
            // JSP는 색인하지 않는다(열린 내용만 진단한다).
        } else if is_java(path) {
            match text.map(java::parse_file).filter(|f| !f.types.is_empty()) {
                Some(found) => self.java.insert(path.to_path_buf(), found),
                None => self.java.remove(path),
            };
        } else {
            match text.and_then(mapper::parse) {
                Some(found) => self.index.mappers.insert(path.to_path_buf(), found),
                None => self.index.mappers.remove(path),
            };
            let aliases = text.map(mapper::aliases).unwrap_or_default();
            if aliases == Aliases::default() {
                self.aliases.remove(path);
            } else {
                self.aliases.insert(path.to_path_buf(), aliases);
            }
        }
    }

    fn publish_all(&mut self) -> Vec<Value> {
        let mut replies = Vec::new();
        for (path, rope) in &self.open {
            let findings = if is_jsp(path) {
                Some(jsp::diagnose(&rope.to_string()))
            } else if is_java(path) {
                self.java
                    .get(path)
                    .and_then(|file| missing_statements(&file.types, &self.index))
            } else {
                self.index
                    .mappers
                    .get(path)
                    .map(|m| mapper::diagnose(m, &self.index))
            };
            let Some(findings) = findings else {
                if self.published.remove(path) {
                    replies.extend(publish(path, Vec::new()));
                }
                continue;
            };
            let diagnostics = findings
                .into_iter()
                .map(|finding| Diagnostic {
                    range: lsp_range(rope, &finding.range),
                    severity: Some(match finding.severity {
                        Severity::Error => DiagnosticSeverity::ERROR,
                        Severity::Warning => DiagnosticSeverity::WARNING,
                        Severity::Information => DiagnosticSeverity::INFORMATION,
                    }),
                    code: Some(NumberOrString::String(finding.code.into())),
                    source: Some("ferrite-egov".into()),
                    message: finding.message,
                    ..Default::default()
                })
                .collect();
            self.published.insert(path.clone());
            replies.extend(publish(path, diagnostics));
        }
        replies
    }

    /// 정의로 이동. 매퍼 XML과 Java 사이를 잇는다(같은 파일·다른 파일).
    fn definition(&self, params: &Value) -> Value {
        let Some(path) = document_path(params) else {
            return Value::Null;
        };
        let (Some(rope), Ok(position)) = (
            self.open.get(&path),
            serde_json::from_value::<Position>(params["position"].clone()),
        ) else {
            return Value::Null;
        };
        let offset = rope.char_to_byte(from_lsp(rope, position, ENCODING));
        let targets = if is_java(&path) {
            self.java_targets(&rope.to_string(), offset)
        } else {
            self.xml_targets(&path, offset)
        };
        // 대상 파일마다 한 번만 읽는다(열려 있으면 편집 중인 내용).
        let mut ropes: HashMap<PathBuf, Rope> = HashMap::new();
        let mut locations = Vec::new();
        for (target, range) in targets {
            let rope = ropes.entry(target.clone()).or_insert_with(|| {
                self.open
                    .get(&target)
                    .cloned()
                    .unwrap_or_else(|| Rope::from_str(&read_text(&target).unwrap_or_default()))
            });
            if let Ok(uri) = path_to_uri(&target) {
                let location = json!({ "uri": uri, "range": lsp_range(rope, &range) });
                if !locations.contains(&location) {
                    locations.push(location);
                }
            }
        }
        if locations.is_empty() {
            Value::Null
        } else {
            Value::Array(locations)
        }
    }

    /// XML: 참조(refid·resultMap·select) → 정의, MyBatis 문장 id → Mapper 인터페이스 메서드.
    fn xml_targets(&self, path: &Path, offset: usize) -> Vec<(PathBuf, Range<usize>)> {
        let Some(mapper) = self.index.mappers.get(path) else {
            return Vec::new();
        };
        let within = |range: &Range<usize>| range.start <= offset && offset <= range.end;
        if let Some(reference) = mapper.refs.iter().find(|r| within(&r.range)) {
            return self
                .index
                .targets(mapper, reference)
                .into_iter()
                .map(|(found, def)| {
                    (
                        found.cloned().unwrap_or_else(|| path.to_path_buf()),
                        def.range.clone(),
                    )
                })
                .collect();
        }
        let statement = mapper
            .defs
            .iter()
            .find(|d| d.kind == Kind::Statement && within(&d.range));
        match (statement, &mapper.namespace, mapper.dialect) {
            (Some(def), Some(namespace), Dialect::MyBatis) => self
                .java
                .iter()
                .flat_map(|(file, java)| {
                    java.types
                        .iter()
                        .filter(|t| t.is_interface && &t.fqn == namespace)
                        .flat_map(|t| t.methods.iter().filter(|m| m.name == def.id))
                        .map(|m| (file.clone(), m.range.clone()))
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Java: Mapper 메서드(선언·호출)나 문자열 문장 id → 매퍼 XML 문장.
    fn java_targets(&self, text: &str, offset: usize) -> Vec<(PathBuf, Range<usize>)> {
        let found = match java::target_at(text, offset) {
            Some(Target::Method { namespace, name }) => self.index.find(
                Kind::Statement,
                |m| m.namespace.as_deref() == Some(namespace.as_str()),
                &name,
            ),
            // DAO 방식: "namespace.id"(MyBatis·iBatis), iBatis는 id 그대로도 쓴다.
            Some(Target::StatementId(id)) => {
                let mut found = self.index.qualified(Kind::Statement, &id);
                found.extend(self.index.find(
                    Kind::Statement,
                    |m| m.dialect == Dialect::IBatis,
                    &id,
                ));
                found
            }
            None => Vec::new(),
        };
        found
            .into_iter()
            .filter_map(|(file, def)| Some((file?.clone(), def.range.clone())))
            .collect()
    }

    /// 자동완성: `#{}` 안의 속성(VO 필드·`@Param` 이름), 클래스 이름을 받는 속성값,
    /// resultMap의 `property`.
    fn completion(&self, params: &Value) -> Value {
        let Some(path) = document_path(params) else {
            return Value::Null;
        };
        let (Some(rope), Some(mapper), Ok(position)) = (
            self.open.get(&path),
            self.index.mappers.get(&path),
            serde_json::from_value::<Position>(params["position"].clone()),
        ) else {
            return Value::Null;
        };
        let text = rope.to_string();
        let offset = rope.char_to_byte(from_lsp(rope, position, ENCODING));
        let (items, incomplete) = match mapper::context_at(&text, mapper.dialect, offset) {
            Some(Context::Parameter {
                parameter_type,
                statement_id,
                path,
            }) => (
                self.parameter_items(
                    mapper,
                    parameter_type.as_deref(),
                    statement_id.as_deref(),
                    &path,
                ),
                false,
            ),
            Some(Context::Property { owner }) => {
                let fields = owner
                    .and_then(|owner| self.class(&self.resolve_alias(&owner)))
                    .map(|(file, decl)| self.fields(file, decl))
                    .unwrap_or_default();
                (field_items(&fields), false)
            }
            Some(Context::ClassName { replace, typed }) => self.class_items(rope, replace, &typed),
            None => return Value::Null,
        };
        json!({ "isIncomplete": incomplete, "items": items })
    }

    /// `#{}` 안 속성. 타입은 문장의 parameterType(별칭 풀기), 없으면 namespace 인터페이스 메서드의
    /// 매개변수에서 찾는다. `path`(`user.` 같은 앞 조각)를 따라 필드의 타입으로 내려간다.
    fn parameter_items(
        &self,
        mapper: &mapper::Mapper,
        parameter_type: Option<&str>,
        statement_id: Option<&str>,
        path: &[String],
    ) -> Vec<Value> {
        let mut current: Option<(&JavaFile, &TypeDecl)> = None;
        let mut rest = path;
        if let Some(ty) = parameter_type {
            if is_map(ty) {
                return Vec::new();
            }
            current = self.class(&self.resolve_alias(ty));
        } else if let (Some(namespace), Some(id)) = (&mapper.namespace, statement_id) {
            let method = self.java.values().find_map(|file| {
                file.types
                    .iter()
                    .filter(|t| t.is_interface && &t.fqn == namespace)
                    .flat_map(|t| t.methods.iter())
                    .find(|m| m.name == id)
                    .map(|m| (file, m))
            });
            let Some((file, method)) = method else {
                return Vec::new();
            };
            let bound: Vec<&java::Param> =
                method.params.iter().filter(|p| p.bound.is_some()).collect();
            if !bound.is_empty() {
                // `@Param`이 있으면 그 이름이 첫 조각이다.
                let Some((first, more)) = path.split_first() else {
                    return bound
                        .iter()
                        .map(|p| item(p.bound.as_deref().unwrap_or_default(), &p.ty, 6))
                        .collect();
                };
                let param = bound
                    .iter()
                    .find(|p| p.bound.as_deref() == Some(first.as_str()));
                current = param.and_then(|p| self.class(&file.resolve(&p.ty)));
                rest = more;
            } else if let [param] = method.params.as_slice() {
                current = self.class(&file.resolve(&param.ty));
            }
        }
        for segment in rest {
            let Some((file, decl)) = current else {
                return Vec::new();
            };
            let field = self
                .fields(file, decl)
                .into_iter()
                .find(|(f, _)| &f.name == segment);
            current = field.and_then(|(f, owner)| self.class(&owner.resolve(&f.ty)));
        }
        current
            .map(|(file, decl)| field_items(&self.fields(file, decl)))
            .unwrap_or_default()
    }

    /// 클래스 이름 후보: 프로젝트 클래스(단순 이름이나 FQN이 친 값으로 시작), 별칭, MyBatis 기본 별칭.
    /// 고르면 속성값 전체를 FQN(별칭이면 별칭)으로 바꾼다.
    fn class_items(&self, rope: &Rope, replace: Range<usize>, typed: &str) -> (Vec<Value>, bool) {
        let range = lsp_range(rope, &replace);
        let typed = typed.to_lowercase();
        let simple_typed = typed.rsplit('.').next().unwrap_or(&typed);
        let edit = |label: &str, detail: &str, kind: u8, new_text: &str| {
            json!({
                "label": label,
                "kind": kind,
                "detail": detail,
                "filterText": label,
                "textEdit": { "range": range, "newText": new_text },
            })
        };
        let mut items = Vec::new();
        let mut classes: Vec<&str> = self
            .java
            .values()
            .flat_map(|f| f.types.iter().map(|t| t.fqn.as_str()))
            .filter(|fqn| {
                let lower = fqn.to_lowercase();
                let simple = lower.rsplit('.').next().unwrap_or(&lower);
                simple.starts_with(simple_typed) || lower.starts_with(&typed)
            })
            .collect();
        classes.sort_unstable();
        classes.dedup();
        let incomplete = classes.len() > MAX_CLASS_ITEMS;
        for fqn in classes.into_iter().take(MAX_CLASS_ITEMS) {
            let (package, simple) = fqn.rsplit_once('.').unwrap_or(("", fqn));
            items.push(edit(simple, package, 7, fqn));
        }
        for aliases in self.aliases.values() {
            for (alias, ty) in &aliases.explicit {
                if alias.to_lowercase().starts_with(simple_typed) {
                    items.push(edit(alias, &format!("별칭 → {ty}"), 18, alias));
                }
            }
        }
        for alias in BUILTIN_ALIASES
            .iter()
            .filter(|a| a.starts_with(simple_typed))
        {
            items.push(edit(alias, "MyBatis 기본 별칭", 18, alias));
        }
        (items, incomplete)
    }

    fn resolve_alias(&self, name: &str) -> String {
        let aliases: Vec<&Aliases> = self.aliases.values().collect();
        let classes: Vec<&str> = self
            .java
            .values()
            .flat_map(|f| f.types.iter().map(|t| t.fqn.as_str()))
            .collect();
        mapper::resolve_alias(name, &aliases, &classes)
    }

    fn class(&self, fqn: &str) -> Option<(&JavaFile, &TypeDecl)> {
        self.java
            .values()
            .find_map(|file| file.types.iter().find(|t| t.fqn == fqn).map(|t| (file, t)))
    }

    /// 상속한 필드까지(상위 클래스는 선언한 파일 기준으로 이름을 푼다). 가까운 클래스가 먼저다.
    fn fields<'a>(
        &'a self,
        file: &'a JavaFile,
        decl: &'a TypeDecl,
    ) -> Vec<(&'a java::Field, &'a JavaFile)> {
        let mut found = Vec::new();
        let mut current = Some((file, decl));
        let mut seen = HashSet::new();
        while let Some((file, decl)) = current {
            if !seen.insert(decl.fqn.as_str()) {
                break;
            }
            found.extend(decl.fields.iter().map(|f| (f, file)));
            current = decl
                .superclass
                .as_ref()
                .and_then(|parent| self.class(&file.resolve(parent)));
        }
        found
    }

    /// 프로젝트 폴더의 매퍼와 Java 인터페이스를 모두 읽어 색인한다. 열린 문서는 그대로 둔다.
    fn index_project(&mut self, root: &Path) {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if !SKIP_DIRS.iter().any(|s| entry.file_name() == *s) {
                        stack.push(path);
                    }
                    continue;
                }
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                // 대부분의 파일은 대상이 아니므로 내용을 풀기 전에 바이트로 거른다.
                let wanted: &[&[u8]] = match ext.to_ascii_lowercase().as_str() {
                    "xml" => &[b"<mapper", b"<sqlMap", b"<typeAlias"],
                    // VO 클래스의 필드도 자동완성에 쓰므로 Java 파일은 모두 읽는다.
                    "java" => &[],
                    _ => continue,
                };
                if self.open.contains_key(&path) {
                    continue;
                }
                let Ok(bytes) = text_file::read(&path) else {
                    continue;
                };
                if wanted.is_empty()
                    || wanted
                        .iter()
                        .any(|w| bytes.windows(w.len()).any(|b| b == *w))
                {
                    let text = text_file::decode(&bytes).ok().map(|c| c.text);
                    self.reindex(&path, text.as_deref());
                }
            }
        }
    }
}

fn is_map(ty: &str) -> bool {
    MAP_TYPES.contains(&ty.to_lowercase().as_str())
}

/// 자동완성 항목 하나. `kind`는 LSP CompletionItemKind(5 필드, 6 변수, 7 클래스, 18 참조).
fn item(label: &str, detail: &str, kind: u8) -> Value {
    json!({ "label": label, "kind": kind, "detail": detail })
}

fn field_items(fields: &[(&java::Field, &JavaFile)]) -> Vec<Value> {
    let mut seen = HashSet::new();
    fields
        .iter()
        .filter(|(f, _)| seen.insert(f.name.as_str()))
        .map(|(f, _)| item(&f.name, &f.ty, 5))
        .collect()
}

/// XML 문장이 없는 Mapper 메서드(실행하면 BindingException). 매퍼 XML의 namespace와 이어진
/// 인터페이스만 본다. 이어진 인터페이스가 없는 파일이면 None(진단 대상이 아님).
fn missing_statements(types: &[TypeDecl], index: &Index) -> Option<Vec<Finding>> {
    let mut bound = false;
    let mut findings = Vec::new();
    for interface in types.iter().filter(|t| t.is_interface) {
        let namespace = |m: &mapper::Mapper| {
            m.dialect == Dialect::MyBatis && m.namespace.as_deref() == Some(interface.fqn.as_str())
        };
        if !index.mappers.values().any(namespace) {
            continue;
        }
        bound = true;
        for method in interface.methods.iter().filter(|m| m.needs_statement) {
            if index
                .find(Kind::Statement, namespace, &method.name)
                .is_empty()
            {
                findings.push(Finding {
                    range: method.range.clone(),
                    severity: Severity::Warning,
                    code: "missing-statement",
                    message: format!(
                        "매퍼 XML(namespace {})에 id '{}'인 문장이 없습니다. 실행하면 바인딩 오류(BindingException)가 납니다.",
                        interface.fqn, method.name
                    ),
                });
            }
        }
    }
    bound.then_some(findings)
}

fn lsp_range(rope: &Rope, range: &Range<usize>) -> lsp_types::Range {
    let to = |byte: usize| {
        to_lsp(
            rope,
            rope.byte_to_char(byte.min(rope.len_bytes())),
            ENCODING,
        )
    };
    lsp_types::Range::new(to(range.start), to(range.end))
}

/// 디스크 파일을 읽는다. 예전 egov 프로젝트의 EUC-KR(CP949) 파일도 읽는다.
fn read_text(path: &Path) -> Option<String> {
    let bytes = text_file::read(path).ok()?;
    Some(text_file::decode(&bytes).ok()?.text)
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
