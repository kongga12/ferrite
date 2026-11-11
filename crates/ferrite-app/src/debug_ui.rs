//! 디버거: 디버그 세션 하나와 그 화면(도구 막대·호출 스택·변수 창, 디버그 출력 창).
//! 멈추면 호출 스택과 맨 위 프레임의 변수를 스스로 불러오고, 앱에 멈춘 자리를 알린다.
//! 로컬 파일이 없는 스크립트(브라우저 화면의 인라인 스크립트)는 어댑터에서 내용을 받아 임시 폴더에
//! 담아 읽기 전용으로 연다. 그 파일에 건 중단점은 원래 소스로 건다.
//! js-debug면 그 스크립트를 만든 JSP를 내용으로 찾아, JSP에 건 중단점을 화면 스크립트의 줄로 바꿔 걸고 멈춘
//! 자리도 JSP의 줄로 보인다. 화면에 인라인 스크립트가 여럿이면(Tiles·`<jsp:include>`·`<script>` 여럿) 블록마다
//! 그것을 만든 JSP를 찾는다. 블록의 글은 js-debug의 CDP 프록시로 받은 화면의 원래 HTML에서 잘라 낸다(원래
//! HTML을 주지 않는 프레임 안 화면은 브라우저의 디버거에서 블록마다 받는다).
//! 화면을 불러올 때 도는 코드에 건 JSP 중단점도 걸리게 화면마다 처음 맞춘 뒤 한 번 다시 부른다.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use ferrite_core::debug_config::{self, CustomAdapter, DebugConfig};
use ferrite_dap::cdp::{self, Proxy};
use ferrite_dap::page_lines::{self, PageMap};
use ferrite_dap::{
    Adapter, Breakpoint, BreakpointState, DebugEvent, DebugSession, Launch, Scope, Source,
    StackFrame, State, Variable, adapters, types,
};
use serde_json::{Value, json};

/// 디버그 출력 창에 남길 최대 글자 수(넘으면 앞에서 버린다).
// ponytail: 문자열 하나에 이어 붙인다. 출력이 아주 많으면 줄 단위 가상 스크롤로 바꾼다.
const CONSOLE_LIMIT: usize = 1 << 20;
/// 멈춤을 누른 뒤 어댑터가 대상을 끝내고 알려 주기를 기다리는 시간. 넘으면 어댑터를 끝낸다.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// 응답을 기다리는 요청.
enum Request {
    Threads,
    StackTrace,
    Scopes,
    Variables(i64),
    Evaluate(String),
    LoadedSources,
    /// 로컬 파일이 없는 소스의 내용.
    Source(Source),
    /// 화면(열쇠)의 인라인 스크립트 글들. js-debug는 같은 화면의 스크립트를 하나만 남겨 내용을 다 줄 수
    /// 없으므로 화면에서 읽는다(CDP 프록시로 원래 HTML을 받지 못할 때).
    PageScripts(String),
    /// js-debug의 CDP 프록시 주소(화면의 원래 HTML을 받는다).
    CdpProxy,
}

/// 로컬 파일이 없는 소스를 담아 두는 폴더(이 앱 프로세스의 것). 앱을 닫을 때 지운다.
pub fn sources_dir() -> PathBuf {
    std::env::temp_dir().join(format!("ferrite-debug-sources-{}", std::process::id()))
}

/// 어댑터에 내용을 물어야 하는 소스(로컬 파일이 없다).
fn is_remote(source: &Source) -> bool {
    source.source_reference > 0
        && !source
            .path
            .as_ref()
            .is_some_and(|p| p.is_absolute() && p.is_file())
}

/// 어댑터가 준 소스 이름을 보일 글. js-debug는 경로에 쓸 수 없는 `:` 대신 `꞉`(U+A789)을 쓰는데 글꼴에 없다.
fn shown(name: &str) -> String {
    name.replace('\u{a789}', ":")
}

/// 소스를 담을 파일 이름: 경로의 마지막 조각에서 파일 이름에 못 쓰는 글자를 빼고, HTML이면 `.html`을 붙인다.
fn stored_name(source: &Source, content: &str) -> String {
    let text = source
        .path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| source.name.clone())
        .unwrap_or_default();
    let last = text.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut name: String = last
        .chars()
        .map(|c| match c {
            '\u{a789}' | ':' | '<' | '>' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    if name.trim_matches(['_', '.']).is_empty() {
        name = "script".into();
    }
    // 화면이면 HTML, 아니면 스크립트로 강조한다(js-debug는 인라인 스크립트를 화면 줄에 맞춰 빈 줄을
    // 채운 스크립트로 준다).
    let lower = name.to_ascii_lowercase();
    let (extension, known): (&str, &[&str]) = if content.trim_start().starts_with('<') {
        (".html", &[".html", ".htm"])
    } else {
        (".js", &[".js", ".mjs", ".cjs", ".ts", ".jsx", ".tsx"])
    };
    if !known.iter().any(|e| lower.ends_with(e)) {
        name.push_str(extension);
    }
    name
}

/// 아직 화면 스크립트에 걸지 못한 JSP 중단점(디버그 중에는 빈 점으로 보인다).
static UNBOUND: BreakpointState = BreakpointState {
    id: None,
    verified: false,
    line: None,
    message: None,
};

/// js-debug 세션에서 JSP와 화면 스크립트를 잇는다. js-debug는 화면 주소를 만든 JSP를 모르므로 JSP에 건
/// 중단점을 걸지 못한다. 그래서 화면 스크립트를 불러올 때마다 내용으로 JSP를 찾아 그 줄로 바꿔 건다.
struct Pages {
    /// JSP 파일에 건 중단점.
    breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
    /// 담은 파일에 직접 건 중단점.
    direct: HashMap<PathBuf, Vec<Breakpoint>>,
    /// 담은 파일 → 그 화면 스크립트(블록마다)를 만든 JSP와 줄.
    lines: HashMap<PathBuf, PageMap>,
    /// 담은 파일에 보낸 중단점 중 JSP에서 온 것(직접 건 것 뒤에 붙인다): (JSP, 그 JSP의 중단점 번호).
    sent: HashMap<PathBuf, Vec<(PathBuf, usize)>>,
    /// 내용을 묻고 있는 소스(같은 소스를 거듭 묻지 않는다).
    asking: HashSet<String>,
    /// 화면(열쇠)마다 알려 온 인라인 스크립트.
    inline: HashMap<String, Inline>,
    /// js-debug의 CDP 프록시(처음 쓸 때 묻는다).
    proxy: ProxyState,
    /// CDP 프록시로 따로 한 일의 결과.
    events: (Sender<PageEvent>, Receiver<PageEvent>),
    /// 화면(열쇠)마다 처음 맞춘 뒤 다시 부르는 일.
    reloads: HashMap<String, Reload>,
    /// 화면을 읽는 중에 멈춰 미룬 원래 HTML 받기(그동안은 CDP가 답하지 않는다). 이어 가면 받는다.
    deferred: Vec<String>,
    /// 사용자가 계속을 눌렀다(다음 `continued`에 미룬 다시 부르기를 한다).
    continuing: bool,
}

impl Default for Pages {
    fn default() -> Self {
        Self {
            breakpoints: BTreeMap::new(),
            direct: HashMap::new(),
            lines: HashMap::new(),
            sent: HashMap::new(),
            asking: HashSet::new(),
            inline: HashMap::new(),
            proxy: ProxyState::default(),
            events: channel(),
            reloads: HashMap::new(),
            deferred: Vec::new(),
            continuing: false,
        }
    }
}

/// CDP 프록시로 따로(UI를 막지 않게) 한 일의 결과. 화면 열쇠와 함께 온다.
enum PageEvent {
    /// 화면의 블록들(시작 줄·칸은 1부터, 글): 원래 HTML에서 잘랐거나 브라우저의 디버거에서 받았다.
    Blocks(String, Result<Vec<(usize, usize, String)>, String>),
    /// 화면을 읽는 중에 멈춰 있어 HTML을 받지 않았다.
    Deferred(String),
    /// 화면을 다 읽었다(다시 불러도 되는지).
    Loaded(String, Result<cdp::Loaded, String>),
    /// 다시 불렀다.
    Reloaded(String, Result<(), String>),
}

/// 화면을 다 읽기를 기다리는 시간(다시 부르기 전).
const LOAD_WAIT: Duration = Duration::from_secs(10);
/// 원래 HTML을 받기 전에 화면을 다 읽기를 기다리는 시간. 넘으면 미뤄 다시 묻는다(멈춰 있으면 이어 간 뒤에).
const HTML_WAIT: Duration = Duration::from_secs(1);

/// 처음 맞춘 화면을 한 번 다시 부르는 일. 화면을 불러올 때 도는 코드(맨 위 코드, `$(function () { … })`)는
/// JSP 중단점을 화면 줄로 바꿔 걸기 전에 돌아 버리므로, 화면마다 처음 맞춘 뒤 한 번 다시 불러 그 코드에서도
/// 멈추게 한다.
#[derive(Clone, PartialEq, Eq)]
enum Reload {
    /// 화면을 다 읽기를 기다리며 다시 불러도 되는지 보는 중.
    Checking,
    /// 맞춤이 끝나고 보낸 중단점의 답을 받으면 그곳(맨 위 화면이나 그 프레임)을 부른다(그래야 어댑터가
    /// 중단점을 건 뒤에 부른다).
    Deciding(cdp::Target),
    /// 멈춰 있다. 사용자가 계속을 누르면 부른다.
    AfterContinue(cdp::Target),
    /// 불렀거나 부르지 않기로 했다.
    Done,
}

/// CDP 프록시를 아는지.
#[derive(Default)]
enum ProxyState {
    #[default]
    Unknown,
    /// 묻는 중. 답을 받으면 HTML을 받을 화면들.
    Asking(Vec<String>),
    Ready(Proxy),
    /// 없다(js-debug가 아니거나 옛 판). 화면에서 글을 읽는다.
    Missing,
}

/// 한 화면의 인라인 스크립트들. js-debug는 같은 화면 주소의 스크립트가 새로 오면 앞의 것을 지우지만(번호도
/// 다시 쓴다), 중단점은 화면 주소와 화면 줄로 걸어 모든 블록에 걸린다.
#[derive(Default)]
struct Inline {
    /// 블록의 시작 (줄, 칸), 1부터. 이름 끝의 `꞉줄:칸`에서 읽는다.
    starts: BTreeSet<(usize, usize)>,
    /// 마지막으로 알려 온 블록(js-debug에 살아 있는 것). 담은 파일의 중단점은 이것으로 보낸다.
    live: Option<Source>,
    /// 글을 묻는 동안 블록이 더 왔다(답을 받으면 다시 묻는다).
    again: bool,
}

/// 멈춘 프레임이 JSP가 만든 화면 스크립트면 그 줄이 든 블록의 JSP와 그 줄로 바꾼다.
fn to_jsp(frame: &mut StackFrame, map: &PageMap) {
    let Some((file, line)) = map.to_file(frame.line.max(1) as usize - 1) else {
        return;
    };
    let Some(source) = frame.source.as_mut() else {
        return;
    };
    source.path = Some(file.to_path_buf());
    source.name = file.file_name().map(|n| n.to_string_lossy().into_owned());
    frame.line = line as i64 + 1;
}

/// 빌드가 끝나기를 기다리는 시작. 빌드에 성공하면 세션을 띄운다.
struct PendingLaunch {
    adapter: Adapter,
    launch: Launch,
    breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
    /// 빌드 출력. 실패(0이 아닌 종료 코드)면 Err.
    build: Receiver<Result<String, String>>,
}

/// 앱이 할 일.
pub enum DebugNotice {
    /// 이 파일의 이 줄(0부터)을 보여 준다.
    Reveal(PathBuf, usize),
    /// 상태바에 보일 글.
    Status(String),
}

#[derive(Default)]
pub struct Debugger {
    session: Option<DebugSession>,
    pending: Option<PendingLaunch>,
    /// 다시 그리게 할 때 쓴다(멈춤 기다림이 끝날 때).
    ctx: Option<egui::Context>,
    /// 멈춤을 누른 때. 그동안은 세션을 살려 두어 어댑터가 대상을 끝내게 한다.
    stopping: Option<Instant>,
    /// 스레드를 모를 때 누른 일시 정지. 스레드 목록을 받으면 보낸다.
    pause_requested: bool,
    /// 지금 세션의 구성 이름과 어댑터 이름.
    name: String,
    adapter: String,
    started: bool,
    thread_id: Option<i64>,
    frames: Vec<StackFrame>,
    /// 고른 프레임(호출 스택 창에서 누른 것).
    frame: usize,
    scopes: Vec<Scope>,
    /// 불러온 변수(참조 번호 → 자식).
    children: HashMap<i64, Vec<Variable>>,
    requests: HashMap<i64, Request>,
    /// 어댑터가 확인한 중단점(파일별, 보낸 순서).
    states: HashMap<PathBuf, Vec<BreakpointState>>,
    console: String,
    expression: String,
    /// 담아 둔 소스: 어댑터의 소스 경로(글) → 로컬 파일.
    stored: HashMap<String, PathBuf>,
    /// 불러온 스크립트 중 로컬 파일이 없는 것(디버그 창 목록).
    scripts: Vec<Source>,
    /// 내용을 받으면 열 소스(목록에서 누른 것).
    open_after: Option<String>,
    /// 어댑터에 준 세션 임시 폴더(js-debug). 세션이 끝나면 지운다.
    scratch: Option<PathBuf>,
    /// 담아 둔 소스를 둘 폴더(없으면 `sources_dir()`). 시험은 시험마다 따로 준다(같은 프로세스에서 함께 돌아
    /// 한 폴더를 쓰면 먼저 끝난 시험이 지운다).
    sources: Option<PathBuf>,
    /// 프로젝트 폴더(화면 스크립트를 만든 JSP를 찾는다).
    root: Option<PathBuf>,
    /// js-debug 세션의 JSP 잇기. 다른 어댑터면 None.
    pages: Option<Pages>,
}

/// 어댑터가 준 소스의 경로(없으면 이름, 그것도 없으면 번호).
fn source_text(source: &Source) -> String {
    source
        .path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| source.name.clone())
        .unwrap_or_else(|| source.source_reference.to_string())
}

