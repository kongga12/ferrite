//! 디버거: 디버그 세션 하나와 그 화면(도구 막대·호출 스택·변수 창, 디버그 출력 창).
//! 멈추면 호출 스택과 맨 위 프레임의 변수를 스스로 불러오고, 앱에 멈춘 자리를 알린다.
//! 로컬 파일이 없는 스크립트(브라우저 화면의 인라인 스크립트)는 어댑터에서 내용을 받아 임시 폴더에
//! 담아 읽기 전용으로 연다. 그 파일에 건 중단점은 원래 소스로 건다.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use ferrite_core::debug_config::DebugConfig;
use ferrite_dap::{
    Adapter, Breakpoint, BreakpointState, DebugEvent, DebugSession, Launch, Scope, Source,
    StackFrame, State, Variable, adapters, types,
};
use serde_json::Value;

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
}

/// 담아 둔 소스를 찾는 열쇠(같은 화면을 다시 불러도 같다. 번호는 바뀐다).
fn source_key(source: &Source) -> String {
    source
        .path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| source.name.clone())
        .unwrap_or_else(|| source.source_reference.to_string())
}

impl Debugger {
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
            ..Default::default()
        };
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
        self.states.remove(path);
        if let Some(session) = &mut self.session {
            session.set_breakpoints(path, breakpoints);
        }
    }

    /// 어댑터가 확인한 그 파일 `index`번째 중단점(세션이 없으면 None).
    pub fn breakpoint_state(&self, path: &Path, index: usize) -> Option<&BreakpointState> {
        self.session.as_ref()?;
        self.states.get(path)?.get(index)
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
        let dir = sources_dir();
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
                DebugEvent::Continued => self.clear_stop(),
                DebugEvent::Output { category, text } => {
                    if category != "telemetry" {
                        self.push_console(&text);
                    }
                }
                DebugEvent::Breakpoints { path, states } => {
                    self.states.insert(path, states);
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
                for source in self.frames.iter_mut().filter_map(|f| f.source.as_mut()) {
                    if is_remote(source) {
                        match self.stored.get(&source_key(source)) {
                            Some(local) => source.path = Some(local.clone()),
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
                    self.send(Request::Source(source));
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
                // 멈춘 프레임이 이 소스면 담은 파일을 가리키게 한다.
                for frame_source in self.frames.iter_mut().filter_map(|f| f.source.as_mut()) {
                    if is_remote(frame_source) && source_key(frame_source) == key {
                        frame_source.path = Some(local.clone());
                    }
                }
                if self.open_after.as_ref() == Some(&key) {
                    self.open_after = None;
                    return Some(DebugNotice::Reveal(local, 0));
                }
                self.reveal_frame()
            }
        }
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
                        let name = shown(source.name.as_deref().unwrap_or("(이름 없음)"));
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

/// 빌드 명령을 플랫폼 셸로 돌려 출력(UTF-8·CP949)을 모은다. 실패(0이 아닌 종료 코드)면 Err.
fn run_build(line: &str, dir: &Path) -> Result<String, String> {
    let mut command = shell(line);
    command.current_dir(dir);
    ferrite_lsp::client::hide_console(&mut command);
    let output = command
        .output()
        .map_err(|e| format!("빌드 명령을 실행하지 못했습니다: {e}\n"))?;
    let decode = |bytes: &[u8]| {
        ferrite_core::text_file::decode(bytes)
            .map(|c| c.text)
            .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned())
    };
    let text = decode(&output.stdout) + &decode(&output.stderr);
    if output.status.success() {
        Ok(text)
    } else {
        Err(format!("{text}[빌드 실패: {}]\n", output.status))
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
        assert_eq!(
            source_key(&page),
            page.path.as_ref().unwrap().to_string_lossy()
        );
    }
}
