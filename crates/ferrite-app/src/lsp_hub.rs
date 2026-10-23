//! 언어 서버 관리: 언어별로 서버를 띄우고, 열린 문서를 동기화하고, 진단을 문서로 넘긴다.
//! 서버는 (언어, 프로젝트 루트)마다 하나다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use ferrite_core::buffer::Buffer;
use ferrite_core::syntax::{Lang, MAX_SYNTAX_BYTES};
use ferrite_lsp::client::{
    LanguageServer, RpcError, ServerCommand, ServerEvent, find_program, parse_diagnostics,
};
use ferrite_lsp::edits::{FileEdit, parse_workspace_edit};
use ferrite_lsp::position::PositionEncoding;
use lsp_types::Diagnostic;
use serde_json::Value;

/// 입력이 멈춘 뒤 서버에 보내기까지 기다리는 시간(타이핑마다 문서 전체를 보내지 않게).
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(150);

/// 언어별 서버 실행 파일 이름과 인자. PATH에서 찾는다.
// ponytail: PATH만 본다. Phase 2-4 언어 서버 매니저가 내려받아 둔 경로를 먼저 보도록 바꾼다.
fn server_program(lang: Lang) -> Option<(&'static str, &'static [&'static str])> {
    Some(match lang {
        Lang::Rust => ("rust-analyzer", &[]),
        Lang::Go => ("gopls", &[]),
        Lang::Python => ("basedpyright-langserver", &["--stdio"]),
        Lang::C | Lang::Cpp => ("clangd", &[]),
        Lang::Php => ("phpactor", &["language-server"]),
        _ => return None,
    })
}

pub enum ServerState {
    Starting,
    Ready,
    /// 실행 파일을 PATH에서 찾지 못했다.
    NotInstalled,
    Failed(String),
}

struct Server {
    name: &'static str,
    lsp: Option<LanguageServer>,
    state: ServerState,
    /// 서버에 연 문서와 마지막으로 보낸 LSP 버전.
    opened: HashMap<PathBuf, i32>,
}

/// 문서마다 가지는 동기화 상태.
#[derive(Default)]
pub struct DocSync {
    pub problems: Vec<ferrite_lsp::problem::Problem>,
    /// 마지막으로 서버에 보낸 버퍼 버전.
    sent_version: Option<u64>,
    /// 보내지 않은 변경이 처음 생긴 시각(디바운스 기준).
    changed_since: Option<Instant>,
}

impl DocSync {
    /// 버퍼를 통째로 바꿨을 때(디스크에서 다시 불러오기 등) 다음 동기화에서 반드시 보내게 한다.
    pub fn content_replaced(&mut self) {
        self.sent_version = None;
    }
}

/// 서버 이벤트를 처리한 결과. 앱이 문서와 상태바에 반영한다.
pub enum HubUpdate {
    Diagnostics {
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
        encoding: PositionEncoding,
    },
    Message(String),
    /// `request`로 보낸 요청의 결과.
    Response {
        lang: Lang,
        id: i64,
        result: Result<Value, RpcError>,
        encoding: PositionEncoding,
    },
    /// 서버가 편집 적용을 요청했다(명령 실행 결과 등). 적용한 뒤 `reply`로 답해야 한다.
    ApplyEdit {
        lang: Lang,
        id: Value,
        edits: Vec<FileEdit>,
        encoding: PositionEncoding,
    },
}

pub struct LspHub {
    root: Option<PathBuf>,
    servers: HashMap<Lang, Server>,
    ctx: egui::Context,
}

impl LspHub {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            root: None,
            servers: HashMap::new(),
            ctx,
        }
    }

    /// 프로젝트가 바뀌면 서버를 모두 내린다. 다음 동기화 때 새 루트로 다시 띄운다.
    pub fn set_root(&mut self, root: Option<&Path>) {
        if self.root.as_deref() != root {
            self.servers.clear();
            self.root = root.map(Path::to_path_buf);
        }
    }

    /// 상태바 표시용 (서버 이름, 상태). 이 언어에 서버가 없으면 None.
    pub fn status(&self, lang: Lang) -> Option<(&'static str, &ServerState)> {
        self.servers.get(&lang).map(|s| (s.name, &s.state))
    }

    /// 실패한 서버를 내려 다음 동기화 때 다시 띄우게 한다.
    // ponytail: 수동 재시작. Phase 2-6에서 지수 백오프 자동 재시작으로.
    pub fn restart(&mut self, lang: Lang) {
        self.servers.remove(&lang);
    }

    /// 문서를 서버와 맞춘다. 매 프레임 열린 문서마다 부른다.
    pub fn sync(&mut self, path: &Path, lang: Lang, buffer: &Buffer, sync: &mut DocSync) {
        self.sync_inner(path, lang, buffer, sync, false);
    }

    /// 저장을 알린다. 아직 보내지 않은 변경이 있으면 디바운스를 무시하고 먼저 보낸다
    /// (서버가 저장 직전 내용을 기준으로 검사하도록).
    pub fn saved(&mut self, path: &Path, lang: Lang, buffer: &Buffer, sync: &mut DocSync) {
        self.sync_inner(path, lang, buffer, sync, true);
        if let Some(server) = self.servers.get(&lang)
            && let Some(lsp) = &server.lsp
            && server.opened.contains_key(path)
        {
            lsp.did_save(path);
        }
    }

    fn sync_inner(
        &mut self,
        path: &Path,
        lang: Lang,
        buffer: &Buffer,
        sync: &mut DocSync,
        force: bool,
    ) {
        // 대용량 파일은 서버도 감당하지 못하므로 구문 강조와 같은 기준으로 끈다.
        if buffer.rope().len_bytes() > MAX_SYNTAX_BYTES {
            return;
        }
        let root = self
            .root
            .clone()
            .or_else(|| path.parent().map(Path::to_path_buf));
        let ctx = &self.ctx;
        let Some(server) = self
            .servers
            .entry(lang)
            .or_insert_with(|| start_server(lang, root.as_deref(), ctx))
            .lsp_server()
        else {
            return;
        };
        let (lsp, opened) = server;
        let version = buffer.version();
        match opened.get_mut(path) {
            None => {
                lsp.did_open(path, lang.lsp_id(), 1, &buffer.text());
                opened.insert(path.to_path_buf(), 1);
                sync.sent_version = Some(version);
                sync.changed_since = None;
            }
            Some(lsp_version) if sync.sent_version != Some(version) => {
                let since = *sync.changed_since.get_or_insert_with(Instant::now);
                if force || since.elapsed() >= CHANGE_DEBOUNCE {
                    *lsp_version += 1;
                    lsp.did_change_full(path, *lsp_version, &buffer.text());
                    sync.sent_version = Some(version);
                    sync.changed_since = None;
                } else {
                    self.ctx.request_repaint_after(CHANGE_DEBOUNCE);
                }
            }
            Some(_) => {}
        }
    }

    /// 문서 위치 기반 기능 요청을 보낸다. 서버가 준비되지 않았으면 None.
    /// 요청 전에 보내지 않은 변경을 먼저 보내 서버가 지금 내용을 기준으로 답하게 한다.
    /// `params`는 서버가 고른 위치 인코딩을 받아 매개변수를 만든다.
    pub fn request(
        &mut self,
        path: &Path,
        lang: Lang,
        buffer: &Buffer,
        sync: &mut DocSync,
        method: &str,
        params: impl FnOnce(PositionEncoding) -> Option<Value>,
    ) -> Option<i64> {
        self.sync_inner(path, lang, buffer, sync, true);
        let server = self.servers.get_mut(&lang)?;
        let lsp = server.lsp.as_mut().filter(|l| l.is_ready())?;
        if !server.opened.contains_key(path) {
            return None;
        }
        let params = params(lsp.encoding())?;
        Some(lsp.request(method, params))
    }

    /// 입력하면 자동완성을 여는 문자(`.`, `::`의 `:` 등). 서버가 알려 준 것.
    pub fn trigger_characters(&self, lang: Lang) -> Vec<char> {
        let capabilities = self
            .servers
            .get(&lang)
            .and_then(|s| s.lsp.as_ref())
            .and_then(LanguageServer::capabilities);
        capabilities
            .and_then(|c| c.completion_provider)
            .and_then(|p| p.trigger_characters)
            .into_iter()
            .flatten()
            .filter_map(|s| s.chars().next())
            .collect()
    }

    /// 서버에 마지막으로 보낸 문서 버전(열지 않은 문서면 None). 서버가 편집에 붙인 버전과 비교한다.
    pub fn document_version(&self, lang: Lang, path: &Path) -> Option<i32> {
        self.servers.get(&lang)?.opened.get(path).copied()
    }

    /// `HubUpdate::ApplyEdit`에 답한다.
    pub fn reply(&self, lang: Lang, id: Value, result: Value) {
        if let Some(lsp) = self.servers.get(&lang).and_then(|s| s.lsp.as_ref()) {
            lsp.reply(id, result);
        }
    }

    pub fn closed(&mut self, path: &Path, lang: Lang) {
        if let Some(server) = self.servers.get_mut(&lang)
            && server.opened.remove(path).is_some()
            && let Some(lsp) = &server.lsp
        {
            lsp.did_close(path);
        }
    }

    /// 쌓인 서버 이벤트를 처리한다.
    pub fn process_events(&mut self) -> Vec<HubUpdate> {
        let mut updates = Vec::new();
        for (&lang, server) in &mut self.servers {
            let Some(lsp) = &server.lsp else {
                continue;
            };
            let encoding = lsp.encoding();
            for event in lsp.poll_events() {
                match event {
                    ServerEvent::Ready => server.state = ServerState::Ready,
                    ServerEvent::Exited(reason) => {
                        let first = reason.lines().last().unwrap_or("").to_owned();
                        updates.push(HubUpdate::Message(format!("{} 종료: {first}", server.name)));
                        server.state = ServerState::Failed(reason);
                    }
                    ServerEvent::Notification { method, params } => {
                        if let Some(update) = notification(&method, &params, encoding) {
                            updates.push(update);
                        }
                    }
                    ServerEvent::Response { id, result } => updates.push(HubUpdate::Response {
                        lang,
                        id,
                        result,
                        encoding,
                    }),
                    ServerEvent::Request { id, params, .. } => updates.push(HubUpdate::ApplyEdit {
                        lang,
                        id,
                        edits: params
                            .get("edit")
                            .map(parse_workspace_edit)
                            .unwrap_or_default(),
                        encoding,
                    }),
                }
            }
            if matches!(server.state, ServerState::Failed(_)) {
                // 프로세스는 이미 끝났다. 다시 시작하면 문서를 처음부터 다시 연다.
                server.lsp = None;
                server.opened.clear();
            }
        }
        updates
    }
}

