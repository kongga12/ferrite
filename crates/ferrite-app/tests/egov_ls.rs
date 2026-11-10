//! `ferrite --egov-ls`가 창을 띄우지 않고 egov 언어 서버로 동작하는지 실제 실행 파일로 확인한다.

use std::path::Path;
use std::time::{Duration, Instant};

use ferrite_lsp::catalog::EGOV_LS_FLAG;
use ferrite_lsp::client::{LanguageServer, ServerCommand, ServerEvent, parse_diagnostics};

#[test]
fn ferrite_실행_파일이_egov_언어_서버로_진단한다() {
    let root = std::env::temp_dir().join(format!("ferrite-egov-flag-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("Sample_SQL.xml");
    let text = "<mapper namespace=\"s\"><select id=\"a\">SELECT ${col} FROM t</select></mapper>";
    std::fs::write(&file, text).unwrap();

    let command = ServerCommand {
        program: env!("CARGO_BIN_EXE_ferrite").into(),
        args: vec![EGOV_LS_FLAG.to_owned()],
        ..Default::default()
    };
    let lsp = LanguageServer::start(&command, &root, || {}).unwrap();
    lsp.did_open(&file, "xml", 1, text);
    let deadline = Instant::now() + Duration::from_secs(20);
    let diagnostics = 'wait: loop {
        for event in lsp.poll_events() {
            match event {
                ServerEvent::Notification { method, params }
                    if method == "textDocument/publishDiagnostics" =>
                {
                    break 'wait parse_diagnostics(&params).unwrap().1;
                }
                ServerEvent::Exited(reason) => panic!("서버가 끝남: {reason}"),
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "진단이 오지 않음");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.contains("SQL Injection"));

    drop(lsp);
    remove(&root);
}

/// 시험 폴더를 지운다. 서버는 내린 뒤에도 잠시(정상 종료 유예 2초, 그 뒤 강제 종료) 살아 그 폴더를 작업
/// 폴더로 쥐고 있어 바로 지우면 Windows에서 실패해 폴더가 남는다. 다 지워질 때까지 다시
/// 시도한다(최대 15초).
fn remove(dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while dir.exists() && Instant::now() < deadline {
        let _ = std::fs::remove_dir_all(dir);
        if dir.exists() {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if dir.exists() {
        eprintln!("시험 폴더를 지우지 못했습니다: {}", dir.display());
    }
}
