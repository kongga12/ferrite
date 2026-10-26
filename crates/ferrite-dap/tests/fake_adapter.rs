//! 각본대로 응답하는 가짜 어댑터(TCP)로 시작 순서·중단점·멈춤·변수·실행 재개·종료를 본다.
//! 실제 어댑터처럼 launch 응답을 configurationDone 뒤로 미루고, 거꾸로 오는 요청도 섞는다.

use std::collections::BTreeMap;
use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use ferrite_dap::{Adapter, Breakpoint, DebugEvent, DebugSession, Launch, State, types};
use ferrite_lsp::transport::{read_message, write_message};
use serde_json::{Value, json};

struct Fake {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    seq: i64,
}

impl Fake {
    /// 다음 요청을 읽고 명령 이름을 확인한다.
    fn expect(&mut self, command: &str) -> Value {
        let body = read_message(&mut self.reader)
            .unwrap()
            .expect("연결이 끊김");
        let message: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(message["command"], command, "받은 메시지: {message}");
        message
    }

    fn send(&mut self, mut message: Value) {
        self.seq += 1;
        message["seq"] = json!(self.seq);
        write_message(&mut self.writer, &serde_json::to_vec(&message).unwrap()).unwrap();
    }

    fn respond(&mut self, request: &Value, body: Value) {
        self.send(json!({
            "type": "response", "request_seq": request["seq"], "success": true,
            "command": request["command"], "body": body,
        }));
    }

    fn event(&mut self, event: &str, body: Value) {
        self.send(json!({ "type": "event", "event": event, "body": body }));
    }
}

/// 가짜 어댑터를 띄우고 주소를 돌려준다. 각본이 어긋나면 그 스레드가 패닉하고 연결이 끊긴다.
fn spawn_fake(
    script: impl FnOnce(&mut Fake) + Send + 'static,
) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut fake = Fake {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
            seq: 0,
        };
        script(&mut fake);
        fake.writer.flush().unwrap();
    });
    (address, handle)
}

/// 받은 이벤트 전부와, 다음에 볼 자리.
#[derive(Default)]
struct Log {
    seen: Vec<DebugEvent>,
    next: usize,
}

/// 아직 보지 않은 이벤트 중 `done`이 참인 첫 이벤트를 기다린다(최대 10초). 한 번에 여러 이벤트가
/// 오므로 받은 것은 모두 쌓아 두고, 찾은 이벤트 다음부터 다시 본다.
fn wait_for(
    session: &mut DebugSession,
    log: &mut Log,
    done: impl Fn(&DebugEvent) -> bool,
) -> DebugEvent {
    let started = Instant::now();
    loop {
        if let Some(offset) = log.seen[log.next..].iter().position(&done) {
            let index = log.next + offset;
            log.next = index + 1;
            return log.seen[index].clone();
        }
        if started.elapsed() > Duration::from_secs(10) {
            panic!("시간 초과. 받은 이벤트: {:#?}", log.seen);
        }
        log.seen.extend(session.poll());
        thread::sleep(Duration::from_millis(10));
    }
}

fn response_body(event: DebugEvent, id: i64) -> Value {
    match event {
        DebugEvent::Response {
            id: got, result, ..
        } if got == id => result.unwrap(),
        other => panic!("응답이 아님: {other:?}"),
    }
}

