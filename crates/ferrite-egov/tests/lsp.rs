//! `ferrite-egov-ls`를 Ferrite의 LSP 클라이언트로 띄워 실제 통신으로 진단을 확인한다.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ferrite_lsp::client::{LanguageServer, ServerCommand, ServerEvent, parse_diagnostics};
use ferrite_lsp::position::{PositionEncoding, from_lsp};
use lsp_types::Diagnostic;
use ropey::Rope;

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrite-egov-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src/main/resources/mapper")).unwrap();
    fs::canonicalize(&dir)
        .map(|p| match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
            Some(plain) => PathBuf::from(plain),
            None => p,
        })
        .unwrap()
}

/// 이 파일의 진단이 올 때까지 기다린다.
fn diagnostics_for(lsp: &LanguageServer, file: &str) -> Vec<Diagnostic> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for event in lsp.poll_events() {
            match event {
                ServerEvent::Notification { method, params }
                    if method == "textDocument/publishDiagnostics" =>
                {
                    let (path, diagnostics) = parse_diagnostics(&params).unwrap();
                    if path.file_name().is_some_and(|n| n == file) {
                        return diagnostics;
                    }
                }
                ServerEvent::Exited(reason) => panic!("서버가 끝남: {reason}"),
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "진단이 오지 않음");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn 매퍼를_열면_sql_injection과_참조_진단을_보내고_고치면_지운다() {
    let root = project("lsp");
    let mappers = root.join("src/main/resources/mapper");
    // 디스크에만 있는 다른 매퍼: 색인으로 찾아야 하는 공통 조각.
    fs::write(
        mappers.join("Common_SQL.xml"),
        r#"<mapper namespace="common"><sql id="paging">LIMIT 10</sql></mapper>"#,
    )
    .unwrap();
    // 빌드 결과물 안의 사본은 색인하지 않는다.
    fs::create_dir_all(root.join("target/classes")).unwrap();
    fs::write(
        root.join("target/classes/Only_SQL.xml"),
        r#"<mapper namespace="onlyInTarget"><sql id="x">1</sql></mapper>"#,
    )
    .unwrap();

    let file = mappers.join("Board_SQL.xml");
    let text = "<mapper namespace=\"board\">\n  <!-- 게시판 -->\n  <select id=\"list\">\n    SELECT * FROM 게시판 WHERE 제목 LIKE '%${keyword}%'\n    <include refid=\"common.paging\"/>\n    <include refid=\"onlyInTarget.x\"/>\n  </select>\n</mapper>\n";
    fs::write(&file, text).unwrap();

    let command = ServerCommand {
        program: env!("CARGO_BIN_EXE_ferrite-egov-ls").into(),
        ..Default::default()
    };
    let lsp = LanguageServer::start(&command, &root, || {}).unwrap();
    lsp.did_open(&file, "xml", 1, text);
    let diagnostics = diagnostics_for(&lsp, "Board_SQL.xml");
    assert_eq!(
        lsp.encoding(),
        PositionEncoding::Utf8,
        "클라이언트가 먼저 제안한 UTF-8"
    );

    let rope = Rope::from_str(text);
    let spans: Vec<(String, String)> = diagnostics
        .iter()
        .map(|d| {
            let start = from_lsp(&rope, d.range.start, lsp.encoding());
            let end = from_lsp(&rope, d.range.end, lsp.encoding());
            let code = match &d.code {
                Some(lsp_types::NumberOrString::String(code)) => code.clone(),
                other => format!("{other:?}"),
            };
            (code, rope.slice(start..end).to_string())
        })
        .collect();
    assert_eq!(
        spans,
        [
            ("sql-injection".to_owned(), "${keyword}".to_owned()),
            ("unresolved-ref".to_owned(), "onlyInTarget.x".to_owned()),
        ],
        "한글 앞뒤 위치도 정확해야 하고, common.paging은 디스크 색인으로 찾는다"
    );

    let fixed = text
        .replace("'%${keyword}%'", "'%' || #{keyword} || '%'")
        .replace("    <include refid=\"onlyInTarget.x\"/>\n", "");
    lsp.did_change_full(&file, 2, &fixed);
    assert!(
        diagnostics_for(&lsp, "Board_SQL.xml").is_empty(),
        "고치면 지운다"
    );

    drop(lsp);
    for _ in 0..50 {
        if fs::remove_dir_all(&root).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
