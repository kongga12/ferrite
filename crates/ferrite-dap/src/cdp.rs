//! js-debug의 CDP 프록시(`requestCDPProxy`가 주는 웹소켓)로 브라우저에 CDP 명령을 보낸다. 프록시는 받은 명령을
//! js-debug 자신의 CDP 세션으로 보내므로, 도메인을 켜고 끄는 명령(`*.enable`·`*.disable`)은 js-debug의 상태를
//! 바꾼다(`Debugger.disable`이면 js-debug의 중단점이 모두 풀린다). 그래서 상태를 묻는 명령과, 걸자마자 지우는
//! 중단점만 쓴다.
//! - 화면의 원래 HTML(`Page.getResourceContent`): js-debug는 같은 화면 주소의 인라인 스크립트를 하나만 남겨
//!   블록마다 내용을 줄 수 없지만, 원래 HTML이 있으면 블록 자리에서 그 글을 잘라 낼 수 있다.
//! - 블록의 글(`Debugger.getScriptSource`): 프레임 안 화면은 원래 HTML을 주지 않아("not cached") 블록 자리의
//!   스크립트를 찾아 그 글을 받는다.
//! - 다시 부르기: 맨 위 화면은 `Page.reload`, 프레임 안 화면은 그 프레임만 같은 주소로 다시 연다.
//!
//! 로컬 프록시와 글 틀만 주고받는 가장 작은 웹소켓 클라이언트다.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// 답을 기다리는 시간.
const TIMEOUT: Duration = Duration::from_secs(5);
/// 받을 메시지의 상한(화면 HTML도 이보다 작다).
const MAX_MESSAGE: u64 = 64 << 20;

/// 화면을 연 프레임이 지금 없다(그사이 다른 화면으로 갔거나, 품은 화면이 다시 열리는 중이라 아직 없다).
pub const NOT_OPEN: &str = "화면이 지금 열려 있지 않습니다";

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
/// 없으면 `NOT_OPEN`이다(다른 화면의 HTML로 블록을 맞추면 틀린다). 프레임 안 화면은 브라우저가 HTML을 남겨
/// 두지 않아 받지 못한다.
pub fn page_html(proxy: &Proxy, page: &str) -> Result<String, String> {
    let mut socket = Socket::connect(proxy)?;
    let tree = socket.call(1, "Page.getFrameTree", json!({}))?;
    let frame = find_frame(&tree["frameTree"], page).ok_or(NOT_OPEN)?;
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

/// `page`를 연 프레임의 인라인 스크립트 블록 중 시작 (줄, 칸)(1부터)이 `starts`인 것들의 글. 블록 자리에 조건이
/// 거짓인 중단점을 걸어 그 자리의 스크립트를 찾고(`Debugger.setBreakpointByUrl`의 `locations`), 그 글을 받은
/// 뒤(`Debugger.getScriptSource`) 중단점을 바로 지운다. 조건이 거짓이라 그사이에 멈추지 않는다. 그 자리의
/// 스크립트가 하나가 아니면(찾지 못했거나 여럿이면) 실패다. 원래 HTML을 받을 수 없는 프레임 안 화면에 쓴다.
pub fn script_blocks(
    proxy: &Proxy,
    page: &str,
    starts: &[(usize, usize)],
) -> Result<Vec<(usize, usize, String)>, String> {
    let mut socket = Socket::connect(proxy)?;
    let tree = socket.call(1, "Page.getFrameTree", json!({}))?;
    let url = find_frame(&tree["frameTree"], page).ok_or(NOT_OPEN)?["url"]
        .as_str()
        .ok_or("화면 프레임이 없습니다")?
        .to_owned();
    let mut id = 1;
    let mut next = || {
        id += 1;
        id
    };
    let mut blocks = Vec::new();
    for &(line, column) in starts {
        let set = socket.call(
            next(),
            "Debugger.setBreakpointByUrl",
            json!({ "url": url, "lineNumber": line.saturating_sub(1),
                    "columnNumber": column.saturating_sub(1), "condition": "false" }),
        )?;
        let mut scripts: Vec<&str> = set["locations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|location| location["scriptId"].as_str())
            .collect();
        scripts.dedup();
        let text = match scripts.as_slice() {
            [script] => socket
                .call(
                    next(),
                    "Debugger.getScriptSource",
                    json!({ "scriptId": script }),
                )
                .map(|source| source["scriptSource"].as_str().map(str::to_owned)),
            _ => Ok(None),
        };
        // 글을 받았든 못 받았든 건 중단점은 지운다.
        if let Some(breakpoint) = set["breakpointId"].as_str() {
            socket.call(
                next(),
                "Debugger.removeBreakpoint",
                json!({ "breakpointId": breakpoint }),
            )?;
        }
        match text? {
            Some(text) => blocks.push((line, column, text)),
            None => {
                return Err(format!(
                    "{line}:{column} 블록의 스크립트를 가려내지 못했습니다"
                ));
            }
        }
    }
    Ok(blocks)
}

/// 다시 부를 곳.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// 맨 위 화면(`Page.reload`).
    Top,
    /// 프레임 안 화면: 그 프레임만 같은 주소로 다시 연다(`Page.navigate`, GET).
    Frame {
        id: String,
        url: String,
        /// 그 프레임을 품은 화면들의 주소(스킴·`#` 뒤를 뺀 것, 맨 위 화면부터). 품은 화면을 다시 부르면 이
        /// 프레임도 다시 열린다.
        ancestors: Vec<String>,
    },
}

impl Target {
    /// 이 곳을 품은 화면들의 주소(맨 위 화면이면 없다).
    pub fn ancestors(&self) -> &[String] {
        match self {
            Self::Top => &[],
            Self::Frame { ancestors, .. } => ancestors,
        }
    }
}

/// 화면을 다 읽은 뒤의 처지(다시 불러도 되는지 가린다).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loaded {
    /// 다시 불러도 된다.
    Reload(Target),
    /// 폼을 보내 연 맨 위 화면(다시 부르면 폼을 다시 보낼 수 있다).
    FormSubmit,
    /// 처음 주소(`src`)에서 링크·폼으로 다른 화면으로 간 프레임(폼을 보냈을 수 있다).
    FrameMoved,
}

