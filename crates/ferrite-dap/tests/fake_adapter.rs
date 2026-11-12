//! 각본대로 응답하는 가짜 어댑터(TCP)로 시작 순서·중단점·멈춤·변수·실행 재개·종료를 본다.
//! 실제 어댑터처럼 launch 응답을 configurationDone 뒤로 미루고, 거꾸로 오는 요청도 섞는다.

use std::collections::BTreeMap;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use ferrite_dap::{Adapter, Breakpoint, DebugEvent, DebugSession, Launch, State, types};
use ferrite_lsp::transport::{read_message, write_message};
use serde_json::{Value, json};

/// 받은 연결 하나로 읽고 쓴다(쓰기는 `reader.get_ref()`로). `try_clone`한 소켓은 Windows에서 다른 시험이
/// 띄운 자식 프로세스에 물려져, 가짜가 끝나도 그 자식이 끝날 때까지 연결이 닫히지 않는다.
struct Fake {
    reader: BufReader<TcpStream>,
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
        write_message(
            &mut self.reader.get_ref(),
            &serde_json::to_vec(&message).unwrap(),
        )
        .unwrap();
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
        script(&mut fake_on(stream));
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

/// 받은 연결로 가짜 어댑터를 만든다.
fn fake_on(stream: TcpStream) -> Fake {
    Fake {
        reader: BufReader::new(stream),
        seq: 0,
    }
}

/// initialize → launch → initialized → setBreakpoints → configurationDone → launch 응답. 처음 중단점 요청을
/// 돌려준다(응답은 부르는 쪽이 한다).
fn boot(f: &mut Fake, launch_check: impl Fn(&Value)) -> (Value, Value) {
    let initialize = f.expect("initialize");
    f.respond(
        &initialize,
        json!({ "supportsConfigurationDoneRequest": true, "supportsLoadedSourcesRequest": true }),
    );
    let launch = f.expect("launch");
    launch_check(&launch);
    f.event("initialized", json!({}));
    let set = f.expect("setBreakpoints");
    (launch, set)
}

#[test]
fn 거꾸로_요청한_자식_세션에서_멈추고_페이지_스크립트에_중단점을_건다() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let app_js = std::env::temp_dir()
        .join("ferrite-dap-child")
        .join("app.js");
    let local = std::env::temp_dir()
        .join("ferrite-dap-child")
        .join("list.do.html");
    let expected_js = app_js.clone();
    let root = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut f = fake_on(stream);
        let initialize = f.expect("initialize");
        assert_eq!(
            initialize["arguments"]["supportsStartDebuggingRequest"], true,
            "자식 세션을 받는다고 알린다"
        );
        f.respond(
            &initialize,
            json!({ "supportsConfigurationDoneRequest": true }),
        );
        let launch = f.expect("launch");
        assert_eq!(launch["arguments"]["type"], "pwa-chrome");
        f.event("initialized", json!({}));
        let set = f.expect("setBreakpoints");
        // 부모는 페이지가 없으므로 아직 걸지 못한다.
        f.respond(
            &set,
            json!({ "breakpoints": [{ "id": 1, "verified": false }] }),
        );
        let done = f.expect("configurationDone");
        f.respond(&done, json!({}));
        f.respond(&launch, json!({}));
        // 페이지가 생겼다: 자식 세션을 거꾸로 요청한다.
        f.send(json!({
            "type": "request", "command": "startDebugging",
            "arguments": { "request": "launch", "configuration": { "type": "pwa-chrome", "__pendingTargetId": "t1" } },
        }));
        let child = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut f = fake_on(stream);
            let (launch, set) = boot(&mut f, |launch| {
                assert_eq!(launch["arguments"]["__pendingTargetId"], "t1");
            });
            assert_eq!(set["arguments"]["source"]["path"], json!(expected_js));
            f.respond(
                &set,
                json!({ "breakpoints": [{ "id": 7, "verified": false }] }),
            );
            let done = f.expect("configurationDone");
            f.respond(&done, json!({}));
            f.respond(&launch, json!({}));
            // 스크립트를 불러오며 건 중단점을 알리고, 그 줄에서 멈춘다.
            f.event(
                "breakpoint",
                json!({ "reason": "changed", "breakpoint": { "id": 7, "verified": true, "line": 3 } }),
            );
            f.event("stopped", json!({ "reason": "breakpoint", "threadId": 1 }));
            let trace = f.expect("stackTrace");
            f.respond(
                &trace,
                json!({ "stackFrames": [{ "id": 10, "name": "tick", "line": 2, "column": 1,
                    "source": { "name": "list.do", "path": "localhost:8080/app/list.do", "sourceReference": 5 } }] }),
            );
            let source = f.expect("source");
            assert_eq!(source["arguments"]["sourceReference"], 5);
            f.respond(
                &source,
                json!({ "content": "<script>\ntick();\n</script>" }),
            );
            // 담아 둔 파일의 중단점은 원래 소스로 건다.
            let set = f.expect("setBreakpoints");
            assert_eq!(
                set["arguments"]["source"],
                json!({ "name": "list.do", "path": "localhost:8080/app/list.do", "sourceReference": 5 })
            );
            f.respond(
                &set,
                json!({ "breakpoints": [{ "id": 8, "verified": true, "line": 2 }] }),
            );
            // js-debug는 같은 중단점을 다시 받으면 바꿀 것이 없어 빈 목록으로 답한다(걸어 둔 것은 그대로다).
            let again = f.expect("setBreakpoints");
            assert_eq!(again["arguments"]["breakpoints"], json!([{ "line": 2 }]));
            f.respond(&again, json!({ "breakpoints": [] }));
            let resume = f.expect("continue");
            f.respond(&resume, json!({}));
            f.event("terminated", json!({}));
        });
        let reply = read_message(&mut f.reader).unwrap().unwrap();
        let reply: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(
            (&reply["command"], &reply["success"]),
            (&json!("startDebugging"), &json!(true))
        );
        let set = f.expect("setBreakpoints");
        assert_eq!(set["arguments"]["source"]["sourceReference"], 5);
        f.respond(
            &set,
            json!({ "breakpoints": [{ "id": 2, "verified": false }] }),
        );
        let again = f.expect("setBreakpoints");
        f.respond(
            &again,
            json!({ "breakpoints": [{ "id": 3, "verified": false }] }),
        );
        let disconnect = f.expect("disconnect");
        assert_eq!(disconnect["arguments"]["terminateDebuggee"], true);
        f.respond(&disconnect, json!({}));
        f.event("terminated", json!({}));
        child.join().expect("자식 세션 각본이 어긋남");
    });

    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({ "type": "pwa-chrome" }),
    };
    let breakpoints = BTreeMap::from([(
        app_js.clone(),
        vec![Breakpoint {
            line: 3,
            condition: None,
        }],
    )]);
    let mut session =
        DebugSession::start(&Adapter::Tcp(address), launch, breakpoints, || {}).unwrap();
    let mut log = Log::default();
    // 자식이 확인한 중단점이 부모의 "아직 못 걸음"을 대신한다.
    let event = wait_for(
        &mut session,
        &mut log,
        |e| matches!(e, DebugEvent::Breakpoints { states, .. } if states.first().is_some_and(|s| s.verified)),
    );
    assert!(
        matches!(&event, DebugEvent::Breakpoints { path, states } if *path == app_js && states[0].line == Some(3)),
        "{event:?}"
    );
    wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Stopped(_))
    });
    assert!(
        matches!(session.state(), State::Stopped(_)),
        "자식이 멈추면 세션도 멈춤"
    );

    let id = session.stack_trace(1);
    let body = response_body(
        wait_for(&mut session, &mut log, |e| {
            matches!(e, DebugEvent::Response { .. })
        }),
        id,
    );
    let frame = types::stack_frames(&body).remove(0);
    let source = frame.source.unwrap();
    assert_eq!(source.source_reference, 5);
    let id = session.source(&source, None).unwrap();
    let body = response_body(
        wait_for(&mut session, &mut log, |e| {
            matches!(e, DebugEvent::Response { .. })
        }),
        id,
    );
    assert_eq!(body["content"], "<script>\ntick();\n</script>");

    session.map_source(&local, &source);
    session.set_breakpoints(
        &local,
        vec![Breakpoint {
            line: 2,
            condition: None,
        }],
    );
    let event = wait_for(
        &mut session,
        &mut log,
        |e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    assert!(
        matches!(&event, DebugEvent::Breakpoints { states, .. } if states[0].verified),
        "부모의 미확인 응답은 버린다: {event:?}"
    );
    session.set_breakpoints(
        &local,
        vec![Breakpoint {
            line: 2,
            condition: None,
        }],
    );
    let event = wait_for(
        &mut session,
        &mut log,
        |e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    assert!(
        matches!(&event, DebugEvent::Breakpoints { states, .. }
            if states.len() == 1 && states[0].verified && states[0].id == Some(8)),
        "다시 보낸 중단점에 빈 목록이 오면 앞의 상태를 쓴다: {event:?}"
    );
    session.resume(1);
    // 자식(페이지)이 끝나도 세션은 이어진다. 부모가 끝나야 끝난다.
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(300) {
        log.seen.extend(session.poll());
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !log.seen.iter().any(|e| matches!(e, DebugEvent::Terminated)),
        "{:#?}",
        log.seen
    );
    session.stop();
    wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Terminated)
    });
    root.join().expect("부모 세션 각본이 어긋남");
}

