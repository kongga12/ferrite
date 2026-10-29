//! 실제 테스트 도구(Maven·pytest·Go·CTest)를 돌려 보고서를 읽고, 실패 스택의 줄을 프로젝트에서 찾고,
//! 실패한 것만 다시 돌린다. 정적 분석기(SpotBugs)가 낸 SARIF도 읽는다. 도구가 필요하고 Maven은
//! 의존성을 받으므로 기본으로는 돌지 않는다. CI의 3개 OS 전체 작업이 `--ignored`로 돌린다. 도구가 없으면
//! 로컬에서는 건너뛰고 CI에서는 실패한다.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use ferrite_core::test_report::{self, Case, Outcome, Tool};

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("ferrite-test-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 셸로 명령 줄을 돌려 (성공했는지, 표준 출력)을 돌려준다(테스트 창과 같은 셸).
fn run(dir: &Path, line: &str) -> (bool, String) {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd.exe");
        command
            .args(["/d", "/s", "/c"])
            .raw_arg(format!("\"{line}\""));
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", line]);
        command
    };
    let output = command.current_dir(dir).output().unwrap();
    eprintln!("> {line}\n{}", String::from_utf8_lossy(&output.stderr));
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// 도구가 있는지. 없으면 로컬은 건너뛰고(false) CI는 실패한다.
fn require(line: &str) -> bool {
    let found = run(&std::env::temp_dir(), line).0;
    if !found {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 `{line}`이 돼야 합니다"
        );
        eprintln!("`{line}`이 안 돼 건너뜁니다");
    }
    found
}

fn junit(tool: Tool, root: &Path, report: &Path, since: SystemTime) -> Vec<Case> {
    let files = test_report::report_files(tool, root, report, since);
    assert!(!files.is_empty(), "보고서가 없음");
    files
        .iter()
        .flat_map(|f| test_report::parse_junit(&std::fs::read_to_string(f).unwrap()).unwrap())
        .collect()
}

