//! 언어 서버 프로세스 하나. 읽기 스레드가 서버 메시지를 받아 채널로 넘기고,
//! UI는 프레임마다 `poll_events`로 꺼내 처리한다(비동기 런타임 없이 스레드와 채널만 쓴다).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use lsp_types::{
    Diagnostic, InitializeResult, PositionEncodingKind, PublishDiagnosticsParams,
    ServerCapabilities,
};
use serde_json::{Value, json};

use crate::position::PositionEncoding;
use crate::transport::{read_message, write_message};
use crate::uri::{path_to_uri, uri_to_path};

/// 종료 사유로 보여 줄 stderr 마지막 줄 수.
const STDERR_TAIL_LINES: usize = 20;
/// 종료 요청 후 서버가 스스로 끝나기를 기다리는 시간. 넘으면 강제로 끝낸다.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const INITIALIZE_ID: i64 = 0;

#[derive(Debug, Default)]
pub struct ServerCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// `initialize`의 `initializationOptions`. 필요 없으면 Null.
    pub init_options: Value,
}

/// PATH 다음으로 찾아볼 흔한 설치 위치. macOS에서 Finder로 실행한 앱은 셸의 PATH를 물려받지
/// 못해(launchd의 최소 PATH만 받음) cargo·Go·Homebrew로 설치한 서버를 PATH로는 못 찾는다.
fn fallback_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        dirs.extend([".cargo/bin", "go/bin", ".local/bin"].map(|d| home.join(d)));
    }
    if !cfg!(windows) {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    dirs
}

/// PATH와 흔한 설치 위치에서 실행 파일을 찾는다. Windows는 `.exe`·`.cmd`·`.bat`을 붙여 본다.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) {
        &["exe", "cmd", "bat"]
    } else {
        &[""]
    };
    let paths = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&paths)
        .chain(fallback_dirs())
        .find_map(|dir| {
            extensions
                .iter()
                .map(|ext| {
                    let mut file = dir.join(name).into_os_string();
                    if !ext.is_empty() {
                        file.push(".");
                        file.push(ext);
                    }
                    PathBuf::from(file)
                })
                .find(|p| p.is_file())
        })
}

/// 클라이언트가 직접 답하지 않고 `ServerEvent::Request`로 넘기는 서버 요청.
fn is_relayed_request(method: &str) -> bool {
    matches!(
        method,
        "workspace/applyEdit" | "workspace/executeClientCommand"
    ) || method.starts_with("sts/")
}

/// 창 없는 앱(릴리스 빌드)에서 콘솔 프로그램을 띄우면 검은 창이 잠깐 뜨므로 막는다.
pub(crate) fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// 서버가 돌려준 오류 응답.
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    /// 문서가 바뀌는 중이라 서버가 버린 요청(ContentModified, RequestCancelled).
    /// LSP 명세상 사용자에게 보일 오류가 아니며 같은 요청을 다시 보내면 된다.
    pub fn is_retryable(&self) -> bool {
        matches!(self.code, -32801 | -32800)
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug)]
pub enum ServerEvent {
    /// 초기화가 끝나 기능을 쓸 수 있다.
    Ready,
    Response {
        id: i64,
        result: Result<Value, RpcError>,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// 앱이 처리하고 `reply`로 답해야 하는 서버 요청(지금은 `workspace/applyEdit`뿐).
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// 프로세스가 끝났거나 초기화에 실패했다. 사유(stderr 마지막 줄 등)를 담는다.
    Exited(String),
}

/// 초기화 전에 보낸 메시지는 큐에 모았다가 `initialized` 직후 순서대로 보낸다.
struct Shared {
    ready: bool,
    queue: Vec<Vec<u8>>,
    encoding: PositionEncoding,
    capabilities: Option<ServerCapabilities>,
}

pub struct LanguageServer {
    child: Option<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    shared: Arc<Mutex<Shared>>,
    next_id: i64,
    events: Receiver<ServerEvent>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // 다른 스레드가 패닉해도 상태 자체는 쓸 수 있으므로 계속 진행한다.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn encode(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

impl LanguageServer {
    /// 서버를 띄우고 `initialize`를 보낸다. 응답이 오면 `ServerEvent::Ready`가 온다.
    /// `wake`는 이벤트가 생길 때마다 불린다(UI를 다시 그리게 하는 용도).
    pub fn start(
        command: &ServerCommand,
        root: &Path,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("{} 실행 실패: {e}", command.program.display()))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            return Err("언어 서버의 표준 입출력을 열지 못했습니다".into());
        };
        let stdin = Arc::new(Mutex::new(stdin));
        let shared = Arc::new(Mutex::new(Shared {
            ready: false,
            queue: Vec::new(),
            encoding: PositionEncoding::Utf16,
            capabilities: None,
        }));