#[test]
fn 중단점에서_멈추고_변수를_읽고_이어_가서_끝난다() {
    let source = std::env::temp_dir()
        .join("ferrite-dap-fake")
        .join("main.py");
    let expected_path = source.clone();
    let (address, fake) = spawn_fake(move |f| {
        let initialize = f.expect("initialize");
        assert_eq!(initialize["arguments"]["adapterID"], "fake");
        assert_eq!(initialize["arguments"]["linesStartAt1"], true);
        f.respond(
            &initialize,
            json!({ "supportsConfigurationDoneRequest": true, "supportsConditionalBreakpoints": true }),
        );
        // 실제 어댑터(debugpy 등)처럼 launch 응답은 configurationDone 뒤에 준다.
        let launch = f.expect("launch");
        assert_eq!(launch["arguments"]["program"], "main.py");
        f.event("initialized", json!({}));
        let set = f.expect("setBreakpoints");
        assert_eq!(set["arguments"]["source"]["path"], json!(expected_path));
        assert_eq!(
            set["arguments"]["breakpoints"],
            json!([{ "line": 3 }, { "line": 5, "condition": "x > 40" }])
        );
        f.respond(
            &set,
            json!({ "breakpoints": [{ "verified": true, "line": 3 }, { "verified": false, "message": "줄 없음" }] }),
        );
        let done = f.expect("configurationDone");
        f.respond(&done, json!({}));
        f.respond(&launch, json!({}));
        // 어댑터가 거꾸로 묻는 요청은 거절 응답을 받아야 한다.
        f.send(json!({ "type": "request", "command": "runInTerminal", "arguments": {} }));
        let reply = read_message(&mut f.reader).unwrap().unwrap();
        let reply: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(
            (reply["type"].as_str(), reply["success"].as_bool()),
            (Some("response"), Some(false))
        );
        assert_eq!(reply["request_seq"], json!(f.seq));

        f.event(
            "stopped",
            json!({ "reason": "breakpoint", "threadId": 1, "allThreadsStopped": true }),
        );
        let trace = f.expect("stackTrace");
        assert_eq!(trace["arguments"]["threadId"], 1);
        f.respond(&trace, json!({ "stackFrames": [
            { "id": 7, "name": "main", "line": 3, "column": 1, "source": { "path": "/p/main.py" } }
        ]}));
        let scopes = f.expect("scopes");
        assert_eq!(scopes["arguments"]["frameId"], 7);
        f.respond(
            &scopes,
            json!({ "scopes": [{ "name": "Locals", "variablesReference": 11 }] }),
        );
        let variables = f.expect("variables");
        assert_eq!(variables["arguments"]["variablesReference"], 11);
        f.respond(
            &variables,
            json!({ "variables": [
                { "name": "x", "value": "42", "type": "int", "variablesReference": 0 }
            ]}),
        );

        let resume = f.expect("continue");
        f.respond(&resume, json!({ "allThreadsContinued": true }));
        f.event(
            "output",
            json!({ "category": "stdout", "output": "한글 출력\n" }),
        );
        f.event("exited", json!({ "exitCode": 3 }));
        f.event("terminated", json!({}));
        let disconnect = f.expect("disconnect");
        assert_eq!(disconnect["arguments"]["terminateDebuggee"], true);
        f.respond(&disconnect, json!({}));
    });

    let breakpoints = BTreeMap::from([(
        source.clone(),
        vec![
            Breakpoint {
                line: 3,
                condition: None,
            },
            Breakpoint {
                line: 5,
                condition: Some("x > 40".into()),
            },
        ],
    )]);
    let launch = Launch {
        adapter_id: "fake".into(),
        attach: false,
        arguments: json!({ "program": "main.py" }),
    };
    let mut session =
        DebugSession::start(&Adapter::Tcp(address), launch, breakpoints, || {}).unwrap();
    assert_eq!(*session.state(), State::Starting);
    let mut log = Log::default();

    let event = wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Breakpoints { .. })
    });
    let DebugEvent::Breakpoints { path, states } = event else {
        unreachable!()
    };
    assert_eq!(path, source);
    assert!(states[0].verified && !states[1].verified);
    assert_eq!(states[1].message.as_deref(), Some("줄 없음"));
    assert!(session.capabilities().supports_conditional_breakpoints);
    wait_for(&mut session, &mut log, |e| matches!(e, DebugEvent::Started));

    let event = wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Stopped(_))
    });
    let DebugEvent::Stopped(stopped) = event else {
        unreachable!()
    };
    assert_eq!(
        (stopped.reason.as_str(), stopped.thread_id),
        ("breakpoint", Some(1))
    );
    assert!(matches!(session.state(), State::Stopped(_)));

    let id = session.stack_trace(1);
    let frames = types::stack_frames(&response_body(
        wait_for(&mut session, &mut log, |e| {
            matches!(e, DebugEvent::Response { .. })
        }),
        id,
    ));
    assert_eq!((frames[0].id, frames[0].line), (7, 3));
    let id = session.scopes(frames[0].id);
    let scopes = types::scopes(&response_body(
        wait_for(&mut session, &mut log, |e| {
            matches!(e, DebugEvent::Response { .. })
        }),
        id,
    ));
    let id = session.variables(scopes[0].variables_reference);
    let variables = types::variables(&response_body(
        wait_for(&mut session, &mut log, |e| {
            matches!(e, DebugEvent::Response { .. })
        }),
        id,
    ));
    assert_eq!(
        (variables[0].name.as_str(), variables[0].value.as_str()),
        ("x", "42")
    );

    session.resume(1);
    assert_eq!(*session.state(), State::Running);
    let event = wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Output { .. })
    });
    assert!(
        matches!(event, DebugEvent::Output { category, text } if category == "stdout" && text == "한글 출력\n")
    );
    let event = wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Exited { .. })
    });
    assert!(matches!(event, DebugEvent::Exited { code: 3 }));
    wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Terminated)
    });
    assert_eq!(*session.state(), State::Terminated);

    session.stop();
    wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Closed(_))
    });
    assert!(
        !log.seen.iter().any(|e| matches!(e, DebugEvent::Error(_))),
        "오류가 없어야 한다: {:#?}",
        log.seen
    );
    fake.join().expect("가짜 어댑터 각본이 어긋남");
}

