//! 언어 서버 관리: 서버를 띄우고, 열린 문서를 동기화하고, 진단을 문서로 넘긴다.
//! 한 언어에 서버가 여럿일 수 있다(Java = jdt.ls + Spring Boot LS). 문서는 모두에게 보내고 진단은
//! 서버별로 모은다. 기능 요청은 주 서버(목록에서 먼저 오는 것)에만 보낸다.
//! 서버는 (서버, 프로젝트 루트)마다 하나다. Spring Boot LS와 jdt.ls 사이의 메시지는 여기서 중계한다.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use ferrite_core::buffer::Buffer;
use ferrite_core::syntax::{Lang, MAX_SYNTAX_BYTES};
use ferrite_lsp::catalog::{self, Launch, Server as ServerSpec, Source};
use ferrite_lsp::client::{
    DIAGNOSTIC_REFRESH, LanguageServer, RpcError, ServerEvent, parse_diagnostics,
    parse_pulled_diagnostics,
};
use ferrite_lsp::edits::{FileEdit, parse_workspace_edit};
use ferrite_lsp::install;
use ferrite_lsp::position::PositionEncoding;
use ferrite_lsp::problem::{Problem, to_problems};
use ferrite_lsp::spring;
use lsp_types::{CodeActionProviderCapability, Diagnostic, HoverProviderCapability, OneOf};
use ropey::Rope;
use serde_json::{Value, json};

/// 입력이 멈춘 뒤 서버에 보내기까지 기다리는 시간(타이핑마다 문서 전체를 보내지 않게).
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(150);
/// 실행 중이던 서버가 죽으면 다시 띄우기까지 기다리는 시간. 연달아 죽을수록 늘리고, 다 쓰면
/// 자동 재시작을 멈춘다(상태바에서 눌러 다시 시작).
const RESTART_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];
/// 이만큼 돌다가 죽었으면 연달아 죽은 것으로 치지 않는다(횟수를 처음부터 센다).
const STABLE_UPTIME: Duration = Duration::from_secs(60);

/// 서버가 죽었을 때: (연달아 죽은 횟수, 다시 띄우기까지 기다릴 시간). 너무 자주 죽으면 None.
fn restart_after(previous_crashes: usize, uptime: Duration) -> (usize, Option<Duration>) {
    let crashes = if uptime >= STABLE_UPTIME {
        1
    } else {
        previous_crashes + 1
    };
    (crashes, RESTART_DELAYS.get(crashes - 1).copied())
}

/// 여러 서버에 보내 결과를 합치는 요청. 서버마다 아는 것이 달라 모두에게 묻는다
/// (jdt.ls는 Java, lemminx는 XML 구조, egov 서버는 매퍼 XML ↔ Java).
const FAN_OUT: [&str; 2] = ["textDocument/definition", "textDocument/completion"];