/// 중단점 없이 시작 순서를 밟는다(launch 응답은 configurationDone 뒤에 준다).
fn boot_plain(f: &mut Fake) {
    let initialize = f.expect("initialize");
    f.respond(
        &initialize,
        json!({ "supportsConfigurationDoneRequest": true }),
    );
    let launch = f.expect("launch");
    f.event("initialized", json!({}));
    let done = f.expect("configurationDone");
    f.respond(&done, json!({}));
    f.respond(&launch, json!({}));
}

/// 거꾸로 자식 세션을 요청한다(답은 자식 연결을 받은 뒤 `child_started`로 읽는다).
fn ask_child(f: &mut Fake, target: &str) {
    f.send(json!({
        "type": "request", "command": "startDebugging",
        "arguments": { "request": "launch", "configuration": { "type": "pwa-chrome", "__pendingTargetId": target } },
    }));
}

fn child_started(f: &mut Fake) {
    let reply = read_message(&mut f.reader).unwrap().unwrap();
    let reply: Value = serde_json::from_slice(&reply).unwrap();
    assert_eq!(
        (&reply["command"], &reply["success"]),
        (&json!("startDebugging"), &json!(true))
    );
}

/// 끝내기를 받고 끝났다고 알린다. 품은 세션이 먼저 끝나면 클라이언트가 이 세션도 함께 버려 연결이 이미 닫혔을
/// 수 있으므로 쓰기 실패는 넘긴다.
fn finish(f: &mut Fake) {
    let disconnect = f.expect("disconnect");
    for message in [
        json!({ "type": "response", "request_seq": disconnect["seq"], "success": true,
            "command": "disconnect", "body": {} }),
        json!({ "type": "event", "event": "terminated", "body": {} }),
    ] {
        let _ = write_message(
            &mut f.reader.get_ref(),
            &serde_json::to_vec(&message).unwrap(),
        );
    }
}

