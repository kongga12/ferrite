//! 실제 rust-analyzer와 주고받는 통합 테스트. CI는 rust-analyzer 컴포넌트를 설치해 3개 OS에서 돌린다.
//! 로컬에 없으면(`rustup component add rust-analyzer`) 건너뛴다. CI에서는 없으면 실패한다.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ferrite_lsp::client::{
    LanguageServer, ServerCommand, ServerEvent, find_program, parse_diagnostics,
};
use ferrite_lsp::features::{
    completion_params, parse_completion, parse_hover, parse_locations, position_params,
};
use ropey::Rope;

const TIMEOUT: Duration = Duration::from_secs(120);

fn rust_analyzer() -> Option<ServerCommand> {
    // rustup 프록시는 컴포넌트가 없어도 실행 파일이 있으므로, 실제로 실행되는지까지 확인한다.
    let program = find_program("rust-analyzer")?;
    let ok = std::process::Command::new(&program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    ok.then(|| ServerCommand {
        program,
        args: Vec::new(),
    })
}

/// rust-analyzer가 없으면 None. CI에서는 반드시 있어야 하므로 실패시킨다.
fn require_rust_analyzer() -> Option<ServerCommand> {
    let command = rust_analyzer();
    if command.is_none() {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 rust-analyzer가 설치되어 있어야 합니다"
        );
        eprintln!("rust-analyzer가 없어 건너뜁니다");
    }
    command
}

/// 같은 테스트 바이너리의 테스트는 병렬로 돌므로 이름으로 폴더를 나눈다.
fn temp_project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrite-ra-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"ra_test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // 표준 경로로 맞춰야 서버가 돌려준 경로와 비교할 수 있다(macOS /var → /private/var 등).
    fs::canonicalize(&dir).map(dunce_like).unwrap()
}

/// Windows canonicalize의 `\\?\` 접두사를 뗀다(앱의 real_path와 같은 규칙).
fn dunce_like(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) if !plain.starts_with(r"UNC\") => PathBuf::from(plain),
        _ => path,
    }
}

#[test]
fn rust_analyzer가_문법_오류를_진단으로_보낸다() {
    let Some(command) = require_rust_analyzer() else {
        return;
    };
    let root = temp_project("diagnostics");
    let main = root.join("src").join("main.rs");
    let text = "fn main() {\n    let 이름 = ;\n}\n";
    fs::write(&main, text).unwrap();

    let server = LanguageServer::start(&command, &root, || {}).expect("서버 시작");
    // 초기화 응답 전에 보내도 큐에 있다가 순서대로 전달되어야 한다
    server.did_open(&main, "rust", 1, text);

    let started = Instant::now();
    let deadline = started + TIMEOUT;
    let mut ready = false;
    let diagnostics = loop {
        assert!(
            Instant::now() < deadline,
            "시간 안에 진단이 오지 않음 (ready={ready})"
        );
        let mut found = None;
        for event in server.poll_events() {
            match event {
                ServerEvent::Ready => {
                    ready = true;
                    eprintln!("초기화 완료: {:?}", started.elapsed());
                }
                ServerEvent::Exited(reason) => panic!("서버 종료: {reason}"),
                ServerEvent::Notification { method, params }
                    if method == "textDocument/publishDiagnostics" =>
                {
                    if let Some((path, diags)) = parse_diagnostics(&params)
                        && path == main
                        && !diags.is_empty()
                    {
                        found = Some(diags);
                    }
                }
                _ => {}
            }
        }
        if let Some(diags) = found {
            eprintln!("진단 수신: {:?}", started.elapsed());
            break diags;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(ready);
    assert!(
        diagnostics.iter().any(|d| d.range.start.line == 1),
        "둘째 줄의 문법 오류가 있어야 한다: {diagnostics:#?}"
    );
    drop(server);
    let _ = fs::remove_dir_all(&root);
}

/// 요청 하나의 응답을 기다린다(다른 이벤트는 버린다).
fn wait_response(server: &LanguageServer, id: i64, deadline: Instant) -> serde_json::Value {
    loop {
        assert!(Instant::now() < deadline, "응답 시간 초과(요청 {id})");
        for event in server.poll_events() {
            match event {
                ServerEvent::Response { id: got, result } if got == id => {
                    return result.expect("서버 오류 응답");
                }
                ServerEvent::Exited(reason) => panic!("서버 종료: {reason}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn rust_analyzer로_자동완성_호버_정의를_받는다() {
    let Some(command) = require_rust_analyzer() else {
        return;
    };
    let root = temp_project("features");
    let main = root.join("src").join("main.rs");
    let text = "struct Point {\n    x: i32,\n}\n\nimpl Point {\n    fn length(&self) -> i32 {\n        self.x\n    }\n}\n\nfn main() {\n    let p = Point { x: 1 };\n    p.length();\n    p.le\n}\n";
    fs::write(&main, text).unwrap();
    let rope = Rope::from_str(text);
    let at = |needle: &str, offset: usize| rope.byte_to_char(text.find(needle).unwrap() + offset);

    let mut server = LanguageServer::start(&command, &root, || {}).expect("서버 시작");
    server.did_open(&main, "rust", 1, text);
    let deadline = Instant::now() + TIMEOUT;
    while !server.is_ready() {
        assert!(Instant::now() < deadline, "초기화 시간 초과");
        std::thread::sleep(Duration::from_millis(50));
    }
    let encoding = server.encoding();

    // 자동완성: 색인이 끝나기 전에는 빈 목록이 올 수 있어 다시 요청한다
    let after_dot = at("p.le\n", 4);
    let completion = loop {
        let params = completion_params(&main, &rope, after_dot, encoding, None).unwrap();
        let id = server.request("textDocument/completion", params);
        let list = parse_completion(&wait_response(&server, id, deadline), &rope, encoding);
        if let Some(item) = list
            .items
            .into_iter()
            .find(|c| c.label.starts_with("length"))
        {
            break item;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    assert_eq!(completion.kind, "함수");
    assert!(completion.text.starts_with("length"), "{completion:?}");

    // 호버: 타입 이름 위
    let on_type = at("= Point {", 2);
    let id = server.request(
        "textDocument/hover",
        position_params(&main, &rope, on_type, encoding).unwrap(),
    );
    let hover = parse_hover(&wait_response(&server, id, deadline)).expect("호버 내용");
    assert!(hover.contains("struct Point"), "{hover}");

    // 정의로 이동: 메서드 호출 → impl 안의 fn length(5번째 줄, 0부터)
    let on_call = at("p.length()", 3);
    let id = server.request(
        "textDocument/definition",
        position_params(&main, &rope, on_call, encoding).unwrap(),
    );
    let targets = parse_locations(&wait_response(&server, id, deadline));
    assert_eq!(targets.len(), 1, "{targets:?}");
    assert_eq!(targets[0].path, main);
    assert_eq!(targets[0].range.start.line, 5);

    drop(server);
    let _ = fs::remove_dir_all(&root);
}