/// 담아 둔 소스를 찾는 열쇠(같은 화면을 다시 불러도 같다. 번호는 바뀐다). 인라인 스크립트는 화면 주소로
/// 묶는다(블록마다 이름 끝에 `꞉줄:칸`이 붙는다).
fn source_key(source: &Source) -> String {
    let text = source_text(source);
    match page_lines::inline_script(&text) {
        Some((page, ..)) if page.contains('/') => page.to_owned(),
        _ => text,
    }
}

impl Debugger {
    /// 에이전트(MCP)에 보일 상태: 구성·상태·호출 스택·고른 프레임의 불러온 변수.
    pub fn mcp_state(&self) -> Option<ferrite_mcp::Debug> {
        if !self.is_active() {
            return None;
        }
        let state = if self.session.is_none() {
            "starting"
        } else if self.is_stopped() {
            "stopped"
        } else {
            "running"
        };
        let frames = self
            .frames
            .iter()
            .map(|f| ferrite_mcp::Frame {
                name: f.name.clone(),
                path: f
                    .source
                    .as_ref()
                    .and_then(|s| s.path.as_ref())
                    .map(|p| p.display().to_string()),
                line: f.line,
            })
            .collect();
        let variables = self
            .scopes
            .iter()
            .flat_map(|scope| {
                self.children
                    .get(&scope.variables_reference)
                    .into_iter()
                    .flatten()
                    .map(move |v| ferrite_mcp::Variable {
                        scope: scope.name.clone(),
                        name: v.name.clone(),
                        value: v.value.clone(),
                        type_name: v.type_name.clone(),
                    })
            })
            .collect();
        Some(ferrite_mcp::Debug {
            name: self.name.clone(),
            adapter: self.adapter.clone(),
            state: state.into(),
            frames,
            variables,
        })
    }

    /// 세션이 있거나 시작 전 빌드 중.
    pub fn is_active(&self) -> bool {
        self.session.is_some() || self.pending.is_some()
    }

    pub fn is_stopped(&self) -> bool {
        matches!(
            self.session.as_ref().map(DebugSession::state),
            Some(State::Stopped(_))
        )
    }

    /// 구성으로 세션을 시작한다. 빌드 명령이 있으면 먼저 별도 스레드에서 빌드하고, 성공하면 띄운다.
    /// 돌고 있는 세션은 먼저 끝낸다. 어댑터는 부르는 쪽이 고른다(`adapters::resolve`, java-debug는 jdt.ls가
    /// 연 포트).
    pub fn start(
        &mut self,
        config: &DebugConfig,
        root: &Path,
        file: Option<&Path>,
        breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
        adapter: Adapter,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        // 인자 문제는 빌드하기 전에 알린다.
        let mut arguments = config.resolved_arguments(root, file)?;
        adapters::complete_arguments(&config.adapter, &mut arguments);
        let launch = Launch {
            adapter_id: config.adapter.clone(),
            attach: config.attach(),
            arguments,
        };
        *self = Self {
            ctx: Some(ctx.clone()),
            name: config.name.clone(),
            adapter: config.adapter.clone(),
            console: format!("[디버그 시작: {}]\n", config.name),
            root: Some(root.to_path_buf()),
            ..Default::default()
        };
        // JSP 중단점은 js-debug에 보내지 않고 화면 스크립트를 불러올 때 그 줄로 바꿔 건다.
        let mut breakpoints = breakpoints;
        if config.adapter == "js-debug" {
            let (jsp, rest): (BTreeMap<_, _>, _) = breakpoints
                .into_iter()
                .partition(|(path, _)| page_lines::is_page_file(path));
            breakpoints = rest;
            // 맞춘 JSP는 실제 경로로 적으므로 같은 꼴로 둔다(macOS `/var` 링크, Windows 짧은 이름).
            self.pages = Some(Pages {
                breakpoints: jsp
                    .into_iter()
                    .map(|(path, list)| (crate::app::real_path(&path), list))
                    .collect(),
                ..Default::default()
            });
        }
        let Some(build) = &config.build else {
            return self.launch(&adapter, launch, breakpoints);
        };
        self.push_console(&format!("[빌드: {build}]\n"));
        let (done, result) = channel();
        let (line, dir, wake) = (build.clone(), root.to_path_buf(), ctx.clone());
        thread::spawn(move || {
            let _ = done.send(run_build(&line, &dir));
            wake.request_repaint();
        });
        self.pending = Some(PendingLaunch {
            adapter,
            launch,
            breakpoints,
            build: result,
        });
        Ok(())
    }

    fn launch(
        &mut self,
        adapter: &Adapter,
        launch: Launch,
        breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
    ) -> Result<(), String> {
        let wake = self.ctx.clone();
        self.scratch = adapters::scratch_of(adapter);
        let session = DebugSession::start(adapter, launch, breakpoints, move || {
            if let Some(ctx) = &wake {
                ctx.request_repaint();
            }
        })?;
        self.session = Some(session);
        Ok(())
    }

    /// 대상을 끝낸다. 바로 세션을 버리면 어댑터가 대상을 끝내기 전에 죽어 대상이 남을 수 있으므로,
    /// 끝났다는 알림을 기다리고 `STOP_GRACE`가 지나면 어댑터를 끝낸다.
    pub fn stop(&mut self) {
        // 빌드 중이면 시작만 취소한다(빌드 명령은 끝까지 돌고 결과는 버린다).
        if self.pending.take().is_some() {
            self.push_console("[시작을 취소했습니다]\n");
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        if self.stopping.is_none() {
            session.stop();
            self.stopping = Some(Instant::now());
            self.clear_stop();
            self.push_console("[멈추는 중…]\n");
            if let Some(ctx) = &self.ctx {
                ctx.request_repaint_after(STOP_GRACE);
            }
        }
    }

    /// 앱을 닫을 때: 대상을 끝내라고 하고 끝났다는 알림을 잠깐(`STOP_GRACE`) 기다린 뒤 세션을 버린다.
    /// 바로 버리면 어댑터만 죽고, 어댑터가 띄운 브라우저가 늦게 끝나 임시 폴더를 쥐고 있다.
    pub fn shutdown(&mut self) {
        self.pending = None;
        if let Some(session) = &mut self.session {
            session.stop();
            let deadline = Instant::now() + STOP_GRACE;
            while Instant::now() < deadline && *session.state() != State::Terminated {
                session.poll();
                thread::sleep(Duration::from_millis(50));
            }
        }
        self.session = None;
    }

    fn end(&mut self, note: &str) {
        self.session = None;
        if let Some(dir) = self.scratch.take() {
            adapters::remove_later(dir);
        }
        self.stopping = None;
        self.clear_stop();
        self.push_console(&format!("{note}\n"));
    }

    fn clear_stop(&mut self) {
        self.frames.clear();
        self.frame = 0;
        self.scopes.clear();
        self.children.clear();
    }

    /// 계속(F5)·한 줄씩(F10)·들어가기(F11)·나오기(Shift+F11). 멈춘 스레드에 보낸다.
    pub fn resume(&mut self, how: Step) {
        let (Some(session), Some(thread)) = (&mut self.session, self.thread_id) else {
            return;
        };
        if self.stopping.is_some() || !matches!(session.state(), State::Stopped(_)) {
            return;
        }
        match how {
            Step::Continue => session.resume(thread),
            Step::Over => session.step_over(thread),
            Step::In => session.step_in(thread),
            Step::Out => session.step_out(thread),
        }
        // 미룬 다시 부르기는 계속을 누른 뒤에만 한다(한 줄씩 가는 중에 화면이 바뀌지 않게).
        if matches!(how, Step::Continue)
            && let Some(pages) = &mut self.pages
        {
            pages.continuing = true;
        }
        self.clear_stop();
    }

    pub fn pause(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        match self.thread_id {
            Some(thread) => session.pause(thread),
            None => {
                self.pause_requested = true;
                self.send(Request::Threads);
            }
        }
    }

    pub fn set_breakpoints(&mut self, path: &Path, breakpoints: Vec<Breakpoint>) {
        let stored = self.is_stored(path);
        if let Some(pages) = &mut self.pages {
            if page_lines::is_page_file(path) {
                let path = crate::app::real_path(path);
                pages.breakpoints.insert(path.clone(), breakpoints);
                let scripts: Vec<PathBuf> = pages
                    .lines
                    .iter()
                    .filter(|(_, map)| map.files().contains(&path.as_path()))
                    .map(|(local, _)| local.clone())
                    .collect();
                for local in scripts {
                    self.send_page(&local);
                }
                return;
            }
            if stored {
                pages.direct.insert(path.to_path_buf(), breakpoints);
                self.send_page(path);
                return;
            }
        }
        self.states.remove(path);
        if let Some(session) = &mut self.session {
            session.set_breakpoints(path, breakpoints);
        }
    }

    /// 담은 화면 스크립트의 중단점을 보낸다: 직접 건 것과, 그 스크립트를 만든 JSP에 건 것을 화면 줄로
    /// 바꾼 것(화면에 그대로 나온 줄만).
    fn send_page(&mut self, local: &Path) {
        let Some(pages) = &mut self.pages else {
            return;
        };
        let mut list = pages.direct.get(local).cloned().unwrap_or_default();
        let mut origins = Vec::new();
        if let Some(map) = pages.lines.get(local) {
            for file in map.files() {
                for (i, b) in pages
                    .breakpoints
                    .get(file)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    // 반복으로 여러 번 나온 블록이면 나온 곳마다 건다.
                    for line in map.to_pages(file, b.line.saturating_sub(1)) {
                        list.push(Breakpoint {
                            line: line + 1,
                            condition: b.condition.clone(),
                        });
                        origins.push((file.to_path_buf(), i));
                    }
                }
            }
        }
        // 보낸 적이 없고 보낼 것도 없으면 보내지 않는다(화면을 불러올 때마다 스크립트가 여럿이다).
        if list.is_empty() && !pages.sent.contains_key(local) {
            return;
        }
        pages.sent.insert(local.to_path_buf(), origins);
        self.states.remove(local);
        if let Some(session) = &mut self.session {
            session.set_breakpoints(local, list);
        }
    }

    /// 어댑터가 확인한 그 파일 `index`번째 중단점(세션이 없으면 None). JSP면 그 중단점을 바꿔 건 화면
    /// 스크립트의 것(걸린 것이 먼저)이고, 아직 건 곳이 없으면 걸리지 않은 것으로 본다.
    pub fn breakpoint_state(&self, path: &Path, index: usize) -> Option<&BreakpointState> {
        self.session.as_ref()?;
        if let Some(pages) = &self.pages
            && page_lines::is_page_file(path)
        {
            let mut found = None;
            for (local, origins) in &pages.sent {
                let direct = pages.direct.get(local).map_or(0, Vec::len);
                // 반복으로 여러 곳에 건 것은 하나라도 걸리면 걸린 것이다.
                for (k, _) in origins
                    .iter()
                    .enumerate()
                    .filter(|(_, (f, i))| f == path && *i == index)
                {
                    if let Some(state) = self.states.get(local).and_then(|s| s.get(direct + k)) {
                        if state.verified {
                            return Some(state);
                        }
                        found = Some(state);
                    }
                }
            }
            return Some(found.unwrap_or(&UNBOUND));
        }
        self.states.get(path)?.get(index)
    }

    /// 프로젝트의 JSP(UTF-8·CP949로 읽고 경로는 열린 문서와 같은 꼴로).
    fn page_files(&self) -> Option<Vec<(PathBuf, String)>> {
        let root = self.root.as_ref()?;
        Some(
            page_lines::candidates(root)
                .into_iter()
                .filter_map(|path| {
                    let bytes = std::fs::read(&path).ok()?;
                    let text = ferrite_core::text_file::decode(&bytes).ok()?.text;
                    Some((crate::app::real_path(&path), text))
                })
                .collect(),
        )
    }