impl Server {
    fn lsp_server(&mut self) -> Option<(&LanguageServer, &mut HashMap<PathBuf, i32>)> {
        Some((self.lsp.as_ref()?, &mut self.opened))
    }
}

fn start_server(lang: Lang, root: Option<&Path>, ctx: &egui::Context) -> Server {
    let (name, args) = server_program(lang).unwrap_or(("", &[]));
    let mut server = Server {
        name,
        lsp: None,
        state: ServerState::NotInstalled,
        opened: HashMap::new(),
    };
    let (Some(program), Some(root)) = (find_program(name).filter(|_| !name.is_empty()), root)
    else {
        return server;
    };
    let command = ServerCommand {
        program,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
    };
    let ctx = ctx.clone();
    match LanguageServer::start(&command, root, move || ctx.request_repaint()) {
        Ok(lsp) => {
            server.lsp = Some(lsp);
            server.state = ServerState::Starting;
        }
        Err(e) => server.state = ServerState::Failed(e),
    }
    server
}

fn notification(method: &str, params: &Value, encoding: PositionEncoding) -> Option<HubUpdate> {
    match method {
        "textDocument/publishDiagnostics" => {
            let (path, diagnostics) = parse_diagnostics(params)?;
            Some(HubUpdate::Diagnostics {
                path,
                diagnostics,
                encoding,
            })
        }
        // 오류 수준(type 1) 메시지만 상태바에 보여 준다.
        "window/showMessage" if params.get("type").and_then(Value::as_i64) == Some(1) => {
            let message = params.get("message").and_then(Value::as_str)?;
            Some(HubUpdate::Message(message.to_owned()))
        }
        _ => None,
    }
}
