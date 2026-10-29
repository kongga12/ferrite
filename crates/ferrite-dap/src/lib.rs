//! Ferrite DAP 클라이언트: 디버그 어댑터와 Debug Adapter Protocol로 말한다.
//!
//! 언어 서버 클라이언트처럼 비동기 런타임 없이 스레드와 채널만 쓴다. 읽기 스레드가 어댑터 메시지를
//! 채널로 넘기고, UI는 프레임마다 [`DebugSession::poll`]로 꺼내 처리한다. 시작 순서
//! (initialize → launch/attach → `initialized` 이벤트 → 중단점 → configurationDone)도 `poll`이
//! 진행한다. 어댑터마다 다른 것(띄우는 명령, launch 인자)은 [`Adapter`]와 [`Launch`]로 받는다.
//!
//! 어댑터가 거꾸로 `startDebugging`을 요청하면(js-debug: 브라우저 페이지마다) 같은 주소로 자식 세션을
//! 열어 이 세션 안에 둔다. 화면은 세션 하나만 다루고, 요청은 멈춘 자식에게 간다.

pub mod adapters;
pub mod java;
pub mod types;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use ferrite_lsp::transport::{read_message, write_message};
use serde_json::{Value, json};

pub use types::{BreakpointState, Capabilities, Scope, Source, StackFrame, Thread, Variable};

type Wake = Arc<dyn Fn() + Send + Sync>;

/// 종료 사유로 보여 줄 어댑터 stderr 마지막 줄 수.
const STDERR_TAIL_LINES: usize = 20;
/// TCP 어댑터에 붙을 때 기다리는 시간.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 띄운 어댑터가 듣는 주소를 알릴 때까지 기다리는 시간.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);

/// 어댑터에 닿는 방법.
#[derive(Clone, Debug)]
pub enum Adapter {
    /// 표준 입출력으로 말하는 어댑터 프로세스를 띄운다(debugpy, codelldb, vscode-php-debug).
    Stdio {
        program: PathBuf,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
    },
    /// 이미 듣고 있는 어댑터에 붙는다(java-debug).
    Tcp(SocketAddr),
    /// 어댑터를 띄우고, 표준 출력에 알리는 주소("… listening at: 127.0.0.1:PORT")로 붙는다
    /// (delve `dlv dap --listen=127.0.0.1:0`).
    Listening {
        program: PathBuf,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
    },
}

/// 디버그 대상을 띄우거나 붙는 요청.
#[derive(Clone, Debug)]
pub struct Launch {
    /// `initialize`의 `adapterID`(예: "debugpy", "go", "java").
    pub adapter_id: String,
    /// true면 `attach`, 아니면 `launch`.
    pub attach: bool,
    /// 어댑터마다 다른 인자(program, cwd, args …).
    pub arguments: Value,
}

/// 소스 중단점. 줄은 1부터 센다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Breakpoint {
    pub line: usize,
    /// 참일 때만 멈추는 식(어댑터 언어로 적는다).
    pub condition: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stopped {
    /// "breakpoint", "step", "pause", "exception", "entry" 등.
    pub reason: String,
    pub thread_id: Option<i64>,
    pub description: Option<String>,
    pub all_threads: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// 초기화 중(아직 대상이 돌지 않는다).
    Starting,
    Running,
    Stopped(Stopped),
    Terminated,
}

#[derive(Clone, Debug)]
pub enum DebugEvent {
    /// 대상을 띄웠거나 붙었다(launch/attach 응답).
    Started,
    Stopped(Stopped),
    Continued,
    /// 대상의 출력이나 어댑터의 알림. `category`는 "stdout", "stderr", "console" 등.
    Output {
        category: String,
        text: String,
    },
    /// 어댑터가 확인한 그 파일의 중단점(보낸 순서대로).
    Breakpoints {
        path: PathBuf,
        states: Vec<BreakpointState>,
    },
    /// [`DebugSession::request`]로 보낸 요청의 결과.
    Response {
        id: i64,
        command: String,
        result: Result<Value, String>,
    },
    /// 실패한 요청 등 사용자에게 알릴 오류.
    Error(String),
    Exited {
        code: i64,
    },
    /// 디버그 세션이 끝났다.
    Terminated,
    /// 어댑터와의 연결이 끊겼다. 프로세스 어댑터면 stderr 마지막 줄을 담는다.
    Closed(Option<String>),
}