/// `page`를 연 프레임이 화면을 다 읽을 때까지(`document.readyState`가 "loading"이 아닐 때까지, 넉넉히 `wait`)
/// 기다린 뒤 다시 불러도 되는지 알린다. 다 읽은 뒤면 화면의 인라인 스크립트가 모두 돌아 js-debug가 블록을 다
/// 알렸다.
/// - 맨 위 화면: 폼을 보내 열었으면(`Page.getNavigationHistory`의 지금 항목이 "form_submit") 다시 부르지 않는다.
/// - 프레임 안 화면: 프레임이 처음 주소(`frameElement.src`)를 그대로 보이면(그 주소를 GET으로 열었다) 그
///   프레임만 다시 연다. 그 프레임의 문서에서 묻는다(격리된 세계라 화면의 스크립트와 섞이지 않는다).
pub fn page_loaded(proxy: &Proxy, page: &str, wait: Duration) -> Result<Loaded, String> {
    let mut socket = Socket::connect(proxy)?;
    let tree = socket.call(1, "Page.getFrameTree", json!({}))?;
    let path = find_path(&tree["frameTree"], page).ok_or(NOT_OPEN)?;
    let (frame, ancestors) = path.split_last().ok_or("화면 프레임이 없습니다")?;
    let mut id = 2;
    if ancestors.is_empty() {
        if !socket.wait_loaded(&mut id, wait, None)? {
            return Err("화면을 다 읽지 못했습니다".into());
        }
        let history = socket.call(id, "Page.getNavigationHistory", json!({}))?;
        let current = history["currentIndex"]
            .as_u64()
            .and_then(|i| history["entries"].get(usize::try_from(i).ok()?));
        return Ok(
            if current.is_some_and(|entry| entry["transitionType"] == "form_submit") {
                Loaded::FormSubmit
            } else {
                Loaded::Reload(Target::Top)
            },
        );
    }
    let (Some(frame_id), Some(url)) = (frame["id"].as_str(), frame["url"].as_str()) else {
        return Err("화면 프레임이 없습니다".into());
    };
    let world = socket.call(
        id,
        "Page.createIsolatedWorld",
        json!({ "frameId": frame_id, "worldName": "ferrite" }),
    )?;
    id += 1;
    let context = world["executionContextId"]
        .as_i64()
        .ok_or("프레임에서 물을 곳이 없습니다")?;
    if !socket.wait_loaded(&mut id, wait, Some(context))? {
        return Err("화면을 다 읽지 못했습니다".into());
    }
    let from_src = socket.call(
        id,
        "Runtime.evaluate",
        json!({ "contextId": context, "returnByValue": true,
                "expression": "!!window.frameElement && window.frameElement.src.split('#')[0] === location.href.split('#')[0]" }),
    )?;
    Ok(if from_src["result"]["value"] == true {
        Loaded::Reload(Target::Frame {
            id: frame_id.to_owned(),
            url: url.to_owned(),
            ancestors: ancestors
                .iter()
                .map(|frame| bare(frame["url"].as_str().unwrap_or_default()).to_owned())
                .collect(),
        })
    } else {
        Loaded::FrameMoved
    })
}

