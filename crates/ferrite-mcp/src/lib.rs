//! MCP(Model Context Protocol) 서버: 외부 코딩 에이전트가 Ferrite의 열린 파일(저장 안 한 내용 포함)·
//! 진단·디버거 상태를 읽는다. 이 PC(127.0.0.1)에서 Streamable HTTP(`POST /mcp`, JSON 응답)로 듣고
//! `Authorization: Bearer <토큰>`을 확인한다. 브라우저에서 온 요청(다른 Origin)은 받지 않는다. 읽기만 한다.
//! 앱은 상태를 스냅샷으로 넣어 두고, 서버 스레드는 그것만 읽는다(앱 창이 내려가 있어도 답한다).

mod http;
mod tools;

use std::io::{self, BufReader};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

/// 아는 프로토콜 판(새 것부터).
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// 앱이 넣어 두는 상태.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub root: Option<String>,
    pub active: Option<Active>,
    pub documents: Vec<Document>,
    pub diagnostics: Vec<Diagnostic>,
    pub debug: Option<Debug>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Document {
    pub path: String,
    pub language: String,
    /// 저장하지 않은 변경이 있다.
    pub dirty: bool,
    pub active: bool,
    pub lines: usize,
    #[serde(skip)]
    pub text: Arc<str>,
}

/// 활성 에디터. 줄·열은 1부터.
#[derive(Clone, Debug, Serialize)]
pub struct Active {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub selection: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub severity: String,
    pub message: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Debug {
    pub name: String,
    pub adapter: String,
    /// "running"·"stopped"·"starting".
    pub state: String,
    pub frames: Vec<Frame>,
    /// 고른 프레임의 불러온 변수(범위의 첫 단).
    pub variables: Vec<Variable>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Frame {
    pub name: String,
    pub path: Option<String>,
    pub line: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Variable {
    pub scope: String,
    pub name: String,
    pub value: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
}

struct Shared {
    token: String,
    snapshot: Mutex<Arc<Snapshot>>,
    stop: AtomicBool,
}

/// 듣고 있는 서버. 버리면 멈춘다.
pub struct Server {
    address: SocketAddr,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    /// 127.0.0.1의 그 포트(0이면 빈 포트)에서 듣는다.
    pub fn start(port: u16, token: &str) -> io::Result<Server> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let address = listener.local_addr()?;
        let shared = Arc::new(Shared {
            token: token.to_owned(),
            snapshot: Mutex::new(Arc::new(Snapshot::default())),
            stop: AtomicBool::new(false),
        });
        let accepting = shared.clone();
        let accept = thread::spawn(move || {
            for stream in listener.incoming() {
                if accepting.stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else {
                    continue;
                };
                let shared = accepting.clone();
                thread::spawn(move || {
                    let _ = serve(stream, &shared);
                });
            }
        });
        Ok(Server {
            address,
            shared,
            accept: Some(accept),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// 에이전트가 읽을 상태를 바꾼다.
    pub fn set_snapshot(&self, snapshot: Snapshot) {
        *self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(snapshot);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        // 받기에서 막힌 스레드를 깨운다.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(1));
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

/// 새 토큰(무작위 32바이트, 16진수).
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS 난수");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 이 PC의 페이지(localhost)가 아니면 브라우저 요청으로 보고 막는다(DNS 다시 묶기 공격).
fn local_origin(origin: &str) -> bool {
    let host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or_default();
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// 두 글이 같은지 글자마다 다 견준다(맞힌 앞부분 길이로 토큰을 알아내지 못하게).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn serve(stream: TcpStream, shared: &Shared) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let Some(request) = http::read_request(&mut reader)? else {
        return Ok(());
    };
    let json = |status: u16, body: &Value, extra: Vec<(&'static str, String)>| {
        let mut headers = vec![("Content-Type", "application/json; charset=utf-8".to_owned())];
        headers.extend(extra);
        (status, headers, body.to_string().into_bytes())
    };
    let empty = |status: u16| (status, Vec::new(), Vec::new());
    let (status, headers, body) = if request.path != "/mcp" {
        empty(404)
    } else if request.header("origin").is_some_and(|o| !local_origin(o)) {
        empty(403)
    } else if !request
        .header("authorization")
        .and_then(|a| a.strip_prefix("Bearer "))
        .is_some_and(|t| same(t.trim(), &shared.token))
    {
        (
            401,
            vec![("WWW-Authenticate", "Bearer".to_owned())],
            Vec::new(),
        )
    } else {
        match request.method.as_str() {
            "POST" => match serde_json::from_slice::<Value>(&request.body) {
                Ok(message) => {
                    let snapshot = shared
                        .snapshot
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    let initialize = message["method"] == "initialize";
                    let answer = match &message {
                        // 예전 판의 묶음 요청.
                        Value::Array(batch) => {
                            let answers: Vec<Value> = batch
                                .iter()
                                .filter_map(|m| tools::handle(m, &snapshot))
                                .collect();
                            (!answers.is_empty()).then_some(Value::Array(answers))
                        }
                        message => tools::handle(message, &snapshot),
                    };
                    match answer {
                        Some(answer) => {
                            let session = if initialize {
                                vec![("Mcp-Session-Id", new_token())]
                            } else {
                                Vec::new()
                            };
                            json(200, &answer, session)
                        }
                        // 알림과 응답은 받기만 한다.
                        None => empty(202),
                    }
                }
                Err(e) => json(
                    400,
                    &serde_json::json!({
                        "jsonrpc": "2.0", "id": null,
                        "error": { "code": -32700, "message": format!("JSON을 읽지 못했습니다: {e}") },
                    }),
                    Vec::new(),
                ),
            },
            "DELETE" => empty(204),
            // 서버가 먼저 보내는 흐름(GET 이벤트 스트림)은 쓰지 않는다.
            _ => (405, vec![("Allow", "POST, DELETE".to_owned())], Vec::new()),
        }
    };
    http::write_response(&mut writer, status, &headers, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin과_토큰_견주기() {
        for origin in [
            "http://localhost:3000",
            "https://127.0.0.1",
            "http://[::1]:8080",
            "localhost",
        ] {
            assert!(local_origin(origin), "{origin}");
        }
        for origin in [
            "http://evil.example",
            "http://localhost.evil.example",
            "null",
        ] {
            assert!(!local_origin(origin), "{origin}");
        }
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd") && !same("abc", "ab"));
        let token = new_token();
        assert_eq!(token.len(), 64);
        assert_ne!(token, new_token());
    }
}
