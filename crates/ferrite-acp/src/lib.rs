//! ACP(Agent Client Protocol) 클라이언트. 코딩 에이전트 프로세스(Gemini CLI `--acp`, Claude Code의 ACP
//! 어댑터 등)를 띄워 표준 입출력으로 JSON-RPC를 줄마다 하나씩 주고받는다. 시작하면 `initialize` →
//! `session/new`를 차례로 보내고, `session/prompt`로 대화한다. 에이전트가 보내는 것:
//! - `session/update`(알림): 답 글·생각·도구 호출(diff 포함)·계획.
//! - `session/request_permission`·`fs/read_text_file`·`fs/write_text_file`(요청): 앱이 답한다.
//!
//! 터미널 기능은 주지 않는다(에이전트가 자기 셸 도구를 쓰고 권한 요청으로 묻는다).

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use serde_json::{Value, json};

/// ACP 판(깨지는 변경에만 오른다).
pub const PROTOCOL_VERSION: u64 = 1;
/// 에이전트가 끝났을 때 보일 표준 오류의 끝 줄 수.
const STDERR_TAIL: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Agent,
    Thought,
    User,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    pub path: PathBuf,
    /// 새 파일이면 None.
    pub old_text: Option<String>,
    pub new_text: String,
}

/// 도구 호출. 고침(`tool_call_update`)은 바뀐 칸만 Some이다.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub title: Option<String>,
    /// read·edit·delete·move·search·execute·think·fetch·switch_mode·other
    pub kind: Option<String>,
    /// pending·in_progress·completed·failed
    pub status: Option<String>,
    pub diffs: Vec<Diff>,
    pub text: Option<String>,
}

