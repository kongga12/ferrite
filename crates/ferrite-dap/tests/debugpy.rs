//! 실제 디버그 어댑터(debugpy)로 조건부 중단점에서 멈추고 변수를 읽는다. CI는 debugpy를 설치해 3개 OS에서
//! 돌린다. 로컬에 없으면 건너뛴다(CI에서는 실패). `pip install --target`으로 따로 둔 debugpy는
//! `FERRITE_DEBUGPY_PATH`로 알려 준다.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use ferrite_dap::{Adapter, Breakpoint, DebugEvent, DebugSession, Launch, types};
use serde_json::{Value, json};

const PROGRAM: &str = "\
def add(a, b):
    total = a + b
    return total


for x in (1, 41):
    y = add(x, 1)
print(\"결과\", y)
";

fn python() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

/// debugpy를 쓸 수 있는 환경 변수(PYTHONPATH). 없으면 None, CI에서는 실패.
fn debugpy_env() -> Option<Vec<(String, String)>> {
    let env: Vec<(String, String)> = std::env::var("FERRITE_DEBUGPY_PATH")
        .map(|path| vec![("PYTHONPATH".to_owned(), path)])
        .unwrap_or_default();
    let found = Command::new(python())
        .args(["-c", "import debugpy"])
        .envs(env.iter().map(|(k, v)| (k, v)))
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 debugpy가 설치되어 있어야 합니다"
        );
        eprintln!("debugpy가 없어 건너뜁니다");
        return None;
    }
    Some(env)
}

/// 아직 보지 않은 이벤트 중 `done`이 참인 첫 이벤트를 기다린다(최대 60초, 처음 띄울 때 느리다).
fn wait_for(
    session: &mut DebugSession,
    seen: &mut Vec<DebugEvent>,
    next: &mut usize,
    done: impl Fn(&DebugEvent) -> bool,
) -> DebugEvent {
    let started = Instant::now();
    loop {
        if let Some(offset) = seen[*next..].iter().position(&done) {
            *next += offset + 1;
            return seen[*next - 1].clone();
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "시간 초과. 받은 이벤트: {seen:#?}"
        );
        seen.extend(session.poll());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn response(event: DebugEvent) -> Value {
    match event {
        DebugEvent::Response { result, .. } => result.expect("요청 실패"),
        other => panic!("응답이 아님: {other:?}"),
    }
}

#[test]
fn debugpy로_조건부_중단점에서_멈추고_변수를_읽는다() {
    let Some(env) = debugpy_env() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("ferrite-debugpy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("main.py");
    std::fs::write(&program, PROGRAM).unwrap();

    let adapter = Adapter::Stdio {
        program: PathBuf::from(python()),
        args: vec!["-m".into(), "debugpy.adapter".into()],
        cwd: Some(dir.clone()),
        env,
    };
    let launch = Launch {
        adapter_id: "debugpy".into(),
        attach: false,
        arguments: json!({
            "program": program,
            "cwd": dir,
            "console": "internalConsole",
            "justMyCode": true,
        }),
    };
    // `return total` 줄에서, 두 번째 호출(a == 41)에서만 멈춘다.
    let breakpoints = BTreeMap::from([(
        program.clone(),
        vec![Breakpoint {
            line: 3,
            condition: Some("a == 41".into()),
        }],
    )]);
    let mut session = DebugSession::start(&adapter, launch, breakpoints, || {}).unwrap();
    let (mut seen, mut next) = (Vec::new(), 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };

    let event = wait(&mut session, &|e| {
        matches!(e, DebugEvent::Breakpoints { .. })
    });
    assert!(
        matches!(&event, DebugEvent::Breakpoints { states, .. } if states.len() == 1 && states[0].verified),
        "{event:?}"
    );
    let event = wait(&mut session, &|e| matches!(e, DebugEvent::Stopped(_)));
    let DebugEvent::Stopped(stopped) = event else {
        unreachable!()
    };
    assert_eq!(stopped.reason, "breakpoint");
    let thread = stopped.thread_id.expect("멈춘 스레드");

    session.stack_trace(thread);
    let frames = types::stack_frames(&response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "stackTrace"),
    )));
    assert_eq!((frames[0].name.as_str(), frames[0].line), ("add", 3));
    session.scopes(frames[0].id);
    let scopes = types::scopes(&response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "scopes"),
    )));
    session.variables(scopes[0].variables_reference);
    let variables = types::variables(&response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "variables"),
    )));
    let value = |name: &str| {
        variables
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.value.as_str())
            .unwrap_or_else(|| panic!("{name} 없음: {variables:?}"))
    };
    assert_eq!((value("a"), value("b"), value("total")), ("41", "1", "42"));

    session.resume(thread);
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Exited { code: 0 })
    });
    wait(&mut session, &|e| matches!(e, DebugEvent::Terminated));
    // print은 출력을 여러 조각("결과", " 42")으로 보내므로 이어 붙여 본다.
    let stdout: String = seen
        .iter()
        .filter_map(|e| match e {
            DebugEvent::Output { category, text } if category == "stdout" => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(stdout.contains("결과 42"), "{stdout:?}");
    drop(session);
    let _ = std::fs::remove_dir_all(&dir);
}