    /// 화면 스크립트 내용으로 그것을 만든 JSP와 줄을 찾는다.
    fn match_page(&self, source: &Source, content: &str) -> Option<PageMap> {
        let files = self.page_files()?;
        page_lines::find(content, &files, &shown(&source_key(source))).map(PageMap::single)
    }

    /// 어댑터에서 받아 담아 둔 소스 파일인지(읽기 전용으로 연다).
    pub fn is_stored(&self, path: &Path) -> bool {
        self.stored.values().any(|p| p == path)
    }

    /// 받은 소스 내용을 파일에 담는다. 이미 담은 소스면 내용만 고친다(화면을 다시 불렀을 때).
    fn store(&mut self, source: &Source, content: &str) -> std::io::Result<PathBuf> {
        let key = source_key(source);
        if let Some(local) = self.stored.get(&key) {
            std::fs::write(local, content)?;
            return Ok(local.clone());
        }
        let dir = self.sources.clone().unwrap_or_else(sources_dir);
        std::fs::create_dir_all(&dir)?;
        let dir = crate::app::real_path(&dir);
        let name = stored_name(source, content);
        let mut local = dir.join(&name);
        let mut n = 2;
        while self.stored.values().any(|p| *p == local) {
            local = dir.join(format!("{n}-{name}"));
            n += 1;
        }
        std::fs::write(&local, content)?;
        self.stored.insert(key, local.clone());
        Ok(local)
    }

    /// 고른 프레임의 파일과 줄(0부터). 멈춰 있을 때만.
    pub fn location(&self) -> Option<(&Path, usize)> {
        let frame = self.frames.get(self.frame)?;
        let path = frame.source.as_ref()?.path.as_deref()?;
        Some((path, frame.line.max(1) as usize - 1))
    }

    fn push_console(&mut self, text: &str) {
        self.console.push_str(text);
        if self.console.len() > CONSOLE_LIMIT {
            let mut cut = self.console.len() - CONSOLE_LIMIT;
            while !self.console.is_char_boundary(cut) {
                cut += 1;
            }
            self.console.drain(..cut);
        }
    }

    fn send(&mut self, request: Request) {
        let Some(session) = &mut self.session else {
            return;
        };
        let id = match &request {
            Request::Threads => session.threads(),
            Request::StackTrace => match self.thread_id {
                Some(thread) => session.stack_trace(thread),
                None => return,
            },
            Request::Scopes => match self.frames.get(self.frame) {
                Some(frame) => session.scopes(frame.id),
                None => return,
            },
            Request::Variables(reference) => session.variables(*reference),
            Request::Evaluate(expression) => {
                let frame = self.frames.get(self.frame).map(|f| f.id);
                session.evaluate(expression, frame)
            }
            Request::LoadedSources => session.loaded_sources(),
            Request::Source(source) => session.source(source),
            Request::PageScripts(_) => session.request(
                "evaluate",
                json!({ "expression": page_lines::PAGE_SCRIPTS, "context": "clipboard" }),
            ),
            Request::CdpProxy => session.request("requestCDPProxy", json!({})),
        };
        self.requests.insert(id, request);
    }

    fn reveal_frame(&self) -> Option<DebugNotice> {
        self.location()
            .map(|(path, line)| DebugNotice::Reveal(path.to_path_buf(), line))
    }

    /// 세션 이벤트를 처리한다. 매 프레임 부른다.
    pub fn poll(&mut self) -> Vec<DebugNotice> {
        let mut notices = Vec::new();
        if let Some(result) = self.pending.as_ref().and_then(|p| p.build.try_recv().ok()) {
            let Some(pending) = self.pending.take() else {
                return notices;
            };
            match result {
                Ok(output) => {
                    self.push_console(&output);
                    if let Err(e) =
                        self.launch(&pending.adapter, pending.launch, pending.breakpoints)
                    {
                        self.push_console(&format!("[오류] {e}\n"));
                        notices.push(DebugNotice::Status(format!(
                            "디버그를 시작하지 못했습니다: {e}"
                        )));
                    }
                }
                Err(output) => {
                    self.push_console(&output);
                    notices.push(DebugNotice::Status(
                        "빌드에 실패해 디버그를 시작하지 않았습니다(디버그 출력 참고)".into(),
                    ));
                }
            }
        }
        let Some(session) = &mut self.session else {
            return notices;
        };
        let events = session.poll();
        if self.stopping.is_some_and(|at| at.elapsed() >= STOP_GRACE) {
            self.end("[디버그를 멈췄습니다]");
            notices.push(DebugNotice::Status("디버그를 멈췄습니다".into()));
            return notices;
        }
        // CDP 프록시로 따로 한 일: 받은 화면 HTML로 블록을 맞추고, 처음 맞춘 화면을 다시 부른다.
        let done: Vec<PageEvent> = self
            .pages
            .as_ref()
            .map_or_else(Vec::new, |pages| pages.events.1.try_iter().collect());
        for event in done {
            match event {
                PageEvent::Blocks(key, blocks) => notices.extend(self.on_page_blocks(key, blocks)),
                PageEvent::Deferred(key) => self.defer_page(key),
                PageEvent::Loaded(key, loaded) => self.on_page_loaded(key, loaded),
                PageEvent::Reloaded(key, result) => {
                    let page = shown(&key);
                    self.push_console(&match result {
                        Ok(()) => format!(
                            "[화면을 다시 불렀습니다: {page}] 화면을 불러올 때 도는 코드에 건 JSP 중단점도 걸리게 \
                             처음 맞춘 뒤 한 번 다시 부릅니다.\n"
                        ),
                        Err(e) => format!("[화면을 다시 부르지 못했습니다: {page}] {e}\n"),
                    });
                }
            }
        }
        for event in events {
            match event {
                DebugEvent::Started => {
                    self.started = true;
                    notices.push(DebugNotice::Status(format!("디버그 중: {}", self.name)));
                }
                DebugEvent::Stopped(stopped) => {
                    self.clear_stop();
                    let reason = match stopped.reason.as_str() {
                        "breakpoint" => "중단점",
                        "step" => "단계 실행",
                        "pause" => "일시 정지",
                        "exception" => "예외",
                        "entry" => "시작",
                        other => other,
                    };
                    notices.push(DebugNotice::Status(format!("멈춤: {reason}")));
                    match stopped.thread_id {
                        Some(thread) => {
                            self.thread_id = Some(thread);
                            self.send(Request::StackTrace);
                        }
                        // 스레드를 알려 주지 않으면 스레드 목록에서 첫 스레드를 쓴다.
                        None => self.send(Request::Threads),
                    }
                }
                DebugEvent::Continued => {
                    self.clear_stop();
                    self.continued();
                }
                DebugEvent::Output { category, text } => {
                    if category != "telemetry" {
                        self.push_console(&text);
                    }
                }
                DebugEvent::Breakpoints { path, states } => {
                    self.states.insert(path.clone(), states);
                    self.breakpoints_answered(&path);
                }
                DebugEvent::Response { id, result, .. } => {
                    if let Some(request) = self.requests.remove(&id) {
                        notices.extend(self.on_response(request, result));
                    }
                }
                DebugEvent::Error(message) => {
                    self.push_console(&format!("[오류] {message}\n"));
                    notices.push(DebugNotice::Status(message));
                }
                DebugEvent::Exited { code } => {
                    self.push_console(&format!("[종료 코드 {code}]\n"));
                }
                // 화면을 불러오면 인라인 스크립트의 글을 받아 JSP를 찾는다(JSP 중단점을 걸려고). 화면
                // 주소가 있는 소스만 묻는다(`eval`로 만든 스크립트 등은 뺀다).
                DebugEvent::LoadedSource { reason, source } => {
                    let key = source_key(&source);
                    if reason != "removed" && is_remote(&source) && key.contains('/') {
                        self.loaded(key, source);
                    }
                }
                DebugEvent::Terminated => {
                    self.end("[디버그를 마쳤습니다]");
                    notices.push(DebugNotice::Status("디버그를 마쳤습니다".into()));
                    break;
                }
                DebugEvent::Closed(stderr) => {
                    let stderr = stderr.unwrap_or_default();
                    // 시작도 못 하고 끊겼으면 어댑터 문제다. stderr로 고칠 방법을 짐작한다.
                    if !self.started {
                        let hint = adapters::hint(&self.adapter, &stderr)
                            .map(str::to_owned)
                            .unwrap_or_else(|| {
                                stderr.lines().last().unwrap_or("").trim().to_owned()
                            });
                        let message =
                            format!("디버그 어댑터가 끝났습니다({}): {hint}", self.adapter);
                        self.push_console(&format!("{stderr}\n"));
                        notices.push(DebugNotice::Status(message.clone()));
                        self.end(&format!("[{message}]"));
                    } else {
                        self.end("[디버그 어댑터 연결이 끊겼습니다]");
                    }
                    break;
                }
            }
        }
        notices
    }

    fn on_response(
        &mut self,
        request: Request,
        result: Result<Value, String>,
    ) -> Option<DebugNotice> {
        let body = match result {
            Ok(body) => body,
            // 화면에서 글을 읽지 못하면 살아 있는 블록의 내용으로 맞춘다(그 블록만 JSP를 찾는다).
            Err(_) if matches!(request, Request::PageScripts(_)) => {
                if let Request::PageScripts(key) = request {
                    self.page_fallback(&key);
                }
                return None;
            }
            // CDP 프록시가 없으면 화면에서 글을 읽는다.
            Err(_) if matches!(request, Request::CdpProxy) => {
                self.proxy_answered(None);
                return None;
            }
            Err(e) => {
                self.push_console(&format!("[오류] {e}\n"));
                return None;
            }
        };
        match request {
            Request::Threads => {
                self.thread_id = types::threads(&body).first().map(|t| t.id);
                // 스레드가 없으면 다시 묻지 않는다(되풀이 방지).
                if std::mem::take(&mut self.pause_requested) && self.thread_id.is_some() {
                    self.pause();
                } else if self.is_stopped() {
                    self.send(Request::StackTrace);
                }
                None
            }
            Request::StackTrace => {
                self.frames = types::stack_frames(&body);
                // 어댑터가 준 경로(드라이브 글자 대소문자 등)를 열린 문서 경로와 같은 꼴로 맞춘다. 로컬 파일이
                // 없는 소스는 담아 둔 파일로 바꾸고, 아직 없으면 내용을 묻는다.
                let mut fetch: Vec<Source> = Vec::new();
                for frame in &mut self.frames {
                    let Some(source) = frame.source.as_mut() else {
                        continue;
                    };
                    if is_remote(source) {
                        match self.stored.get(&source_key(source)) {
                            Some(local) => {
                                source.path = Some(local.clone());
                                if let Some(map) =
                                    self.pages.as_ref().and_then(|p| p.lines.get(local))
                                {
                                    to_jsp(frame, map);
                                }
                            }
                            None if !fetch.iter().any(|f| source_key(f) == source_key(source)) => {
                                fetch.push(source.clone());
                            }
                            None => {}
                        }
                    } else if let Some(path) = source.path.as_mut() {
                        *path = crate::app::real_path(path);
                    }
                }
                for source in fetch {
                    let key = source_key(&source);
                    // 인라인 스크립트는 화면의 글로 맞춘다(지워진 블록은 내용을 물을 수 없다).
                    if self.pages.is_some()
                        && page_lines::inline_script(&source_text(&source)).is_some()
                    {
                        self.loaded(key, source);
                    } else {
                        self.send(Request::Source(source));
                    }
                }
                self.frame = 0;
                self.send(Request::Scopes);
                self.reveal_frame()
            }
            Request::Scopes => {
                self.scopes = types::scopes(&body);
                // 비싸지 않은 범위(지역 변수 등)는 바로 불러와 펼쳐 둔다.
                let cheap: Vec<i64> = self
                    .scopes
                    .iter()
                    .filter(|s| !s.expensive)
                    .map(|s| s.variables_reference)
                    .collect();
                for reference in cheap {
                    self.send(Request::Variables(reference));
                }
                None
            }
            Request::Variables(reference) => {
                self.children.insert(reference, types::variables(&body));
                None
            }
            Request::Evaluate(expression) => {
                let value = body.get("result").and_then(Value::as_str).unwrap_or("");
                self.push_console(&format!("> {expression}\n{value}\n"));
                None
            }
            Request::LoadedSources => {
                self.scripts = types::sources(&body)
                    .into_iter()
                    .filter(is_remote)
                    .collect();
                self.scripts.sort_by_key(source_key);
                None
            }
            Request::Source(source) => {
                let content = body.get("content").and_then(Value::as_str).unwrap_or("");
                let local = match self.store(&source, content) {
                    Ok(local) => local,
                    Err(e) => {
                        self.push_console(&format!("[오류] 스크립트를 담지 못했습니다: {e}\n"));
                        return None;
                    }
                };
                if let Some(session) = &mut self.session {
                    session.map_source(&local, &source);
                }
                let key = source_key(&source);
                // js-debug면 이 스크립트를 만든 JSP를 찾아 그 JSP의 중단점을 건다.
                let map = self
                    .pages
                    .is_some()
                    .then(|| self.match_page(&source, content));
                if let Some(pages) = &mut self.pages {
                    pages.asking.remove(&key);
                }
                self.placed(&key, local, map)
            }
            Request::PageScripts(key) => {
                // 화면의 글이 알려 온 블록과 같은 수면 차례로 짝짓는다(화면에는 스크립트가 끼워 넣은 것도
                // 있어 수가 다르면 어느 것이 블록인지 모른다).
                let texts = body
                    .get("result")
                    .and_then(Value::as_str)
                    .and_then(page_lines::page_scripts);
                let inline = self.pages.as_mut()?.inline.get_mut(&key)?;
                let starts: Vec<(usize, usize)> = inline.starts.iter().copied().collect();
                match texts.filter(|t| t.len() == starts.len()) {
                    Some(texts) => {
                        let blocks: Vec<(usize, usize, &str)> = starts
                            .iter()
                            .zip(&texts)
                            .map(|(&(line, column), text)| (line, column, text.as_str()))
                            .collect();
                        self.place_blocks(&key, &blocks)
                    }
                    None => {
                        if std::mem::take(&mut inline.again) {
                            self.send(Request::PageScripts(key));
                        } else {
                            self.page_fallback(&key);
                        }
                        None
                    }
                }
            }
            Request::CdpProxy => {
                self.proxy_answered(Proxy::from_json(&body));
                None
            }
        }
    }