        // stderr를 읽어 비우지 않으면 파이프가 가득 차 서버가 멈춘다. 종료 사유로 쓸 마지막 몇 줄만 남긴다.
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        {
            let tail = tail.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut tail = lock(&tail);
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            });
        }

        let initialize = json!({
            "jsonrpc": "2.0",
            "id": INITIALIZE_ID,
            "method": "initialize",
            "params": initialize_params(root, &command.init_options)?,
        });
        if let Err(e) = write_message(&mut *lock(&stdin), &encode(&initialize)) {
            let _ = child.kill();
            return Err(format!("언어 서버에 초기화 요청을 보내지 못했습니다: {e}"));
        }

        let (tx, events) = channel();
        {
            let (stdin, shared) = (stdin.clone(), shared.clone());
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
            thread::spawn(move || {
                reader_loop(BufReader::new(stdout), &stdin, &shared, &tx, &*wake, &tail)
            });
        }
        Ok(Self {
            child: Some(child),
            stdin,
            shared,
            next_id: INITIALIZE_ID + 1,
            events,
        })
    }

    pub fn is_ready(&self) -> bool {
        lock(&self.shared).ready
    }

    pub fn encoding(&self) -> PositionEncoding {
        lock(&self.shared).encoding
    }

    pub fn capabilities(&self) -> Option<ServerCapabilities> {
        lock(&self.shared).capabilities.clone()
    }

    pub fn poll_events(&self) -> Vec<ServerEvent> {
        self.events.try_iter().collect()
    }

    /// 보낼 수 없으면(서버 종료) 조용히 버린다. 종료는 `ServerEvent::Exited`로 따로 알린다.
    fn send(&self, message: &Value) {
        let bytes = encode(message);
        let mut shared = lock(&self.shared);
        if shared.ready {
            let _ = write_message(&mut *lock(&self.stdin), &bytes);
        } else {
            shared.queue.push(bytes);
        }
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// 요청 번호를 돌려준다. 결과는 같은 번호의 `ServerEvent::Response`로 온다.
    pub fn request(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    pub fn did_open(&self, path: &Path, language_id: &str, version: i32, text: &str) {
        let Ok(uri) = path_to_uri(path) else { return };
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": language_id, "version": version, "text": text } }),
        );
    }

    /// 문서 전체를 보낸다(범위 없는 변경은 모든 서버가 받아들인다).
    // ponytail: 매번 전체 전송. 수 MB 파일에서 느려지면 증분 동기화(범위 변경)로.
    pub fn did_change_full(&self, path: &Path, version: i32, text: &str) {
        let Ok(uri) = path_to_uri(path) else { return };
        self.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": uri, "version": version }, "contentChanges": [{ "text": text }] }),
        );
    }

    pub fn did_save(&self, path: &Path) {
        let Ok(uri) = path_to_uri(path) else { return };
        self.notify(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": uri } }),
        );
    }

    pub fn did_close(&self, path: &Path) {
        let Ok(uri) = path_to_uri(path) else { return };
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        );
    }

    /// `ServerEvent::Request`에 답한다.
    pub fn reply(&self, id: Value, result: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }
}