#[test]
fn 다른_사이트_iframe_세션은_그_세션에_묻고_멈추지_않았으면_페이지_세션에_묻는다() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // 브라우저 세션 → 페이지 세션 → 그 페이지 안 다른 사이트 `<iframe>`의 세션(실제 js-debug의 꼴).
    let root = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut f = fake_on(stream);
        boot_plain(&mut f);
        ask_child(&mut f, "page");
        let page = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut f = fake_on(stream);
            boot_plain(&mut f);
            f.event(
                "loadedSource",
                json!({ "reason": "new", "source": { "name": "127.0.0.1\u{a789}8080/app/list.do\u{a789}3:9",
                    "path": "127.0.0.1\u{a789}8080/app/list.do\u{a789}3:9", "sourceReference": 1 } }),
            );
            ask_child(&mut f, "frame");
            let frame = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut f = fake_on(stream);
                boot_plain(&mut f);
                // 소스 번호는 세션마다 따로라 페이지의 것과 겹친다.
                f.event(
                    "loadedSource",
                    json!({ "reason": "new", "source": { "name": "localhost\u{a789}8080/app/frag.do\u{a789}4:9",
                        "path": "localhost\u{a789}8080/app/frag.do\u{a789}4:9", "sourceReference": 1 } }),
                );
                let proxy = f.expect("requestCDPProxy");
                f.respond(
                    &proxy,
                    json!({ "host": "127.0.0.1", "port": 1, "path": "/frame" }),
                );
                let source = f.expect("source");
                assert_eq!(source["arguments"]["sourceReference"], 1);
                f.respond(&source, json!({ "content": "frame" }));
                f.event("stopped", json!({ "reason": "breakpoint", "threadId": 1 }));
                let evaluate = f.expect("evaluate");
                f.respond(
                    &evaluate,
                    json!({ "result": "frame", "variablesReference": 0 }),
                );
                // 페이지의 스크립트를 모른다.
                let set = f.expect("setBreakpoints");
                f.respond(&set, json!({ "breakpoints": [{ "verified": false }] }));
                finish(&mut f);
            });
            child_started(&mut f);
            let evaluate = f.expect("evaluate");
            f.respond(
                &evaluate,
                json!({ "result": "page", "variablesReference": 0 }),
            );
            let set = f.expect("setBreakpoints");
            f.respond(
                &set,
                json!({ "breakpoints": [{ "id": 4, "verified": true, "line": 3 }] }),
            );
            finish(&mut f);
            frame.join().expect("iframe 세션 각본이 어긋남");
        });
        child_started(&mut f);
        // 브라우저 세션은 화면이 없다.
        let set = f.expect("setBreakpoints");
        f.respond(&set, json!({ "breakpoints": [{ "verified": false }] }));
        finish(&mut f);
        page.join().expect("페이지 세션 각본이 어긋남");
    });

    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({ "type": "pwa-chrome" }),
    };
    let mut session =
        DebugSession::start(&Adapter::Tcp(address), launch, BTreeMap::new(), || {}).unwrap();
    let mut log = Log::default();
    let mut loaded = Vec::new();
    while loaded.len() < 2 {
        if let DebugEvent::LoadedSource {
            source, session, ..
        } = wait_for(&mut session, &mut log, |e| {
            matches!(e, DebugEvent::LoadedSource { .. })
        }) {
            loaded.push((source, session));
        }
    }
    let of = |page: &str| {
        loaded
            .iter()
            .find(|(source, _)| source.name.as_deref().is_some_and(|n| n.contains(page)))
            .cloned()
            .unwrap()
    };
    let ((_, page_id), (frame_source, frame_id)) = (of("list.do"), of("frag.do"));
    assert_ne!(page_id, frame_id);
    assert_eq!(session.parents(frame_id), [page_id]);
    assert!(
        session.parents(page_id).is_empty(),
        "브라우저 세션은 화면이 없어 뺀다"
    );
    let answer = |session: &mut DebugSession, log: &mut Log, id: i64| {
        response_body(
            wait_for(session, log, |e| matches!(e, DebugEvent::Response { .. })),
            id,
        )
    };
    // 멈추지 않았으면 요청은 페이지 세션에 간다(iframe 세션이 나중에 열렸어도).
    assert_eq!(session.focus_id(), page_id);
    let id = session.request("evaluate", json!({ "expression": "location.href" }));
    assert_eq!(answer(&mut session, &mut log, id)["result"], "page");
    // iframe의 것은 iframe 세션에 묻는다.
    let id = session
        .request_in(frame_id, "requestCDPProxy", json!({}))
        .unwrap();
    assert_eq!(answer(&mut session, &mut log, id)["path"], "/frame");
    let id = session.source(&frame_source, Some(frame_id)).unwrap();
    assert_eq!(answer(&mut session, &mut log, id)["content"], "frame");
    // 끝난(없는) 세션에는 보내지 않는다.
    assert_eq!(session.request_in(u64::MAX, "source", json!({})), None);
    // iframe이 멈추면 요청은 그리로 간다.
    wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Stopped(_))
    });
    assert_eq!(session.focus_id(), frame_id);
    let id = session.request("evaluate", json!({ "expression": "location.href" }));
    assert_eq!(answer(&mut session, &mut log, id)["result"], "frame");
    // 중단점 확인은 세션들의 것을 합친다: 페이지 세션이 걸었으면 브라우저·iframe 세션의 "못 걸음"이 늦게 와도
    // 걸린 것이다.
    let page_js = std::env::temp_dir()
        .join("ferrite-dap-nested")
        .join("list.js");
    session.set_breakpoints(
        &page_js,
        vec![Breakpoint {
            line: 3,
            condition: None,
        }],
    );
    let (mut answers, mut last) = (0, Vec::new());
    while answers < 3 {
        if let DebugEvent::Breakpoints { states, .. } = wait_for(
            &mut session,
            &mut log,
            |e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == page_js),
        ) {
            answers += 1;
            last = states;
        }
    }
    assert!(last[0].verified && last[0].line == Some(3), "{last:?}");
    session.stop();
    wait_for(&mut session, &mut log, |e| {
        matches!(e, DebugEvent::Terminated)
    });
    root.join().expect("브라우저 세션 각본이 어긋남");
}