    /// 화면 스크립트 블록이 알려 왔다. 처음 보는 화면이면 글을 묻는다(묻는 중이면 답을 받은 뒤 다시 묻게
    /// 한다). 인라인 스크립트가 아니면 그 소스의 내용을 묻는다.
    fn loaded(&mut self, key: String, source: Source) {
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        let Some((_, line, column)) = page_lines::inline_script(&source_text(&source)) else {
            if pages.asking.insert(key) {
                self.send(Request::Source(source));
            }
            return;
        };
        let asking = pages.asking.insert(key.clone());
        let inline = pages.inline.entry(key.clone()).or_default();
        inline.starts.insert((line, column));
        inline.live = Some(source);
        if asking {
            self.ask_page(key);
        } else {
            inline.again = true;
        }
    }

    /// 화면의 블록 글을 묻는다: CDP 프록시가 있으면 따로 받고(처음이면 프록시부터 묻는다), 없으면 화면에서
    /// 읽는다.
    fn ask_page(&mut self, key: String) {
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        let proxy = match &mut pages.proxy {
            ProxyState::Ready(proxy) => proxy.clone(),
            ProxyState::Asking(waiting) => return waiting.push(key),
            ProxyState::Unknown => {
                pages.proxy = ProxyState::Asking(vec![key]);
                return self.send(Request::CdpProxy);
            }
            ProxyState::Missing => return self.send(Request::PageScripts(key)),
        };
        let starts: Vec<(usize, usize)> = pages
            .inline
            .get(&key)
            .map(|inline| inline.starts.iter().copied().collect())
            .unwrap_or_default();
        let page = shown(&key);
        self.spawn_cdp(move || {
            // 화면을 다 읽은 뒤에 받는다(HTML이 다 왔다). 읽는 중에 멈춰 있으면(다시 부른 화면의 맨 위 코드에서
            // 멈췄을 때) CDP가 HTML에 답하지 않으므로, 오래 읽는 중이면 미뤄 둔다(멈춰 있으면 이어 간 뒤에 받는다).
            if cdp::wait_loaded(&proxy, HTML_WAIT) == Ok(false) {
                return PageEvent::Deferred(key);
            }
            // 원래 HTML에서 블록 자리의 글을 자른다. 프레임 안 화면처럼 HTML을 받지 못하면(자리가 맞지 않아도)
            // 블록마다 브라우저의 디버거에서 받는다.
            let from_html = cdp::page_html(&proxy, &page).ok().and_then(|html| {
                page_lines::scripts_at(&html, &starts).map(|blocks| {
                    blocks
                        .into_iter()
                        .map(|(line, column, text)| (line, column, text.to_owned()))
                        .collect()
                })
            });
            let blocks = match from_html {
                Some(blocks) => Ok(blocks),
                None => cdp::script_blocks(&proxy, &page, &starts),
            };
            PageEvent::Blocks(key, blocks)
        });
    }

