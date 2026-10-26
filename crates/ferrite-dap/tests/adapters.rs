//! 디버그 어댑터를 실제 배포처에서 받아 설치하고(`ferrite_lsp::install`), 언어마다 같은 예제
//! (`add(41, 1)`)의 `return` 줄에서 멈춰 변수 a=41·b=1·total=42를 읽는다. 내려받기가 커서 기본은
//! 건너뛰고, `.github/workflows/adapters.yml`이 3개 OS에서 `--ignored`로 돌린다. 필요한 도구(Go,
//! PHP+Xdebug, node, rustc)가 없으면 로컬에서는 건너뛰고 CI에서는 실패한다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ferrite_dap::{Adapter, Breakpoint, DebugEvent, DebugSession, Launch, adapters, types};
use ferrite_lsp::client::find_program;
use ferrite_lsp::install;
use serde_json::{Value, json};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrite-adapter-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 도구가 있는지. 없으면 로컬은 건너뛰고(false) CI는 실패한다.
fn require(what: &str, found: bool) -> bool {
    if !found {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 {what}이(가) 있어야 합니다"
        );
        eprintln!("{what}이(가) 없어 건너뜁니다");
    }
    found
}

/// 어댑터를 받아 설치하고 띄울 방법을 돌려준다.
fn install_adapter(servers: &Path, name: &str, project: &Path) -> Adapter {
    let server = adapters::find(name).unwrap();
    install::install(servers, server, &mut |_, _| {})
        .unwrap_or_else(|e| panic!("{name} 설치 실패: {e}"));
    assert!(install::installed(servers, server).is_some());
    adapters::resolve(name, servers, project).unwrap()
}

/// 아직 보지 않은 이벤트 중 `done`이 참인 첫 이벤트(최대 120초, 첫 빌드·첫 실행이 느리다).
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
            started.elapsed() < Duration::from_secs(120),
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

/// `source`의 `line`(1부터)에 중단점을 걸고 띄워, 멈추면 맨 위 프레임의 지역 변수를 읽은 뒤 이어 가서
/// 끝낸다. (변수 이름에서 `$`를 뺀 것, 값)을 돌려준다.
fn locals_at_breakpoint(
    adapter: &Adapter,
    adapter_id: &str,
    mut arguments: Value,
    source: &Path,
    line: usize,
) -> Vec<(String, String)> {
    adapters::complete_arguments(adapter_id, &mut arguments);
    let launch = Launch {
        adapter_id: adapter_id.into(),
        attach: false,
        arguments,
    };
    let breakpoints = BTreeMap::from([(
        source.to_path_buf(),
        vec![Breakpoint {
            line,
            condition: None,
        }],
    )]);
    let mut session = DebugSession::start(adapter, launch, breakpoints, || {}).unwrap();
    let (mut seen, mut next) = (Vec::new(), 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    let event = wait(&mut session, &|e| matches!(e, DebugEvent::Stopped(_)));
    let DebugEvent::Stopped(stopped) = event else {
        unreachable!()
    };
    assert_eq!(stopped.reason, "breakpoint", "{stopped:?}");
    let thread = match stopped.thread_id {
        Some(thread) => thread,
        None => {
            session.threads();
            let threads = types::threads(&response(wait(
                &mut session,
                &|e| matches!(e, DebugEvent::Response { command, .. } if command == "threads"),
            )));
            threads[0].id
        }
    };
    session.stack_trace(thread);
    let frames = types::stack_frames(&response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "stackTrace"),
    )));
    assert_eq!(frames[0].line as usize, line, "멈춘 줄: {frames:?}");
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
    // 끝날 때까지 이어 간다. 한 줄에 걸 자리가 여럿이면(Rust의 println! 펼침) 다시 멈출 수 있다.
    session.resume(thread);
    for _ in 0..10 {
        let event = wait(&mut session, &|e| {
            matches!(
                e,
                DebugEvent::Stopped(_)
                    | DebugEvent::Terminated
                    | DebugEvent::Exited { .. }
                    | DebugEvent::Closed(_)
            )
        });
        match event {
            DebugEvent::Stopped(again) => session.resume(again.thread_id.unwrap_or(thread)),
            _ => break,
        }
    }
    variables
        .into_iter()
        .map(|v| (v.name.trim_start_matches('$').to_owned(), v.value))
        .collect()
}

fn assert_locals(locals: &[(String, String)]) {
    let value = |name: &str| {
        locals
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("{name} 없음: {locals:?}"))
    };
    assert_eq!(
        (value("a"), value("b"), value("total")),
        ("41", "1", "42"),
        "{locals:?}"
    );
}