/// 잠깐 살아 있다 끝나는 프로그램(사용자 어댑터 대신 띄운다).
fn short_lived(seconds: u32) -> (PathBuf, Vec<String>) {
    if cfg!(windows) {
        (
            PathBuf::from("cmd"),
            vec![
                "/c".into(),
                format!("ping -n {} 127.0.0.1 > nul", seconds + 1),
            ],
        )
    } else {
        (
            PathBuf::from("sh"),
            vec!["-c".into(), format!("sleep {seconds}")],
        )
    }
}

#[test]
fn 띄운_어댑터가_정해_둔_주소를_늦게_열어도_붙고_먼저_끝나면_바로_실패한다() {
    // 빈 포트를 얻어 닫아 두고, 어댑터를 띄운 뒤에 그 포트를 연다(제조사 어댑터가 뜨는 동안).
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let late = thread::spawn(move || {
        thread::sleep(Duration::from_millis(700));
        let listener = TcpListener::bind(address).unwrap();
        let (stream, _) = listener.accept().unwrap();
        // 붙어서 첫 요청을 받으면 된다(나머지 시작 순서는 poll이 진행한다).
        fake_on(stream).expect("initialize");
    });
    let (program, args) = short_lived(5);
    let adapter = Adapter::Listening {
        program,
        args,
        cwd: None,
        env: Vec::new(),
        address: Some(address),
    };
    let launch = || Launch {
        adapter_id: "vendor".into(),
        attach: true,
        arguments: json!({}),
    };
    let session = DebugSession::start(&adapter, launch(), BTreeMap::new(), || {}).unwrap();
    late.join().expect("가짜 어댑터 각본이 어긋남");
    drop(session);

    // 어댑터가 주소를 열지 않고 끝나면 기다리지 않는다.
    let (program, args) = short_lived(0);
    let adapter = Adapter::Listening {
        program,
        args,
        cwd: None,
        env: Vec::new(),
        address: Some(address),
    };
    let started = Instant::now();
    let error = DebugSession::start(&adapter, launch(), BTreeMap::new(), || {})
        .err()
        .expect("실패해야 한다");
    assert!(error.contains(&address.to_string()), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn 세션을_버리면_어댑터가_곧바로_연결이_닫힌_것을_안다() {
    // 앱은 `terminated`를 받으면 연결이 열려 있어도 세션을 버린다. 붙은 어댑터는 계속 떠 있으므로
    // 연결이 닫혀야 세션이 끝난 것을 안다.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let session = DebugSession::start(
        &Adapter::Tcp(address),
        Launch {
            adapter_id: "fake".into(),
            attach: false,
            arguments: json!({}),
        },
        BTreeMap::new(),
        || {},
    )
    .unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let initialize: Value =
        serde_json::from_slice(&read_message(&mut reader).unwrap().unwrap()).unwrap();
    assert_eq!(initialize["command"], "initialize");
    drop(session);
    let closed = read_message(&mut reader);
    assert!(matches!(closed, Ok(None)), "{closed:?}");
}
