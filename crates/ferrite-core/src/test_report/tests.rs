use super::*;

/// Maven Surefire 3.5.4(JUnit 5.12)가 실제로 남긴 보고서에서 속성만 뺐다.
const SUREFIRE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="3.0.2" name="com.example.board.CalcTest" time="0.056" tests="4" errors="1" skipped="1" failures="1">
  <properties>
    <property name="file.encoding" value="UTF-8"/>
  </properties>
  <testcase name="adds" classname="com.example.board.CalcTest" time="0.022"/>
  <testcase name="throwsError" classname="com.example.board.CalcTest" time="0.005">
    <error message="게시판 오류" type="java.lang.IllegalStateException"><![CDATA[java.lang.IllegalStateException: 게시판 오류
	at com.example.board.CalcTest.throwsError(CalcTest.java:21)
	at java.base/java.lang.reflect.Method.invoke(Method.java:580)
]]></error>
  </testcase>
  <testcase name="addsWrong" classname="com.example.board.CalcTest" time="0.004">
    <failure message="더하기 ==&gt; expected: &lt;4&gt; but was: &lt;3&gt;" type="org.opentest4j.AssertionFailedError"><![CDATA[org.opentest4j.AssertionFailedError: 더하기 ==> expected: <4> but was: <3>
	at org.junit.jupiter.api.AssertionFailureBuilder.build(AssertionFailureBuilder.java:151)
	at org.junit.jupiter.api.Assertions.assertEquals(Assertions.java:563)
	at com.example.board.CalcTest.addsWrong(CalcTest.java:16)
	at java.base/java.lang.reflect.Method.invoke(Method.java:580)
]]></failure>
  </testcase>
  <testcase name="skipped" classname="com.example.board.CalcTest" time="0.0">
    <skipped message="나중에"/>
  </testcase>
</testsuite>"#;

/// Gradle 8(JUnit 5): 메서드 이름에 `()`, 스택에 로더 앞붙이(`app//`).
const GRADLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="com.example.board.CalcTest" tests="2" skipped="0" failures="1" errors="0" timestamp="2026-10-29T05:00:00" hostname="pc" time="0.03">
  <properties/>
  <testcase name="adds()" classname="com.example.board.CalcTest" time="0.01"/>
  <testcase name="addsWrong()" classname="com.example.board.CalcTest" time="0.02">
    <failure message="org.opentest4j.AssertionFailedError: expected: &lt;4&gt; but was: &lt;3&gt;" type="org.opentest4j.AssertionFailedError">org.opentest4j.AssertionFailedError: expected: &lt;4&gt; but was: &lt;3&gt;
	at app//org.junit.jupiter.api.AssertEquals.assertEquals(AssertEquals.java:150)
	at app//com.example.board.CalcTest$Inner.addsWrong(CalcTest.java:16)
</failure>
  </testcase>
  <system-out><![CDATA[]]></system-out>
  <system-err><![CDATA[]]></system-err>
</testsuite>"#;

/// pytest 8(`junit_family=xunit2`, Windows 경로).
const PYTEST: &str = r#"<?xml version="1.0" encoding="utf-8"?><testsuites name="pytest tests"><testsuite name="pytest" errors="0" failures="1" skipped="1" tests="3" time="0.031" hostname="pc"><testcase classname="tests.test_calc" name="test_add" time="0.001" /><testcase classname="tests.test_calc" name="test_fail" time="0.002"><failure message="assert 3 == 4&#10; +  where 3 = add(1, 2)">def test_fail():
&gt;       assert add(1, 2) == 4
E       assert 3 == 4

tests\test_calc.py:8: AssertionError</failure></testcase><testcase classname="tests.test_calc" name="test_skip" time="0.000"><skipped type="pytest.skip" message="나중에">tests\test_calc.py:10: 나중에</skipped></testcase></testsuite></testsuites>"#;

/// CTest 3.2x(`--output-junit`): 결과는 status, 출력은 system-out에만. CMake의 JUnit 쓰기(cmCTestTestHandler)를
/// 따랐다: disabled는 요소 없이, notrun은 `<skipped message="사유">`.
const CTEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="Linux-c++" tests="3" failures="1" disabled="1" skipped="0" hostname="" time="0" timestamp="2026-10-29T05:00:00" >
	<testcase name="ok" classname="ok" time="0.0123" status="run">
		<system-out>ok
