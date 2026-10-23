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
        PositionEncoding::Utf16,
        "다른 서버와 결과를 합치므로 그 서버들과 같은 UTF-16"
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

/// 요청의 응답이 올 때까지 기다린다(그사이 온 진단은 버린다).
fn response(lsp: &LanguageServer, id: i64) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for event in lsp.poll_events() {
            match event {
                ServerEvent::Response { id: got, result } if got == id => {
                    return result.unwrap_or_else(|e| panic!("오류 응답: {}", e.message));
                }
                ServerEvent::Exited(reason) => panic!("서버가 끝남: {reason}"),
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "응답이 오지 않음");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn 매퍼_xml과_java_사이를_정의로_이동하고_xml_문장이_없는_메서드를_경고한다() {
    use ferrite_lsp::features::{parse_locations, position_params};

    let root = project("nav");
    let java_dir = root.join("src/main/java/egov/sample");
    fs::create_dir_all(&java_dir).unwrap();
    let mappers = root.join("src/main/resources/mapper");
    let mapper_java = java_dir.join("SampleMapper.java");
    fs::write(
        &mapper_java,
        "package egov.sample;\n\npublic interface SampleMapper {\n    List<SampleVO> selectSampleList(SampleVO vo);\n    int deleteSample(String id);\n}\n",
    )
    .unwrap();
    let service = java_dir.join("SampleServiceImpl.java");
    let service_text = "package egov.sample;\n\npublic class SampleServiceImpl {\n    private SampleMapper sampleMapper;\n\n    public List<SampleVO> list(SampleVO vo) {\n        dao.list(\"Board.selectBoard\", vo);\n        return sampleMapper.selectSampleList(vo);\n    }\n}\n";
    fs::write(&service, service_text).unwrap();
    fs::write(
        mappers.join("Common_SQL.xml"),
        "<mapper namespace=\"common\">\n  <sql id=\"paging\">LIMIT 10</sql>\n</mapper>\n",
    )
    .unwrap();
    fs::write(
        mappers.join("Board_SQL.xml"),
        "<sqlMap namespace=\"Board\">\n  <select id=\"Board.selectBoard\">SELECT 1</select>\n</sqlMap>\n",
    )
    .unwrap();
    let sample_xml = mappers.join("Sample_SQL.xml");
    let sample_text = "<mapper namespace=\"egov.sample.SampleMapper\">\n  <select id=\"selectSampleList\">\n    SELECT * FROM sample\n    <include refid=\"common.paging\"/>\n  </select>\n</mapper>\n";
    fs::write(&sample_xml, sample_text).unwrap();

    let command = ServerCommand {
        program: env!("CARGO_BIN_EXE_ferrite-egov-ls").into(),
        ..Default::default()
    };
    let mut lsp = LanguageServer::start(&command, &root, || {}).unwrap();
    let encoding = lsp.encoding();
    let definition =
        |lsp: &mut LanguageServer, path: &std::path::Path, text: &str, needle: &str| {
            let rope = Rope::from_str(text);
            let char_idx = rope.byte_to_char(text.find(needle).unwrap() + 1);
            let params = position_params(path, &rope, char_idx, encoding).unwrap();
            let id = lsp.request("textDocument/definition", params);
            response(lsp, id)
        };
    let lines_of = |result: &serde_json::Value| -> Vec<(String, u32)> {
        parse_locations(result)
            .into_iter()
            .map(|t| {
                let name = t.path.file_name().unwrap().to_string_lossy().into_owned();
                (name, t.range.start.line)
            })
            .collect()
    };

    lsp.did_open(&sample_xml, "xml", 1, sample_text);
    lsp.did_open(&service, "java", 1, service_text);
    assert_eq!(
        lines_of(&definition(
            &mut lsp,
            &sample_xml,
            sample_text,
            "common.paging"
        )),
        [("Common_SQL.xml".to_owned(), 1)],
        "refid → 다른 파일의 <sql>"
    );
    assert_eq!(
        lines_of(&definition(
            &mut lsp,
            &sample_xml,
            sample_text,
            "selectSampleList"
        )),
        [("SampleMapper.java".to_owned(), 3)],
        "문장 id → Mapper 인터페이스 메서드"
    );
    assert_eq!(
        lines_of(&definition(
            &mut lsp,
            &service,
            service_text,
            "selectSampleList"
        )),
        [("Sample_SQL.xml".to_owned(), 1)],
        "Mapper 호출 → XML 문장(필드 타입으로 Mapper를 찾음)"
    );
    assert_eq!(
        lines_of(&definition(
            &mut lsp,
            &service,
            service_text,
            "Board.selectBoard"
        )),
        [("Board_SQL.xml".to_owned(), 1)],
        "DAO 문자열 id → iBatis 문장"
    );

    let mapper_text = fs::read_to_string(&mapper_java).unwrap();
    lsp.did_open(&mapper_java, "java", 1, &mapper_text);
    let warnings = diagnostics_for(&lsp, "SampleMapper.java");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("deleteSample"));
    assert_eq!(warnings[0].range.start.line, 4);

    drop(lsp);
    for _ in 0..50 {
        if fs::remove_dir_all(&root).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
