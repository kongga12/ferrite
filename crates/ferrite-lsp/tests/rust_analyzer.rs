//! 실제 rust-analyzer와 주고받는 통합 테스트. CI는 rust-analyzer 컴포넌트를 설치해 3개 OS에서 돌린다.
//! 로컬에 없으면(`rustup component add rust-analyzer`) 건너뛴다. CI에서는 없으면 실패한다.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ferrite_lsp::client::{
    LanguageServer, ServerCommand, ServerEvent, find_program, parse_diagnostics,
};

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

fn temp_project() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrite-ra-test-{}", std::process::id()));
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
    let Some(command) = rust_analyzer() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 rust-analyzer가 설치되어 있어야 합니다"
        );
        eprintln!("rust-analyzer가 없어 건너뜁니다");
        return;
    };
    let root = temp_project();
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