#[test]
fn 시작_실패와_중단점_변경을_알린다() {
    let (address, fake) = spawn_fake(|f| {
        let initialize = f.expect("initialize");
        f.respond(&initialize, json!({}));
        let launch = f.expect("attach");
        f.event("initialized", json!({}));
        // configurationDone을 지원하지 않는 어댑터에는 보내지 않는다. 중단점 파일이 없으면 setBreakpoints도 없다.
        f.send(json!({
            "type": "response", "request_seq": launch["seq"], "success": false, "command": "attach",
            "message": "launch failed", "body": { "error": { "id": 1, "format": "프로그램이 없습니다: main.py" } },
        }));
        // 시작 뒤에 바꾼 중단점은 바로 보낸다(빈 목록이면 모두 지움).
        let set = f.expect("setBreakpoints");
        assert_eq!(set["arguments"]["breakpoints"], json!([]));
        f.respond(&set, json!({ "breakpoints": [] }));
    });
    let launch = Launch {
        adapter_id: "fake".into(),
        attach: true,
        arguments: json!({}),
    };
    let mut session =
        DebugSession::start(&Adapter::Tcp(address), launch, BTreeMap::new(), || {}).unwrap();
    let mut log = Log::default();
    let event = wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Error(_))
    });
    assert!(
        matches!(&event, DebugEvent::Error(m) if m.contains("프로그램이 없습니다: main.py")),
        "{event:?}"
    );
    session.set_breakpoints(&PathBuf::from("a.py"), Vec::new());
    let event = wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Breakpoints { .. })
    });
    assert!(matches!(event, DebugEvent::Breakpoints { states, .. } if states.is_empty()));
    wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Closed(_))
    });
    fake.join().expect("가짜 어댑터 각본이 어긋남");
}

#[test]
fn 어댑터를_띄우지_못하면_오류() {
    let adapter = Adapter::Stdio {
        program: PathBuf::from("ferrite-없는-어댑터"),
        args: Vec::new(),
        cwd: None,
        env: Vec::new(),
    };
    let launch = Launch {
        adapter_id: "none".into(),
        attach: false,
        arguments: json!({}),
    };
    let error = DebugSession::start(&adapter, launch, BTreeMap::new(), || {})
        .err()
        .expect("실패해야 한다");
    assert!(error.contains("ferrite-없는-어댑터"), "{error}");
}