/// 응답을 기다리는 요청.
enum Pending {
    Initialize,
    Launch,
    Breakpoints(PathBuf),
    /// 결과는 버리고 실패만 알린다(제어 명령, configurationDone 등).
    Internal(String),
    /// 결과를 [`DebugEvent::Response`]로 돌려준다.
    User(String),
}

enum Incoming {
    Message(Value),
    Closed,
}

pub struct DebugSession {
    child: Option<Child>,
    writer: Box<dyn Write + Send>,
    incoming: Receiver<Incoming>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    /// 부모와 자식 세션이 나눠 쓰는 요청 번호. 응답 번호가 세션끼리 겹치지 않는다.
    seq: Arc<AtomicI64>,
    wake: Wake,
    /// TCP 어댑터의 주소. 자식 세션은 같은 주소로 새로 붙는다.
    address: Option<SocketAddr>,
    /// 어댑터가 거꾸로 요청한 자식 세션(js-debug의 브라우저 페이지).
    children: Vec<DebugSession>,
    /// 어댑터가 확인한 중단점. `breakpoint` 이벤트가 번호로 바꾼 것을 고쳐 다시 알린다.
    states: HashMap<PathBuf, Vec<BreakpointState>>,
    /// 로컬 파일이 없는 스크립트를 담아 둔 파일 → 어댑터의 원래 소스. 그 파일의 중단점은 원래 소스로 건다.
    sources: HashMap<PathBuf, Value>,
    pending: HashMap<i64, Pending>,
    capabilities: Capabilities,
    launch: Launch,
    breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
    /// `initialized` 이벤트를 받아 중단점을 보냈는지. 그 뒤에 바뀐 중단점은 바로 보낸다.
    configured: bool,
    state: State,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // 다른 스레드가 패닉해도 상태 자체는 쓸 수 있으므로 계속 진행한다.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl DebugSession {
    /// 어댑터를 띄우거나(붙고) `initialize`를 보낸다. 나머지 시작 순서는 `poll`이 진행한다.
    /// `wake`는 어댑터 메시지가 올 때마다 읽기 스레드에서 불린다(UI를 다시 그리게).
    pub fn start(
        adapter: &Adapter,
        launch: Launch,
        breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        Self::connect(
            adapter,
            launch,
            breakpoints,
            Arc::new(wake),
            Arc::new(AtomicI64::new(1)),
        )
    }

