use super::*;

/// SpotBugs 4.10.5(spotbugs-maven-plugin 4.10.5.0, `-Dspotbugs.sarifOutput=true`)가 실제로 낸 것에서 규칙
/// 설명과 위치 이름만 줄였다. 파일은 원본 폴더 기준 상대 경로이고 기준 주소가 없다.
const SPOTBUGS: &str = r#"{
  "version": "2.1.0",
  "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
  "runs": [{
    "tool": {"driver": {"name": "SpotBugs", "version": "4.10.5", "language": "ko", "rules": [
      {"id": "ES_COMPARING_PARAMETER_STRING_WITH_EQ", "shortDescription": {"text": "Comparison of String parameter using == or !=."}, "messageStrings": {"default": {"text": "Comparison of String parameter using == or != in {0}."}}, "helpUri": "https://spotbugs.readthedocs.io/en/latest/bugDescriptions.html#ES_COMPARING_PARAMETER_STRING_WITH_EQ"},
      {"id": "NP_ALWAYS_NULL", "shortDescription": {"text": "Null pointer dereference."}, "messageStrings": {"default": {"text": "Null pointer dereference of {0} in {1}."}}, "helpUri": "https://spotbugs.readthedocs.io/en/latest/bugDescriptions.html#NP_ALWAYS_NULL"},
      {"id": "NP_LOAD_OF_KNOWN_NULL_VALUE", "shortDescription": {"text": "Load of known null value."}, "messageStrings": {"default": {"text": "Load of known null value in {0}."}}}
    ]}},
    "invocations": [{"exitCode": 1, "exitCodeDescription": "BUGS FOUND", "executionSuccessful": true}],
    "results": [
      {"ruleId": "ES_COMPARING_PARAMETER_STRING_WITH_EQ", "ruleIndex": 0, "message": {"id": "default", "text": "Comparison of String parameter using == or !=", "arguments": ["com.example.board.Board.same(String)"]}, "level": "warning", "locations": [{"physicalLocation": {"artifactLocation": {"uri": "com/example/board/Board.java"}, "region": {"startLine": 7}}}]},
      {"ruleId": "NP_ALWAYS_NULL", "ruleIndex": 1, "message": {"id": "default", "text": "Null pointer dereference", "arguments": ["s", "com.example.board.Board.length(String)"]}, "level": "error", "locations": [{"physicalLocation": {"artifactLocation": {"uri": "com/example/board/Board.java"}, "region": {"startLine": 12}}}]},
      {"ruleId": "NP_LOAD_OF_KNOWN_NULL_VALUE", "ruleIndex": 2, "message": {"id": "default", "text": "Load of known null value", "arguments": ["com.example.board.Board.length(String)"]}, "level": "note", "locations": [{"physicalLocation": {"artifactLocation": {"uri": "com/example/board/Board.java"}, "region": {"startLine": 12}}}]}
    ]
  }]
}"#;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ferrite-sarif-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn spotbugs_결과를_읽고_원본_폴더_기준_경로로_파일을_찾는다() {
    let root = temp_root("spotbugs");
    let java = root.join("src/main/java/com/example/board");
    std::fs::create_dir_all(&java).unwrap();
    std::fs::write(java.join("Board.java"), "class Board {}").unwrap();
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let findings = parse(SPOTBUGS, &root, &target).unwrap();
    assert_eq!(findings.len(), 3);
    let np = &findings[1];
    assert_eq!(
        (np.tool.as_str(), np.rule.as_str()),
        ("SpotBugs", "NP_ALWAYS_NULL")
    );
    assert_eq!(np.level, Level::Error);
    assert_eq!(
        np.message, "Null pointer dereference of s in com.example.board.Board.length(String).",
        "규칙의 틀에 인자를 채운다"
    );
    assert_eq!((np.line, np.column, np.end_line), (12, 0, 12));
    assert_eq!(np.path.as_deref(), Some(java.join("Board.java").as_path()));
    assert!(np.help.as_deref().unwrap().ends_with("#NP_ALWAYS_NULL"));
    assert_eq!(findings[2].level, Level::Note);
    assert_eq!(findings[0].level, Level::Warning);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn 기준_주소와_file_주소와_규칙_기본_수준을_따른다() {
    let root = temp_root("bases");
    std::fs::create_dir_all(root.join("src/my app")).unwrap();
    std::fs::write(root.join("src/my app/main.py"), "").unwrap();
    std::fs::write(root.join("util.c"), "").unwrap();
    let base = format!(
        "file:///{}/",
        root.display()
            .to_string()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    let text = serde_json::json!({
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {"name": "Semgrep", "rules": [
                {"id": "python.lang.eval", "defaultConfiguration": {"level": "error"}, "shortDescription": {"text": "eval 사용"}}
            ]}},
            "originalUriBaseIds": {"SRCROOT": {"uri": base}},
            "results": [
                {"ruleId": "python.lang.eval", "message": {"text": "eval은 위험합니다"},
                 "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/my%20app/main.py", "uriBaseId": "SRCROOT"},
                                                     "region": {"startLine": 3, "startColumn": 5, "endLine": 4, "endColumn": 9}}}]},
                {"ruleId": "python.lang.eval", "kind": "pass", "message": {"text": "통과"},
                 "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/my%20app/main.py", "uriBaseId": "SRCROOT"}}}]},
                {"ruleId": "c.leak", "level": "warning", "message": {"text": "새는 {0}{{x}}", "arguments": ["메모리"]},
                 "locations": [{"physicalLocation": {"artifactLocation": {"uri": format!("{base}util.c")}, "region": {"startLine": 1}}}]},
                {"ruleId": "x", "message": {"text": "파일 없음"},
                 "locations": [{"physicalLocation": {"artifactLocation": {"uri": "nowhere/x.c"}}}]}
            ]
        }]
    })
    .to_string();
    let findings = parse(&text, &root, &root.join("reports")).unwrap();
    assert_eq!(findings.len(), 3, "통과(kind=pass)는 뺀다");
    let eval = &findings[0];
    assert_eq!(eval.level, Level::Error, "결과에 없으면 규칙의 기본 수준");
    assert_eq!(eval.message, "eval은 위험합니다");
    assert_eq!(
        (eval.line, eval.column, eval.end_line, eval.end_column),
        (3, 5, 4, 9)
    );
    assert!(
        eval.path
            .as_ref()
            .is_some_and(|p| p.is_file() && p.ends_with("my app/main.py"))
    );
    assert_eq!(findings[1].message, "새는 메모리{x}");
    assert!(
        findings[1]
            .path
            .as_ref()
            .is_some_and(|p| p.ends_with("util.c"))
    );
    assert_eq!((findings[2].path.as_ref(), findings[2].line), (None, 0));
    assert!(parse("{}", &root, &root).is_err());
    assert!(parse("깨짐", &root, &root).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn 메시지_틀을_채운다() {
    let args = ["a".to_owned(), "b".to_owned()];
    assert_eq!(fill("{0}와 {1}", &args), "a와 b");
    assert_eq!(fill("{{0}} {2} {", &args), "{0} {2} {");
    assert_eq!(fill("끝 {1", &args), "끝 {1");
}

#[test]
fn 프로젝트의_sarif_파일을_찾는다() {
    let root = temp_root("candidates");
    for dir in ["target", "build/reports/spotbugs", "node_modules/x"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("target/spotbugsSarif.json"), "{}").unwrap();
    std::fs::write(root.join("build/reports/spotbugs/main.sarif"), "{}").unwrap();
    std::fs::write(root.join("semgrep.sarif.json"), "{}").unwrap();
    std::fs::write(root.join("target/other.json"), "{}").unwrap();
    std::fs::write(root.join("node_modules/x/a.sarif"), "{}").unwrap();
    let mut found: Vec<String> = candidates(&root)
        .iter()
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            "build/reports/spotbugs/main.sarif",
            "semgrep.sarif.json",
            "target/spotbugsSarif.json"
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}
