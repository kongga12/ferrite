//! 각본대로 답하는 가짜 에이전트(같은 프로세스의 파이프)와 시작 순서·대화·요청 답하기를 본다.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, PipeReader, PipeWriter, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use ferrite_acp::{
    Agent, ConfigOption, ConfigValue, Diff, Event, MessageKind, PermissionOption, PlanEntry,
};
use serde_json::{Value, json};

struct Fake {
    lines: std::io::Lines<BufReader<PipeReader>>,
    out: PipeWriter,
}

impl Fake {
    fn next(&mut self) -> Value {
        serde_json::from_str(&self.lines.next().expect("끊김").unwrap()).unwrap()
    }

    fn expect(&mut self, method: &str) -> Value {
        let message = self.next();
        assert_eq!(message["method"], method, "{message}");
        message
    }

    fn send(&mut self, message: Value) {
        writeln!(self.out, "{message}").unwrap();
    }

    fn respond(&mut self, request: &Value, result: Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }));
    }

    fn update(&mut self, update: Value) {
        self.send(json!({
            "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s1", "update": update },
        }));
    }

    fn ask(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let answer = self.next();
        assert_eq!(answer["id"], id, "{answer}");
        answer
    }
}

/// 가짜 에이전트를 띄우고 그것과 말하는 클라이언트를 돌려준다.
fn connect(
    mcp: Vec<Value>,
    script: impl FnOnce(&mut Fake) + Send + 'static,
) -> (Agent, thread::JoinHandle<()>) {
    let (client_in, agent_out) = std::io::pipe().unwrap();
    let (agent_in, client_out) = std::io::pipe().unwrap();
    let handle = thread::spawn(move || {
        let mut fake = Fake {
            lines: BufReader::new(agent_in).lines(),
            out: agent_out,
        };
        script(&mut fake);
    });
    (
        Agent::with_streams(client_in, client_out, Path::new("/p"), mcp),
        handle,
    )
}