</system-out>
	</testcase>
	<testcase name="bad" classname="bad" time="0.0101" status="fail">
		<failure message="Failed"/>
		<system-out>bad output
</system-out>
	</testcase>
	<testcase name="off" classname="off" time="0" status="disabled">
	</testcase>
	<testcase name="nocfg" classname="nocfg" time="0" status="notrun">
		<skipped message="Required configuration not found"/>
		<system-out>Test not available without configuration.  (Missing "-C &lt;config&gt;"?)
</system-out>
	</testcase>
	<testcase name="later" classname="later" time="0" status="notrun">
		<skipped message="SKIP_RETURN_CODE=4"/>
	</testcase>
</testsuite>"#;

/// `go test -json`: 통과·실패·건너뜀, 부속 테스트, Go 1.24의 빌드 실패(`build-output`).
const GO: &str = r##"{"Action":"start","Package":"example.com/board/calc"}
{"Action":"run","Package":"example.com/board/calc","Test":"TestAdd"}
{"Action":"output","Package":"example.com/board/calc","Test":"TestAdd","Output":"=== RUN   TestAdd\n"}
{"Action":"output","Package":"example.com/board/calc","Test":"TestAdd","Output":"--- PASS: TestAdd (0.00s)\n"}
{"Action":"pass","Package":"example.com/board/calc","Test":"TestAdd","Elapsed":0}
{"Action":"run","Package":"example.com/board/calc","Test":"TestWrong"}
{"Action":"run","Package":"example.com/board/calc","Test":"TestWrong/sub"}
{"Action":"output","Package":"example.com/board/calc","Test":"TestWrong/sub","Output":"    calc_test.go:12: 더하기: got 3, want 4\n"}
{"Action":"fail","Package":"example.com/board/calc","Test":"TestWrong/sub","Elapsed":0}
{"Action":"fail","Package":"example.com/board/calc","Test":"TestWrong","Elapsed":0.01}
{"Action":"run","Package":"example.com/board/calc","Test":"TestSkip"}
{"Action":"output","Package":"example.com/board/calc","Test":"TestSkip","Output":"    calc_test.go:16: 나중에\n"}
{"Action":"skip","Package":"example.com/board/calc","Test":"TestSkip","Elapsed":0}
{"Action":"output","Package":"example.com/board/calc","Output":"FAIL\n"}
{"Action":"fail","Package":"example.com/board/calc","Elapsed":0.2}
{"ImportPath":"example.com/board/broken [example.com/board/broken.test]","Action":"build-output","Output":"# example.com/board/broken\n"}
{"ImportPath":"example.com/board/broken [example.com/board/broken.test]","Action":"build-output","Output":"broken/x_test.go:5:2: undefined: nope\n"}
{"ImportPath":"example.com/board/broken [example.com/board/broken.test]","Action":"build-fail"}
{"Action":"fail","Package":"example.com/board/broken","Elapsed":0,"FailedBuild":"example.com/board/broken [example.com/board/broken.test]"}
not json
"##;

fn outcomes(cases: &[Case]) -> Vec<(&str, Outcome)> {
    cases.iter().map(|c| (c.name.as_str(), c.outcome)).collect()
}

#[test]
fn 명령으로_테스트_도구를_알아낸다() {
    assert_eq!(Tool::of("mvn test"), Some(Tool::Maven));
    assert_eq!(Tool::of("./mvnw -q verify"), Some(Tool::Maven));
    assert_eq!(Tool::of("mvnw.cmd test"), Some(Tool::Maven));
    assert_eq!(Tool::of("mvn package"), None, "테스트 명령이 아니다");
    assert_eq!(Tool::of("gradlew.bat test"), Some(Tool::Gradle));
    assert_eq!(Tool::of("gradle bootRun"), None);
    assert_eq!(Tool::of("python -m pytest -q"), Some(Tool::Pytest));
    assert_eq!(Tool::of("pytest tests"), Some(Tool::Pytest));
    assert_eq!(Tool::of("ctest --test-dir build"), Some(Tool::Ctest));
    assert_eq!(Tool::of("go test ./..."), Some(Tool::Go));
    assert_eq!(Tool::of("go build ./..."), None);
    assert_eq!(Tool::of("cargo test"), None);
}