impl Drop for LanguageServer {
    /// 정상 종료(shutdown → exit)를 요청하고, 시간 안에 끝나지 않으면 강제로 끝낸다.
    /// 기다리는 동안 UI가 멈추지 않도록 별도 스레드에서 처리한다.
    fn drop(&mut self) {
        if self.is_ready() {
            let id = self.next_id;
            for message in [
                json!({ "jsonrpc": "2.0", "id": id, "method": "shutdown", "params": null }),
                json!({ "jsonrpc": "2.0", "method": "exit", "params": null }),
            ] {
                let _ = write_message(&mut *lock(&self.stdin), &encode(&message));
            }
        }
        if let Some(mut child) = self.child.take() {
            thread::spawn(move || {
                let deadline = Instant::now() + SHUTDOWN_GRACE;
                while Instant::now() < deadline {
                    if !matches!(child.try_wait(), Ok(None)) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

/// `textDocument/publishDiagnostics` 알림을 (파일 경로, 진단 목록)으로 바꾼다.
pub fn parse_diagnostics(params: &Value) -> Option<(PathBuf, Vec<Diagnostic>)> {
    let params: PublishDiagnosticsParams = serde_json::from_value(params.clone()).ok()?;
    Some((uri_to_path(&params.uri)?, params.diagnostics))
}

fn initialize_params(root: &Path, init_options: &Value) -> Result<Value, String> {
    let root_uri = path_to_uri(root)?;
    let name = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut params = json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "Ferrite", "version": env!("CARGO_PKG_VERSION") },
        "rootUri": root_uri,
        "rootPath": root,
        "workspaceFolders": [{ "uri": root_uri, "name": name }],
        "capabilities": {
            // UTF-8을 먼저 제안한다. 서버가 지원하면 한글 위치 계산이 바이트 그대로라 단순하다.
            "general": { "positionEncodings": ["utf-8", "utf-16"] },
            "textDocument": {
                "synchronization": { "didSave": true, "dynamicRegistration": false },
                "publishDiagnostics": { "relatedInformation": false, "versionSupport": true },
                // 스니펫($1 등)은 아직 펼치지 못하므로 받지 않는다고 알린다.
                "completion": {
                    "completionItem": { "snippetSupport": false, "labelDetailsSupport": true },
                    "contextSupport": true,
                },
                "hover": { "contentFormat": ["plaintext", "markdown"] },
                "definition": { "linkSupport": true },
                "references": {},
                "formatting": {},
                "rename": { "prepareSupport": false },
                // 코드 액션은 편집을 나중에 받는(resolve) 방식도 지원한다(rust-analyzer 어시스트 등).
                "codeAction": {
                    "codeActionLiteralSupport": {
                        "codeActionKind": { "valueSet": [
                            "", "quickfix", "refactor", "refactor.extract", "refactor.inline",
                            "refactor.rewrite", "source", "source.organizeImports",
                        ] },
                    },
                    "resolveSupport": { "properties": ["edit"] },
                    "dataSupport": true,
                },
            },
            // 파일 만들기·이름 바꾸기·지우기(resourceOperations)는 아직 적용하지 못한다고 알린다.
            "workspace": {
                "workspaceFolders": true,
                "configuration": true,
                "applyEdit": true,
                // 코드 액션 명령·중계로 workspace/executeCommand를 보낸다. Spring Boot LS는 이것이
                // 없으면 명령 목록을 만들지 않아 초기화하다 죽는다.
                "executeCommand": { "dynamicRegistration": false },
                "workspaceEdit": { "documentChanges": true, "resourceOperations": [] },
            },
            "window": { "workDoneProgress": false },
        },
    });
    if !init_options.is_null() {
        params["initializationOptions"] = init_options.clone();
    }
    Ok(params)
}

fn reader_loop(
    mut reader: BufReader<ChildStdout>,
    stdin: &Mutex<ChildStdin>,
    shared: &Mutex<Shared>,
    tx: &Sender<ServerEvent>,
    wake: &dyn Fn(),
    tail: &Mutex<VecDeque<String>>,
) {
    let send = |event: ServerEvent| {
        let _ = tx.send(event);
        wake();
    };
    loop {
        let body = match read_message(&mut reader) {
            Ok(Some(body)) => body,
            Ok(None) => break,
            Err(e) => {
                lock(tail).push_back(format!("메시지 읽기 오류: {e}"));
                break;
            }
        };
        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str);
        match (message.get("id"), method) {
            // 편집 적용은 앱의 문서를 바꿔야 하고, 클라이언트 명령·Spring(`sts/`) 요청은 다른 서버로
            // 중계해야 하므로 넘긴다. 나머지는 바로 답한다.
            (Some(id), Some(method)) if is_relayed_request(method) => send(ServerEvent::Request {
                id: id.clone(),
                method: method.to_owned(),
                params: message.get("params").cloned().unwrap_or(Value::Null),
            }),
            (Some(id), Some(method)) => {
                reply_to_server_request(stdin, id, method, message.get("params"))
            }
            (Some(id), None) => {
                let result = match message.get("error") {
                    Some(error) => Err(RpcError {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("알 수 없는 오류")
                            .to_owned(),
                    }),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                if id.as_i64() == Some(INITIALIZE_ID) {
                    match result {
                        Ok(result) => {
                            finish_initialize(stdin, shared, &result);
                            send(ServerEvent::Ready);
                        }
                        Err(e) => send(ServerEvent::Exited(format!("초기화 실패: {e}"))),
                    }
                } else if let Some(id) = id.as_i64() {
                    send(ServerEvent::Response { id, result });
                }
            }
            (None, Some(method)) => send(ServerEvent::Notification {
                method: method.to_owned(),
                params: message.get("params").cloned().unwrap_or(Value::Null),
            }),
            (None, None) => {}
        }
    }
    let reason = lock(tail).iter().cloned().collect::<Vec<_>>().join("\n");
    send(ServerEvent::Exited(if reason.is_empty() {
        "언어 서버가 종료되었습니다".into()
    } else {
        reason
    }));
}

/// 서버 기능을 기록하고 `initialized`와 그동안 쌓인 메시지를 보낸다.
/// 앱 쪽 `send`와 같은 순서(shared → stdin)로 잠가, 큐에 넣는 중인 메시지가 빠지지 않게 한다.
fn finish_initialize(stdin: &Mutex<ChildStdin>, shared: &Mutex<Shared>, result: &Value) {
    let init: Option<InitializeResult> = serde_json::from_value(result.clone()).ok();
    let mut shared = lock(shared);
    if let Some(init) = init {
        if init.capabilities.position_encoding == Some(PositionEncodingKind::UTF8) {
            shared.encoding = PositionEncoding::Utf8;
        }
        shared.capabilities = Some(init.capabilities);
    }
    let mut stdin = lock(stdin);
    let initialized = json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} });
    let _ = write_message(&mut *stdin, &encode(&initialized));
    for message in shared.queue.drain(..) {
        let _ = write_message(&mut *stdin, &message);
    }
    shared.ready = true;
}

/// 서버가 클라이언트에게 보내는 요청에 답한다. 답하지 않으면 서버가 기다리며 멈출 수 있다.
fn reply_to_server_request(
    stdin: &Mutex<ChildStdin>,
    id: &Value,
    method: &str,
    params: Option<&Value>,
) {
    let result = match method {
        // 설정은 아직 보내지 않는다: 항목 수만큼 null을 돌려주면 서버가 기본값을 쓴다.
        "workspace/configuration" => {
            let count = params
                .and_then(|p| p.get("items"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; count])
        }
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create"
        | "window/showMessageRequest"
        | "workspace/semanticTokens/refresh"
        | "workspace/inlayHint/refresh"
        | "workspace/codeLens/refresh"
        | "workspace/diagnostic/refresh" => Value::Null,
        _ => {
            let error = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("지원하지 않는 요청: {method}") } });
            let _ = write_message(&mut *lock(stdin), &encode(&error));
            return;
        }
    };
    let reply = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let _ = write_message(&mut *lock(stdin), &encode(&reply));
}