#[test]
#[ignore = "PyPI에서 debugpy(약 5MB)를 받아 가상환경에 설치한다. Python이 필요하다"]
fn debugpy를_설치해_python을_디버그한다() {
    let python = if cfg!(windows) { "python" } else { "python3" };
    if !require("Python", find_program(python).is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("debugpy-servers"), temp_dir("debugpy"));
    let source = project.join("main.py");
    std::fs::write(
        &source,
        "def add(a, b):\n    total = a + b\n    return total\n\n\ny = add(41, 1)\nprint(\"결과\", y)\n",
    )
    .unwrap();
    let adapter = install_adapter(&servers, "debugpy", &project);
    let arguments = json!({ "program": source, "cwd": project, "console": "internalConsole" });
    assert_locals(&locals_at_breakpoint(
        &adapter, "debugpy", arguments, &source, 3,
    ));
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
#[ignore = "GitHub에서 codelldb(약 45~56MB)를 받는다. rustc가 필요하다"]
fn codelldb를_설치해_rust를_디버그한다() {
    if !require("rustc", find_program("rustc").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("codelldb-servers"), temp_dir("codelldb"));
    let source = project.join("main.rs");
    // 마지막 식(`total`)에는 코드가 없을 수 있어(Windows PDB에서는 다음 줄로 옮겨 걸림) 확실히 코드가 있는
    // println! 줄(3번)에 멈춘다.
    std::fs::write(
        &source,
        "fn add(a: i32, b: i32) -> i32 {\n    let total = a + b;\n    println!(\"합 {total}\");\n    total\n}\n\nfn main() {\n    let y = add(41, 1);\n    println!(\"결과 {y}\");\n}\n",
    )
    .unwrap();
    let program = project.join(format!("main{}", std::env::consts::EXE_SUFFIX));
    let built = Command::new("rustc")
        .args(["-g", "-C", "opt-level=0", "-o"])
        .arg(&program)
        .arg(&source)
        .status()
        .unwrap();
    assert!(built.success(), "rustc 실패");
    let adapter = install_adapter(&servers, "codelldb", &project);
    let arguments = json!({
        "program": program,
        "cwd": project,
        "args": [],
        "sourceLanguages": ["rust"],
    });
    assert_locals(&locals_at_breakpoint(
        &adapter, "codelldb", arguments, &source, 3,
    ));
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
#[ignore = "go install로 delve를 받아 빌드한다. Go가 필요하다"]
fn delve를_설치해_go를_디버그한다() {
    if !require("Go", find_program("go").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("delve-servers"), temp_dir("delve"));
    std::fs::write(
        project.join("go.mod"),
        "module example.com/demo\n\ngo 1.21\n",
    )
    .unwrap();
    let source = project.join("main.go");
    std::fs::write(
        &source,
        "package main\n\nimport \"fmt\"\n\nfunc add(a, b int) int {\n\ttotal := a + b\n\treturn total\n}\n\nfunc main() {\n\ty := add(41, 1)\n\tfmt.Println(\"결과\", y)\n}\n",
    )
    .unwrap();
    let adapter = install_adapter(&servers, "delve", &project);
    let arguments = json!({ "mode": "debug", "program": project });
    assert_locals(&locals_at_breakpoint(
        &adapter, "delve", arguments, &source, 7,
    ));
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}

/// PHP에 Xdebug 확장이 켜져 있는지.
fn php_with_xdebug() -> bool {
    Command::new("php")
        .arg("-m")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Xdebug"))
}

#[test]
#[ignore = "GitHub에서 vscode-php-debug(약 2MB)를 받는다. node와 Xdebug가 켜진 PHP가 필요하다"]
fn php_debug를_설치해_php를_디버그한다() {
    if !require("node", find_program("node").is_some())
        || !require("Xdebug가 켜진 PHP", php_with_xdebug())
    {
        return;
    }
    let (servers, project) = (temp_dir("php-servers"), temp_dir("php"));
    let source = project.join("index.php");
    std::fs::write(
        &source,
        "<?php\nfunction add($a, $b) {\n    $total = $a + $b;\n    return $total;\n}\n\n$y = add(41, 1);\necho \"결과 $y\\n\";\n",
    )
    .unwrap();
    let adapter = install_adapter(&servers, "php-debug", &project);
    let arguments = json!({
        "program": source,
        "cwd": project,
        "port": 0,
        "runtimeArgs": ["-dxdebug.start_with_request=yes"],
        "env": { "XDEBUG_MODE": "debug,develop", "XDEBUG_CONFIG": "client_port=${port}" },
    });
    assert_locals(&locals_at_breakpoint(
        &adapter,
        "php-debug",
        arguments,
        &source,
        4,
    ));
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}