#[test]
fn surefire_보고서의_실패_오류_건너뜀을_읽는다() {
    let cases = parse_junit(SUREFIRE).unwrap();
    assert_eq!(
        outcomes(&cases),
        [
            ("adds", Outcome::Passed),
            ("throwsError", Outcome::Failed),
            ("addsWrong", Outcome::Failed),
            ("skipped", Outcome::Skipped),
        ]
    );
    let wrong = &cases[2];
    assert_eq!(wrong.suite, "com.example.board.CalcTest");
    assert_eq!(
        wrong.message.as_deref(),
        Some("더하기 ==> expected: <4> but was: <3>")
    );
    assert_eq!(wrong.seconds, Some(0.004));
    let found = frames(wrong);
    let first = test_frame(wrong, &found).unwrap();
    assert_eq!(
        (&found[first].file, found[first].line),
        (&"CalcTest.java".to_owned(), 16),
        "라이브러리 프레임이 아니라 테스트 클래스의 줄"
    );
    assert_eq!(cases[3].message.as_deref(), Some("나중에"));
}

#[test]
fn gradle_pytest_ctest_보고서를_읽는다() {
    let gradle = parse_junit(GRADLE).unwrap();
    assert_eq!(gradle[1].name, "addsWrong()");
    let found = frames(&gradle[1]);
    assert_eq!(found.len(), 2, "app// 앞붙이도 읽는다");
    assert_eq!(
        found[1].class.as_deref(),
        Some("com.example.board.CalcTest"),
        "안쪽 클래스는 바깥 클래스로"
    );
    assert_eq!(test_frame(&gradle[1], &found), Some(1));

    let pytest = parse_junit(PYTEST).unwrap();
    assert_eq!(
        outcomes(&pytest),
        [
            ("test_add", Outcome::Passed),
            ("test_fail", Outcome::Failed),
            ("test_skip", Outcome::Skipped),
        ]
    );
    assert_eq!(pytest[1].suite, "tests.test_calc");
    assert_eq!(pytest[1].message.as_deref(), Some("assert 3 == 4"));
    let found = frames(&pytest[1]);
    assert_eq!(
        (&found[0].file, found[0].line),
        (&"tests\\test_calc.py".to_owned(), 8)
    );
    assert_eq!(test_frame(&pytest[1], &found), Some(0));

    let ctest = parse_junit(CTEST).unwrap();
    assert_eq!(
        outcomes(&ctest),
        [
            ("ok", Outcome::Passed),
            ("bad", Outcome::Failed),
            ("off", Outcome::Skipped),
            ("nocfg", Outcome::Failed),
            ("later", Outcome::Skipped),
        ]
    );
    assert_eq!(ctest[1].message.as_deref(), Some("Failed"));
    assert!(ctest[1].detail.contains("bad output"), "실패 출력");
    // 돌리지 못한 것(다중 구성 빌드에 -C 빠짐)은 CTest처럼 실패, SKIP_*로 건너뛴 것은 건너뜀.
    assert_eq!(
        ctest[3].message.as_deref(),
        Some("돌리지 못함: Required configuration not found")
    );
    assert!(ctest[3].detail.contains("Missing \"-C <config>\""));
    assert!(parse_junit("<testsuite>").is_err());
}

#[test]
fn go_test_json을_읽고_빌드_실패는_패키지로_보인다() {
    let cases = parse_go_json(GO);
    assert_eq!(
        outcomes(&cases),
        [
            ("TestAdd", Outcome::Passed),
            ("TestWrong/sub", Outcome::Failed),
            ("TestWrong", Outcome::Failed),
            ("TestSkip", Outcome::Skipped),
            (PACKAGE_CASE, Outcome::Failed),
        ]
    );
    let sub = &cases[1];
    assert_eq!(sub.message.as_deref(), Some("더하기: got 3, want 4"));
    let found = frames(sub);
    assert_eq!(
        (&found[0].file, found[0].line),
        (&"calc_test.go".to_owned(), 12)
    );
    assert_eq!(cases[0].message, None);
    assert!(!cases[0].detail.contains("=== RUN"));
    let broken = &cases[4];
    assert_eq!(broken.suite, "example.com/board/broken");
    assert!(
        broken.detail.contains("undefined: nope"),
        "{}",
        broken.detail
    );
}