/// `done`이 참인 이벤트까지 모은다(요청에는 `answer`가 답한다). 한 번에 받은 것 중 남은 것은
/// `backlog`에 두어 다음에 본다.
fn run_until(
    agent: &mut Agent,
    backlog: &mut VecDeque<Event>,
    mut answer: impl FnMut(&mut Agent, &Event),
    done: impl Fn(&Event) -> bool,
) -> Vec<Event> {
    let started = Instant::now();
    let mut seen = Vec::new();
    loop {
        while let Some(event) = backlog.pop_front() {
            answer(agent, &event);
            let finished = done(&event);
            seen.push(event);
            if finished {
                return seen;
            }
        }
        backlog.extend(agent.poll());
        if backlog.is_empty() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "시간 초과: {seen:#?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// 고르기 설정이 `done`을 맞출 때까지 받는다(그동안의 이벤트는 버린다).
fn wait_config(agent: &mut Agent, done: impl Fn(&[ConfigOption]) -> bool) {
    let started = Instant::now();
    while !done(agent.config()) {
        agent.poll();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "시간 초과: {:#?}",
            agent.config()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// 세션을 열 때까지 각본을 돌린다. `session`은 `session/new`의 응답.
fn open(f: &mut Fake, session: Value) {
    let init = f.expect("initialize");
    f.respond(
        &init,
        json!({ "protocolVersion": 1, "agentCapabilities": {}, "authMethods": [] }),
    );
    let new = f.expect("session/new");
    f.respond(&new, session);
}

#[test]
fn 고르기_설정을_보이고_바꾸며_에이전트가_바꾼_것도_따른다() {
    let options = |mode: &str, model: &str| {
        json!([
            { "id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": mode,
              "options": [{ "value": "default", "name": "Default", "description": "묻는다" }, { "value": "auto", "name": "Auto" }] },
            { "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": model,
              "options": [{ "group": "g", "name": "묶음", "options": [{ "value": "m1", "name": "M1" }, { "value": "m2", "name": "M2" }] }] },
            { "id": "fast", "name": "Fast", "type": "boolean", "currentValue": false },
        ])
    };
    let (mut agent, fake) = connect(Vec::new(), move |f| {
        // 둘 다 주면 새 판(configOptions)을 쓴다.
        open(
            f,
            json!({ "sessionId": "s1", "configOptions": options("auto", "m1"),
                    "modes": { "currentModeId": "auto", "availableModes": [{ "id": "auto", "name": "Auto" }] } }),
        );
        let set = f.expect("session/set_config_option");
        assert_eq!(
            set["params"],
            json!({ "sessionId": "s1", "configId": "mode", "value": "default" })
        );
        f.respond(&set, json!({ "configOptions": options("default", "m1") }));
        f.update(json!({ "sessionUpdate": "config_option_update", "configOptions": options("default", "m2") }));
        let bad = f.expect("session/set_config_option");
        f.send(json!({ "jsonrpc": "2.0", "id": bad["id"], "error": { "code": -32602, "message": "unknown value" } }));
    });
    let mut backlog = VecDeque::new();
    run_until(&mut agent, &mut backlog, |_, _| {}, |e| *e == Event::Ready);
    let config = agent.config();
    assert_eq!(config.len(), 2, "불린은 받는다고 알리지 않아 뺀다");
    assert_eq!(
        (
            config[0].category.as_deref(),
            config[0].current.as_str(),
            &config[0].values[0]
        ),
        (
            Some("mode"),
            "auto",
            &ConfigValue {
                value: "default".into(),
                name: "Default".into(),
                description: Some("묻는다".into())
            }
        )
    );
    let models: Vec<&str> = config[1].values.iter().map(|v| v.value.as_str()).collect();
    assert_eq!(models, ["m1", "m2"], "묶음은 펼친다");

    agent.set_config("mode", "default").unwrap();
    wait_config(&mut agent, |c| c[0].current == "default");
    wait_config(&mut agent, |c| c[1].current == "m2");
    agent.set_config("model", "nope").unwrap();
    let events = run_until(
        &mut agent,
        &mut backlog,
        |_, _| {},
        |e| matches!(e, Event::Error(_)),
    );
    assert_eq!(
        events.last(),
        Some(&Event::Error(
            "설정을 바꾸지 못했습니다: unknown value".into()
        ))
    );
    assert_eq!(agent.config()[1].current, "m2", "거절되면 그대로");
    fake.join().expect("가짜 에이전트 각본이 어긋남");
}

#[test]
fn 예전_판의_세션_모드만_주면_모드_설정_하나로_보이고_set_mode로_바꾼다() {
    let (mut agent, fake) = connect(Vec::new(), |f| {
        open(
            f,
            json!({ "sessionId": "s1", "modes": { "currentModeId": "auto", "availableModes": [
                { "id": "default", "name": "Default", "description": "묻는다" }, { "id": "auto", "name": "Auto" },
            ] } }),
        );
        let set = f.expect("session/set_mode");
        assert_eq!(
            set["params"],
            json!({ "sessionId": "s1", "modeId": "default" })
        );
        f.respond(&set, json!({}));
        // 에이전트가 스스로 바꾼 것(계획 모드를 마침 등)도 따른다.
        let prompt = f.expect("session/prompt");
        f.update(json!({ "sessionUpdate": "current_mode_update", "currentModeId": "auto" }));
        f.respond(&prompt, json!({ "stopReason": "end_turn" }));
    });
    assert!(
        agent.set_config("mode", "default").is_err(),
        "세션이 열리기 전"
    );
    let mut backlog = VecDeque::new();
    run_until(&mut agent, &mut backlog, |_, _| {}, |e| *e == Event::Ready);
    assert_eq!(
        agent.config(),
        [ConfigOption {
            id: "mode".into(),
            name: "모드".into(),
            description: None,
            category: Some("mode".into()),
            current: "auto".into(),
            values: vec![
                ConfigValue {
                    value: "default".into(),
                    name: "Default".into(),
                    description: Some("묻는다".into())
                },
                ConfigValue {
                    value: "auto".into(),
                    name: "Auto".into(),
                    description: None
                },
            ],
        }]
    );
    agent.set_config("mode", "default").unwrap();
    wait_config(&mut agent, |c| c[0].current == "default");
    agent.prompt("계획대로 해").unwrap();
    run_until(
        &mut agent,
        &mut backlog,
        |_, _| {},
        |e| matches!(e, Event::TurnEnded(_)),
    );
    assert_eq!(agent.config()[0].current, "auto");
    fake.join().expect("가짜 에이전트 각본이 어긋남");
}

#[test]
fn 세션을_열고_대화하며_읽기_권한_쓰기_요청에_답한다() {
    let mcp = vec![
        json!({ "type": "http", "name": "ferrite", "url": "http://127.0.0.1:47711/mcp", "headers": [] }),
        json!({ "name": "tool", "command": "/bin/tool", "args": [], "env": [] }),
    ];
    let (mut agent, fake) = connect(mcp, |f| {
        let init = f.expect("initialize");
        assert_eq!(init["params"]["protocolVersion"], 1);
        assert_eq!(
            init["params"]["clientCapabilities"]["fs"]["writeTextFile"],
            true
        );
        assert_eq!(init["params"]["clientCapabilities"]["terminal"], false);
        f.respond(&init, json!({ "protocolVersion": 1, "agentCapabilities": { "mcpCapabilities": { "http": false } }, "authMethods": [] }));
        let new = f.expect("session/new");
        assert_eq!(
            new["params"]["cwd"],
            Path::new("/p").to_string_lossy().as_ref()
        );
        // HTTP MCP를 못 받는 에이전트에는 표준 입출력 서버만 넘긴다.
        assert_eq!(new["params"]["mcpServers"].as_array().unwrap().len(), 1);
        assert_eq!(new["params"]["mcpServers"][0]["name"], "tool");
        f.respond(&new, json!({ "sessionId": "s1" }));

        let prompt = f.expect("session/prompt");
        assert_eq!(prompt["params"]["sessionId"], "s1");
        assert_eq!(prompt["params"]["prompt"][0]["text"], "안녕");
        f.update(json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "생각 중" } }));
        f.update(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "반가" } }));
        f.update(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "워요" } }));
        f.update(json!({
            "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Board.java 고치기", "kind": "edit", "status": "pending",
            "content": [
                { "type": "diff", "path": "/p/Board.java", "oldText": "a", "newText": "b" },
                { "type": "content", "content": { "type": "text", "text": "설명" } },
            ],
        }));
        let read = f.ask(
            100,
            "fs/read_text_file",
            json!({ "sessionId": "s1", "path": "/p/Board.java", "line": 2, "limit": 1 }),
        );
        assert_eq!(read["result"]["content"], "둘째 줄");
        let permission = f.ask(
            101,
            "session/request_permission",
            json!({
                "sessionId": "s1", "toolCall": { "toolCallId": "t1", "title": "쓰기" },
                "options": [
                    { "optionId": "ok", "name": "허락", "kind": "allow_once" },
                    { "optionId": "no", "name": "거절", "kind": "reject_once" },
                ],
            }),
        );
        assert_eq!(
            permission["result"]["outcome"],
            json!({ "outcome": "selected", "optionId": "ok" })
        );
        let write = f.ask(
            102,
            "fs/write_text_file",
            json!({ "sessionId": "s1", "path": "/p/Board.java", "content": "새 내용" }),
        );
        assert_eq!(write["result"], json!({}));
        let rejected = f.ask(
            103,
            "fs/write_text_file",
            json!({ "sessionId": "s1", "path": "/p/A.java", "content": "x" }),
        );
        assert_eq!(rejected["error"]["message"], "사용자가 버렸습니다");
        // 주지 않은 터미널 기능은 모르는 메서드다.
        let terminal = f.ask(
            104,
            "terminal/create",
            json!({ "sessionId": "s1", "command": "ls" }),
        );
        assert_eq!(terminal["error"]["code"], -32601);
        f.update(json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed" }));
        f.update(json!({ "sessionUpdate": "plan", "entries": [
            { "content": "읽기", "priority": "high", "status": "completed" },
            { "content": "고치기", "priority": "high", "status": "in_progress" },
        ] }));
        f.respond(&prompt, json!({ "stopReason": "end_turn" }));

        // 멈춤은 알림으로 보낸다.
        let prompt = f.expect("session/prompt");
        assert_eq!(f.expect("session/cancel")["params"]["sessionId"], "s1");
        f.respond(&prompt, json!({ "stopReason": "cancelled" }));
    });

    let mut backlog = VecDeque::new();
    let ready = run_until(
        &mut agent,
        &mut backlog,
        |_, _| {},
        |e| matches!(e, Event::Ready | Event::Error(_)),
    );
    assert_eq!(ready.last(), Some(&Event::Ready));
    assert!(agent.is_ready());
    agent.prompt("안녕").unwrap();
    let mut writes = 0;
    let events = run_until(
        &mut agent,
        &mut backlog,
        |agent, event| match event {
            Event::ReadFile {
                id, line, limit, ..
            } => {
                assert_eq!((*line, *limit), (Some(2), Some(1)));
                agent.answer_read(id, Ok("둘째 줄".into()));
            }
            Event::Permission { id, options, .. } => {
                agent.answer_permission(id, Some(&options[0].id))
            }
            Event::WriteFile { id, .. } => {
                writes += 1;
                agent.answer_write(
                    id,
                    if writes == 1 {
                        Ok(())
                    } else {
                        Err("사용자가 버렸습니다".into())
                    },
                );
            }
            _ => {}
        },
        |e| matches!(e, Event::TurnEnded(_)),
    );
    assert!(events.contains(&Event::Message(MessageKind::Thought, "생각 중".into())));
    let said: String = events
        .iter()
        .filter_map(|e| match e {
            Event::Message(MessageKind::Agent, text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(said, "반가워요");
    let Some(Event::ToolCall(call)) = events.iter().find(|e| matches!(e, Event::ToolCall(_)))
    else {
        panic!("도구 호출 없음");
    };
    assert_eq!(
        (
            call.title.as_deref(),
            call.kind.as_deref(),
            call.text.as_deref()
        ),
        (Some("Board.java 고치기"), Some("edit"), Some("설명"))
    );
    assert_eq!(
        call.diffs,
        [Diff {
            path: PathBuf::from("/p/Board.java"),
            old_text: Some("a".into()),
            new_text: "b".into()
        }]
    );
    let Some(Event::Permission { options, tool, .. }) = events
        .iter()
        .find(|e| matches!(e, Event::Permission { .. }))
    else {
        panic!("권한 요청 없음");
    };
    assert_eq!(tool.title.as_deref(), Some("쓰기"));
    assert_eq!(
        options[1],
        PermissionOption {
            id: "no".into(),
            name: "거절".into(),
            kind: "reject_once".into()
        }
    );
    let mut merged = call.clone();
    for e in &events {
        if let Event::ToolCallUpdate(update) = e {
            merged.merge(update.clone());
        }
    }
    assert_eq!(
        (merged.status.as_deref(), merged.title.as_deref()),
        (Some("completed"), Some("Board.java 고치기"))
    );
    assert!(events.contains(&Event::Plan(vec![
        PlanEntry {
            content: "읽기".into(),
            status: "completed".into()
        },
        PlanEntry {
            content: "고치기".into(),
            status: "in_progress".into()
        },
    ])));
    assert_eq!(events.last(), Some(&Event::TurnEnded("end_turn".into())));

    agent.prompt("긴 일").unwrap();
    agent.cancel();
    let events = run_until(
        &mut agent,
        &mut backlog,
        |_, _| {},
        |e| matches!(e, Event::TurnEnded(_)),
    );
    assert_eq!(events.last(), Some(&Event::TurnEnded("cancelled".into())));
    fake.join().expect("가짜 에이전트 각본이 어긋남");
    // 에이전트가 끝나면 알린다.
    let events = run_until(
        &mut agent,
        &mut backlog,
        |_, _| {},
        |e| matches!(e, Event::Exited(_)),
    );
    assert!(matches!(events.last(), Some(Event::Exited(_))));
    assert!(!agent.is_ready());
}

#[test]
fn 로그인이_필요하면_알리고_없는_프로그램은_오류() {
    let (mut agent, fake) = connect(Vec::new(), |f| {
        let init = f.expect("initialize");
        f.respond(&init, json!({ "protocolVersion": 1, "agentCapabilities": {}, "authMethods": [{ "id": "oauth", "name": "Google" }] }));
        let new = f.expect("session/new");
        f.send(json!({ "jsonrpc": "2.0", "id": new["id"], "error": { "code": -32000, "message": "Authentication required" } }));
    });
    let mut backlog = VecDeque::new();
    let events = run_until(
        &mut agent,
        &mut backlog,
        |_, _| {},
        |e| matches!(e, Event::Error(_)),
    );
    let Some(Event::Error(message)) = events.last() else {
        unreachable!()
    };
    assert!(
        message.contains("로그인") && message.contains("Authentication required"),
        "{message}"
    );
    assert!(agent.prompt("x").is_err(), "세션이 없으면 보낼 수 없다");
    fake.join().unwrap();

    let error = Agent::spawn(
        "ferrite-없는-에이전트",
        &[],
        &[],
        Path::new("."),
        Vec::new(),
    )
    .err()
    .unwrap();
    assert!(error.contains("ferrite-없는-에이전트"), "{error}");
}