fn outcomes(cases: &[Case]) -> Vec<(String, Outcome)> {
    let mut found: Vec<(String, Outcome)> =
        cases.iter().map(|c| (c.name.clone(), c.outcome)).collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// 실패한 테스트의 먼저 갈 줄(파일, 줄).
fn test_line(root: &Path, dir: &Path, case: &Case) -> (PathBuf, u32) {
    let frames = test_report::frames(case);
    let frame = &frames[test_report::test_frame(case, &frames).expect("테스트 프레임")];
    (
        test_report::resolve(root, dir, frame).expect("프로젝트의 파일"),
        frame.line,
    )
}

fn failed(cases: &[Case]) -> Vec<&Case> {
    cases
        .iter()
        .filter(|c| c.outcome == Outcome::Failed)
        .collect()
}

#[test]
#[ignore = "Maven으로 JUnit 5 테스트를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_surefire_보고서와_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    let root = temp_dir("maven");
    let java = root.join("src/test/java/com/example/board");
    std::fs::create_dir_all(&java).unwrap();
    std::fs::write(
        root.join("pom.xml"),
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>board</artifactId>
  <version>1.0</version>
  <properties>
    <maven.compiler.release>17</maven.compiler.release>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
  </properties>
  <dependencies>
    <dependency>
      <groupId>org.junit.jupiter</groupId>
      <artifactId>junit-jupiter</artifactId>
      <version>5.12.2</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.4</version>
      </plugin>
    </plugins>
  </build>
</project>
"#,
    )
    .unwrap();
    std::fs::write(
        java.join("CalcTest.java"),
        "package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Disabled;\nimport org.junit.jupiter.api.Test;\n\nclass CalcTest {\n    @Test\n    void adds() {\n        assertEquals(3, 1 + 2);\n    }\n\n    @Test\n    void addsWrong() {\n        assertEquals(4, 1 + 2, \"더하기\");\n    }\n\n    @Test\n    void throwsError() {\n        throw new IllegalStateException(\"게시판 오류\");\n    }\n\n    @Disabled(\"나중에\")\n    @Test\n    void skipped() {\n    }\n}\n",
    )
    .unwrap();
    let report = root.join("reports");
    let command = "mvn -B -q test";
    let since = SystemTime::now();
    let (ok, _) = run(
        &root,
        &test_report::with_report(Tool::Maven, command, &report),
    );
    assert!(!ok, "실패한 테스트가 있으면 mvn이 실패한다");
    let cases = junit(Tool::Maven, &root, &report, since);
    assert_eq!(
        outcomes(&cases),
        [
            ("adds".to_owned(), Outcome::Passed),
            ("addsWrong".to_owned(), Outcome::Failed),
            ("skipped".to_owned(), Outcome::Skipped),
            ("throwsError".to_owned(), Outcome::Failed),
        ]
    );
    let wrong = cases.iter().find(|c| c.name == "addsWrong").unwrap();
    assert_eq!(
        test_line(&root, &root, wrong),
        (java.join("CalcTest.java"), 16)
    );

    let since = SystemTime::now();
    let rerun = test_report::rerun(Tool::Maven, command, &failed(&cases)).unwrap();
    run(&root, &rerun);
    let again = junit(Tool::Maven, &root, &report, since);
    assert_eq!(
        outcomes(&again),
        [
            ("addsWrong".to_owned(), Outcome::Failed),
            ("throwsError".to_owned(), Outcome::Failed),
        ],
        "실패한 것만 다시"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "pytest가 필요하다"]
fn pytest_junitxml_보고서와_실패만_다시() {
    if !require("python -m pytest --version") {
        return;
    }
    let root = temp_dir("pytest");
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(
        root.join("tests/test_calc.py"),
        "import pytest\n\n\ndef add(a, b):\n    return a + b\n\n\ndef test_add():\n    assert add(1, 2) == 3\n\n\ndef test_fail():\n    assert add(1, 2) == 4\n\n\n@pytest.mark.skip(reason=\"later\")\ndef test_skip():\n    pass\n",
    )
    .unwrap();
    let report = root.join("reports");
    std::fs::create_dir_all(&report).unwrap();
    // 실패만 다시(--lf)는 지난 실패를 pytest 캐시(.pytest_cache)에서 읽는다.
    let command = "python -m pytest -q";
    let since = SystemTime::now();
    run(
        &root,
        &test_report::with_report(Tool::Pytest, command, &report),
    );
    let cases = junit(Tool::Pytest, &root, &report, since);
    assert_eq!(
        outcomes(&cases),
        [
            ("test_add".to_owned(), Outcome::Passed),
            ("test_fail".to_owned(), Outcome::Failed),
            ("test_skip".to_owned(), Outcome::Skipped),
        ]
    );
    let fail = cases.iter().find(|c| c.name == "test_fail").unwrap();
    let (path, line) = test_line(&root, &root, fail);
    assert!(
        path.ends_with("test_calc.py") && path.is_file(),
        "{}",
        path.display()
    );
    assert_eq!(line, 13);

    let since = SystemTime::now();
    let rerun = test_report::rerun(Tool::Pytest, command, &failed(&cases)).unwrap();
    run(
        &root,
        &test_report::with_report(Tool::Pytest, &rerun, &report),
    );
    let again = junit(Tool::Pytest, &root, &report, since);
    assert_eq!(
        outcomes(&again),
        [("test_fail".to_owned(), Outcome::Failed)],
        "실패한 것만 다시"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Go가 필요하다"]
fn go_test_json과_실패만_다시() {
    if !require("go version") {
        return;
    }
    let root = temp_dir("go");
    std::fs::create_dir_all(root.join("calc")).unwrap();
    std::fs::write(root.join("go.mod"), "module example.com/board\n\ngo 1.21\n").unwrap();
    std::fs::write(
        root.join("calc/calc.go"),
        "package calc\n\nfunc Add(a, b int) int { return a + b }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("calc/calc_test.go"),
        "package calc\n\nimport \"testing\"\n\nfunc TestAdd(t *testing.T) {\n\tif Add(1, 2) != 3 {\n\t\tt.Fatal(\"add\")\n\t}\n}\n\nfunc TestWrong(t *testing.T) {\n\tif got := Add(1, 2); got != 4 {\n\t\tt.Errorf(\"got %d, want 4\", got)\n\t}\n}\n\nfunc TestSkip(t *testing.T) {\n\tt.Skip(\"later\")\n}\n",
    )
    .unwrap();
    let command = "go test ./...";
    let (_, stdout) = run(&root, &test_report::with_report(Tool::Go, command, &root));
    let cases = test_report::parse_go_json(&stdout);
    assert_eq!(
        outcomes(&cases),
        [
            ("TestAdd".to_owned(), Outcome::Passed),
            ("TestSkip".to_owned(), Outcome::Skipped),
            ("TestWrong".to_owned(), Outcome::Failed),
        ]
    );
    let wrong = cases.iter().find(|c| c.name == "TestWrong").unwrap();
    assert_eq!(wrong.suite, "example.com/board/calc");
    assert_eq!(wrong.message.as_deref(), Some("got 3, want 4"));
    assert_eq!(
        test_line(&root, &root, wrong),
        (root.join("calc").join("calc_test.go"), 13)
    );

    let rerun = test_report::rerun(Tool::Go, command, &failed(&cases)).unwrap();
    let (_, stdout) = run(&root, &rerun);
    assert_eq!(
        outcomes(&test_report::parse_go_json(&stdout)),
        [("TestWrong".to_owned(), Outcome::Failed)],
        "실패한 것만 다시"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "CMake 3.21 이상이 필요하다"]
fn ctest_output_junit과_실패만_다시() {
    if !require("ctest --version") {
        return;
    }
    let root = temp_dir("ctest");
    std::fs::write(
        root.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.21)\nproject(t NONE)\nenable_testing()\nadd_test(NAME ok COMMAND ${CMAKE_COMMAND} -E true)\nadd_test(NAME bad COMMAND ${CMAKE_COMMAND} -E false)\nadd_test(NAME off COMMAND ${CMAKE_COMMAND} -E true)\nset_tests_properties(off PROPERTIES DISABLED TRUE)\n",
    )
    .unwrap();
    assert!(run(&root, "cmake -S . -B build").0, "cmake 구성");
    let report = root.join("reports");
    std::fs::create_dir_all(&report).unwrap();
    // Windows의 Visual Studio 생성기는 다중 구성이라 `-C`가 없으면 테스트를 돌리지 않는다(Not Run).
    let command = "ctest --test-dir build -C Debug";
    let since = SystemTime::now();
    run(
        &root,
        &test_report::with_report(Tool::Ctest, command, &report),
    );
    let cases = junit(Tool::Ctest, &root, &report, since);
    assert_eq!(
        outcomes(&cases),
        [
            ("bad".to_owned(), Outcome::Failed),
            ("off".to_owned(), Outcome::Skipped),
            ("ok".to_owned(), Outcome::Passed),
        ]
    );
    let since = SystemTime::now();
    let rerun = test_report::rerun(Tool::Ctest, command, &failed(&cases)).unwrap();
    run(
        &root,
        &test_report::with_report(Tool::Ctest, &rerun, &report),
    );
    let again = junit(Tool::Ctest, &root, &report, since);
    assert_eq!(
        outcomes(&again),
        [("bad".to_owned(), Outcome::Failed)],
        "실패한 것만 다시"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Maven으로 SpotBugs(spotbugs-maven-plugin 4.10.5.0, 약 31MB)를 받아 돌린다. Java 17 이상과 mvn이 필요하다"]
fn spotbugs가_낸_sarif를_읽고_원본_파일을_찾는다() {
    use ferrite_core::sarif::{self, Level};

    if !require("mvn -v") {
        return;
    }
    let root = temp_dir("spotbugs");
    let java = root.join("src/main/java/com/example/board");
    std::fs::create_dir_all(&java).unwrap();
    std::fs::write(
        root.join("pom.xml"),
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>board</artifactId>
  <version>1.0</version>
  <properties>
    <maven.compiler.release>17</maven.compiler.release>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
  </properties>
</project>
"#,
    )
    .unwrap();
    std::fs::write(
        java.join("Board.java"),
        "package com.example.board;\n\npublic class Board {\n    private String title;\n\n    public boolean same(String other) {\n        return title == other;\n    }\n\n    public int length(String s) {\n        if (s == null) {\n            return s.length();\n        }\n        return 0;\n    }\n}\n",
    )
    .unwrap();
    let (ok, _) = run(
        &root,
        "mvn -B -q compile com.github.spotbugs:spotbugs-maven-plugin:4.10.5.0:spotbugs -Dspotbugs.sarifOutput=true",
    );
    assert!(ok, "SpotBugs 분석");
    let report = root.join("target/spotbugsSarif.json");
    assert_eq!(
        sarif::candidates(&root),
        std::slice::from_ref(&report),
        "프로젝트에서 찾는다"
    );
    let text = std::fs::read_to_string(&report).unwrap();
    let findings = sarif::parse(&text, &root, &root.join("target")).unwrap();
    let np = findings
        .iter()
        .find(|f| f.rule == "NP_ALWAYS_NULL")
        .expect("null 역참조");
    assert_eq!(
        (np.tool.as_str(), np.level, np.line),
        ("SpotBugs", Level::Error, 12)
    );
    assert_eq!(
        np.message,
        "Null pointer dereference of s in com.example.board.Board.length(String)."
    );
    assert_eq!(np.path.as_deref(), Some(java.join("Board.java").as_path()));
    assert!(
        findings
            .iter()
            .any(|f| f.rule == "ES_COMPARING_PARAMETER_STRING_WITH_EQ" && f.line == 7)
    );
    let _ = std::fs::remove_dir_all(&root);
}