#[test]
fn 보고서_옵션과_실패만_다시_돌리는_명령을_만든다() {
    let dir = Path::new("/tmp/r");
    assert_eq!(with_report(Tool::Maven, "mvn test", dir), "mvn test");
    let pytest = with_report(Tool::Pytest, "python -m pytest", dir);
    assert!(
        pytest.starts_with("python -m pytest \"--junitxml=") && pytest.ends_with("pytest.xml\"")
    );
    assert_eq!(
        with_report(Tool::Pytest, "pytest --junitxml=x.xml", dir),
        "pytest --junitxml=x.xml"
    );
    assert!(with_report(Tool::Ctest, "ctest --test-dir build", dir).contains("--output-junit \""));
    assert_eq!(
        with_report(Tool::Go, "go test ./...", dir),
        "go test -json ./..."
    );
    assert_eq!(
        with_report(Tool::Go, "go test -json ./...", dir),
        "go test -json ./..."
    );

    let maven = parse_junit(SUREFIRE).unwrap();
    let failed: Vec<&Case> = maven
        .iter()
        .filter(|c| c.outcome == Outcome::Failed)
        .collect();
    assert_eq!(
        rerun(Tool::Maven, "mvn test", &failed).unwrap(),
        "mvn test -Dtest=CalcTest#throwsError+addsWrong -Dsurefire.failIfNoSpecifiedTests=false"
    );
    let gradle = parse_junit(GRADLE).unwrap();
    assert_eq!(
        rerun(Tool::Gradle, "gradle test", &[&gradle[1]]).unwrap(),
        "gradle test --tests \"com.example.board.CalcTest.addsWrong\""
    );
    assert_eq!(
        rerun(Tool::Pytest, "pytest", &failed).unwrap(),
        "pytest --lf"
    );
    assert_eq!(
        rerun(Tool::Ctest, "ctest", &failed).unwrap(),
        "ctest --rerun-failed"
    );
    let go = parse_go_json(GO);
    let failed: Vec<&Case> = go.iter().filter(|c| c.outcome == Outcome::Failed).collect();
    assert_eq!(
        rerun(Tool::Go, "go test ./...", &failed).unwrap(),
        "go test -json -run \"^(TestWrong)$\" example.com/board/calc example.com/board/broken"
    );
    assert_eq!(rerun(Tool::Maven, "mvn test", &[]), None);
}

#[test]
fn 이번_실행의_보고서를_찾고_스택의_파일을_프로젝트에서_찾는다() {
    let root = std::env::temp_dir().join(format!("ferrite-test-report-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let reports = root.join("module/target/surefire-reports");
    let java = root.join("module/src/test/java/com/example/board");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::create_dir_all(&java).unwrap();
    std::fs::create_dir_all(root.join("app/build/test-results/test")).unwrap();
    std::fs::write(
        reports.join("TEST-com.example.board.CalcTest.xml"),
        SUREFIRE,
    )
    .unwrap();
    std::fs::write(reports.join("com.example.board.CalcTest.txt"), "").unwrap();
    std::fs::write(root.join("app/build/test-results/test/TEST-x.xml"), GRADLE).unwrap();
    std::fs::write(java.join("CalcTest.java"), "class CalcTest {}").unwrap();
    let before = SystemTime::now() - Duration::from_secs(60);
    let later = SystemTime::now() + Duration::from_secs(60);
    let none = Path::new("/없는/폴더");
    assert_eq!(
        report_files(Tool::Maven, &root, none, before),
        [reports.join("TEST-com.example.board.CalcTest.xml")]
    );
    assert!(
        report_files(Tool::Maven, &root, none, later).is_empty(),
        "지난 실행의 보고서"
    );
    assert_eq!(report_files(Tool::Gradle, &root, none, before).len(), 1);

    let wrong = &parse_junit(SUREFIRE).unwrap()[2];
    let found = frames(wrong);
    let frame = &found[test_frame(wrong, &found).unwrap()];
    assert_eq!(
        resolve(&root, &root, frame),
        Some(java.join("CalcTest.java"))
    );
    // 그 밖은 돌린 폴더 기준, 없으면 경로 끝이 같은 파일. pytest는 Windows에서 `\`로 적는다.
    std::fs::create_dir_all(root.join("py/tests")).unwrap();
    std::fs::write(root.join("py/tests/test_calc.py"), "").unwrap();
    let pytest = &parse_junit(PYTEST).unwrap()[1];
    let frame = &frames(pytest)[0];
    for dir in [root.join("py"), root.clone()] {
        let found = resolve(&root, &dir, frame).unwrap();
        assert!(
            found.is_file() && found.ends_with("test_calc.py"),
            "{}",
            found.display()
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}