impl ToolCall {
    /// 고침을 합친다.
    pub fn merge(&mut self, update: ToolCall) {
        if update.title.is_some() {
            self.title = update.title;
        }
        if update.kind.is_some() {
            self.kind = update.kind;
        }
        if update.status.is_some() {
            self.status = update.status;
        }
        if !update.diffs.is_empty() {
            self.diffs = update.diffs;
        }
        if update.text.is_some() {
            self.text = update.text;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    /// allow_once·allow_always·reject_once·reject_always
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanEntry {
    pub content: String,
    /// pending·in_progress·completed
    pub status: String,
}

/// 에이전트에서 온 것. 요청(`Permission`·`ReadFile`·`WriteFile`)은 `id`로 답한다.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// 세션이 열렸다(대화할 수 있다).
    Ready,
    Message(MessageKind, String),
    ToolCall(ToolCall),
    ToolCallUpdate(ToolCall),
    Plan(Vec<PlanEntry>),
    Permission {
        id: Value,
        tool: ToolCall,
        options: Vec<PermissionOption>,
    },
    ReadFile {
        id: Value,
        path: PathBuf,
        /// 1부터.
        line: Option<usize>,
        limit: Option<usize>,
    },
    WriteFile {
        id: Value,
        path: PathBuf,
        content: String,
    },
    /// 대화 한 차례가 끝났다(end_turn·max_tokens·max_turn_requests·refusal·cancelled).
    TurnEnded(String),
    Error(String),
    /// 에이전트가 끝났다(표준 오류의 끝 줄).
    Exited(String),
}

enum Incoming {
    Message(Value),
    Closed,
}

enum Pending {
    Initialize,
    NewSession,
    Prompt,
}

pub struct Agent {
    writer: Box<dyn Write + Send>,
    rx: Receiver<Incoming>,
    next_id: i64,
    pending: HashMap<i64, Pending>,
    session: Option<String>,
    cwd: PathBuf,
    mcp_servers: Vec<Value>,
    child: Option<Child>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    closed: bool,
}

impl Agent {
    /// 에이전트 프로세스를 띄운다. 이름만 적은 프로그램은 PATH에서 찾는다(Windows의 `.cmd` 포함).
    /// `mcp_servers`는 세션에 넘길 MCP 서버(ACP의 `McpServer` 꼴).
    pub fn spawn(
        program: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &Path,
        mcp_servers: Vec<Value>,
    ) -> Result<Agent, String> {
        let path = if program.contains(['/', '\\']) {
            PathBuf::from(program)
        } else {
            ferrite_lsp::client::find_program(program)
                .ok_or_else(|| format!("{program}을(를) PATH에서 찾지 못했습니다"))?
        };
        let mut command = Command::new(&path);
        command
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        ferrite_lsp::client::hide_console(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("{} 실행 실패: {e}", path.display()))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            return Err("에이전트의 표준 입출력을 열지 못했습니다".into());
        };
        let mut agent = Agent::with_streams(stdout, stdin, cwd, mcp_servers);
        let tail = agent.stderr.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut tail = tail.lock().unwrap_or_else(PoisonError::into_inner);
                if tail.len() == STDERR_TAIL {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
        agent.child = Some(child);
        Ok(agent)
    }

    /// 이미 연 흐름으로 말한다(시험·다른 띄우기). `initialize`를 바로 보낸다.
    pub fn with_streams(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        cwd: &Path,
        mcp_servers: Vec<Value>,
    ) -> Agent {
        let (tx, rx) = channel();
        spawn_reader(reader, tx);
        let mut agent = Agent {
            writer: Box::new(writer),
            rx,
            next_id: 1,
            pending: HashMap::new(),
            session: None,
            cwd: cwd.to_path_buf(),
            mcp_servers,
            child: None,
            stderr: Arc::default(),
            closed: false,
        };
        agent.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true }, "terminal": false },
                "clientInfo": { "name": "ferrite", "title": "Ferrite", "version": env!("CARGO_PKG_VERSION") },
            }),
            Pending::Initialize,
        );
        agent
    }

    /// 세션이 열려 대화할 수 있다.
    pub fn is_ready(&self) -> bool {
        self.session.is_some() && !self.closed
    }

    fn send(&mut self, message: Value) {
        // 쓰기 실패(에이전트가 끝남)는 읽기 쪽이 Exited로 알린다.
        let _ = writeln!(self.writer, "{message}").and_then(|()| self.writer.flush());
    }

    fn request(&mut self, method: &str, params: Value, pending: Pending) {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, pending);
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    fn respond(&mut self, id: &Value, result: Result<Value, String>) {
        let message = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => json!({
                "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": message },
            }),
        };
        self.send(message);
    }

    /// 글을 보낸다. 답은 `Message`·`ToolCall` 등으로 오고 `TurnEnded`로 끝난다.
    pub fn prompt(&mut self, text: &str) -> Result<(), String> {
        let session = self
            .session
            .clone()
            .ok_or("세션이 아직 열리지 않았습니다")?;
        self.request(
            "session/prompt",
            json!({ "sessionId": session, "prompt": [{ "type": "text", "text": text }] }),
            Pending::Prompt,
        );
        Ok(())
    }

    /// 돌고 있는 차례를 멈춘다(에이전트가 `cancelled`로 끝낸다).
    pub fn cancel(&mut self) {
        if let Some(session) = self.session.clone() {
            self.send(json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": session } }));
        }
    }

    /// 권한 요청에 답한다. None이면 물러남(cancelled).
    pub fn answer_permission(&mut self, id: &Value, option: Option<&str>) {
        let outcome = match option {
            Some(option) => json!({ "outcome": "selected", "optionId": option }),
            None => json!({ "outcome": "cancelled" }),
        };
        self.respond(id, Ok(json!({ "outcome": outcome })));
    }

    pub fn answer_read(&mut self, id: &Value, content: Result<String, String>) {
        self.respond(id, content.map(|c| json!({ "content": c })));
    }

    pub fn answer_write(&mut self, id: &Value, result: Result<(), String>) {
        self.respond(id, result.map(|()| json!({})));
    }

    /// 받은 것을 처리해 돌려준다. 매 프레임 부른다.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(incoming) = self.rx.try_recv() {
            match incoming {
                Incoming::Message(message) => self.handle(message, &mut events),
                Incoming::Closed => {
                    self.closed = true;
                    let tail: Vec<String> = self
                        .stderr
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .iter()
                        .cloned()
                        .collect();
                    events.push(Event::Exited(tail.join("\n")));
                }
            }
        }
        events
    }

    fn handle(&mut self, message: Value, events: &mut Vec<Event>) {
        let method = message["method"].as_str().map(str::to_owned);
        match (method, message.get("id").cloned()) {
            // 에이전트의 요청
            (Some(method), Some(id)) => {
                let params = &message["params"];
                let path = PathBuf::from(params["path"].as_str().unwrap_or_default());
                let number = |key: &str| params[key].as_u64().map(|n| n as usize);
                match method.as_str() {
                    "session/request_permission" => events.push(Event::Permission {
                        id,
                        tool: tool_call(&params["toolCall"]),
                        options: params["options"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|o| PermissionOption {
                                id: text(&o["optionId"]),
                                name: text(&o["name"]),
                                kind: text(&o["kind"]),
                            })
                            .collect(),
                    }),
                    "fs/read_text_file" => events.push(Event::ReadFile {
                        id,
                        path,
                        line: number("line"),
                        limit: number("limit"),
                    }),
                    "fs/write_text_file" => events.push(Event::WriteFile {
                        id,
                        path,
                        content: text(&params["content"]),
                    }),
                    _ => self.send(json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": { "code": -32601, "message": format!("지원하지 않는 메서드입니다: {method}") },
                    })),
                }
            }
            // 알림
            (Some(method), None) => {
                if method == "session/update" {
                    events.extend(session_update(&message["params"]["update"]));
                }
            }
            // 우리 요청의 응답
            (None, Some(id)) => {
                let Some(pending) = id.as_i64().and_then(|id| self.pending.remove(&id)) else {
                    return;
                };
                if let Some(error) = message.get("error") {
                    let text = error["message"].as_str().unwrap_or("알 수 없는 오류");
                    // ACP의 인증 필요(-32000)는 다른 거절에도 쓰이므로 글을 보고 로그인 안내를 붙인다.
                    let lower = text.to_lowercase();
                    let auth = lower.contains("auth")
                        || lower.contains("login")
                        || lower.contains("log in");
                    events.push(Event::Error(match pending {
                        Pending::NewSession if auth => format!(
                            "에이전트에 로그인해야 합니다(터미널에서 에이전트를 한 번 실행해 로그인하세요): {text}"
                        ),
                        Pending::NewSession => format!("에이전트가 세션을 열지 않았습니다: {text}"),
                        _ => text.to_owned(),
                    }));
                    if matches!(pending, Pending::Prompt) {
                        events.push(Event::TurnEnded("error".into()));
                    }
                    return;
                }
                let result = &message["result"];
                match pending {
                    Pending::Initialize => {
                        // HTTP MCP를 못 받는 에이전트에는 표준 입출력 서버만 넘긴다.
                        let http = result["agentCapabilities"]["mcpCapabilities"]["http"] == true;
                        let servers: Vec<Value> = self
                            .mcp_servers
                            .iter()
                            .filter(|s| http || s["type"] != "http")
                            .cloned()
                            .collect();
                        let cwd = self.cwd.to_string_lossy().into_owned();
                        self.request(
                            "session/new",
                            json!({ "cwd": cwd, "mcpServers": servers }),
                            Pending::NewSession,
                        );
                    }
                    Pending::NewSession => {
                        self.session = result["sessionId"].as_str().map(str::to_owned);
                        events.push(match &self.session {
                            Some(_) => Event::Ready,
                            None => Event::Error("에이전트가 세션 번호를 주지 않았습니다".into()),
                        });
                    }
                    Pending::Prompt => events.push(Event::TurnEnded(
                        result["stopReason"]
                            .as_str()
                            .unwrap_or("end_turn")
                            .to_owned(),
                    )),
                }
            }
            (None, None) => {}
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_reader(reader: impl Read + Send + 'static, tx: Sender<Incoming>) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if let Ok(message) = serde_json::from_str::<Value>(&line)
                && tx.send(Incoming::Message(message)).is_err()
            {
                return;
            }
        }
        let _ = tx.send(Incoming::Closed);
    });
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn some(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

/// 글 조각(ContentBlock). 글이 아니면 그 종류를 적는다.
fn content_text(content: &Value) -> String {
    match content["type"].as_str() {
        Some("text") => text(&content["text"]),
        Some("resource_link") => format!("[{}]", text(&content["uri"])),
        Some(other) => format!("[{other}]"),
        None => String::new(),
    }
}

fn tool_call(value: &Value) -> ToolCall {
    let mut call = ToolCall {
        id: text(&value["toolCallId"]),
        title: some(&value["title"]),
        kind: some(&value["kind"]),
        status: some(&value["status"]),
        ..Default::default()
    };
    let mut texts = Vec::new();
    for item in value["content"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("diff") => call.diffs.push(Diff {
                path: PathBuf::from(text(&item["path"])),
                old_text: some(&item["oldText"]),
                new_text: text(&item["newText"]),
            }),
            Some("content") => texts.push(content_text(&item["content"])),
            _ => {}
        }
    }
    if !texts.is_empty() {
        call.text = Some(texts.join("\n"));
    }
    call
}

fn session_update(update: &Value) -> Option<Event> {
    let kind = match update["sessionUpdate"].as_str()? {
        "agent_message_chunk" => MessageKind::Agent,
        "agent_thought_chunk" => MessageKind::Thought,
        "user_message_chunk" => MessageKind::User,
        "tool_call" => return Some(Event::ToolCall(tool_call(update))),
        "tool_call_update" => return Some(Event::ToolCallUpdate(tool_call(update))),
        "plan" => {
            return Some(Event::Plan(
                update["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| PlanEntry {
                        content: text(&e["content"]),
                        status: text(&e["status"]),
                    })
                    .collect(),
            ));
        }
        _ => return None,
    };
    Some(Event::Message(kind, content_text(&update["content"])))
}