/// 여러 서버에 보낸 요청 하나. 모두 답하면 합쳐 주 서버의 응답으로 앱에 돌려준다.
struct FanOut {
    method: &'static str,
    lang: Lang,
    /// 주 서버와 그 요청 ID(앱이 기다리는 ID).
    primary: (&'static str, i64),
    encoding: PositionEncoding,
    /// 아직 답하지 않은 (서버, 요청 ID).
    waiting: HashSet<(&'static str, i64)>,
    results: Vec<Value>,
    error: Option<RpcError>,
}

/// 이 서버가 이 요청을 처리한다고 알렸는가.
/// 진단을 묻는 요청(LSP 3.17).
const PULL_DIAGNOSTICS: &str = "textDocument/diagnostic";
const DOCUMENT_SYMBOL: &str = "textDocument/documentSymbol";
const WORKSPACE_SYMBOL: &str = "workspace/symbol";

fn supports(lsp: &LanguageServer, method: &str) -> bool {
    let Some(c) = lsp.capabilities() else {
        return false;
    };
    fn on<T>(provider: Option<OneOf<bool, T>>) -> bool {
        matches!(provider, Some(OneOf::Left(true) | OneOf::Right(_)))
    }
    match method {
        PULL_DIAGNOSTICS => c.diagnostic_provider.is_some(),
        "textDocument/completion" => c.completion_provider.is_some(),
        "textDocument/hover" => c
            .hover_provider
            .is_some_and(|h| !matches!(h, HoverProviderCapability::Simple(false))),
        "textDocument/definition" => on(c.definition_provider),
        DOCUMENT_SYMBOL => on(c.document_symbol_provider),
        WORKSPACE_SYMBOL => on(c.workspace_symbol_provider),
        "textDocument/references" => on(c.references_provider),
        "textDocument/formatting" => on(c.document_formatting_provider),
        "textDocument/rename" => on(c.rename_provider),
        "textDocument/codeAction" | "codeAction/resolve" => c
            .code_action_provider
            .is_some_and(|p| !matches!(p, CodeActionProviderCapability::Simple(false))),
        "workspace/executeCommand" => c.execute_command_provider.is_some(),
        _ => true,
    }
}

/// 위치 응답들(Location, Location[], LocationLink[], null)을 Location 배열 하나로 합친다.
/// LocationLink는 대상의 선택 범위를 쓴다. 같은 위치는 한 번만 둔다.
fn merge_locations(results: &[Value]) -> Value {
    let mut merged: Vec<Value> = Vec::new();
    let items = results.iter().flat_map(|r| match r {
        Value::Array(items) => items.clone(),
        Value::Null => Vec::new(),
        single => vec![single.clone()],
    });
    for item in items {
        let location = match item.get("targetUri") {
            Some(uri) => json!({ "uri": uri, "range": item["targetSelectionRange"] }),
            None => item,
        };
        if !merged.contains(&location) {
            merged.push(location);
        }
    }
    Value::Array(merged)
}

/// 자동완성 응답들(CompletionList, CompletionItem[], null)을 CompletionList 하나로 합친다.
/// 하나라도 미완성이면 미완성이다(입력이 바뀌면 다시 물어야 한다).
fn merge_completions(results: &[Value]) -> Value {
    let mut items = Vec::new();
    let mut incomplete = false;
    for result in results {
        match result {
            Value::Array(list) => items.extend(list.iter().cloned()),
            Value::Object(list) => {
                incomplete |= list.get("isIncomplete").and_then(Value::as_bool) == Some(true);
                if let Some(Value::Array(list)) = list.get("items") {
                    items.extend(list.iter().cloned());
                }
            }
            _ => {}
        }
    }
    json!({ "isIncomplete": incomplete, "items": items })
}

/// 서버가 연달아 죽은 횟수와 다시 띄울 시각.
#[derive(Default)]
struct Crashes {
    count: usize,
    retry_at: Option<Instant>,
}

/// 문서의 LSP 언어 ID. Spring Boot 설정 파일(`application*`, `bootstrap*`)은 Spring 전용 ID라
/// Spring Boot LS가 맡는다. 나머지는 언어 그대로.
pub fn language_id(lang: Lang, path: &Path) -> &'static str {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let spring_config = name.starts_with("application") || name.starts_with("bootstrap");
    match lang {
        Lang::Yaml if spring_config => "spring-boot-properties-yaml",
        Lang::Properties if spring_config => "spring-boot-properties",
        _ => lang.lsp_id(),
    }
}

pub enum ServerState {
    Starting,
    Ready,
    /// Ferrite가 설치한 것도, PATH에 있는 것도 없다.
    NotInstalled,
    /// 실행 중에 죽어서 곧 다시 띄운다(종료 사유).
    Restarting(String),
    Failed(String),
}

struct Server {
    spec: &'static ServerSpec,
    lsp: Option<LanguageServer>,
    state: ServerState,
    started: Instant,
    /// 서버에 연 문서와 마지막으로 보낸 LSP 버전.
    opened: HashMap<PathBuf, i32>,
    /// 진단을 물은(pull) 문서와 그때의 LSP 버전. 버전이 바뀌거나 서버가 다시 물으라고 하면 또 묻는다.
    pulled: HashMap<PathBuf, i32>,
}

/// 문서마다 가지는 동기화 상태.
#[derive(Default)]
pub struct DocSync {
    /// 모든 서버의 진단을 합쳐 위치순으로 정렬한 것.
    pub problems: Vec<Problem>,
    /// 서버별 진단(서버, 물어서 받은 것인지). 한 서버가 새로 보내면 그 몫만 바꾼다. 한 서버가 보내 주는
    /// 진단과 물어서 받는 진단을 섞어 쓸 수 있어 따로 둔다.
    by_server: Vec<((&'static str, bool), Vec<Problem>)>,
    /// 마지막으로 서버들에 보낸 버퍼 버전.
    sent_version: Option<u64>,
    /// 보내지 않은 변경이 처음 생긴 시각(디바운스 기준).
    changed_since: Option<Instant>,
}

impl DocSync {
    /// 버퍼를 통째로 바꿨을 때(디스크에서 다시 불러오기 등) 다음 동기화에서 반드시 보내게 한다.
    pub fn content_replaced(&mut self) {
        self.sent_version = None;
    }

    /// 한 서버의 진단을 바꾸고 전체 목록을 다시 만든다.
    pub fn set_problems(
        &mut self,
        server: &'static str,
        pulled: bool,
        rope: &Rope,
        diagnostics: &[Diagnostic],
        encoding: PositionEncoding,
    ) {
        let problems = to_problems(rope, diagnostics, encoding);
        let key = (server, pulled);
        match self.by_server.iter_mut().find(|(k, _)| *k == key) {
            Some((_, existing)) => *existing = problems,
            None => self.by_server.push((key, problems)),
        }
        self.problems = self
            .by_server
            .iter()
            .flat_map(|(_, p)| p.iter().cloned())
            .collect();
        self.problems.sort_by_key(|p| p.range.start);
    }
}

/// 서버 이벤트를 처리한 결과. 앱이 문서와 상태바에 반영한다.
pub enum HubUpdate {
    Diagnostics {
        path: PathBuf,
        server: &'static str,
        /// 물어서(`textDocument/diagnostic`) 받은 진단이다.
        pulled: bool,
        diagnostics: Vec<Diagnostic>,
        encoding: PositionEncoding,
    },
    Message(String),
    /// `request`로 보낸 요청의 결과. `server`는 답한 서버(편집의 문서 버전을 맞춰 볼 때 쓴다).
    Response {
        lang: Lang,
        server: &'static str,
        id: i64,
        result: Result<Value, RpcError>,
        encoding: PositionEncoding,
    },
    /// 서버가 편집 적용을 요청했다(명령 실행 결과 등). 적용한 뒤 `reply`로 답해야 한다.
    ApplyEdit {
        server: &'static str,
        id: Value,
        edits: Vec<FileEdit>,
        encoding: PositionEncoding,
    },
    /// 문서의 기호(`outline`으로 물은 것).
    Outline {
        path: PathBuf,
        result: Value,
        encoding: PositionEncoding,
    },
    /// 프로젝트 기호(`workspace_symbols`로 물은 것). 서버마다 따로 온다. 실패한 서버는 null이다.
    WorkspaceSymbols {
        generation: u64,
        server: &'static str,
        result: Value,
        encoding: PositionEncoding,
    },
}

/// 다른 서버로 넘긴 요청의 응답을 돌려줄 곳.
struct Relay {
    to: &'static str,
    id: Value,
}

pub struct LspHub {
    root: Option<PathBuf>,
    /// 언어 서버 매니저가 서버를 설치하는 폴더.
    install_root: PathBuf,
    /// 서버 이름 → 서버.
    servers: HashMap<&'static str, Server>,
    /// 앱이 보낸 요청: (서버, 요청 ID) → 요청한 문서의 언어.
    requests: HashMap<(&'static str, i64), Lang>,
    /// 중계 중인 요청: (보낸 서버, 요청 ID) → 응답을 돌려줄 곳.
    relays: HashMap<(&'static str, i64), Relay>,
    /// 허브가 스스로 보낸 요청(응답은 버린다).
    internal: HashSet<(&'static str, i64)>,
    /// 진단을 물은 요청: (서버, 요청 ID) → 문서.
    pulls: HashMap<(&'static str, i64), PathBuf>,
    /// 개요를 물은 요청: (서버, 요청 ID) → 문서.
    outlines: HashMap<(&'static str, i64), PathBuf>,
    /// 프로젝트 기호를 물은 요청: (서버, 요청 ID) → 물은 차례.
    symbol_queries: HashMap<(&'static str, i64), u64>,
    symbol_generation: u64,
    /// jdt.ls 확장이 Spring 프로젝트를 찾아 Spring Boot LS를 띄우라고 알렸다.
    spring_requested: bool,
    /// 실행 중에 죽은 서버(자동 재시작 대기).
    crashes: HashMap<&'static str, Crashes>,
    /// 여러 서버에 보내고 답을 기다리는 요청.
    fan_outs: Vec<FanOut>,
    ctx: egui::Context,
}

impl LspHub {
    pub fn new(ctx: egui::Context, install_root: PathBuf) -> Self {
        Self {
            root: None,
            install_root,
            servers: HashMap::new(),
            requests: HashMap::new(),
            relays: HashMap::new(),
            internal: HashSet::new(),
            pulls: HashMap::new(),
            outlines: HashMap::new(),
            symbol_queries: HashMap::new(),
            symbol_generation: 0,
            spring_requested: false,
            crashes: HashMap::new(),
            fan_outs: Vec::new(),
            ctx,
        }
    }

    /// 프로젝트가 바뀌면 서버를 모두 내린다. 다음 동기화 때 새 루트로 다시 띄운다.
    pub fn set_root(&mut self, root: Option<&Path>) {
        if self.root.as_deref() != root {
            self.servers.clear();
            self.requests.clear();
            self.relays.clear();
            self.internal.clear();
            self.pulls.clear();
            self.outlines.clear();
            self.symbol_queries.clear();
            self.spring_requested = false;
            self.crashes.clear();
            self.fan_outs.clear();
            self.root = root.map(Path::to_path_buf);
        }
    }

    /// 상태바 표시용 (주 서버, 상태). 이 문서를 맡는 서버가 없거나 아직 띄우지 않았으면 None.
    pub fn status(&self, path: &Path, lang: Lang) -> Option<(&'static ServerSpec, &ServerState)> {
        let spec = catalog::for_language(language_id(lang, path))?;
        self.servers.get(spec.name).map(|s| (s.spec, &s.state))
    }

    /// 이 문서의 주 서버를 내려 다음 동기화 때 다시 띄우게 한다(상태바에서 누른 수동 재시작).
    pub fn restart(&mut self, path: &Path, lang: Lang) {
        if let Some(spec) = catalog::for_language(language_id(lang, path)) {
            self.stop(spec.name);
        }
    }

    /// 이 서버를 내린다(설치·삭제 뒤 새 실행 파일로 다시 띄우도록). 연달아 죽은 횟수도 지운다.
    pub fn stop(&mut self, name: &str) {
        self.crashes.retain(|n, _| *n != name);
        self.remove(name);
    }

    /// 서버를 목록에서 뺀다. 다음 동기화 때 다시 띄운다. jdt.ls와 Spring Boot LS는 함께 뺀다:
    /// jdt.ls가 바뀌면 Spring의 클래스패스 연동이 끊기고, Spring Boot LS를 새로 설치·삭제하면
    /// jdt.ls가 실을 확장 번들이 달라진다.
    fn remove(&mut self, name: &str) {
        let paired = [Launch::Jdtls, Launch::SpringBoot]
            .iter()
            .filter_map(|l| catalog::by_launch(*l))
            .any(|s| s.name == name);
        let gone = |spec: &ServerSpec| {
            spec.name == name
                || (paired && matches!(spec.launch, Launch::Jdtls | Launch::SpringBoot))
        };
        self.servers.retain(|_, s| !gone(s.spec));
        let alive = |key: &(&'static str, i64)| {
            catalog::SERVERS.iter().any(|s| s.name == key.0 && !gone(s))
        };
        self.requests.retain(|k, _| alive(k));
        self.relays.retain(|k, _| alive(k));
        self.internal.retain(alive);
        self.pulls.retain(|k, _| alive(k));
        self.outlines.retain(|k, _| alive(k));
        self.symbol_queries.retain(|k, _| alive(k));
        // 빠진 서버에 보낸 것이 있는 묶음은 답이 다 올 수 없어 버린다(서버가 죽으면 요청도 사라진다).
        self.fan_outs.retain(|g| g.waiting.iter().all(alive));
    }

    /// 문서를 서버들과 맞춘다. 매 프레임 열린 문서마다 부른다.
    pub fn sync(&mut self, path: &Path, lang: Lang, buffer: &Buffer, sync: &mut DocSync) {
        self.sync_inner(path, lang, buffer, sync, false);
    }

    /// 저장을 알린다. 아직 보내지 않은 변경이 있으면 디바운스를 무시하고 먼저 보낸다
    /// (서버가 저장 직전 내용을 기준으로 검사하도록).
    pub fn saved(&mut self, path: &Path, lang: Lang, buffer: &Buffer, sync: &mut DocSync) {
        self.sync_inner(path, lang, buffer, sync, true);
        for spec in catalog::servers_for(language_id(lang, path)) {
            if let Some(server) = self.servers.get(spec.name)
                && let Some(lsp) = &server.lsp
                && server.opened.contains_key(path)
            {
                lsp.did_save(path);
            }
        }
    }

    /// 서버가 없으면 띄운다. Spring Boot LS는 jdt.ls와 중계해야 하므로 jdt.ls도 함께 띄운다.
    fn ensure_started(&mut self, spec: &'static ServerSpec, path: &Path) {
        if spec.launch == Launch::SpringBoot
            && let Some(jdtls) = catalog::by_launch(Launch::Jdtls)
        {
            self.ensure_started(jdtls, path);
        }
        if self.servers.contains_key(spec.name) {
            return;
        }
        let root = self
            .root
            .clone()
            .or_else(|| path.parent().map(Path::to_path_buf));
        let server = start_server(spec, &self.install_root, root.as_deref(), &self.ctx);
        self.servers.insert(spec.name, server);
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
        let language = language_id(lang, path);
        let specs: Vec<_> = catalog::servers_for(language).collect();
        if specs.is_empty() {
            return;
        }
        let version = buffer.version();
        // 변경은 모든 서버에 같은 때 보낸다(디바운스도 문서 단위).
        let changed = sync.sent_version != Some(version);
        let due = changed && {
            let since = *sync.changed_since.get_or_insert_with(Instant::now);
            force || since.elapsed() >= CHANGE_DEBOUNCE
        };
        let mut text = None;
        // 이번에 연 서버는 지금 내용을 받았으니 최신이다. 이미 열려 있던 서버가 변경을 못 받았으면 아니다.
        let mut up_to_date = true;
        for spec in specs {
            // Spring Boot LS는 무거워(JVM 힙 1GB) Java 파일만으로는 띄우지 않는다. jdt.ls 확장이 Spring
            // 프로젝트를 찾아 알려 주거나 Spring 설정 파일을 열었을 때 띄운다(VS Code와 같게).
            if spec.launch == Launch::SpringBoot
                && language == "java"
                && !self.spring_requested
                && !self.servers.contains_key(spec.name)
            {
                continue;
            }
            self.ensure_started(spec, path);
            let Some(server) = self.servers.get_mut(spec.name) else {
                continue;
            };
            let Some(lsp) = server.lsp.as_mut() else {
                continue;
            };
            let text = text.get_or_insert_with(|| buffer.text());
            match server.opened.get_mut(path) {
                None => {
                    lsp.did_open(path, language, 1, text);
                    server.opened.insert(path.to_path_buf(), 1);
                }
                Some(lsp_version) if due => {
                    *lsp_version += 1;
                    lsp.did_change_full(path, *lsp_version, text);
                }
                Some(_) => up_to_date &= !changed,
            }
            // 진단을 묻는 서버(TypeScript 7 등): 보낸 판마다 한 번 묻는다. 준비되기 전에 연 문서는
            // 준비된 뒤에 묻는다.
            let sent = server.opened.get(path).copied();
            if lsp.is_ready()
                && supports(lsp, PULL_DIAGNOSTICS)
                && server.pulled.get(path).copied() != sent
                && let Ok(uri) = ferrite_lsp::uri::path_to_uri(path)
            {
                let id = lsp.request(PULL_DIAGNOSTICS, json!({ "textDocument": { "uri": uri } }));
                self.pulls.insert((spec.name, id), path.to_path_buf());
                if let Some(sent) = sent {
                    server.pulled.insert(path.to_path_buf(), sent);
                }
            }
        }
        if due || up_to_date {
            sync.sent_version = Some(version);
            sync.changed_since = None;
        } else {
            self.ctx.request_repaint_after(CHANGE_DEBOUNCE);
        }
    }

    /// 문서의 기호(개요)를 이 문서를 맡은 서버 중 처음으로 기호를 주는 준비된 서버에 묻는다. 답은
    /// `HubUpdate::Outline`으로 온다. 물을 서버가 없으면 false.
    pub fn outline(
        &mut self,
        path: &Path,
        lang: Lang,
        buffer: &Buffer,
        sync: &mut DocSync,
    ) -> bool {
        self.sync_inner(path, lang, buffer, sync, true);
        let Ok(uri) = ferrite_lsp::uri::path_to_uri(path) else {
            return false;
        };
        let name = catalog::servers_for(language_id(lang, path))
            .map(|spec| spec.name)
            .find(|name| {
                self.servers.get(name).is_some_and(|s| {
                    s.opened.contains_key(path)
                        && s.lsp
                            .as_ref()
                            .is_some_and(|l| l.is_ready() && supports(l, DOCUMENT_SYMBOL))
                })
            });
        let Some(lsp) = name.and_then(|n| self.servers.get_mut(n)?.lsp.as_mut()) else {
            return false;
        };
        let id = lsp.request(DOCUMENT_SYMBOL, json!({ "textDocument": { "uri": uri } }));
        if let Some(name) = name {
            self.outlines.insert((name, id), path.to_path_buf());
        }
        true
    }

    /// 떠 있는 준비된 서버 모두에 프로젝트 기호를 묻는다. (이 물음의 차례, 물은 서버 수). 답은
    /// `HubUpdate::WorkspaceSymbols`로 서버마다 온다.
    pub fn workspace_symbols(&mut self, query: &str) -> (u64, usize) {
        self.symbol_generation += 1;
        let generation = self.symbol_generation;
        let mut asked = 0;
        for (name, server) in &mut self.servers {
            if let Some(lsp) = server
                .lsp
                .as_mut()
                .filter(|l| l.is_ready() && supports(l, WORKSPACE_SYMBOL))
            {
                let id = lsp.request(WORKSPACE_SYMBOL, json!({ "query": query }));
                self.symbol_queries.insert((*name, id), generation);
                asked += 1;
            }
        }
        (generation, asked)
    }

    /// Ferrite에 든 서버(egov)를 띄운다. 엔드포인트 창은 Java 파일을 열지 않아도 엔드포인트를 보인다.
    pub fn ensure_builtin(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        for spec in catalog::SERVERS
            .iter()
            .filter(|s| matches!(s.source, Source::Builtin(_)))
        {
            self.ensure_started(spec, &root);
        }
    }

    /// 문서 위치 기반 기능 요청을 주 서버에 보낸다. 서버가 준비되지 않았으면 None.
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
        // 이 요청을 처리한다고 알린 준비된 서버들(목록 순서). 첫 서버가 주 서버다.
        let ready: Vec<&'static str> = catalog::servers_for(language_id(lang, path))
            .map(|spec| spec.name)
            .filter(|name| {
                self.servers.get(name).is_some_and(|s| {
                    s.opened.contains_key(path)
                        && s.lsp
                            .as_ref()
                            .is_some_and(|l| l.is_ready() && supports(l, method))
                })
            })
            .collect();
        let (&primary, others) = ready.split_first()?;
        let encoding = self.servers[primary].lsp.as_ref()?.encoding();
        let params = params(encoding)?;
        // 위치 단위가 같은 서버에만 함께 묻는다(합친 결과를 한 단위로 해석하므로).
        let fan_out = FAN_OUT.iter().copied().find(|m| *m == method);
        let others: Vec<&'static str> = if fan_out.is_some() {
            others
                .iter()
                .copied()
                .filter(|n| {
                    self.servers[n]
                        .lsp
                        .as_ref()
                        .is_some_and(|l| l.encoding() == encoding)
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut send = |name: &'static str| {
            let lsp = self.servers.get_mut(name)?.lsp.as_mut()?;
            Some((name, lsp.request(method, params.clone())))
        };
        let first = send(primary)?;
        if others.is_empty() {
            self.requests.insert(first, lang);
        } else {
            let mut waiting: HashSet<_> = others.into_iter().filter_map(&mut send).collect();
            waiting.insert(first);
            self.fan_outs.push(FanOut {
                method: fan_out?,
                lang,
                primary: first,
                encoding,
                waiting,
                results: Vec::new(),
                error: None,
            });
        }
        Some(first.1)
    }

    /// 이름으로 고른 서버(jdt.ls 등)에 `workspace/executeCommand`를 보낸다. 서버가 준비되지 않았으면 None.
    /// 응답은 `lang`의 `HubUpdate::Response`로 온다.
    pub fn execute_command(
        &mut self,
        server: &str,
        lang: Lang,
        command: &str,
        arguments: Vec<Value>,
    ) -> Option<i64> {
        let (&name, server) = self.servers.iter_mut().find(|(n, _)| **n == server)?;
        let lsp = server.lsp.as_mut().filter(|l| l.is_ready())?;
        let id = lsp.request(
            "workspace/executeCommand",
            json!({ "command": command, "arguments": arguments }),
        );
        self.requests.insert((name, id), lang);
        Some(id)
    }

    /// 입력하면 자동완성을 여는 문자(`.`, `::`의 `:` 등). 자동완성을 맡는 서버들이 알려 준 것을 합친다
    /// (요청도 그 서버들에 함께 간다).
    pub fn trigger_characters(&self, path: &Path, lang: Lang) -> Vec<char> {
        let mut found: Vec<char> = catalog::servers_for(language_id(lang, path))
            .filter_map(|spec| self.servers.get(spec.name)?.lsp.as_ref())
            .filter(|l| l.is_ready() && supports(l, "textDocument/completion"))
            .filter_map(LanguageServer::capabilities)
            .filter_map(|c| c.completion_provider?.trigger_characters)
            .flatten()
            .filter_map(|s| s.chars().next())
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// 이 서버에 마지막으로 보낸 문서 버전(열지 않은 문서면 None). 서버가 편집에 붙인 버전과 비교한다.
    /// 이 서버 프로세스가 떠 있는가(테스트에서 띄우는 조건을 확인할 때).
    #[cfg(test)]
    pub fn is_running(&self, server: &str) -> bool {
        self.servers.get(server).is_some_and(|s| s.lsp.is_some())
    }

    /// 이 서버가 연달아 죽은 횟수(테스트용).
    #[cfg(test)]
    pub fn crash_count(&self, server: &str) -> usize {
        self.crashes.get(server).map_or(0, |c| c.count)
    }

    pub fn document_version(&self, server: &str, path: &Path) -> Option<i32> {
        self.servers.get(server)?.opened.get(path).copied()
    }

    /// `HubUpdate::ApplyEdit`에 답한다.
    pub fn reply(&self, server: &str, id: Value, result: Value) {
        if let Some(lsp) = self.servers.get(server).and_then(|s| s.lsp.as_ref()) {
            lsp.reply(id, result);
        }
    }

    pub fn closed(&mut self, path: &Path, lang: Lang) {
        for spec in catalog::servers_for(language_id(lang, path)) {
            if let Some(server) = self.servers.get_mut(spec.name)
                && server.opened.remove(path).is_some()
                && let Some(lsp) = &server.lsp
            {
                lsp.did_close(path);
            }
        }
    }

    /// 쌓인 서버 이벤트를 처리한다. 중계할 것은 다른 서버로 넘기고, 나머지는 앱에 돌려준다.
    pub fn process_events(&mut self) -> Vec<HubUpdate> {
        // 기다린 시간이 지난 서버는 목록에서 빼 다음 동기화 때 다시 띄운다(연달아 죽은 횟수는 둔다).
        let now = Instant::now();
        let due: Vec<_> = self
            .crashes
            .iter_mut()
            .filter(|(_, c)| c.retry_at.is_some_and(|at| at <= now))
            .map(|(&name, c)| {
                c.retry_at = None;
                name
            })
            .collect();
        for name in due {
            self.remove(name);
        }
        // 이벤트를 먼저 모은다: 중계하려면 다른 서버를 빌려 써야 해서.
        let mut batches = Vec::new();
        for (&name, server) in &mut self.servers {
            let Some(lsp) = &server.lsp else {
                continue;
            };
            let events = lsp.poll_events();
            if !events.is_empty() {
                batches.push((name, lsp.encoding(), events));
            }
        }
        let mut updates = Vec::new();
        for (name, encoding, events) in batches {
            for event in events {
                self.handle_event(name, encoding, event, &mut updates);
            }
            if let Some(server) = self.servers.get_mut(name)
                && matches!(
                    server.state,
                    ServerState::Failed(_) | ServerState::Restarting(_)
                )
            {
                // 프로세스는 이미 끝났다. 다시 시작하면 문서를 처음부터 다시 연다.
                server.lsp = None;
                server.opened.clear();
            }
        }
        updates
    }

    fn handle_event(
        &mut self,
        name: &'static str,
        encoding: PositionEncoding,
        event: ServerEvent,
        updates: &mut Vec<HubUpdate>,
    ) {
        match event {
            ServerEvent::Ready => {
                let Some(server) = self.servers.get_mut(name) else {
                    return;
                };
                server.state = ServerState::Ready;
                if server.spec.launch == Launch::SpringBoot {
                    self.start_spring(name);
                }
            }
            ServerEvent::Exited(reason) => {
                let Some(server) = self.servers.get_mut(name) else {
                    return;
                };
                let last = reason.lines().last().unwrap_or("").to_owned();
                let crashes = self.crashes.entry(name).or_default();
                let (count, delay) = restart_after(crashes.count, server.started.elapsed());
                crashes.count = count;
                crashes.retry_at = delay.map(|d| Instant::now() + d);
                let message = match delay {
                    Some(delay) => {
                        self.ctx.request_repaint_after(delay);
                        server.state = ServerState::Restarting(reason);
                        format!("{name} 종료: {last} ({}초 뒤 다시 시작)", delay.as_secs())
                    }
                    None => {
                        server.state = ServerState::Failed(reason);
                        format!("{name}이(가) 연달아 종료되어 자동 재시작을 멈췄습니다: {last}")
                    }
                };
                updates.push(HubUpdate::Message(message));
            }
            ServerEvent::Notification { method, .. } if method == DIAGNOSTIC_REFRESH => {
                // 다음 동기화 때 열린 문서마다 다시 묻는다.
                if let Some(server) = self.servers.get_mut(name) {
                    server.pulled.clear();
                    self.ctx.request_repaint();
                }
            }
            ServerEvent::Notification { method, params } => {
                if let Some(update) = notification(name, &method, &params, encoding) {
                    updates.push(update);
                }
            }
            ServerEvent::Response { id, result } => {
                if let Some(path) = self.outlines.remove(&(name, id)) {
                    if let Ok(result) = result {
                        updates.push(HubUpdate::Outline {
                            path,
                            result,
                            encoding,
                        });
                    }
                } else if let Some(generation) = self.symbol_queries.remove(&(name, id)) {
                    updates.push(HubUpdate::WorkspaceSymbols {
                        generation,
                        server: name,
                        result: result.unwrap_or(Value::Null),
                        encoding,
                    });
                } else if let Some(path) = self.pulls.remove(&(name, id)) {
                    // 실패(서버가 취소함 등)는 버린다. 다음 판에서 다시 묻는다.
                    if let Some(diagnostics) =
                        result.ok().as_ref().and_then(parse_pulled_diagnostics)
                    {
                        updates.push(HubUpdate::Diagnostics {
                            path,
                            server: name,
                            pulled: true,
                            diagnostics,
                            encoding,
                        });
                    }
                } else if let Some(lang) = self.requests.remove(&(name, id)) {
                    updates.push(HubUpdate::Response {
                        lang,
                        server: name,
                        id,
                        result,
                        encoding,
                    });
                } else if let Some(i) = self
                    .fan_outs
                    .iter()
                    .position(|g| g.waiting.contains(&(name, id)))
                {
                    let group = &mut self.fan_outs[i];
                    group.waiting.remove(&(name, id));
                    match result {
                        Ok(value) => group.results.push(value),
                        Err(e) if group.primary == (name, id) => group.error = Some(e),
                        Err(_) => {}
                    }
                    if group.waiting.is_empty() {
                        let group = self.fan_outs.swap_remove(i);
                        let (merged, empty) = if group.method == "textDocument/completion" {
                            let merged = merge_completions(&group.results);
                            let empty = merged["items"].as_array().is_some_and(Vec::is_empty);
                            (merged, empty)
                        } else {
                            let merged = merge_locations(&group.results);
                            let empty = merged.as_array().is_some_and(Vec::is_empty);
                            (merged, empty)
                        };
                        // 다른 서버가 찾았으면 주 서버의 오류(내용 바뀜 등)는 묻는다.
                        let result = match (empty, group.error) {
                            (true, Some(error)) => Err(error),
                            _ => Ok(merged),
                        };
                        updates.push(HubUpdate::Response {
                            lang: group.lang,
                            server: group.primary.0,
                            id: group.primary.1,
                            result,
                            encoding: group.encoding,
                        });
                    }
                } else if let Some(relay) = self.relays.remove(&(name, id)) {
                    // 중계한 요청이 실패하면 null로 답한다(요청한 서버는 결과 없음으로 처리한다).
                    self.reply(relay.to, relay.id, result.unwrap_or(Value::Null));
                } else {
                    self.internal.remove(&(name, id));
                }
            }
            ServerEvent::Request { id, method, params } => match method.as_str() {
                "workspace/applyEdit" => updates.push(HubUpdate::ApplyEdit {
                    server: name,
                    id,
                    edits: params
                        .get("edit")
                        .map(parse_workspace_edit)
                        .unwrap_or_default(),
                    encoding,
                }),
                // jdt.ls 확장의 콜백(클래스패스 변경 등)은 Spring Boot LS의 명령이다.
                "workspace/executeClientCommand" => {
                    if params.get("command").and_then(Value::as_str) == Some(spring::LS_START) {
                        self.reply(name, id, Value::Null);
                        // 아직 안 떴으면 다음 동기화 때 띄우고, 떠 있으면 연동을 다시 켠다.
                        self.spring_requested = true;
                        if let Some(spring) = catalog::by_launch(Launch::SpringBoot) {
                            self.start_spring(spring.name);
                        }
                        return;
                    }
                    self.relay(Launch::SpringBoot, params, name, id);
                }
                _ => match spring::jdt_command(&method, &params) {
                    Some(command) => self.relay(Launch::Jdtls, command, name, id),
                    None => self.reply(name, id, Value::Null),
                },
            },
        }
    }

    /// `workspace/executeCommand`를 `target` 서버로 보내고, 응답이 오면 `from`의 요청 `id`에 답한다.
    /// 대상 서버가 떠 있지 않으면 바로 null로 답한다.
    fn relay(&mut self, target: Launch, command: Value, from: &'static str, id: Value) {
        let sent = catalog::by_launch(target).and_then(|spec| {
            let lsp = self.servers.get_mut(spec.name)?.lsp.as_mut()?;
            Some((spec.name, lsp.request("workspace/executeCommand", command)))
        });
        match sent {
            Some(key) => {
                self.relays.insert(key, Relay { to: from, id });
            }
            None => self.reply(from, id, Value::Null),
        }
    }

    /// 준비된 Spring Boot LS에 설정을 보내고 jdt.ls 클래스패스 연동을 켠다.
    fn start_spring(&mut self, name: &'static str) {
        let Some(lsp) = self
            .servers
            .get_mut(name)
            .and_then(|s| s.lsp.as_mut())
            .filter(|l| l.is_ready())
        else {
            return;
        };
        lsp.notify(
            "workspace/didChangeConfiguration",
            json!({ "settings": spring::settings() }),
        );
        let id = lsp.request(
            "workspace/executeCommand",
            json!({ "command": spring::ENABLE_CLASSPATH, "arguments": [true] }),
        );
        self.internal.insert((name, id));
    }
}

fn start_server(
    spec: &'static ServerSpec,
    install_root: &Path,
    root: Option<&Path>,
    ctx: &egui::Context,
) -> Server {
    let mut server = Server {
        spec,
        lsp: None,
        state: ServerState::NotInstalled,
        started: Instant::now(),
        opened: HashMap::new(),
        pulled: HashMap::new(),
    };
    let Some(root) = root else {
        return server;
    };
    let command = match install::command(install_root, spec, root) {
        None => return server,
        Some(Err(reason)) => {
            server.state = ServerState::Failed(reason);
            return server;
        }
        Some(Ok(command)) => command,
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

fn notification(
    server: &'static str,
    method: &str,
    params: &Value,
    encoding: PositionEncoding,
) -> Option<HubUpdate> {
    match method {
        "textDocument/publishDiagnostics" => {
            let (path, diagnostics) = parse_diagnostics(params)?;
            Some(HubUpdate::Diagnostics {
                path,
                server,
                pulled: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_설정_파일만_spring_언어_id를_쓴다() {
        let id = |lang, name: &str| {
            language_id(lang, Path::new("src/main/resources").join(name).as_path())
        };
        assert_eq!(
            id(Lang::Yaml, "application.yml"),
            "spring-boot-properties-yaml"
        );
        assert_eq!(
            id(Lang::Yaml, "application-dev.yaml"),
            "spring-boot-properties-yaml"
        );
        assert_eq!(
            id(Lang::Properties, "bootstrap.properties"),
            "spring-boot-properties"
        );
        assert_eq!(id(Lang::Properties, "globals.properties"), "properties");
        assert_eq!(id(Lang::Yaml, "docker-compose.yml"), "yaml");
        assert_eq!(id(Lang::Java, "Application.java"), "java");
    }

    #[test]
    fn 서버별_진단을_합치고_한_서버만_바꿀_수_있다() {
        let rope = Rope::from_str("abc\ndef\n");
        let diag = |line: u32, message: &str| Diagnostic {
            range: lsp_types::Range::new(
                lsp_types::Position::new(line, 0),
                lsp_types::Position::new(line, 1),
            ),
            message: message.into(),
            ..Default::default()
        };
        let mut sync = DocSync::default();
        let utf16 = PositionEncoding::Utf16;
        sync.set_problems("jdtls", false, &rope, &[diag(1, "jdt")], utf16);
        sync.set_problems("spring-boot", false, &rope, &[diag(0, "spring")], utf16);
        let messages: Vec<_> = sync.problems.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(messages, ["spring", "jdt"], "위치순으로 합친다");

        sync.set_problems("jdtls", false, &rope, &[], utf16);
        let messages: Vec<_> = sync.problems.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(messages, ["spring"], "jdtls 몫만 비운다");
    }

    #[test]
    fn 연달아_죽을수록_오래_기다리고_다섯_번이면_멈춘다() {
        let quick = Duration::from_secs(1);
        let mut crashes = 0;
        let mut delays = Vec::new();
        loop {
            let (count, delay) = restart_after(crashes, quick);
            crashes = count;
            match delay {
                Some(delay) => delays.push(delay.as_secs()),
                None => break,
            }
        }
        assert_eq!(delays, [1, 2, 4, 8, 16]);
        assert_eq!(crashes, 6);
        // 1분 넘게 잘 돌다 죽었으면 처음부터 센다.
        assert_eq!(
            restart_after(5, STABLE_UPTIME),
            (1, Some(Duration::from_secs(1)))
        );
    }

    #[test]
    fn 여러_서버의_위치_응답을_location_배열로_합친다() {
        let range = |line: u32| json!({ "start": { "line": line, "character": 0 }, "end": { "line": line, "character": 1 } });
        let jdt = json!({ "uri": "file:///a/Sample.java", "range": range(3) });
        let egov = json!([
            { "targetUri": "file:///a/Sample_SQL.xml", "targetRange": range(0), "targetSelectionRange": range(1) },
            { "uri": "file:///a/Sample.java", "range": range(3) },
        ]);
        let merged = merge_locations(&[jdt, Value::Null, egov]);
        assert_eq!(
            merged,
            json!([
                { "uri": "file:///a/Sample.java", "range": range(3) },
                { "uri": "file:///a/Sample_SQL.xml", "range": range(1) },
            ]),
            "LocationLink는 선택 범위로, 같은 위치는 한 번만"
        );
    }

    #[test]
    fn 여러_서버의_자동완성을_목록_하나로_합친다() {
        let merged = merge_completions(&[
            json!({ "isIncomplete": false, "items": [{ "label": "select" }] }),
            json!([{ "label": "id" }]),
            Value::Null,
            json!({ "isIncomplete": true, "items": [{ "label": "SampleVO" }] }),
        ]);
        assert_eq!(merged["isIncomplete"], true, "하나라도 미완성이면 미완성");
        let labels: Vec<&str> = merged["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["select", "id", "SampleVO"], "주 서버가 먼저");
    }
}
