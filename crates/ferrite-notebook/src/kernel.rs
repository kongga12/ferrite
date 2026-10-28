//! Jupyter 커널 연결. 사용자의 Python으로 사이드카(`sidecar/ferrite_kernel.py`)를 띄우면 사이드카가
//! jupyter_client로 커널을 띄우고 ZeroMQ로 말한다. Ferrite와 사이드카는 표준 입출력의 한 줄 JSON으로
//! 주고받는다. 버리면 사이드카가 커널을 끄고 끝난다(UI가 기다리지 않게 따로 거둔다).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::ops::Range;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

const SIDECAR: &str = include_str!("../sidecar/ferrite_kernel.py");
/// 사이드카가 끝났을 때 까닭으로 보일 표준 오류의 마지막 줄 수.
const LOG_LINES: usize = 20;

/// 실행 하나가 끝난 모양.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Error,
    /// 앞 실행이 오류로 끝나 하지 않았다.
    Aborted,
    /// 실행 중에 커널 프로세스가 죽었다.
    Dead,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// 커널이 준비됐다. `missing`: 노트북의 커널이 없어 이 Python의 python3 커널을 띄웠으면 그 이름.
    Started {
        name: String,
        display_name: String,
        language: String,
        missing: Option<String>,
    },
    /// 실행 `id`의 출력 하나(nbformat 출력 JSON).
    Output { id: u64, output: Value },
    /// 실행 `id`의 출력을 지운다(`clear_output`).
    Clear { id: u64 },
    Done {
        id: u64,
        execution_count: Option<u64>,
        status: Status,
    },
    /// `frames`의 답: 커널의 DataFrame 목록.
    Frames {
        id: u64,
        result: Result<Vec<FrameInfo>, String>,
    },
    /// `page`의 답.
    Page {
        id: u64,
        result: Result<Page, String>,
    },
    /// 사이드카가 끝났다(띄우지 못한 것도). 다시 쓰려면 새로 띄운다.
    Ended(String),
}

/// 커널 이름공간의 DataFrame 하나.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub name: String,
    pub rows: usize,
    pub columns: usize,
}

/// DataFrame의 행 한 묶음(정렬했으면 정렬한 순서로).
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// 지금 DataFrame의 전체 행 수(받는 사이에 바뀌었을 수 있다).
    pub rows: usize,
    pub columns: Vec<String>,
    /// 열마다 pandas 형식 이름(int64, object, datetime64[ns] 등).
    pub dtypes: Vec<String>,
    /// 행마다 색인을 글로.
    pub index: Vec<String>,
    /// 행마다 칸 값(JSON: 수, 글, null, 날짜는 ISO 글).
    pub data: Vec<Vec<Value>>,
}

pub struct Kernel {
    /// 닫으면 사이드카가 커널을 끄고 끝난다.
    input: Option<ChildStdin>,
    child: Option<Child>,
    events: Receiver<Event>,
    next_id: u64,
}