/// 맨 위 화면을 다 읽을 때까지(`document.readyState`가 "loading"이 아닐 때까지, 넉넉히 `wait`) 기다린다. 다
/// 읽었으면 true. 읽는 중에 멈춰 있으면 CDP가 화면 HTML(`Page.getResourceContent`)에 답하지 않으므로 그
/// 전에 본다.
pub fn wait_loaded(proxy: &Proxy, wait: Duration) -> Result<bool, String> {
    Socket::connect(proxy)?.wait_loaded(&mut 1, wait, None)
}

/// 화면을 다시 부른다: 맨 위 화면은 `Page.reload`, 프레임 안 화면은 그 프레임만 같은 주소로 다시 연다.
pub fn reload(proxy: &Proxy, target: &Target) -> Result<(), String> {
    let mut socket = Socket::connect(proxy)?;
    match target {
        Target::Top => socket.call(1, "Page.reload", json!({})).map(drop),
        Target::Frame { id, url, .. } => {
            let result = socket.call(1, "Page.navigate", json!({ "frameId": id, "url": url }))?;
            match result["errorText"].as_str() {
                Some(error) if !error.is_empty() => Err(format!("Page.navigate: {error}")),
                _ => Ok(()),
            }
        }
    }
}

/// 주소에서 스킴과 `#` 뒤를 뺀 것.
fn bare(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('#').next().unwrap_or(rest)
}

/// 주소(스킴·`#` 뒤를 뺀 것)가 `page`인 프레임(자식 프레임까지).
fn find_frame<'a>(node: &'a Value, page: &str) -> Option<&'a Value> {
    find_path(node, page)?.pop()
}

/// 맨 위 프레임부터 주소가 `page`인 프레임까지의 프레임들.
fn find_path<'a>(node: &'a Value, page: &str) -> Option<Vec<&'a Value>> {
    let frame = &node["frame"];
    if bare(frame["url"].as_str().unwrap_or_default()) == page {
        return Some(vec![frame]);
    }
    let mut path = node["childFrames"]
        .as_array()?
        .iter()
        .find_map(|child| find_path(child, page))?;
    path.insert(0, frame);
    Some(path)
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

    /// 화면(`context`가 없으면 맨 위 화면, 있으면 그 실행 문맥의 화면)의 `document.readyState`가 "loading"이
    /// 아닐 때까지(넉넉히 `wait`) 0.1초마다 본다(멈춰 있어도 답한다). 다 읽었으면 true. 명령 번호는 `id`부터
    /// 쓴다.
    fn wait_loaded(
        &mut self,
        id: &mut u64,
        wait: Duration,
        context: Option<i64>,
    ) -> Result<bool, String> {
        let started = Instant::now();
        let mut params = json!({ "expression": "document.readyState", "returnByValue": true });
        if let Some(context) = context {
            params["contextId"] = json!(context);
        }
        loop {
            let state = self.call(*id, "Runtime.evaluate", params.clone())?;
            *id += 1;
            if state["result"]["value"].as_str() != Some("loading") {
                return Ok(true);
            }
            if started.elapsed() >= wait {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(100));
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
