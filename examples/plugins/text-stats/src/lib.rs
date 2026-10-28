//! Ferrite 예제 플러그인. 명령 두 개(선택 영역을 대문자로, 글자 수 알리기)와 패널 하나(문서 통계).
//!
//! 빌드: `cargo build --release --target wasm32-unknown-unknown`
//! 결과 `target/wasm32-unknown-unknown/release/text_stats.wasm`과 `plugin.toml`을 플러그인 폴더에 넣는다.
//!
//! 요청·답의 모양은 `crates/ferrite-plugin/src/lib.rs`에 있다. 새 플러그인은 맨 아래의
//! `ferrite_alloc`·`ferrite_call`을 그대로 베끼고 `handle`만 바꾸면 된다.

use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    Command {
        command: String,
        editor: Option<Editor>,
    },
    Panel {
        event: Option<Event>,
        editor: Option<Editor>,
    },
}

#[derive(Deserialize)]
struct Editor {
    path: String,
    /// 문서가 너무 크면 앱이 넘기지 않는다.
    text: Option<String>,
    selection: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    Click { id: String },
    Change { id: String, value: String },
}

/// 패널에서 찾는 글. 앱은 같은 인스턴스를 계속 부르므로 호출 사이에 남는다.
static FIND: Mutex<String> = Mutex::new(String::new());

fn handle(request: Request) -> Value {
    match request {
        Request::Command { command, editor } => command_reply(&command, editor.as_ref()),
        Request::Panel { event, editor } => panel(event, editor.as_ref()),
    }
}

fn message(text: impl Into<String>) -> Value {
    json!({ "actions": [{ "type": "message", "text": text.into() }] })
}

fn command_reply(id: &str, editor: Option<&Editor>) -> Value {
    let Some(editor) = editor else {
        return message("열린 문서가 없습니다");
    };
    match id {
        "upper" if editor.selection.is_empty() => message("바꿀 글을 선택하세요"),
        "upper" => json!({ "actions": [
            { "type": "replace_selection", "text": editor.selection.to_uppercase() }
        ] }),
        "count" => match &editor.text {
            Some(text) => message(counts(text)),
            None => message("문서가 너무 커서 세지 않았습니다"),
        },
        _ => message(format!("모르는 명령입니다: {id}")),
    }
}

fn counts(text: &str) -> String {
    format!(
        "줄 {} · 낱말 {} · 글자 {}",
        text.lines().count(),
        text.split_whitespace().count(),
        text.chars().count()
    )
}

fn panel(event: Option<Event>, editor: Option<&Editor>) -> Value {
    let mut find = FIND.lock().unwrap();
    let mut actions = Value::Array(Vec::new());
    match event {
        Some(Event::Change { id, value }) if id == "find" => *find = value,
        Some(Event::Click { id }) if id == "upper" => {
            actions = command_reply("upper", editor)["actions"].take();
        }
        _ => {}
    }
    let mut ui = vec![json!({ "type": "heading", "text": "문서 통계" })];
    match editor {
        None => ui.push(json!({ "type": "label", "text": "열린 문서가 없습니다" })),
        Some(editor) => {
            let name = editor.path.rsplit(['/', '\\']).next().unwrap_or_default();
            ui.push(json!({ "type": "label", "text": name }));
            match &editor.text {
                Some(text) => {
                    ui.push(json!({ "type": "code", "text": counts(text) }));
                    ui.push(json!({ "type": "separator" }));
                    // 받은 값을 그대로 돌려줘야 입력칸이 비지 않는다.
                    ui.push(
                        json!({ "type": "input", "id": "find", "value": *find, "hint": "찾을 글" }),
                    );
                    if !find.is_empty() {
                        let n = text.matches(find.as_str()).count();
                        ui.push(json!({ "type": "label", "text": format!("'{find}': {n}곳") }));
                    }
                }
                None => {
                    ui.push(json!({ "type": "label", "text": "문서가 너무 커서 세지 않았습니다" }))
                }
            }
            ui.push(json!({ "type": "separator" }));
            ui.push(json!({ "type": "button", "id": "upper", "text": "선택 영역을 대문자로" }));
        }
    }
    json!({ "actions": actions, "ui": ui })
}

// ---------------------------------------------------------------- 앱과 주고받기(베껴 쓰는 부분)

/// 마지막 답. 앱이 읽을 때까지(다음 호출까지) 살아 있어야 한다.
static REPLY: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// 앱이 요청을 쓸 `len`바이트 자리를 준다. `ferrite_call`이 넘겨받아 푼다.
#[unsafe(no_mangle)]
pub extern "C" fn ferrite_alloc(len: i32) -> i32 {
    let buf = vec![0u8; len as usize].into_boxed_slice();
    Box::into_raw(buf) as *mut u8 as i32
}

/// 요청(JSON)을 받아 답의 위치(위 32비트)와 길이(아래 32비트)를 돌려준다.
///
/// # Safety
/// `ptr`·`len`은 `ferrite_alloc(len)`이 준 자리여야 한다(앱이 지킨다).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ferrite_call(ptr: i32, len: i32) -> i64 {
    let request = unsafe {
        Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            ptr as *mut u8,
            len as usize,
        ))
    };
    let reply = match serde_json::from_slice(&request) {
        Ok(request) => handle(request),
        Err(e) => message(format!("요청을 읽지 못했습니다: {e}")),
    };
    let mut out = REPLY.lock().unwrap();
    *out = serde_json::to_vec(&reply).unwrap();
    ((out.as_ptr() as u32 as i64) << 32) | out.len() as i64
}
