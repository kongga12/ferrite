//! 실제 GitHub에서 언어 서버를 받아 설치하는 확인(리디렉션, TLS, 체크섬, 실행).
//! 네트워크를 쓰고 16MB 남짓 받으므로 기본으로는 돌지 않는다.
//! `cargo test -p ferrite-lsp --test download -- --ignored --nocapture`

use std::fs;
use std::time::{Duration, Instant};

use ferrite_lsp::catalog;
use ferrite_lsp::client::{LanguageServer, ServerEvent};
use ferrite_lsp::install;

#[test]
#[ignore = "GitHub에서 lemminx(약 16MB)를 내려받는다"]
fn github에서_lemminx를_받아_설치하고_띄운다() {
    let server = catalog::for_language("xml").unwrap();
    if !install::installable(server) {
        eprintln!("이 OS·CPU용 lemminx 배포본이 없어 건너뜁니다");
        return;
    }
    let root = std::env::temp_dir().join(format!("ferrite-download-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);

    let started = Instant::now();
    let mut last = (0, 0);
    install::install(&root, server, &mut |done, total| last = (done, total))
        .unwrap_or_else(|e| panic!("설치 실패: {e}"));
    eprintln!(
        "{} {}: {:.1}MB, {:.1}초",
        server.name,
        server.version,
        last.0 as f64 / 1_000_000.0,
        started.elapsed().as_secs_f64()
    );
    assert!(
        last.0 > 0 && last.0 == last.1,
        "진행률이 끝까지 오지 않음: {last:?}"
    );

    let installed = install::installed(&root, server).expect("설치 기록이 없음");
    assert!(!installed.is_outdated(server));
    let command = install::command(&root, server).unwrap();
    assert_eq!(command.program, installed.bin);

    // 받은 실행 파일이 실제로 언어 서버로 뜨는지(initialize 응답까지) 본다.
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    let lsp = LanguageServer::start(&command, &project, || {}).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !lsp.is_ready() {
        for event in lsp.poll_events() {
            if let ServerEvent::Exited(reason) = event {
                panic!("서버가 끝남: {reason}");
            }
        }
        assert!(Instant::now() < deadline, "60초 안에 준비되지 않음");
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("준비됨: {}", installed.bin.display());

    drop(lsp);
    // Windows는 서버가 완전히 끝나야 실행 파일을 지울 수 있다.
    for _ in 0..50 {
        if fs::remove_dir_all(&root).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
