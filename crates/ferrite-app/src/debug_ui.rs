//! 디버거: 디버그 세션 하나와 그 화면(도구 막대·호출 스택·변수 창, 디버그 출력 창).
//! 멈추면 호출 스택과 맨 위 프레임의 변수를 스스로 불러오고, 앱에 멈춘 자리를 알린다.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use ferrite_core::debug_config::DebugConfig;
use ferrite_dap::{
    Breakpoint, BreakpointState, DebugEvent, DebugSession, Launch, Scope, StackFrame, State,
    Variable, adapters, types,
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
}

impl Debugger {
    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }

    pub fn is_stopped(&self) -> bool {
        matches!(
            self.session.as_ref().map(DebugSession::state),
            Some(State::Stopped(_))
        )
    }

    /// 구성으로 세션을 시작한다. 돌고 있는 세션은 먼저 끝낸다.
    pub fn start(
        &mut self,
        config: &DebugConfig,
        root: &Path,
        file: Option<&Path>,
        breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        let adapter = adapters::resolve(&config.adapter, root)?;
        let arguments = config.resolved_arguments(root, file)?;
        let launch = Launch {
            adapter_id: config.adapter.clone(),
            attach: config.attach(),
            arguments,
        };
        let wake = ctx.clone();
        let session = DebugSession::start(&adapter, launch, breakpoints, move || {
            wake.request_repaint()
        })?;
        *self = Self {
            session: Some(session),
            ctx: Some(ctx.clone()),
            name: config.name.clone(),
            adapter: config.adapter.clone(),
            console: format!("[디버그 시작: {}]\n", config.name),
            ..Default::default()
        };
        Ok(())
    }

    /// 대상을 끝낸다. 바로 세션을 버리면 어댑터가 대상을 끝내기 전에 죽어 대상이 남을 수 있으므로,
    /// 끝났다는 알림을 기다리고 `STOP_GRACE`가 지나면 어댑터를 끝낸다.
    pub fn stop(&mut self) {
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

    fn end(&mut self, note: &str) {
        self.session = None;
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
        };
        self.requests.insert(id, request);
    }

    fn reveal_frame(&self) -> Option<DebugNotice> {
        self.location()
            .map(|(path, line)| DebugNotice::Reveal(path.to_path_buf(), line))
    }

    /// 세션 이벤트를 처리한다. 매 프레임 부른다.
    pub fn poll(&mut self) -> Vec<DebugNotice> {
        let Some(session) = &mut self.session else {
            return Vec::new();
        };
        let events = session.poll();
        let mut notices = Vec::new();
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
                // 어댑터가 준 경로(드라이브 글자 대소문자 등)를 열린 문서 경로와 같은 꼴로 맞춘다.
                for frame in &mut self.frames {
                    if let Some(path) = frame.source.as_mut().and_then(|s| s.path.as_mut()) {
                        *path = crate::app::real_path(path);
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
        }
    }

    /// "디버그" 창: 도구 막대, 호출 스택, 변수.
    pub fn panel_ui(&mut self, ui: &mut egui::Ui) -> Option<DebugNotice> {
        let mut step = None;
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
            let text = format!("{}  {place}:{}", frame.name, frame.line);
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
