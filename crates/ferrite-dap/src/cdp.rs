//! js-debug의 CDP 프록시(`requestCDPProxy`가 주는 웹소켓)로 브라우저에 CDP 명령을 보낸다. 화면의 원래
//! HTML(`Page.getResourceContent`)을 받는 데 쓴다. js-debug는 같은 화면 주소의 인라인 스크립트를 하나만
//! 남겨 블록마다 내용을 줄 수 없지만, 원래 HTML이 있으면 블록 자리에서 그 글을 잘라 낼 수 있다.
//! 로컬 프록시와 글 틀만 주고받는 가장 작은 웹소켓 클라이언트다.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

/// 답을 기다리는 시간.
const TIMEOUT: Duration = Duration::from_secs(5);
/// 받을 메시지의 상한(화면 HTML도 이보다 작다).
const MAX_MESSAGE: u64 = 64 << 20;

/// 프록시 주소(`requestCDPProxy`의 답).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Proxy {
    pub fn from_json(body: &Value) -> Option<Self> {
        Some(Self {
            host: body["host"].as_str()?.to_owned(),
            port: u16::try_from(body["port"].as_u64()?).ok()?,
            path: body["path"].as_str().unwrap_or("/").to_owned(),
        })
    }
}

/// `page`(스킴 없는 화면 주소, 예 `127.0.0.1:8080/app/list.do`)를 연 프레임이 받은 원래 HTML. 그런 프레임이
/// 없으면 맨 위 프레임의 것.
pub fn page_html(proxy: &Proxy, page: &str) -> Result<String, String> {
    let mut socket = Socket::connect(proxy)?;
    let tree = socket.call(1, "Page.getFrameTree", json!({}))?;
    let top = &tree["frameTree"];
    let frame = find_frame(top, page).unwrap_or(&top["frame"]);
    let (Some(id), Some(url)) = (frame["id"].as_str(), frame["url"].as_str()) else {
        return Err("화면 프레임이 없습니다".into());
    };
    let content = socket.call(
        2,
        "Page.getResourceContent",
        json!({ "frameId": id, "url": url }),
    )?;
    if content["base64Encoded"].as_bool() == Some(true) {
        return Err("화면 HTML이 이진으로 왔습니다".into());
    }
    content["content"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "화면 HTML이 없습니다".into())
}

/// 주소(스킴·`#` 뒤를 뺀 것)가 `page`인 프레임(자식 프레임까지).
fn find_frame<'a>(node: &'a Value, page: &str) -> Option<&'a Value> {
    let url = node["frame"]["url"].as_str().unwrap_or_default();
    let bare = url.split_once("://").map_or(url, |(_, rest)| rest);
    let bare = bare.split('#').next().unwrap_or(bare);
    if bare == page {
        return Some(&node["frame"]);
    }
    node["childFrames"]
        .as_array()?
        .iter()
        .find_map(|child| find_frame(child, page))
}

struct Socket {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Socket {
    fn connect(proxy: &Proxy) -> Result<Self, String> {
        let io = |e: std::io::Error| format!("CDP 프록시: {e}");
        let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port)).map_err(io)?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(io)?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(io)?;
        // 키는 16바이트의 base64여야 한다(js-debug의 프록시가 따진다). 로컬 프록시라 `Sec-WebSocket-Accept`는
        // 보지 않는다.
        write!(
            stream,
            "GET {} HTTP/1.1\r\nHost: {}:{}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: ZmVycml0ZS1jZHAtcHJveA==\r\nSec-WebSocket-Version: 13\r\n\r\n",
            proxy.path, proxy.host, proxy.port
        )
        .map_err(io)?;
        let mut reader = BufReader::new(stream.try_clone().map_err(io)?);
        let mut status = String::new();
        reader.read_line(&mut status).map_err(io)?;
        if status.split_whitespace().nth(1) != Some("101") {
            return Err(format!(
                "CDP 프록시가 웹소켓을 열지 않았습니다: {}",
                status.trim()
            ));
        }
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).map_err(io)? == 0 {
                return Err("CDP 프록시가 연결을 닫았습니다".into());
            }
            if header.trim().is_empty() {
                break;
            }
        }
        Ok(Self { stream, reader })
    }

    /// CDP 명령 하나를 보내고 그 답의 `result`를 돌려준다(그 사이에 온 다른 메시지는 버린다).
    fn call(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        let request = json!({ "id": id, "method": method, "params": params });
        self.send(0x1, request.to_string().as_bytes())?;
        loop {
            let message: Value = serde_json::from_str(&self.receive()?)
                .map_err(|e| format!("CDP 답을 읽지 못했습니다: {e}"))?;
            if message["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!(
                    "{method}: {}",
                    error["message"].as_str().unwrap_or("실패")
                ));
            }
            return Ok(message["result"].clone());
        }
    }

    /// 틀 하나(끝 표시). 클라이언트가 보내는 틀은 가려야 한다.
    fn send(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let mut frame = vec![0x80 | opcode];
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if let Ok(short) = u16::try_from(len) {
            frame.push(0x80 | 126);
            frame.extend(short.to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend((len as u64).to_be_bytes());
        }
        // ponytail: 가림쇠를 고정한다. 로컬 프록시라 가림은 형식만 맞추면 된다.
        let mask = [0x6d, 0x61, 0x73, 0x6b];
        frame.extend(mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream
            .write_all(&frame)
            .map_err(|e| format!("CDP 프록시: {e}"))
    }

    /// 글 메시지 하나(나뉜 틀은 잇는다). ping에는 pong으로 답하고, 닫힘이면 실패다.
    fn receive(&mut self) -> Result<String, String> {
        let io = |e: std::io::Error| format!("CDP 프록시: {e}");
        let mut message = Vec::new();
        loop {
            let mut head = [0u8; 2];
            self.reader.read_exact(&mut head).map_err(io)?;
            let (fin, opcode) = (head[0] & 0x80 != 0, head[0] & 0x0f);
            let mut len = u64::from(head[1] & 0x7f);
            if len == 126 {
                let mut short = [0u8; 2];
                self.reader.read_exact(&mut short).map_err(io)?;
                len = u64::from(u16::from_be_bytes(short));
            } else if len == 127 {
                let mut long = [0u8; 8];
                self.reader.read_exact(&mut long).map_err(io)?;
                len = u64::from_be_bytes(long);
            }
            let mask = if head[1] & 0x80 != 0 {
                let mut mask = [0u8; 4];
                self.reader.read_exact(&mut mask).map_err(io)?;
                Some(mask)
            } else {
                None
            };
            if len > MAX_MESSAGE || message.len() as u64 + len > MAX_MESSAGE {
                return Err("CDP 메시지가 너무 큽니다".into());
            }
            let mut payload = vec![0u8; len as usize];
            self.reader.read_exact(&mut payload).map_err(io)?;
            if let Some(mask) = mask {
                payload
                    .iter_mut()
                    .enumerate()
                    .for_each(|(i, b)| *b ^= mask[i % 4]);
            }
            match opcode {
                0x0 | 0x1 => {
                    message.extend(payload);
                    if fin {
                        return String::from_utf8(message)
                            .map_err(|_| "CDP 메시지가 UTF-8이 아닙니다".into());
                    }
                }
                0x8 => return Err("CDP 프록시가 연결을 닫았습니다".into()),
                0x9 => self.send(0xa, &payload)?,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;