    /// CDP 프록시로 할 일을 따로 돌린다(UI를 막지 않는다). 결과는 `poll`에서 받는다.
    fn spawn_cdp(&self, work: impl FnOnce() -> PageEvent + Send + 'static) {
        let Some(pages) = &self.pages else {
            return;
        };
        let (sender, ctx) = (pages.events.0.clone(), self.ctx.clone());
        thread::spawn(move || {
            let _ = sender.send(work());
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }

    /// 화면을 다 읽지 않아 HTML을 받지 않았다: 멈춰 있으면 이어 갈 때까지 미루고, 아니면 다시 묻는다.
    fn defer_page(&mut self, key: String) {
        let stopped = self.is_stopped();
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        if !stopped {
            self.ask_page(key);
        } else if !pages.deferred.contains(&key) {
            pages.deferred.push(key);
        }
    }

    /// 대상이 이어 갔다: 미룬 HTML을 받고, 사용자가 계속을 눌렀으면 미룬 다시 부르기를 한다.
    fn continued(&mut self) {
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        let deferred = std::mem::take(&mut pages.deferred);
        let reload: Vec<(String, cdp::Target)> = if std::mem::take(&mut pages.continuing) {
            pages
                .reloads
                .iter()
                .filter_map(|(key, reload)| match reload {
                    Reload::AfterContinue(target) => Some((key.clone(), target.clone())),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        for key in deferred {
            self.ask_page(key);
        }
        for (key, target) in reload {
            self.reload_page(key, target);
        }
    }

    /// 화면(`key`)을 맞춘 담은 파일에 JSP에서 온 중단점을 보냈는지.
    fn has_page_breakpoints(&self, key: &str) -> bool {
        let (Some(pages), Some(local)) = (&self.pages, self.stored.get(key)) else {
            return false;
        };
        pages
            .sent
            .get(local)
            .is_some_and(|origins| !origins.is_empty())
    }

    /// 화면(`key`)을 처음 맞췄다: 화면을 다 읽기를 기다려 다시 불러도 되는지 본다(화면마다 한 번).
    fn check_reload(&mut self, key: &str) {
        let has_breakpoints = self.has_page_breakpoints(key);
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        if !pages.inline.contains_key(key) || pages.reloads.contains_key(key) {
            return;
        }
        let ProxyState::Ready(proxy) = &pages.proxy else {
            // 옛 js-debug(CDP 프록시 없음): 폼을 보내 연 화면인지 알 수 없어 다시 부르지 않는다.
            pages.reloads.insert(key.to_owned(), Reload::Done);
            if has_breakpoints {
                self.push_console(&format!(
                    "[화면을 다시 부르지 않았습니다: {}] 이 js-debug에는 CDP 프록시가 없습니다. 화면을 불러올 때 \
                     도는 코드에 건 JSP 중단점은 화면을 다시 불러야 걸립니다.\n",
                    shown(key)
                ));
            }
            return;
        };
        let (proxy, page, key) = (proxy.clone(), shown(key), key.to_owned());
        pages.reloads.insert(key.clone(), Reload::Checking);
        self.spawn_cdp(move || {
            let loaded = cdp::page_loaded(&proxy, &page, LOAD_WAIT);
            PageEvent::Loaded(key, loaded)
        });
    }

    /// 화면을 다 읽었다: 다시 불러도 되면 맞춘 중단점을 다시 보내 그 답을 받은 뒤 부른다(프레임 안 화면은 그
    /// 프레임만). 폼을 보내 연 화면, 처음 주소에서 다른 화면으로 간 프레임은 부르지 않고 알린다.
    fn on_page_loaded(&mut self, key: String, loaded: Result<cdp::Loaded, String>) {
        let target = match loaded {
            Ok(cdp::Loaded::Reload(target)) => Ok(target),
            Ok(cdp::Loaded::FormSubmit) => {
                Err("폼을 보내 연 화면이라 다시 부르면 폼을 다시 보낼 수 있습니다".to_owned())
            }
            Ok(cdp::Loaded::FrameMoved) => Err(
                "프레임이 처음 주소(src)에서 링크나 폼으로 다른 화면으로 가 있어 같은 화면을 다시 열 수 \
                 없을 수 있습니다"
                    .to_owned(),
            ),
            Err(e) => Err(e),
        };
        let has_breakpoints = self.has_page_breakpoints(&key);
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        let target = match target {
            Ok(target) => target,
            Err(why) => {
                pages.reloads.insert(key.clone(), Reload::Done);
                if has_breakpoints {
                    self.push_console(&format!(
                        "[화면을 다시 부르지 않았습니다: {}] {why}. 화면을 불러올 때 도는 코드에 건 JSP 중단점은 \
                         화면을 다시 불러야 걸립니다.\n",
                        shown(&key)
                    ));
                }
                return;
            }
        };
        // 블록이 더 와서 다시 맞추는 중이면 그 맞춤이 보낸 중단점의 답을 기다린다.
        if pages.asking.contains(&key) {
            pages.reloads.insert(key, Reload::Deciding(target));
        } else if has_breakpoints {
            pages.reloads.insert(key.clone(), Reload::Deciding(target));
            if let Some(local) = self.stored.get(&key).cloned() {
                self.send_page(&local);
            }
        } else {
            pages.reloads.insert(key, Reload::Done);
        }
    }

    /// 어댑터가 담은 파일(`local`)의 중단점에 답했다: 다시 부르기로 한 화면이면 이제 부른다(멈춰 있으면
    /// 사용자가 계속을 누른 뒤에).
    fn breakpoints_answered(&mut self, local: &Path) {
        let Some(key) = self
            .stored
            .iter()
            .find(|(_, path)| *path == local)
            .map(|(key, _)| key.clone())
        else {
            return;
        };
        let (has_breakpoints, stopped) = (self.has_page_breakpoints(&key), self.is_stopped());
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        let Some(Reload::Deciding(target)) = pages.reloads.get(&key).cloned() else {
            return;
        };
        if pages.asking.contains(&key) {
            return;
        }
        if !has_breakpoints {
            pages.reloads.insert(key, Reload::Done);
        } else if stopped {
            pages.reloads.insert(key, Reload::AfterContinue(target));
        } else {
            self.reload_page(key, target);
        }
    }

    /// 화면을 다시 부른다(맨 위 화면이나 그 프레임, 한 번).
    fn reload_page(&mut self, key: String, target: cdp::Target) {
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        pages.reloads.insert(key.clone(), Reload::Done);
        let ProxyState::Ready(proxy) = &pages.proxy else {
            return;
        };
        let proxy = proxy.clone();
        self.spawn_cdp(move || {
            let result = cdp::reload(&proxy, &target);
            PageEvent::Reloaded(key, result)
        });
    }

    /// CDP 프록시 답을 받았다: 기다리던 화면들의 글을 묻는다.
    fn proxy_answered(&mut self, proxy: Option<Proxy>) {
        let Some(pages) = self.pages.as_mut() else {
            return;
        };
        let state = proxy.map_or(ProxyState::Missing, ProxyState::Ready);
        let waiting = match std::mem::replace(&mut pages.proxy, state) {
            ProxyState::Asking(waiting) => waiting,
            _ => Vec::new(),
        };
        for key in waiting {
            self.ask_page(key);
        }
    }

    /// 따로 받은 화면의 블록들로 맞춘다. 받지 못했으면(그사이 화면이 바뀌었다) 화면에서 글을 읽는다.
    fn on_page_blocks(
        &mut self,
        key: String,
        blocks: Result<Vec<(usize, usize, String)>, String>,
    ) -> Option<DebugNotice> {
        match blocks {
            Ok(blocks) if !blocks.is_empty() => {
                let blocks: Vec<(usize, usize, &str)> = blocks
                    .iter()
                    .map(|(line, column, text)| (*line, *column, text.as_str()))
                    .collect();
                self.place_blocks(&key, &blocks)
            }
            _ => {
                self.send(Request::PageScripts(key));
                None
            }
        }
    }

    /// 화면(`key`)의 블록들(시작 줄·칸은 1부터, 글)을 화면 줄에 놓아 담고, 블록마다 JSP를 찾는다. 담은 파일은
    /// 살아 있는 블록으로 잇는다(중단점이 화면 주소로 걸려 모든 블록에 든다). 묻는 사이 블록이 더 왔으면
    /// 다시 묻는다.
    fn place_blocks(&mut self, key: &str, blocks: &[(usize, usize, &str)]) -> Option<DebugNotice> {
        let inline = self.pages.as_mut()?.inline.get_mut(key)?;
        let again = std::mem::take(&mut inline.again);
        let live = inline.live.clone()?;
        let named = Source {
            name: Some(key.to_owned()),
            path: Some(PathBuf::from(key)),
            source_reference: live.source_reference,
        };
        let local = match self.store(&named, &page_lines::assemble(blocks)) {
            Ok(local) => local,
            Err(e) => {
                self.push_console(&format!("[오류] 스크립트를 담지 못했습니다: {e}\n"));
                return None;
            }
        };
        if let Some(session) = &mut self.session {
            session.map_source(&local, &live);
        }
        let map = self
            .page_files()
            .map(|files| PageMap::find(blocks, &files, &shown(key)));
        if again {
            self.ask_page(key.to_owned());
        } else if let Some(pages) = &mut self.pages {
            pages.asking.remove(key);
        }
        self.placed(key, local, map)
    }

    /// 화면에서 글을 읽지 못했다: 살아 있는 블록의 내용으로 맞춘다.
    fn page_fallback(&mut self, key: &str) {
        let live = self
            .pages
            .as_ref()
            .and_then(|pages| pages.inline.get(key)?.live.clone());
        match live {
            Some(live) => self.send(Request::Source(live)),
            None => {
                if let Some(pages) = &mut self.pages {
                    pages.asking.remove(key);
                }
            }
        }
    }

    /// 담은 화면 스크립트(`key`)의 JSP 맞춤을 바꾸고 중단점을 다시 보낸다(화면을 다시 불렀으면 소스 번호가
    /// 바뀌었다). 멈춘 프레임이 이 소스면 담은 파일(JSP를 찾았으면 JSP)을 가리키게 한다.
    fn placed(
        &mut self,
        key: &str,
        local: PathBuf,
        map: Option<Option<PageMap>>,
    ) -> Option<DebugNotice> {
        if let Some(map) = map {
            if let Some(pages) = &mut self.pages {
                match map {
                    Some(map) => pages.lines.insert(local.clone(), map),
                    None => pages.lines.remove(&local),
                };
            }
            self.send_page(&local);
            self.check_reload(key);
        }
        let map = self
            .pages
            .as_ref()
            .and_then(|p| p.lines.get(&local))
            .cloned();
        for frame in &mut self.frames {
            let Some(frame_source) = frame.source.as_mut() else {
                continue;
            };
            if is_remote(frame_source) && source_key(frame_source) == key {
                frame_source.path = Some(local.clone());
                if let Some(map) = &map {
                    to_jsp(frame, map);
                }
            }
        }
        if self.open_after.as_deref() == Some(key) {
            self.open_after = None;
            return Some(DebugNotice::Reveal(local, 0));
        }
        self.reveal_frame()
    }

    /// "디버그" 창: 도구 막대, 호출 스택, 변수.
    pub fn panel_ui(&mut self, ui: &mut egui::Ui) -> Option<DebugNotice> {
        let mut step = None;
        let mut script = None;
        let active = self.is_active() && self.stopping.is_none();
        let stopped = active && self.is_stopped();
        ui.horizontal(|ui| {
            for (label, shortcut, how, enabled) in [
                ("계속", "F5", Step::Continue, stopped),
                ("한 줄씩", "F10", Step::Over, stopped),
                ("들어가기", "F11", Step::In, stopped),
                ("나오기", "Shift+F11", Step::Out, stopped),
            ] {
                if ui
                    .add_enabled(enabled, egui::Button::new(label))
                    .on_hover_text(shortcut)
                    .clicked()
                {
                    step = Some(how);
                }
            }
            if ui
                .add_enabled(active && !stopped, egui::Button::new("일시 정지"))
                .on_hover_text("F6")
                .clicked()
            {
                self.pause();
            }
            if ui
                .add_enabled(active, egui::Button::new("멈춤"))
                .on_hover_text("Shift+F5")
                .clicked()
            {
                self.stop();
            }
            // 로컬 파일이 없는 스크립트(JSP 인라인 스크립트 등)를 열어 중단점을 건다. 창이 낮아도 보이게
            // 펼침 메뉴로 두고, 펼칠 때 목록을 묻는다.
            let loadable = active
                && self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.capabilities().supports_loaded_sources_request);
            if loadable {
                let menu = ui.menu_button("화면 스크립트", |ui| {
                    if self.scripts.is_empty() {
                        ui.weak("프로젝트 파일이 없는 스크립트가 아직 없습니다");
                    }
                    for source in &self.scripts {
                        let mut name = shown(source.name.as_deref().unwrap_or("(이름 없음)"));
                        // JSP를 찾은 스크립트는 그 JSP 이름도 보인다(블록마다 다르면 모두).
                        if let Some(map) = self
                            .stored
                            .get(&source_key(source))
                            .and_then(|local| self.pages.as_ref()?.lines.get(local))
                        {
                            let jsps: Vec<String> = map
                                .files()
                                .iter()
                                .filter_map(|f| f.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .collect();
                            name.push_str(&format!(" → {}", jsps.join(", ")));
                        }
                        if ui.selectable_label(false, name).clicked() {
                            script = Some(source.clone());
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("새로 고침").clicked() {
                        self.send(Request::LoadedSources);
                    }
                });
                if menu.response.clicked() {
                    self.send(Request::LoadedSources);
                }
                menu.response.on_hover_text(
                    "브라우저가 불러온 스크립트 중 프로젝트 파일이 없는 것. 열어서 중단점을 건다",
                );
            }
        });
        if let Some(how) = step {
            self.resume(how);
        }
        match (active, stopped) {
            (false, _) if self.stopping.is_some() => ui.weak("멈추는 중…"),
            (false, _) => ui.weak("디버그 중이 아닙니다. F5로 시작하세요."),
            (true, false) => ui.weak(format!("실행 중: {}", self.name)),
            (true, true) => ui.label(format!("멈춤: {}", self.name)),
        };
        ui.separator();

        let mut notice = None;
        ui.strong("호출 스택");
        let mut picked = None;
        for (i, frame) in self.frames.iter().enumerate() {
            let place = frame
                .source
                .as_ref()
                .and_then(|s| {
                    s.name.clone().or_else(|| {
                        s.path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                })
                .unwrap_or_default();
            let text = format!("{}  {}:{}", frame.name, shown(&place), frame.line);
            if ui.selectable_label(i == self.frame, text).clicked() {
                picked = Some(i);
            }
        }
        if let Some(i) = picked.filter(|&i| i != self.frame) {
            self.frame = i;
            self.scopes.clear();
            self.children.clear();
            self.send(Request::Scopes);
            notice = self.reveal_frame();
        } else if picked.is_some() {
            notice = self.reveal_frame();
        }
        ui.separator();

        if let Some(source) = script {
            let key = source_key(&source);
            match self.stored.get(&key) {
                Some(local) => notice = Some(DebugNotice::Reveal(local.clone(), 0)),
                None => {
                    self.open_after = Some(key);
                    self.send(Request::Source(source));
                }
            }
        }

        ui.strong("변수");
        let mut load = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("debug_variables")
            .auto_shrink(false)
            .show(ui, |ui| {
                for scope in &self.scopes {
                    egui::CollapsingHeader::new(&scope.name)
                        .id_salt(("scope", scope.variables_reference))
                        .default_open(!scope.expensive)
                        .show(ui, |ui| {
                            variables_ui(
                                ui,
                                scope.variables_reference,
                                &self.children,
                                &self.requests,
                                &mut load,
                            );
                        });
                }
            });
        for reference in load {
            self.send(Request::Variables(reference));
        }
        notice
    }

    /// "디버그 출력" 창: 대상의 출력과 식 계산.
    pub fn console_ui(&mut self, ui: &mut egui::Ui) {
        let mut evaluate = false;
        ui.horizontal(|ui| {
            let label = ui.label("식");
            let field = ui
                .add_enabled(
                    self.is_stopped(),
                    egui::TextEdit::singleline(&mut self.expression)
                        .hint_text("멈춘 자리에서 계산할 식")
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(label.id);
            evaluate = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
        if evaluate && !self.expression.trim().is_empty() {
            let expression = std::mem::take(&mut self.expression);
            self.send(Request::Evaluate(expression));
        }
        egui::ScrollArea::vertical()
            .id_salt("debug_console")
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&self.console).monospace()).wrap());
            });
    }

    #[cfg(test)]
    pub fn console(&self) -> &str {
        &self.console
    }
}

/// 빌드 명령을 플랫폼 셸로 돌려 출력을 모은다. 실패(0이 아닌 종료 코드)면 Err.
fn run_build(line: &str, dir: &Path) -> Result<String, String> {
    let output = run_hidden(line, dir, &BTreeMap::new())
        .map_err(|e| format!("빌드 명령을 실행하지 못했습니다: {e}\n"))?;
    let text = output.stdout + &output.stderr;
    if output.status.success() {
        Ok(text)
    } else {
        Err(format!("{text}[빌드 실패: {}]\n", output.status))
    }
}

/// 숨겨 돌린 명령의 결과. 출력은 UTF-8·CP949를 가려 읽는다.
pub(crate) struct Output {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// 명령 줄을 콘솔 창 없이 플랫폼 셸로 돌려 끝날 때까지 기다린다(빌드·의존성 풀기·저장 시 작업).
pub(crate) fn run_hidden(
    line: &str,
    dir: &Path,
    env: &BTreeMap<String, String>,
) -> std::io::Result<Output> {
    let mut command = shell(line);
    command.current_dir(dir).envs(env);
    ferrite_lsp::client::hide_console(&mut command);
    let output = command.output()?;
    let decode = |bytes: &[u8]| {
        ferrite_core::text_file::decode(bytes)
            .map(|c| c.text)
            .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned())
    };
    Ok(Output {
        status: output.status,
        stdout: decode(&output.stdout),
        stderr: decode(&output.stderr),
    })
}

/// 출력의 끝 몇 줄(빈 줄은 빼고 ` / `로 잇는다). 실패를 한 줄로 알릴 때.
pub(crate) fn tail(text: &str, lines: usize) -> String {
    let mut tail: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rev()
        .take(lines)
        .collect();
    tail.reverse();
    tail.join(" / ")
}

/// 구성의 어댑터를 띄울 방법. `debug.toml`의 `[[adapter]]`가 먼저이고, 없으면 목록의 어댑터(설치한 것,
/// 없으면 PATH).
pub(crate) fn resolve_adapter(name: &str, servers: &Path, root: &Path) -> Result<Adapter, String> {
    let custom = debug_config::load_adapters(root)?;
    match custom.iter().find(|a| a.name == name) {
        Some(adapter) => custom_adapter(adapter, root),
        None => adapters::resolve(name, servers, root),
    }
}

/// 사용자 어댑터: command만이면 표준 입출력, address만이면 붙기, 둘 다면 띄우고 그 주소에 붙기.
fn custom_adapter(adapter: &CustomAdapter, root: &Path) -> Result<Adapter, String> {
    let address = adapter.socket()?;
    let root_text = root.to_string_lossy();
    let command = adapter
        .command
        .as_deref()
        .map(|c| c.trim().replace("${workspaceFolder}", &root_text))
        .filter(|c| !c.is_empty());
    let program = match command {
        None => None,
        // 경로로 적었으면 프로젝트 루트 기준, 이름만 적었으면 PATH에서 찾는다.
        Some(command) if command.contains(['/', '\\']) => Some(root.join(command)),
        Some(command) => Some(ferrite_lsp::client::find_program(&command).ok_or_else(|| {
            format!(
                "디버그 어댑터 {}의 {command}을(를) PATH에서 찾지 못했습니다",
                adapter.name
            )
        })?),
    };
    let args = adapter.resolved_args(root);
    let env: Vec<(String, String)> = adapter.env.clone().into_iter().collect();
    let cwd = Some(root.to_path_buf());
    match (program, address) {
        (Some(program), None) => Ok(Adapter::Stdio {
            program,
            args,
            cwd,
            env,
        }),
        (None, Some(address)) => Ok(Adapter::Tcp(address)),
        (Some(program), Some(address)) => Ok(Adapter::Listening {
            program,
            args,
            cwd,
            env,
            address: Some(address),
        }),
        (None, None) => Err(format!(
            "디버그 어댑터 {}에는 command나 address가 있어야 합니다",
            adapter.name
        )),
    }
}

/// 명령 줄을 실행할 플랫폼 셸(터미널의 실행 구성과 같다).
pub(crate) fn shell(line: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd.exe");
        command
            .args(["/d", "/s", "/c"])
            .raw_arg(format!("\"{line}\""));
        command
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
        let mut command = Command::new(shell);
        command.args(["-l", "-c", line]);
        command
    }
}

#[derive(Clone, Copy)]
pub enum Step {
    Continue,
    Over,
    In,
    Out,
}

/// 변수 목록. 펼칠 수 있는 변수는 펼칠 때 자식을 불러온다(`load`에 모아 그린 뒤 요청한다).
fn variables_ui(
    ui: &mut egui::Ui,
    reference: i64,
    children: &HashMap<i64, Vec<Variable>>,
    requests: &HashMap<i64, Request>,
    load: &mut Vec<i64>,
) {
    let Some(variables) = children.get(&reference) else {
        let requested = requests
            .values()
            .any(|r| matches!(r, Request::Variables(x) if *x == reference));
        if !requested {
            load.push(reference);
        }
        ui.weak("불러오는 중…");
        return;
    };
    if variables.is_empty() {
        ui.weak("(없음)");
    }
    for variable in variables {
        let text = format!("{} = {}", variable.name, variable.value);
        let hover = variable.type_name.as_deref().unwrap_or("");
        if variable.variables_reference > 0 {
            egui::CollapsingHeader::new(text)
                .id_salt(("variable", variable.variables_reference))
                .show(ui, |ui| {
                    variables_ui(ui, variable.variables_reference, children, requests, load);
                })
                .header_response
                .on_hover_text(hover);
        } else {
            let label = ui.label(text);
            if !hover.is_empty() {
                label.on_hover_text(hover);
            }
        }
    }
}

#[cfg(test)]
impl Debugger {
    /// 시험용: 어댑터에서 받은 것처럼 그 파일을 담아 둔 소스로 둔다.
    pub(crate) fn mark_stored(&mut self, path: PathBuf) {
        self.stored
            .insert(path.to_string_lossy().into_owned(), path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_toml의_사용자_어댑터를_먼저_고른다() {
        let root = std::env::temp_dir().join(format!("ferrite-custom-dap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".ferrite")).unwrap();
        std::fs::write(
            root.join(".ferrite/debug.toml"),
            "[[adapter]]\nname = \"vendor-stdio\"\ncommand = \"tools/dap\"\nargs = [\"--root\", \"${workspaceFolder}\"]\n\
             env = { VENDOR = \"1\" }\n\n\
             [[adapter]]\nname = \"vendor-tcp\"\naddress = \"127.0.0.1:4711\"\n\n\
             [[adapter]]\nname = \"vendor-server\"\ncommand = \"${workspaceFolder}/tools/server\"\naddress = \"127.0.0.1:4712\"\n\n\
             [[adapter]]\nname = \"없는 프로그램\"\ncommand = \"ferrite-no-such-dap\"\n",
        )
        .unwrap();
        let servers = root.join("servers");
        let resolve = |name: &str| resolve_adapter(name, &servers, &root);
        match resolve("vendor-stdio").unwrap() {
            Adapter::Stdio {
                program, args, env, ..
            } => {
                assert_eq!(program, root.join("tools/dap"));
                assert_eq!(
                    args,
                    ["--root".to_owned(), root.to_string_lossy().into_owned()]
                );
                assert_eq!(env, [("VENDOR".to_owned(), "1".to_owned())]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            resolve("vendor-tcp").unwrap(),
            Adapter::Tcp(a) if a.port() == 4711
        ));
        match resolve("vendor-server").unwrap() {
            Adapter::Listening {
                program, address, ..
            } => {
                assert_eq!(program, root.join("tools/server"));
                assert_eq!(address.map(|a| a.port()), Some(4712));
            }
            other => panic!("{other:?}"),
        }
        let error = resolve("없는 프로그램").err().unwrap();
        assert!(error.contains("ferrite-no-such-dap"), "{error}");
        // 적지 않은 이름은 목록의 어댑터로 고른다.
        let error = resolve("vendor-없음").err().unwrap();
        assert!(error.contains("알 수 없는 디버그 어댑터"), "{error}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn source(path: &str, reference: i64) -> Source {
        Source {
            name: Some(path.into()),
            path: Some(path.into()),
            source_reference: reference,
        }
    }

    #[test]
    fn 화면_스크립트를_담을_이름과_로컬_파일이_없는_소스를_가린다() {
        let page = source("127.0.0.1\u{a789}8080/app/list.do\u{a789}5:9", 7);
        assert!(is_remote(&page));
        assert_eq!(
            shown(page.name.as_deref().unwrap()),
            "127.0.0.1:8080/app/list.do:5:9"
        );
        assert_eq!(stored_name(&page, "<!doctype html>"), "list.do_5_9.html");
        assert_eq!(stored_name(&page, "\n\nvar x;"), "list.do_5_9.js");
        assert_eq!(stored_name(&source("eval-1", 3), "x()"), "eval-1.js");
        assert_eq!(stored_name(&source("a/b.mjs", 3), "x()"), "b.mjs");
        assert_eq!(stored_name(&source("/", 3), ""), "script.js");
        // 로컬 파일이 있거나 번호가 없으면 어댑터에 묻지 않는다.
        let here = std::env::current_dir().unwrap().join("Cargo.toml");
        assert!(!is_remote(&source(&here.to_string_lossy(), 3)));
        assert!(!is_remote(&source("x.js", 0)));
        // 인라인 스크립트는 블록 자리(`꞉5:9`)를 뗀 화면 주소로 묶고, 다른 소스는 경로 그대로다.
        assert_eq!(source_key(&page), "127.0.0.1\u{a789}8080/app/list.do");
        assert_eq!(
            source_key(&source("127.0.0.1\u{a789}8080/app/js/app.js", 3)),
            "127.0.0.1\u{a789}8080/app/js/app.js"
        );
    }

    /// 가짜 CDP 프록시(웹소켓): 손짓에 101로 답하고, 프레임 트리(맨 위 화면 list.do)와 화면의 원래
    /// HTML(`html`), 다 읽은 화면, 탐색 기록(지금 화면을 연 방식 `transition`)을 준다. 받은 명령 이름을
    /// 돌려준다.
    fn fake_cdp(
        html: &'static str,
        transition: &'static str,
    ) -> (u16, std::sync::mpsc::Receiver<String>) {
        fake_cdp_frame(html, transition, &[], true)
    }

    /// `fake_cdp`에 자식 프레임(detail.do)을 더한 것. 실제 브라우저처럼 자식 프레임의 원래 HTML은 주지 않고,
    /// 블록 글은 디버거로(시작 줄 → 글, `frame_blocks`) 준다. 자식 프레임이 처음 주소(src)를 보이는지는
    /// `from_src`다.
    fn fake_cdp_frame(
        html: &'static str,
        transition: &'static str,
        frame_blocks: &'static [(u64, &'static str)],
        from_src: bool,
    ) -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen, methods) = channel();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let seen = seen.clone();
                thread::spawn(move || {
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                        line.clear();
                    }
                    stream
                        .write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n")
                        .unwrap();
                    loop {
                        let mut head = [0u8; 2];
                        if reader.read_exact(&mut head).is_err() {
                            return;
                        }
                        let mut len = usize::from(head[1] & 0x7f);
                        if len == 126 {
                            let mut short = [0u8; 2];
                            reader.read_exact(&mut short).unwrap();
                            len = usize::from(u16::from_be_bytes(short));
                        }
                        let mut mask = [0u8; 4];
                        reader.read_exact(&mut mask).unwrap();
                        let mut payload = vec![0u8; len];
                        reader.read_exact(&mut payload).unwrap();
                        payload
                            .iter_mut()
                            .enumerate()
                            .for_each(|(i, b)| *b ^= mask[i % 4]);
                        let request: Value = serde_json::from_slice(&payload).unwrap();
                        let method = request["method"].as_str().unwrap_or_default().to_owned();
                        let params = &request["params"];
                        let result = match method.as_str() {
                            "Page.getFrameTree" => json!({ "frameTree": {
                                "frame": { "id": "top", "url": "http://127.0.0.1:8080/app/list.do" },
                                "childFrames": [{ "frame": {
                                    "id": "child", "url": "http://127.0.0.1:8080/app/detail.do" } }] } }),
                            "Runtime.evaluate" if params["expression"] == "document.readyState" => {
                                json!({ "result": { "type": "string", "value": "complete" } })
                            }
                            "Runtime.evaluate" => {
                                json!({ "result": { "type": "boolean", "value": from_src } })
                            }
                            "Page.createIsolatedWorld" => json!({ "executionContextId": 7 }),
                            "Page.getNavigationHistory" => json!({ "currentIndex": 0, "entries": [
                                { "id": 1, "url": "http://127.0.0.1:8080/app/list.do",
                                  "transitionType": transition }] }),
                            "Page.reload" => json!({}),
                            "Page.navigate" => json!({ "frameId": params["frameId"] }),
                            "Debugger.setBreakpointByUrl" => {
                                let line = params["lineNumber"].as_u64().unwrap_or_default() + 1;
                                json!({ "breakpointId": format!("bp{line}"), "locations": [
                                    { "scriptId": format!("{line}"), "lineNumber": line, "columnNumber": 0 }] })
                            }
                            "Debugger.getScriptSource" => {
                                let line: u64 =
                                    params["scriptId"].as_str().unwrap().parse().unwrap();
                                let text = frame_blocks
                                    .iter()
                                    .find(|(l, _)| *l == line)
                                    .map(|(_, t)| *t);
                                json!({ "scriptSource": text })
                            }
                            "Debugger.removeBreakpoint" => json!({}),
                            _ if params["frameId"] == "child" => json!(null),
                            _ => json!({ "content": html, "base64Encoded": false }),
                        };
                        let _ = seen.send(method);
                        // 자식 프레임의 원래 HTML은 실제 브라우저처럼 주지 않는다.
                        let body = if result.is_null() {
                            json!({ "id": request["id"], "error": { "code": -32000,
                                "message": "Content unavailable. Resource was not cached" } })
                        } else {
                            json!({ "id": request["id"], "result": result })
                        }
                        .to_string();
                        let mut frame = vec![0x81u8];
                        if body.len() < 126 {
                            frame.push(body.len() as u8);
                        } else {
                            frame.push(126);
                            frame.extend((body.len() as u16).to_be_bytes());
                        }
                        frame.extend(body.as_bytes());
                        stream.write_all(&frame).unwrap();
                    }
                });
            }
        });
        (port, methods)
    }

    /// 각본대로 답하는 가짜 js-debug: 시작한 뒤 화면의 인라인 스크립트 블록들(소스, 화면에서의 글)을
    /// 불러왔다고 알린다. 실제 js-debug처럼 내용은 마지막 블록만 주고, 화면에서 읽히는 글(`dom`, 없으면
    /// 블록의 글)은 `evaluate`로 준다. `cdp`(가짜 CDP 프록시의 포트)가 있으면 CDP 프록시를 알려 준다. 마지막
    /// 블록으로 건 중단점에 `stop`(블록 번호, 화면 줄)이 들면 그 블록의 그 줄에서 멈춘다. 받은 중단점은 (소스
    /// 번호 또는 경로, 줄들)로 돌려준다.
    fn fake_js_debug(
        blocks: Vec<(Source, &'static str)>,
        dom: Option<Vec<&'static str>>,
        cdp_port: Option<u16>,
        stop: (usize, i64),
    ) -> (
        std::net::SocketAddr,
        std::sync::mpsc::Receiver<(String, Vec<i64>)>,
    ) {
        use ferrite_lsp::transport::{read_message, write_message};
        use serde_json::json;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, received) = channel();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut writer = &stream;
            let mut seq = 0;
            let mut send = |message: Value| {
                seq += 1;
                let mut message = message;
                message["seq"] = json!(seq);
                write_message(&mut writer, &serde_json::to_vec(&message).unwrap()).unwrap();
            };
            let live = blocks.last().unwrap().0.clone();
            let (_, line, column) =
                page_lines::inline_script(live.name.as_deref().unwrap()).unwrap();
            let live_content = page_lines::assemble(&[(line, column, blocks.last().unwrap().1)]);
            let texts: Vec<&str> =
                dom.unwrap_or_else(|| blocks.iter().map(|(_, text)| *text).collect());
            let scripts = serde_json::to_string(&serde_json::to_string(&texts).unwrap()).unwrap();
            while let Ok(Some(body)) = read_message(&mut reader) {
                let request: Value = serde_json::from_slice(&body).unwrap();
                let command = request["command"].as_str().unwrap_or_default().to_owned();
                let mut body = json!({});
                let mut after = Vec::new();
                match command.as_str() {
                    "initialize" => {
                        body = json!({ "supportsConfigurationDoneRequest": true, "supportsLoadedSourcesRequest": true });
                        after.push(json!({ "type": "event", "event": "initialized", "body": {} }));
                    }
                    "setBreakpoints" => {
                        let source = &request["arguments"]["source"];
                        let lines: Vec<i64> = request["arguments"]["breakpoints"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|b| b["line"].as_i64().unwrap())
                            .collect();
                        let name = match source["sourceReference"].as_i64() {
                            Some(reference) => format!("#{reference}"),
                            None => source["path"].as_str().unwrap_or_default().to_owned(),
                        };
                        body = json!({ "breakpoints": lines.iter().enumerate()
                            .map(|(i, l)| json!({ "id": i + 1, "verified": true, "line": l }))
                            .collect::<Vec<_>>() });
                        if name == format!("#{}", live.source_reference) && lines.contains(&stop.1)
                        {
                            after.push(json!({ "type": "event", "event": "stopped",
                                "body": { "reason": "breakpoint", "threadId": 1 } }));
                        }
                        let _ = sent.send((name, lines));
                    }
                    "configurationDone" => {
                        for (source, _) in &blocks {
                            after.push(json!({ "type": "event", "event": "loadedSource",
                                "body": { "reason": "new", "source": source.to_json() } }));
                        }
                    }
                    "source" => body = json!({ "content": live_content }),
                    "requestCDPProxy" if cdp_port.is_some() => {
                        body = json!({ "host": "127.0.0.1", "port": cdp_port, "path": "/cdp" });
                    }
                    "evaluate" if request["arguments"]["context"] == "clipboard" => {
                        body =
                            json!({ "result": scripts, "type": "string", "variablesReference": 0 });
                    }
                    "stackTrace" => {
                        body = json!({ "stackFrames": [{ "id": 1, "name": "fn_egov_link_page",
                            "source": blocks[stop.0].0.to_json(), "line": stop.1, "column": 5 }] });
                    }
                    "scopes" => body = json!({ "scopes": [] }),
                    "threads" => body = json!({ "threads": [{ "id": 1, "name": "main" }] }),
                    "continue" => after.push(
                        json!({ "type": "event", "event": "continued", "body": { "threadId": 1 } }),
                    ),
                    _ => {}
                }
                send(
                    json!({ "type": "response", "request_seq": request["seq"], "success": true,
                    "command": command, "body": body }),
                );
                for event in after {
                    send(event);
                }
                if command == "disconnect" {
                    break;
                }
            }
        });
        (address, received)
    }

    #[test]
    fn js_debug에서_jsp_중단점을_화면_스크립트_줄로_걸고_멈춘_자리를_jsp로_보인다() {
        let root = std::env::temp_dir().join(format!("ferrite-jsp-debug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let jsp = root.join("src/main/webapp/WEB-INF/jsp/board/BoardList.jsp");
        std::fs::create_dir_all(jsp.parent().unwrap()).unwrap();
        // CP949로 저장한 JSP(지시자 둘, `<c:url>`, 한글 문자열).
        let text = "<%@ page contentType=\"text/html; charset=euc-kr\" %>\r\n<%@ taglib prefix=\"c\" uri=\"http://java.sun.com/jsp/jstl/core\" %>\r\n<script>\r\nfunction fn_egov_link_page(pageNo) {\r\n    alert(\"쪽 \" + pageNo);\r\n    location.href = \"<c:url value='/board/list.do'/>?pageIndex=\" + pageNo;\r\n}\r\n</script>\r\n";
        let (bytes, _, _) = encoding_rs::EUC_KR.encode(text);
        std::fs::write(&jsp, &bytes).unwrap();
        // 그 JSP가 만든 화면의 인라인 스크립트(화면 13줄의 `<script>` 뒤에서 시작해 열 줄 아래). JSP 5줄은
        // 화면 15줄이다.
        static SCRIPT: &str = "\nfunction fn_egov_link_page(pageNo) {\n    alert(\"쪽 \" + pageNo);\n    location.href = \"/app/board/list.do?pageIndex=\" + pageNo;\n}\n";
        let page = source("127.0.0.1\u{a789}8080/app/board/list.do\u{a789}13:9", 7);
        let (address, received) = fake_js_debug(vec![(page, SCRIPT)], None, None, (0, 15));

        let config = DebugConfig {
            name: "화면".into(),
            adapter: "js-debug".into(),
            request: "launch".into(),
            arguments: serde_json::json!({}),
            build: None,
        };
        // 실제 경로가 아닌 꼴로 준다(macOS 임시 폴더의 `/var` 링크, Windows CI의 짧은 이름 `RUNNER~1`처럼).
        let breakpoints = BTreeMap::from([(
            root.join("src/main/webapp/WEB-INF/../WEB-INF/jsp/board/BoardList.jsp"),
            vec![Breakpoint {
                line: 5,
                condition: None,
            }],
        )]);
        let mut debugger = Debugger::default();
        debugger
            .start(
                &config,
                &root,
                None,
                breakpoints,
                Adapter::Tcp(address),
                &egui::Context::default(),
            )
            .unwrap();
        // 담아 둔 화면 스크립트는 앱이 닫힐 때 지운다. 시험은 프로젝트 폴더에 두어 함께 지운다.
        debugger.sources = Some(root.join("sources"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while debugger
            .location()
            .is_none_or(|(path, _)| path.extension().is_none_or(|e| e != "jsp"))
        {
            assert!(
                Instant::now() < deadline,
                "JSP에서 멈추지 않음: {:?}",
                debugger.location()
            );
            debugger.poll();
            thread::sleep(Duration::from_millis(10));
        }
        let (path, line) = debugger.location().unwrap();
        assert_eq!(path, crate::app::real_path(&jsp));
        assert_eq!(line, 4, "JSP 5줄(0부터 4)");
        // JSP는 어댑터에 보내지 않고 화면 스크립트 15줄로 걸었다. 어댑터가 확인한 것을 JSP의 점으로 보인다.
        let sent: Vec<(String, Vec<i64>)> = received.try_iter().collect();
        assert!(
            sent.iter().all(|(name, _)| !name.ends_with(".jsp")),
            "{sent:?}"
        );
        assert!(sent.contains(&("#7".to_owned(), vec![15])), "{sent:?}");
        assert!(
            debugger
                .breakpoint_state(&crate::app::real_path(&jsp), 0)
                .unwrap()
                .verified
        );
        // 호출 스택에는 JSP 이름으로 보인다.
        assert_eq!(
            debugger.frames[0].source.as_ref().unwrap().name.as_deref(),
            Some("BoardList.jsp")
        );

        // JSP의 중단점을 옮기면 화면 스크립트 줄로 다시 건다. 화면에 나오지 않는 줄(지시자)은 걸 곳이 없다.
        let jsp_path = crate::app::real_path(&jsp);
        debugger.set_breakpoints(
            &jsp_path,
            vec![
                Breakpoint {
                    line: 1,
                    condition: None,
                },
                Breakpoint {
                    line: 6,
                    condition: None,
                },
            ],
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let moved = loop {
            debugger.poll();
            if let Some(found) = received.try_iter().find(|(name, _)| name == "#7") {
                break found;
            }
            assert!(Instant::now() < deadline, "다시 걸지 않음");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(moved.1, [16]);
        while !debugger
            .states
            .contains_key(debugger.stored.values().next().unwrap())
        {
            debugger.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!debugger.breakpoint_state(&jsp_path, 0).unwrap().verified);
        assert!(debugger.breakpoint_state(&jsp_path, 1).unwrap().verified);
        debugger.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn js_debug에서_여러_jsp로_합친_화면의_블록마다_jsp_중단점을_건다() {
        let root =
            std::env::temp_dir().join(format!("ferrite-jsp-tiles-debug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let jsp = root.join("src/main/webapp/WEB-INF/jsp");
        let header = jsp.join("layout/header.jsp");
        let body = jsp.join("board/BoardList.jsp");
        for (path, text) in [
            (
                &header,
                "<%@ page contentType=\"text/html; charset=UTF-8\" %>\n<div id=\"header\">머리글</div>\n<script>\nfunction headerClock() {\n  var now = new Date();\n  return now.getTime();\n}\n</script>\n",
            ),
            (
                &body,
                "<%@ page contentType=\"text/html; charset=UTF-8\" %>\n<h1>${title}</h1>\n<script>\nvar ticks = 0;\nfunction tick() {\n  ticks = ticks + 1;\n  headerClock();\n}\nsetInterval(tick, 200);\n</script>\n",
            ),
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        // Tiles로 합친 화면: 머리글 블록(화면 11줄)과 본문 블록(화면 18줄). 실제 js-debug처럼 뒤의 블록이
        // 앞의 것을 지우고 그 번호를 다시 쓴다(머리글 8은 지워진 번호).
        let blocks = vec![
            (
                source("127.0.0.1\u{a789}8080/app/tiles.do\u{a789}11:9", 8),
                "\nfunction headerClock() {\n  var now = new Date();\n  return now.getTime();\n}\n",
            ),
            (
                source("127.0.0.1\u{a789}8080/app/tiles.do\u{a789}18:9", 7),
                "\nvar ticks = 0;\nfunction tick() {\n  ticks = ticks + 1;\n  headerClock();\n}\nsetInterval(tick, 200);\n",
            ),
        ];
        let (address, received) = fake_js_debug(blocks, None, None, (0, 13));
        let config = DebugConfig {
            name: "화면".into(),
            adapter: "js-debug".into(),
            request: "launch".into(),
            arguments: serde_json::json!({}),
            build: None,
        };
        // 머리글 JSP 5줄(`var now`)은 화면 13줄, 본문 JSP 6줄(`ticks = ticks + 1`)은 화면 21줄이다.
        let breakpoints = BTreeMap::from([
            (
                header.clone(),
                vec![Breakpoint {
                    line: 5,
                    condition: None,
                }],
            ),
            (
                body.clone(),
                vec![Breakpoint {
                    line: 6,
                    condition: None,
                }],
            ),
        ]);
        let mut debugger = Debugger::default();
        debugger
            .start(
                &config,
                &root,
                None,
                breakpoints,
                Adapter::Tcp(address),
                &egui::Context::default(),
            )
            .unwrap();
        // 담아 둔 화면 스크립트는 앱이 닫힐 때 지운다. 시험은 프로젝트 폴더에 두어 함께 지운다.
        debugger.sources = Some(root.join("sources"));
        let header = crate::app::real_path(&header);
        let body = crate::app::real_path(&body);
        let deadline = Instant::now() + Duration::from_secs(10);
        while debugger.location().is_none_or(|(path, _)| path != header) {
            assert!(
                Instant::now() < deadline,
                "머리글 JSP에서 멈추지 않음: {:?}",
                debugger.location()
            );
            debugger.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(debugger.location().unwrap().1, 4, "머리글 JSP 5줄");
        assert_eq!(
            debugger.frames[0].source.as_ref().unwrap().name.as_deref(),
            Some("header.jsp")
        );
        // 두 JSP의 중단점을 살아 있는 블록(7)으로 화면 줄에 건다.
        let sent: Vec<(String, Vec<i64>)> = received.try_iter().collect();
        assert!(sent.contains(&("#7".to_owned(), vec![13, 21])), "{sent:?}");
        while debugger.states.is_empty() {
            debugger.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(debugger.breakpoint_state(&header, 0).unwrap().verified);
        assert!(debugger.breakpoint_state(&body, 0).unwrap().verified);
        debugger.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn js_debug에서_반복된_블록마다_jsp_중단점을_걸고_덧붙인_스크립트는_건너뛴다() {
        let root =
            std::env::temp_dir().join(format!("ferrite-jsp-foreach-debug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let jsp = root.join("src/main/webapp/WEB-INF/jsp/board/list.jsp");
        std::fs::create_dir_all(jsp.parent().unwrap()).unwrap();
        // `<c:forEach>` 안의 스크립트: 항목마다 화면에 나온다.
        std::fs::write(
            &jsp,
            "<c:forEach var=\"item\" items=\"${items}\">\n<script>\nregister(\"${item.id}\");\nlistReady = true;\n</script>\n</c:forEach>\n",
        )
        .unwrap();
        // 항목 둘의 블록(화면 2줄·6줄). 화면에는 스크립트가 끝에 덧붙인 인라인 스크립트도 있다(알리지 않는다).
        let blocks = vec![
            (
                source("127.0.0.1\u{a789}8080/app/list.do\u{a789}2:9", 8),
                "\nregister(\"a\");\nlistReady = true;\n",
            ),
            (
                source("127.0.0.1\u{a789}8080/app/list.do\u{a789}6:9", 7),
                "\nregister(\"b\");\nlistReady = true;\n",
            ),
        ];
        let dom = vec![
            "\nregister(\"a\");\nlistReady = true;\n",
            "\nregister(\"b\");\nlistReady = true;\n",
            "\nadded();\n",
        ];
        // 화면의 원래 HTML(CDP 프록시로 받는다). 화면의 글만으로는 덧붙인 스크립트 때문에 수가 달라 짝짓지
        // 못한다.
        static HTML: &str = "<ul>\n<script>\nregister(\"a\");\nlistReady = true;\n</script>\n<script>\nregister(\"b\");\nlistReady = true;\n</script>\n</ul>\n";
        let (cdp_port, _) = fake_cdp(HTML, "link");
        let (address, received) = fake_js_debug(blocks, Some(dom), Some(cdp_port), (1, 7));
        let config = DebugConfig {
            name: "화면".into(),
            adapter: "js-debug".into(),
            request: "launch".into(),
            arguments: serde_json::json!({}),
            build: None,
        };
        // JSP 3줄(`register`)은 화면 3줄과 7줄이다.
        let breakpoints = BTreeMap::from([(
            jsp.clone(),
            vec![Breakpoint {
                line: 3,
                condition: None,
            }],
        )]);
        let mut debugger = Debugger::default();
        debugger
            .start(
                &config,
                &root,
                None,
                breakpoints,
                Adapter::Tcp(address),
                &egui::Context::default(),
            )
            .unwrap();
        // 담아 둔 화면 스크립트는 앱이 닫힐 때 지운다. 시험은 프로젝트 폴더에 두어 함께 지운다.
        debugger.sources = Some(root.join("sources"));
        let jsp = crate::app::real_path(&jsp);
        let deadline = Instant::now() + Duration::from_secs(10);
        while debugger.location().is_none_or(|(path, _)| path != jsp) {
            assert!(
                Instant::now() < deadline,
                "JSP에서 멈추지 않음: {:?}",
                debugger.location()
            );
            debugger.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            debugger.location().unwrap().1,
            2,
            "두 번째 항목의 블록도 JSP 3줄"
        );
        let sent: Vec<(String, Vec<i64>)> = received.try_iter().collect();
        assert!(sent.contains(&("#7".to_owned(), vec![3, 7])), "{sent:?}");
        while debugger.states.is_empty() {
            debugger.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(debugger.breakpoint_state(&jsp, 0).unwrap().verified);
        debugger.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 띄운 다시 부르기 시험: 디버거, 프로젝트 폴더, 가짜 js-debug가 받은 중단점, 가짜 CDP가 받은 명령.
    type ReloadPage = (
        Debugger,
        PathBuf,
        std::sync::mpsc::Receiver<(String, Vec<i64>)>,
        std::sync::mpsc::Receiver<String>,
    );

    /// 다시 부르기 시험의 화면: 맨 위 코드(`var count = 0;`, JSP 5줄 → 화면 6줄)와 함수가 든 JSP 하나.
    /// JSP 5줄에 중단점을 걸고 가짜 CDP 프록시(`transition`)와 함께 띄운다. `stop`은 가짜 js-debug가
    /// 멈출 (블록, 화면 줄)이다.
    fn start_reload_page(name: &str, transition: &'static str, stop: (usize, i64)) -> ReloadPage {
        let root = std::env::temp_dir().join(format!("ferrite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let jsp = root.join("src/main/webapp/WEB-INF/jsp/board/list.jsp");
        std::fs::create_dir_all(jsp.parent().unwrap()).unwrap();
        std::fs::write(
            &jsp,
            "<script>\nfunction tick() {\n  count = count + 1;\n}\nvar count = 0;\n</script>\n",
        )
        .unwrap();
        static HTML: &str = "<html><body>\n<script>\nfunction tick() {\n  count = count + 1;\n}\nvar count = 0;\n</script>\n</body></html>\n";
        let blocks = vec![(
            source("127.0.0.1\u{a789}8080/app/list.do\u{a789}2:9", 7),
            "\nfunction tick() {\n  count = count + 1;\n}\nvar count = 0;\n",
        )];
        let (cdp_port, methods) = fake_cdp(HTML, transition);
        let (address, received) = fake_js_debug(blocks, None, Some(cdp_port), stop);
        let config = DebugConfig {
            name: "화면".into(),
            adapter: "js-debug".into(),
            request: "launch".into(),
            arguments: serde_json::json!({}),
            build: None,
        };
        let breakpoints = BTreeMap::from([(
            jsp,
            vec![Breakpoint {
                line: 5,
                condition: None,
            }],
        )]);
        let mut debugger = Debugger::default();
        debugger
            .start(
                &config,
                &root,
                None,
                breakpoints,
                Adapter::Tcp(address),
                &egui::Context::default(),
            )
            .unwrap();
        // 담아 둔 화면 스크립트는 앱이 닫힐 때 지운다. 시험은 프로젝트 폴더에 두어 함께 지운다.
        debugger.sources = Some(root.join("sources"));
        (debugger, root, received, methods)
    }

    /// `done`이 참이 될 때까지 디버거를 돌린다(10초).
    fn pump(debugger: &mut Debugger, what: &str, mut done: impl FnMut(&Debugger) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(debugger) {
            assert!(Instant::now() < deadline, "{what}: {}", debugger.console);
            debugger.poll();
            thread::sleep(Duration::from_millis(10));
        }
    }

    const RELOAD_KEY: &str = "127.0.0.1\u{a789}8080/app/list.do";

    #[test]
    fn js_debug에서_처음_맞춘_화면을_한_번_다시_불러_맨_위_코드에도_걸리게_한다() {
        let (mut debugger, root, received, methods) =
            start_reload_page("reload-page", "link", (0, 999));
        let mut seen = Vec::new();
        pump(&mut debugger, "다시 부르지 않음", |_| {
            seen.extend(methods.try_iter());
            seen.iter().any(|m| m == "Page.reload")
        });
        // 화면을 다 읽은 것을 보고, 맞춘 중단점(JSP 5줄 → 화면 6줄)을 다시 보내 답을 받은 뒤에 불렀다.
        let sent: Vec<(String, Vec<i64>)> = received.try_iter().collect();
        assert!(
            sent.iter()
                .filter(|s| *s == &("#7".to_owned(), vec![6]))
                .count()
                >= 2,
            "{sent:?}"
        );
        assert!(
            seen.iter().position(|m| m == "Page.getNavigationHistory")
                < seen.iter().position(|m| m == "Page.reload"),
            "{seen:?}"
        );
        pump(
            &mut debugger,
            "다시 부른 것을 알리지 않음",
            |d| {
                d.console
                    .contains("[화면을 다시 불렀습니다: 127.0.0.1:8080/app/list.do]")
            },
        );
        // 화면마다 한 번만 부른다(다시 부른 화면을 또 맞춰도 부르지 않는다).
        debugger.loaded(
            RELOAD_KEY.to_owned(),
            source("127.0.0.1\u{a789}8080/app/list.do\u{a789}2:9", 9),
        );
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            debugger.poll();
            seen.extend(methods.try_iter());
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            seen.iter().filter(|m| *m == "Page.reload").count(),
            1,
            "{seen:?}"
        );
        assert!(
            seen.iter()
                .filter(|m| *m == "Page.getResourceContent")
                .count()
                >= 2,
            "다시 맞췄다: {seen:?}"
        );
        debugger.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn js_debug에서_폼을_보내_연_화면은_다시_부르지_않고_알린다() {
        let (mut debugger, root, _received, methods) =
            start_reload_page("reload-form", "form_submit", (0, 999));
        pump(&mut debugger, "알리지 않음", |d| {
            d.console
                .contains("[화면을 다시 부르지 않았습니다: 127.0.0.1:8080/app/list.do]")
        });
        assert!(
            debugger.console.contains("폼을 보내 연 화면"),
            "{}",
            debugger.console
        );
        let seen: Vec<String> = methods.try_iter().collect();
        assert!(!seen.iter().any(|m| m == "Page.reload"), "{seen:?}");
        debugger.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn js_debug에서_멈춰_있으면_계속을_누른_뒤에_다시_부른다() {
        // 건 중단점(화면 6줄)에서 바로 멈춘다.
        let (mut debugger, root, _received, methods) =
            start_reload_page("reload-stopped", "link", (0, 6));
        pump(&mut debugger, "계속을 기다리지 않음", |d| {
            d.pages.as_ref().is_some_and(|p| {
                matches!(p.reloads.get(RELOAD_KEY), Some(Reload::AfterContinue(_)))
            })
        });
        assert!(debugger.is_stopped());
        let mut seen: Vec<String> = methods.try_iter().collect();
        assert!(!seen.iter().any(|m| m == "Page.reload"), "{seen:?}");
        // 한 줄씩 가는 것으로는 부르지 않는다. 계속을 누르면 부른다.
        debugger.resume(Step::Continue);
        pump(
            &mut debugger,
            "계속 뒤에 다시 부르지 않음",
            |_| {
                seen.extend(methods.try_iter());
                seen.iter().any(|m| m == "Page.reload")
            },
        );
        debugger.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 프레임 안 화면(detail.do) 시험: 블록 둘(화면 2·8줄)이 든 JSP의 두 블록에 중단점을 걸고, 자식 프레임이 처음
    /// 주소를 보이는지(`from_src`)를 정해 띄운다.
    fn start_frame_page(name: &str, from_src: bool) -> ReloadPage {
        let root = std::env::temp_dir().join(format!("ferrite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let jsp = root.join("src/main/webapp/WEB-INF/jsp/board/detail.jsp");
        std::fs::create_dir_all(jsp.parent().unwrap()).unwrap();
        std::fs::write(
            &jsp,
            "<script>\nfunction first() {\n  return 1;\n}\n</script>\n<p>상세</p>\n<script>\nvar second = 2;\n</script>\n",
        )
        .unwrap();
        const FIRST: &str = "\nfunction first() {\n  return 1;\n}\n";
        const SECOND: &str = "\nvar second = 2;\n";
        static FRAME_BLOCKS: &[(u64, &str)] = &[(2, FIRST), (8, SECOND)];
        let blocks = vec![
            (
                source("127.0.0.1\u{a789}8080/app/detail.do\u{a789}2:9", 6),
                FIRST,
            ),
            (
                source("127.0.0.1\u{a789}8080/app/detail.do\u{a789}8:9", 7),
                SECOND,
            ),
        ];
        // 화면에서 읽히는 글은 맨 위 화면의 것이라(evaluate는 맨 위 화면에서 돈다) 블록과 맞지 않는다.
        let (cdp_port, methods) = fake_cdp_frame("<html></html>", "link", FRAME_BLOCKS, from_src);
        let (address, received) = fake_js_debug(
            blocks,
            Some(vec!["\nvar top = 1;\n"]),
            Some(cdp_port),
            (0, 999),
        );
        let config = DebugConfig {
            name: "프레임 화면".into(),
            adapter: "js-debug".into(),
            request: "launch".into(),
            arguments: serde_json::json!({}),
            build: None,
        };
        // JSP 3줄(`return 1;`)은 화면 4줄, JSP 8줄(`var second`)은 화면 9줄이다.
        let breakpoints = BTreeMap::from([(
            jsp,
            vec![
                Breakpoint {
                    line: 3,
                    condition: None,
                },
                Breakpoint {
                    line: 8,
                    condition: None,
                },
            ],
        )]);
        let mut debugger = Debugger::default();
        debugger
            .start(
                &config,
                &root,
                None,
                breakpoints,
                Adapter::Tcp(address),
                &egui::Context::default(),
            )
            .unwrap();
        // 담아 둔 화면 스크립트는 앱이 닫힐 때 지운다. 시험은 프로젝트 폴더에 두어 함께 지운다.
        debugger.sources = Some(root.join("sources"));
        (debugger, root, received, methods)
    }

    #[test]
    fn js_debug에서_프레임_안_화면도_블록마다_맞추고_그_프레임만_다시_연다() {
        let (mut debugger, root, received, methods) = start_frame_page("frame-page", true);
        let mut seen = Vec::new();
        pump(&mut debugger, "프레임을 다시 열지 않음", |_| {
            seen.extend(methods.try_iter());
            seen.iter().any(|m| m == "Page.navigate")
        });
        // 원래 HTML을 받지 못해 블록 글을 디버거에서 받아, 두 블록의 JSP 중단점을 화면 4·9줄에 걸었다.
        assert!(
            seen.iter().any(|m| m == "Debugger.getScriptSource"),
            "{seen:?}"
        );
        let sent: Vec<(String, Vec<i64>)> = received.try_iter().collect();
        assert!(sent.contains(&("#7".to_owned(), vec![4, 9])), "{sent:?}");
        // 맨 위 화면은 다시 부르지 않는다.
        assert!(!seen.iter().any(|m| m == "Page.reload"), "{seen:?}");
        pump(&mut debugger, "다시 연 것을 알리지 않음", |d| {
            d.console
                .contains("[화면을 다시 불렀습니다: 127.0.0.1:8080/app/detail.do]")
        });
        debugger.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn js_debug에서_처음_주소에서_다른_화면으로_간_프레임은_다시_열지_않고_알린다() {
        let (mut debugger, root, _received, methods) = start_frame_page("frame-moved", false);
        pump(&mut debugger, "알리지 않음", |d| {
            d.console
                .contains("[화면을 다시 부르지 않았습니다: 127.0.0.1:8080/app/detail.do]")
        });
        assert!(
            debugger.console.contains("처음 주소(src)"),
            "{}",
            debugger.console
        );
        let seen: Vec<String> = methods.try_iter().collect();
        assert!(
            !seen
                .iter()
                .any(|m| m == "Page.navigate" || m == "Page.reload"),
            "{seen:?}"
        );
        debugger.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }
}