    fn connect(
        adapter: &Adapter,
        launch: Launch,
        breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
        wake: Wake,
        seq: Arc<AtomicI64>,
    ) -> Result<Self, String> {
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        let mut address = None;
        let (child, reader, writer): (
            Option<Child>,
            Box<dyn BufRead + Send>,
            Box<dyn Write + Send>,
        ) = match adapter {
            Adapter::Stdio {
                program,
                args,
                cwd,
                env,
            } => {
                let mut child = spawn(program, args, cwd.as_deref(), env)?;
                let (Some(stdin), Some(stdout), Some(stderr)) =
                    (child.stdin.take(), child.stdout.take(), child.stderr.take())
                else {
                    let _ = child.kill();
                    return Err("디버그 어댑터의 표준 입출력을 열지 못했습니다".into());
                };
                keep_tail(stderr, &stderr_tail, |_| {});
                (
                    Some(child),
                    Box::new(BufReader::new(stdout)),
                    Box::new(stdin),
                )
            }
            Adapter::Listening {
                program,
                args,
                cwd,
                env,
            } => {
                let mut child = spawn(program, args, cwd.as_deref(), env)?;
                let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take())
                else {
                    let _ = child.kill();
                    return Err("디버그 어댑터의 출력을 열지 못했습니다".into());
                };
                keep_tail(stderr, &stderr_tail, |_| {});
                let (announce, announced) = channel();
                keep_tail(stdout, &stderr_tail, move |line| {
                    if let Some(address) = listening_address(line) {
                        let _ = announce.send(address);
                    }
                });
                let found = match announced.recv_timeout(LISTEN_TIMEOUT) {
                    Ok(found) => found,
                    Err(_) => {
                        let _ = child.kill();
                        let tail: Vec<String> = lock(&stderr_tail).iter().cloned().collect();
                        return Err(format!(
                            "{}이(가) 듣는 주소를 알리지 않았습니다: {}",
                            program.display(),
                            tail.join(" / ")
                        ));
                    }
                };
                let stream = match TcpStream::connect_timeout(&found, CONNECT_TIMEOUT) {
                    Ok(stream) => stream,
                    Err(e) => {
                        let _ = child.kill();
                        return Err(format!("디버그 어댑터({found})에 붙지 못했습니다: {e}"));
                    }
                };
                address = Some(found);
                let _ = stream.set_nodelay(true);
                let reader = stream
                    .try_clone()
                    .map_err(|e| format!("디버그 어댑터 연결을 나누지 못했습니다: {e}"))?;
                (
                    Some(child),
                    Box::new(BufReader::new(reader)),
                    Box::new(stream),
                )
            }
            Adapter::Tcp(at) => {
                let stream = TcpStream::connect_timeout(at, CONNECT_TIMEOUT)
                    .map_err(|e| format!("디버그 어댑터({at})에 붙지 못했습니다: {e}"))?;
                address = Some(*at);
                let _ = stream.set_nodelay(true);
                let reader = stream
                    .try_clone()
                    .map_err(|e| format!("디버그 어댑터 연결을 나누지 못했습니다: {e}"))?;
                (None, Box::new(BufReader::new(reader)), Box::new(stream))
            }
        };

        let (tx, incoming) = channel();
        let reader_wake = wake.clone();
        thread::spawn(move || {
            let wake = reader_wake;
            let mut reader = reader;
            // 끝나거나 읽을 수 없는 메시지면 연결이 끊긴 것으로 본다(틀이 깨지면 이어 읽을 수 없다).
            while let Ok(Some(body)) = read_message(&mut reader) {
                if let Ok(message) = serde_json::from_slice(&body) {
                    if tx.send(Incoming::Message(message)).is_err() {
                        return;
                    }
                    wake();
                }
            }
            let _ = tx.send(Incoming::Closed);
            wake();
        });