impl Kernel {
    /// `python`으로 사이드카를 띄우고 커널(`kernel`, 없으면 python3)을 띄우게 한다. 곧바로 돌아오고
    /// 결과는 `Started`나 `Ended`로 온다. 그 전에 보낸 실행은 커널이 뜬 뒤 차례로 돈다.
    /// `wake`는 이벤트마다 읽기 스레드에서 불린다(UI를 다시 그리게).
    pub fn start(
        python: &Path,
        kernel: Option<&str>,
        cwd: &Path,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        // 소스는 지역 문자 집합과 상관없게 base64로 넘긴다. 노트북 폴더 등 현재 폴더의 같은 이름 모듈을
        // 읽지 않게 sys.path에서 현재 폴더('')를 먼저 뺀다(sys는 내장 모듈이라 가려지지 않는다).
        let bootstrap = "import sys;sys.path=[p for p in sys.path if p];import base64;\
                         exec(compile(base64.b64decode(sys.argv[1]).decode('utf-8'),\
                         'ferrite_kernel.py','exec'))";
        let mut command = Command::new(python);
        command
            .arg("-c")
            .arg(bootstrap)
            .arg(base64::engine::general_purpose::STANDARD.encode(SIDECAR))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("{}: {e}", python.display()))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("커널 사이드카의 입출력을 열지 못했습니다".into());
        };
        let log = Arc::new(Mutex::new(VecDeque::new()));
        let tail = log.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let mut tail = tail.lock().unwrap_or_else(PoisonError::into_inner);
                if tail.len() == LOG_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
        let (sender, events) = channel();
        let python = python.display().to_string();
        thread::spawn(move || {
            let mut error = None;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let event = match message["event"].as_str() {
                    Some("error") => {
                        error = Some(describe_error(&message, &python));
                        continue;
                    }
                    _ => event(&message),
                };
                let Some(event) = event else { continue };
                if sender.send(event).is_err() {
                    return;
                }
                wake();
            }
            // 표준 오류를 마저 읽을 틈을 준다(커널이 뜨다 죽은 까닭이 거기 있다).
            thread::sleep(Duration::from_millis(200));
            let log = log.lock().unwrap_or_else(PoisonError::into_inner);
            let tail: Vec<&str> = log.iter().map(String::as_str).collect();
            let mut reason = error.unwrap_or_else(|| "커널 사이드카가 끝났습니다".into());
            if !tail.is_empty() {
                reason.push('\n');
                reason.push_str(&tail.join("\n"));
            }
            let _ = sender.send(Event::Ended(reason));
            wake();
        });
        let mut started = Self {
            input: Some(stdin),
            child: Some(child),
            events,
            next_id: 0,
        };
        started.send(json!({ "method": "start", "params": { "kernel": kernel, "cwd": cwd } }));
        Ok(started)
    }

    /// 코드를 실행 줄에 세운다. 출력과 끝은 돌려준 id로 온다.
    pub fn execute(&mut self, code: &str) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "method": "execute", "params": { "id": id, "code": code } }));
        id
    }

    /// 실행 중인 코드를 멈춘다(커널에 KeyboardInterrupt). 줄 서 있던 실행은 `Aborted`로 끝난다.
    pub fn interrupt(&mut self) {
        self.send(json!({ "method": "interrupt" }));
    }

    /// 커널의 DataFrame 목록을 묻는다(번호·출력·기록을 남기지 않는 조용한 실행). 답은 `Frames`로 온다.
    /// 실행 중인 셀이 있으면 그 뒤에 답한다.
    pub fn frames(&mut self) -> u64 {
        self.request(json!({ "kind": "frames" }))
    }

    /// DataFrame `name`의 `rows` 행을 묻는다. `sort`: (열 자리, 오름차순)으로 정렬한 순서에서 자른다.
    /// 답은 `Page`로 온다.
    pub fn page(&mut self, name: &str, rows: Range<usize>, sort: Option<(usize, bool)>) -> u64 {
        self.request(json!({
            "kind": "page",
            "name": name,
            "start": rows.start,
            "stop": rows.end,
            "sort": sort.map(|(column, _)| column),
            "ascending": sort.is_none_or(|(_, ascending)| ascending),
        }))
    }

    fn request(&mut self, mut params: Value) -> u64 {
        self.next_id += 1;
        params["id"] = json!(self.next_id);
        self.send(json!({ "method": "data", "params": params }));
        self.next_id
    }

    /// 온 이벤트 하나(기다리지 않는다).
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    fn send(&mut self, request: Value) {
        if let Some(stdin) = self.input.as_mut() {
            // 쓰지 못하면 사이드카가 끝난 것이다(읽기 스레드가 `Ended`를 보낸다).
            let _ = writeln!(stdin, "{request}").and_then(|()| stdin.flush());
        }
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        // 입력을 닫으면 사이드카가 커널을 끄고(늦으면 jupyter_client가 끝낸다) 끝난다. 사이드카가 걸려
        // 있으면 끝낸다. 사이드카가 죽어도 커널은 부모가 사라진 것을 보고 스스로 끝난다.
        self.input.take();
        if let Some(mut child) = self.child.take() {
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(15);
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

fn event(message: &Value) -> Option<Event> {
    let id = message["id"].as_u64().unwrap_or_default();
    let field = |name: &str| message[name].as_str().unwrap_or_default().to_owned();
    Some(match message["event"].as_str()? {
        "started" => Event::Started {
            name: field("name"),
            display_name: field("display_name"),
            language: field("language"),
            missing: message["missing"].as_str().map(str::to_owned),
        },
        "output" => Event::Output {
            id,
            output: message["output"].clone(),
        },
        "clear" => Event::Clear { id },
        "done" => Event::Done {
            id,
            execution_count: message["execution_count"].as_u64(),
            status: match message["status"].as_str() {
                Some("ok") => Status::Ok,
                Some("aborted") => Status::Aborted,
                Some("dead") => Status::Dead,
                _ => Status::Error,
            },
        },
        "value" => {
            let value = match (message["value"].clone(), message["error"].as_str()) {
                (_, Some(error)) => Err(error.to_owned()),
                (value, None) => Ok(value),
            };
            match message["kind"].as_str()? {
                "frames" => Event::Frames {
                    id,
                    result: value.and_then(|v| frames(&v)),
                },
                "page" => Event::Page {
                    id,
                    result: value.and_then(|v| page(&v)),
                },
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn frames(value: &Value) -> Result<Vec<FrameInfo>, String> {
    let list = value.as_array().ok_or("DataFrame 목록이 아닙니다")?;
    Ok(list
        .iter()
        .filter_map(|frame| {
            Some(FrameInfo {
                name: frame["name"].as_str()?.to_owned(),
                rows: frame["rows"].as_u64()? as usize,
                columns: frame["columns"].as_u64()? as usize,
            })
        })
        .collect())
}

fn page(value: &Value) -> Result<Page, String> {
    let strings = |name: &str| -> Vec<String> {
        value[name]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let data = value["data"]
        .as_array()
        .ok_or("DataFrame 페이지가 아닙니다")?
        .iter()
        .map(|row| row.as_array().cloned().unwrap_or_default())
        .collect();
    Ok(Page {
        rows: value["rows"].as_u64().unwrap_or_default() as usize,
        columns: strings("columns"),
        dtypes: strings("dtypes"),
        index: strings("index"),
        data,
    })
}

/// 사이드카가 알린 오류를 사용자에게 보일 글로.
fn describe_error(message: &Value, python: &str) -> String {
    let text = message["message"].as_str().unwrap_or_default();
    let install = format!("\"{python}\" -m pip install ipykernel");
    match message["kind"].as_str().unwrap_or_default() {
        "missing" => format!("{python}에 jupyter_client가 없습니다. {install}로 설치하세요"),
        "LookupError" => {
            format!("Jupyter 커널을 찾지 못했습니다({text}). {install}로 설치하세요")
        }
        kind => format!("커널 오류: {kind}: {text}"),
    }
}

fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 사이드카_알림을_이벤트로_읽는다() {
        let done = json!({"event": "done", "id": 3, "execution_count": 7, "status": "aborted"});
        assert_eq!(
            event(&done),
            Some(Event::Done {
                id: 3,
                execution_count: Some(7),
                status: Status::Aborted
            })
        );
        let started = json!({"event": "started", "name": "python3", "display_name": "Python 3",
            "language": "python", "missing": "ir"});
        assert!(
            matches!(event(&started), Some(Event::Started { missing: Some(m), .. }) if m == "ir")
        );
        assert_eq!(event(&json!({"event": "모름"})), None);
        let frames = json!({"event": "value", "id": 4, "kind": "frames",
            "value": [{"name": "df", "rows": 3, "columns": 2}]});
        assert_eq!(
            event(&frames),
            Some(Event::Frames {
                id: 4,
                result: Ok(vec![FrameInfo {
                    name: "df".into(),
                    rows: 3,
                    columns: 2
                }])
            })
        );
        let failed = json!({"event": "value", "id": 5, "kind": "page", "error": "KeyError: 'x'"});
        assert_eq!(
            event(&failed),
            Some(Event::Page {
                id: 5,
                result: Err("KeyError: 'x'".into())
            })
        );
        let missing = json!({"event": "error", "kind": "missing", "message": "No module"});
        assert!(describe_error(&missing, "py").contains("\"py\" -m pip install ipykernel"));
    }

    #[test]
    fn 없는_python이면_곧바로_알린다() {
        let python = Path::new("ferrite-없는-python");
        assert!(Kernel::start(python, None, Path::new("."), || {}).is_err());
    }
}