        let mut session = Self {
            child,
            writer,
            incoming,
            stderr_tail,
            seq,
            wake,
            address,
            children: Vec::new(),
            states: HashMap::new(),
            sources: HashMap::new(),
            pending: HashMap::new(),
            capabilities: Capabilities::default(),
            launch,
            breakpoints,
            configured: false,
            state: State::Starting,
        };
        let arguments = json!({
            "clientID": "ferrite",
            "clientName": "Ferrite",
            "adapterID": session.launch.adapter_id,
            "locale": "ko",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path",
            "supportsVariableType": true,
            "supportsRunInTerminalRequest": false,
            "supportsStartDebuggingRequest": true,
        });
        session.send_request("initialize", arguments, Pending::Initialize);
        Ok(session)
    }

    /// 세션 전체의 상태: 끝났으면 끝, 아니면 멈춘 자식(없으면 마지막 자식)의 상태.
    pub fn state(&self) -> &State {
        if self.state == State::Terminated {
            return &self.state;
        }
        &self.focus_ref().state
    }

    fn stopped_somewhere(&self) -> bool {
        matches!(self.state, State::Stopped(_)) || self.children.iter().any(Self::stopped_somewhere)
    }

    /// 사용자 요청을 받을 세션: 멈춘 자식 → 멈춘 자신 → 마지막 자식 → 자신. None이면 자신.
    fn focus_index(&self) -> Option<usize> {
        if let Some(i) = self.children.iter().rposition(Self::stopped_somewhere) {
            return Some(i);
        }
        if matches!(self.state, State::Stopped(_)) {
            return None;
        }
        self.children.len().checked_sub(1)
    }

    fn focus(&mut self) -> &mut Self {
        match self.focus_index() {
            Some(i) => self.children[i].focus(),
            None => self,
        }
    }

    fn focus_ref(&self) -> &Self {
        match self.focus_index() {
            Some(i) => self.children[i].focus_ref(),
            None => self,
        }
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// 쓸 수 없으면(어댑터 종료) 버린다. 끊긴 것은 읽기 쪽이 [`DebugEvent::Closed`]로 알린다.
    fn send_request(&mut self, command: &str, arguments: Value, pending: Pending) -> i64 {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let message = json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        self.pending.insert(seq, pending);
        self.write(&message);
        seq
    }

    fn write(&mut self, message: &Value) {
        let body = serde_json::to_vec(message).unwrap_or_default();
        let _ = write_message(&mut self.writer, &body);
    }

    /// 요청을 보내고 번호를 돌려준다. 결과는 같은 번호의 [`DebugEvent::Response`]로 온다.
    /// 자식 세션이 있으면 멈춘 자식에게 보낸다.
    pub fn request(&mut self, command: &str, arguments: Value) -> i64 {
        self.focus()
            .send_request(command, arguments, Pending::User(command.to_owned()))
    }

    /// 불러온 스크립트 목록(`loadedSources`).
    pub fn loaded_sources(&mut self) -> i64 {
        self.request("loadedSources", json!({}))
    }

    /// 로컬 파일이 없는 소스의 내용(`source`).
    pub fn source(&mut self, source: &Source) -> i64 {
        self.request(
            "source",
            json!({ "source": source.to_json(), "sourceReference": source.source_reference }),
        )
    }

    /// 어댑터의 소스를 로컬 파일 `local`에 담아 열었다. 그 파일의 중단점은 원래 소스로 건다.
    pub fn map_source(&mut self, local: &Path, source: &Source) {
        for child in &mut self.children {
            child.map_source(local, source);
        }
        self.sources.insert(local.to_path_buf(), source.to_json());
    }

    pub fn threads(&mut self) -> i64 {
        self.request("threads", json!({}))
    }

    pub fn stack_trace(&mut self, thread_id: i64) -> i64 {
        self.request("stackTrace", json!({ "threadId": thread_id }))
    }

    pub fn scopes(&mut self, frame_id: i64) -> i64 {
        self.request("scopes", json!({ "frameId": frame_id }))
    }

    pub fn variables(&mut self, reference: i64) -> i64 {
        self.request("variables", json!({ "variablesReference": reference }))
    }

    /// 멈춘 자리(`frame_id`)에서 식을 계산한다.
    pub fn evaluate(&mut self, expression: &str, frame_id: Option<i64>) -> i64 {
        self.request(
            "evaluate",
            json!({ "expression": expression, "frameId": frame_id, "context": "watch" }),
        )
    }

    /// 실행을 이어 가는 명령(continue·next·stepIn·stepOut). 어댑터가 `continued`를 보내지 않는
    /// 경우가 있어 보내는 즉시 실행 중으로 본다.
    fn resume_with(&mut self, command: &str, thread_id: i64) {
        let target = self.focus();
        target.send_request(
            command,
            json!({ "threadId": thread_id }),
            Pending::Internal(command.to_owned()),
        );
        if target.state != State::Terminated {
            target.state = State::Running;
        }
    }

    pub fn resume(&mut self, thread_id: i64) {
        self.resume_with("continue", thread_id);
    }

    pub fn step_over(&mut self, thread_id: i64) {
        self.resume_with("next", thread_id);
    }

    pub fn step_in(&mut self, thread_id: i64) {
        self.resume_with("stepIn", thread_id);
    }

    pub fn step_out(&mut self, thread_id: i64) {
        self.resume_with("stepOut", thread_id);
    }

    pub fn pause(&mut self, thread_id: i64) {
        self.focus().send_request(
            "pause",
            json!({ "threadId": thread_id }),
            Pending::Internal("pause".into()),
        );
    }

    /// 대상을 끝내고 연결을 닫게 한다. 끝나면 `Terminated`·`Closed`가 온다. 붙은 대상(attach)은
    /// 끝내지 않고 떨어지기만 한다(사용자의 브라우저·서버). 자식 세션부터 끝낸다(js-debug는 멈춘
    /// 페이지가 남아 있으면 부모의 disconnect에 답하지 않는다).
    pub fn stop(&mut self) {
        for child in &mut self.children {
            child.stop();
        }
        let terminate = !self.launch.attach;
        self.send_request(
            "disconnect",
            json!({ "terminateDebuggee": terminate }),
            Pending::Internal("disconnect".into()),
        );
    }

    /// 그 파일의 중단점을 바꾼다(빈 목록이면 모두 지운다). 시작 순서 전이면 모아 두었다가 보낸다.
    pub fn set_breakpoints(&mut self, path: &Path, breakpoints: Vec<Breakpoint>) {
        for child in &mut self.children {
            child.set_breakpoints(path, breakpoints.clone());
        }
        self.breakpoints.insert(path.to_path_buf(), breakpoints);
        if self.configured {
            self.send_breakpoints(path.to_path_buf());
        }
    }

    fn send_breakpoints(&mut self, path: PathBuf) {
        let breakpoints: Vec<Value> = self
            .breakpoints
            .get(&path)
            .into_iter()
            .flatten()
            .map(|b| match &b.condition {
                Some(condition) => json!({ "line": b.line, "condition": condition }),
                None => json!({ "line": b.line }),
            })
            .collect();
        let source = self
            .sources
            .get(&path)
            .cloned()
            .unwrap_or_else(|| json!({ "path": path }));
        let arguments = json!({
            "source": source,
            "breakpoints": breakpoints,
            "sourceModified": false,
        });
        self.send_request("setBreakpoints", arguments, Pending::Breakpoints(path));
    }

    /// 받은 메시지를 처리해 화면이 알아야 할 일로 바꾼다. 매 프레임 부른다.
    pub fn poll(&mut self) -> Vec<DebugEvent> {
        let mut events = Vec::new();
        self.poll_own(&mut events);
        // 자식의 시작·끝은 세션 전체의 일이 아니다. 끝난 자식은 빼고, 멈춰 있었으면 풀린 것으로 알린다.
        let mut from_children = Vec::new();
        self.children.retain_mut(|child| {
            let mut ended = false;
            for event in child.poll() {
                match event {
                    DebugEvent::Started | DebugEvent::Exited { .. } => {}
                    DebugEvent::Terminated | DebugEvent::Closed(_) => ended = true,
                    other => from_children.push(other),
                }
            }
            if ended && child.stopped_somewhere() {
                from_children.push(DebugEvent::Continued);
            }
            !ended
        });
        // 자식(페이지)이 있으면 중단점 확인은 자식의 것을 쓴다(부모는 늘 "아직 걸지 못함"이다).
        if !self.children.is_empty() {
            events.retain(|e| !matches!(e, DebugEvent::Breakpoints { .. }));
        }
        events.extend(from_children);
        events
    }

    fn poll_own(&mut self, events: &mut Vec<DebugEvent>) {
        while let Ok(incoming) = self.incoming.try_recv() {
            match incoming {
                Incoming::Message(message) => self.handle(&message, events),
                Incoming::Closed => {
                    self.state = State::Terminated;
                    let tail: Vec<String> = lock(&self.stderr_tail).iter().cloned().collect();
                    events.push(DebugEvent::Closed(
                        (!tail.is_empty()).then(|| tail.join("\n")),
                    ));
                }
            }
        }
    }

    fn handle(&mut self, message: &Value, events: &mut Vec<DebugEvent>) {
        let text = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or_default();
        match text("type") {
            "response" => self.on_response(message, events),
            "event" => self.on_event(text("event"), message.get("body"), events),
            // 어댑터가 거꾸로 보내는 요청. 자식 세션(startDebugging)만 받고 나머지(runInTerminal 등)는
            // 지원한다고 알리지 않았으므로 거절한다.
            "request" => {
                let result = match text("command") {
                    "startDebugging" => self
                        .start_child(message.get("arguments").unwrap_or(&Value::Null))
                        .inspect_err(|e| {
                            events.push(DebugEvent::Error(format!(
                                "디버그 자식 세션을 열지 못했습니다: {e}"
                            )));
                        }),
                    _ => Err("Ferrite는 이 요청을 지원하지 않습니다".to_owned()),
                };
                let mut reply = json!({
                    "seq": 0,
                    "type": "response",
                    "request_seq": message.get("seq"),
                    "success": result.is_ok(),
                    "command": text("command"),
                });
                if let Err(e) = result {
                    reply["message"] = Value::from(e);
                }
                self.write(&reply);
            }
            _ => {}
        }
    }

    /// 어댑터가 요청한 자식 세션을 같은 주소로 열고 시작 순서를 밟게 한다.
    fn start_child(&mut self, arguments: &Value) -> Result<(), String> {
        let address = self
            .address
            .ok_or("표준 입출력 어댑터는 자식 세션을 열 수 없습니다")?;
        let launch = Launch {
            adapter_id: self.launch.adapter_id.clone(),
            attach: arguments.get("request").and_then(Value::as_str) == Some("attach"),
            arguments: arguments
                .get("configuration")
                .cloned()
                .unwrap_or_else(|| json!({})),
        };
        let mut child = Self::connect(
            &Adapter::Tcp(address),
            launch,
            self.breakpoints.clone(),
            self.wake.clone(),
            self.seq.clone(),
        )?;
        child.sources = self.sources.clone();
        self.children.push(child);
        Ok(())
    }

    fn on_response(&mut self, message: &Value, events: &mut Vec<DebugEvent>) {
        let Some(pending) = message
            .get("request_seq")
            .and_then(Value::as_i64)
            .and_then(|seq| self.pending.remove(&seq).map(|p| (seq, p)))
        else {
            return;
        };
        let (seq, pending) = pending;
        let body = message.get("body").cloned().unwrap_or(Value::Null);
        let result = if message.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(body)
        } else {
            Err(error_text(message))
        };
        match (pending, result) {
            (Pending::Initialize, Ok(body)) => {
                self.capabilities = serde_json::from_value(body).unwrap_or_default();
                let command = if self.launch.attach {
                    "attach"
                } else {
                    "launch"
                };
                let arguments = self.launch.arguments.clone();
                self.send_request(command, arguments, Pending::Launch);
            }
            (Pending::Initialize, Err(e)) => {
                events.push(DebugEvent::Error(format!("디버그 어댑터 초기화 실패: {e}")));
            }
            (Pending::Launch, Ok(_)) => {
                if self.state == State::Starting {
                    self.state = State::Running;
                }
                events.push(DebugEvent::Started);
            }
            (Pending::Launch, Err(e)) => {
                events.push(DebugEvent::Error(format!(
                    "디버그 대상을 시작하지 못했습니다: {e}"
                )));
            }
            (Pending::Breakpoints(path), Ok(body)) => {
                let states = types::breakpoint_states(&body);
                self.states.insert(path.clone(), states.clone());
                events.push(DebugEvent::Breakpoints { path, states });
            }
            (Pending::Breakpoints(path), Err(e)) => {
                events.push(DebugEvent::Error(format!(
                    "중단점을 걸지 못했습니다({}): {e}",
                    path.display()
                )));
            }
            // 떨어지기를 마쳤으면 끝이다. js-debug는 연결을 닫지 않고 terminated도 보내지 않는다.
            (Pending::Internal(command), Ok(_)) if command == "disconnect" => {
                if self.state != State::Terminated {
                    self.state = State::Terminated;
                    events.push(DebugEvent::Terminated);
                }
            }
            (Pending::Internal(_), Ok(_)) => {}
            (Pending::Internal(command), Err(e)) => {
                events.push(DebugEvent::Error(format!("{command} 실패: {e}")));
            }
            (Pending::User(command), result) => {
                events.push(DebugEvent::Response {
                    id: seq,
                    command,
                    result,
                });
            }
        }
    }

    fn on_event(&mut self, event: &str, body: Option<&Value>, events: &mut Vec<DebugEvent>) {
        let body = body.cloned().unwrap_or(Value::Null);
        let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_owned);
        match event {
            // 이제 중단점을 받을 수 있다. 다 보낸 뒤 configurationDone으로 대상을 출발시킨다.
            "initialized" => {
                self.configured = true;
                let paths: Vec<PathBuf> = self.breakpoints.keys().cloned().collect();
                for path in paths {
                    self.send_breakpoints(path);
                }
                if self.capabilities.exception_breakpoint_filters.is_some() {
                    // 예외에서 멈추는 필터는 아직 고르지 않는다(모두 끔).
                    self.send_request(
                        "setExceptionBreakpoints",
                        json!({ "filters": [] }),
                        Pending::Internal("setExceptionBreakpoints".into()),
                    );
                }
                if self.capabilities.supports_configuration_done_request {
                    self.send_request(
                        "configurationDone",
                        json!({}),
                        Pending::Internal("configurationDone".into()),
                    );
                }
            }
            "stopped" => {
                let stopped = Stopped {
                    reason: text("reason").unwrap_or_default(),
                    thread_id: body.get("threadId").and_then(Value::as_i64),
                    description: text("description").or_else(|| text("text")),
                    all_threads: body
                        .get("allThreadsStopped")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                };
                self.state = State::Stopped(stopped.clone());
                events.push(DebugEvent::Stopped(stopped));
            }
            "continued" => {
                self.state = State::Running;
                events.push(DebugEvent::Continued);
            }
            "output" => events.push(DebugEvent::Output {
                category: text("category").unwrap_or_else(|| "console".into()),
                text: text("output").unwrap_or_default(),
            }),
            "exited" => events.push(DebugEvent::Exited {
                code: body.get("exitCode").and_then(Value::as_i64).unwrap_or(0),
            }),
            "terminated" => {
                self.state = State::Terminated;
                events.push(DebugEvent::Terminated);
            }
            // 어댑터가 나중에 중단점을 걸었다(js-debug는 스크립트를 불러올 때 건다). 번호로 찾아 고친다.
            "breakpoint" if text("reason").as_deref() == Some("changed") => {
                let Some(changed) = body
                    .get("breakpoint")
                    .and_then(|b| serde_json::from_value::<BreakpointState>(b.clone()).ok())
                    .filter(|b| b.id.is_some())
                else {
                    return;
                };
                for (path, states) in &mut self.states {
                    if let Some(state) = states.iter_mut().find(|s| s.id == changed.id) {
                        *state = changed;
                        events.push(DebugEvent::Breakpoints {
                            path: path.clone(),
                            states: states.clone(),
                        });
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

fn spawn(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
) -> Result<Child, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    ferrite_lsp::client::hide_console(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("{} 실행 실패: {e}", program.display()))
}

/// 어댑터 출력을 줄 단위로 읽어 마지막 몇 줄을 남긴다(연결이 끊겼을 때 이유로 보인다). 비우지 않으면
/// 파이프가 가득 차 어댑터가 멈춘다. `each`는 줄마다 불린다.
fn keep_tail(
    output: impl std::io::Read + Send + 'static,
    tail: &Arc<Mutex<VecDeque<String>>>,
    each: impl Fn(&str) + Send + 'static,
) {
    let tail = tail.clone();
    thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            each(&line);
            let mut tail = lock(&tail);
            if tail.len() == STDERR_TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    });
}

/// "DAP server listening at: 127.0.0.1:38297" 같은 줄에서 주소를 읽는다.
fn listening_address(line: &str) -> Option<SocketAddr> {
    let at = line.to_ascii_lowercase().find("listening at")?;
    line[at..].split_whitespace().last()?.parse().ok()
}

/// 실패 응답의 사유. 어댑터는 `body.error.format`에 자세한 글을 담기도 한다.
fn error_text(message: &Value) -> String {
    message
        .pointer("/body/error/format")
        .and_then(Value::as_str)
        .or_else(|| message.get("message").and_then(Value::as_str))
        .unwrap_or("알 수 없는 오류")
        .to_owned()
}

impl Drop for DebugSession {
    /// 프로세스 어댑터는 함께 끝낸다(남으면 디버그 대상까지 고아로 남을 수 있다).
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 듣는_주소를_알리는_줄을_읽는다() {
        assert_eq!(
            listening_address("DAP server listening at: 127.0.0.1:38297"),
            Some("127.0.0.1:38297".parse().unwrap())
        );
        assert_eq!(
            listening_address("API server Listening at: [::1]:1234"),
            Some("[::1]:1234".parse().unwrap())
        );
        assert_eq!(listening_address("Type 'help' for list of commands."), None);
    }
}
