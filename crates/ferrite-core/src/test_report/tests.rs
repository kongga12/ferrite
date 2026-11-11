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
    assert_eq!(
        with_report(Tool::Maven, "mvn test", dir),
        "mvn test -fae -Dmaven.test.failure.ignore=true"
    );
    assert_eq!(
        with_report(
            Tool::Maven,
            "mvn -ff test -Dmaven.test.failure.ignore=false",
            dir
        ),
        "mvn -ff test -Dmaven.test.failure.ignore=false",
        "사용자가 고른 실패 방식은 그대로"
    );
    assert_eq!(
        with_report(Tool::Gradle, "gradle test", dir),
        "gradle test --continue"
    );
    assert_eq!(
        with_report(Tool::Gradle, "gradle test --continue", dir),
        "gradle test --continue"
    );
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
fn maven_통합_테스트는_failsafe_보고서로_가리고_it_test로_다시_돌린다() {
    // failsafe-reports의 보고서는 통합 테스트로, surefire-reports의 것은 단위 테스트로 읽는다.
    let root = std::env::temp_dir().join(format!("ferrite-failsafe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (unit_dir, it_dir) = (
        root.join("target/surefire-reports"),
        root.join("target/failsafe-reports"),
    );
    std::fs::create_dir_all(&unit_dir).unwrap();
    std::fs::create_dir_all(&it_dir).unwrap();
    let report = |suite: &str, names: &[&str]| {
        let cases: String = names
            .iter()
            .map(|name| {
                format!(
                    "<testcase name=\"{name}\" classname=\"com.example.board.{suite}\"><failure message=\"틀림\"/></testcase>"
                )
            })
            .collect();
        format!("<testsuite name=\"com.example.board.{suite}\">{cases}</testsuite>")
    };
    std::fs::write(
        unit_dir.join("TEST-com.example.board.CalcTest.xml"),
        report("CalcTest", &["addsWrong"]),
    )
    .unwrap();
    std::fs::write(
        it_dir.join("TEST-com.example.board.BoardIT.xml"),
        report("BoardIT", &["savesWrong", "listsWrong"]),
    )
    .unwrap();
    let unit = read_report(&unit_dir.join("TEST-com.example.board.CalcTest.xml")).unwrap();
    let it = read_report(&it_dir.join("TEST-com.example.board.BoardIT.xml")).unwrap();
    assert!(unit.iter().all(|c| !c.integration));
    assert!(it.iter().all(|c| c.integration) && it.len() == 2);
    assert!(read_report(&root.join("없음.xml")).is_err());
    std::fs::remove_dir_all(&root).unwrap();

    // 둘 다 실패: 단위 테스트는 `-Dtest`, 통합 테스트는 `-Dit.test`로 거른다. 다중 모듈이면 거른 것이 없는
    // 모듈도 있어 둘 다 "없으면 실패"를 끈다.
    let both: Vec<&Case> = unit.iter().chain(&it).collect();
    assert_eq!(
        rerun(Tool::Maven, "mvn verify", &both).unwrap(),
        "mvn verify -Dtest=CalcTest#addsWrong -Dsurefire.failIfNoSpecifiedTests=false \
         -Dit.test=BoardIT#savesWrong+listsWrong -Dit.failIfNoSpecifiedTests=false"
    );
    // 통합 테스트만 실패: Surefire는 아무것도 돌리지 않는다(맞는 클래스가 없는 이름).
    let only_it: Vec<&Case> = it.iter().collect();
    assert_eq!(
        rerun(Tool::Maven, "mvn verify", &only_it).unwrap(),
        "mvn verify -Dtest=NoUnitTestsToRerun -Dsurefire.failIfNoSpecifiedTests=false \
         -Dit.test=BoardIT#savesWrong+listsWrong -Dit.failIfNoSpecifiedTests=false"
    );
    // 단위 테스트만 실패: 통합 테스트 단계까지 가는 명령이면 통합 테스트는 건너뛴다.
    let only_unit: Vec<&Case> = unit.iter().collect();
    assert_eq!(
        rerun(Tool::Maven, "mvn -B install", &only_unit).unwrap(),
        "mvn -B install -Dtest=CalcTest#addsWrong -Dsurefire.failIfNoSpecifiedTests=false -DskipITs"
    );
    assert_eq!(
        rerun(Tool::Maven, "mvn test", &only_unit).unwrap(),
        "mvn test -Dtest=CalcTest#addsWrong -Dsurefire.failIfNoSpecifiedTests=false"
    );
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

#[test]
fn kotlin_scala_스택의_테스트_줄을_찾는다() {
    let failed = |suite: &str, detail: &str| Case {
        suite: suite.to_owned(),
        name: String::new(),
        outcome: Outcome::Failed,
        seconds: None,
        message: None,
        detail: detail.to_owned(),
        file: None,
        line: None,
        integration: false,
    };
    // Kotlin 백틱 이름은 빈칸이 들고, 그 안의 람다는 클래스 이름에도 든다.
    let kotlin = failed(
        "com.example.board.CalcTest",
        "org.opentest4j.AssertionFailedError: 더하기\n\tat app//org.junit.jupiter.api.Assertions.assertEquals(Assertions.java:563)\n\tat app//com.example.board.CalcTest$adds wrong numbers$1.invoke(CalcTest.kt:15)\n\tat app//com.example.board.CalcTest.adds wrong numbers(CalcTest.kt:14)\n",
    );
    let found = frames(&kotlin);
    assert_eq!(found.len(), 3);
    assert_eq!(
        found[1].class.as_deref(),
        Some("com.example.board.CalcTest"),
        "람다는 바깥 클래스로"
    );
    assert_eq!(test_frame(&kotlin, &found), Some(1));
    assert_eq!((found[2].file.as_str(), found[2].line), ("CalcTest.kt", 14));
    // Scala 람다는 같은 클래스의 `$anonfun$` 메서드다.
    let scala = failed(
        "com.example.board.CalcSuite",
        "org.opentest4j.AssertionFailedError: 더하기\n\tat app//com.example.board.CalcSuite.$anonfun$addsWrong$1(CalcSuite.scala:12)\n\tat app//scala.collection.immutable.List.foreach(List.scala:334)\n",
    );
    let found = frames(&scala);
    assert_eq!(
        found[test_frame(&scala, &found).unwrap()].line,
        12,
        "라이브러리가 아니라 테스트 클래스의 줄"
    );

    // Kotlin 관례대로 공통 루트 패키지를 폴더에서 뺀 파일도 찾는다. Java는 패키지 경로로만 찾는다.
    let root = std::env::temp_dir().join(format!("ferrite-test-report-kt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let kotlin_dir = root.join("kt/src/test/kotlin");
    std::fs::create_dir_all(&kotlin_dir).unwrap();
    std::fs::write(kotlin_dir.join("CalcTest.kt"), "").unwrap();
    std::fs::write(kotlin_dir.join("Assert.java"), "").unwrap();
    let frame = |class: &str, file: &str| Frame {
        text: String::new(),
        file: file.to_owned(),
        line: 1,
        class: Some(class.to_owned()),
    };
    assert_eq!(
        resolve(
            &root,
            &root,
            &frame("com.example.board.CalcTest", "CalcTest.kt")
        ),
        Some(kotlin_dir.join("CalcTest.kt"))
    );
    assert_eq!(
        resolve(&root, &root, &frame("org.junit.Assert", "Assert.java")),
        None,
        "패키지가 다른 같은 이름의 Java 파일"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// Windows·JDK 21에서 한글 경로의 프로젝트를 `gradle test`로 돌린 실제 보고서(Gradle 9.2.0)에서 스택을 줄였다.
const GRADLE_CLASS_NOT_FOUND: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="Gradle Test Executor 1" tests="1" skipped="0" failures="1" errors="0" timestamp="2026-11-10T06:01:02.941Z" hostname="pc" time="0.105">
  <properties/>
  <testcase name="failed to execute tests" classname="Gradle Test Executor 1" time="0.105">
    <failure message="org.gradle.api.internal.tasks.testing.TestSuiteExecutionException: Could not execute test class 'com.example.board.CalcTest'." type="org.gradle.api.internal.tasks.testing.TestSuiteExecutionException">org.gradle.api.internal.tasks.testing.TestSuiteExecutionException: Could not execute test class 'com.example.board.CalcTest'.
	at org.gradle.api.internal.tasks.testing.SuiteTestClassProcessor.processTestClass(SuiteTestClassProcessor.java:56)
	at app//worker.org.gradle.process.internal.worker.GradleWorkerMain.main(GradleWorkerMain.java:74)
Caused by: org.gradle.internal.UncheckedException: java.lang.ClassNotFoundException: com.example.board.CalcTest
	at org.gradle.api.internal.tasks.testing.junitplatform.JUnitPlatformTestClassProcessor.loadClass(JUnitPlatformTestClassProcessor.java:186)
	... 17 more
Caused by: java.lang.ClassNotFoundException: com.example.board.CalcTest
	at java.base/jdk.internal.loader.BuiltinClassLoader.loadClass(BuiltinClassLoader.java:641)
	... 22 more
</failure>
  </testcase>
  <system-out><![CDATA[]]></system-out>
  <system-err><![CDATA[]]></system-err>
</testsuite>"#;

/// Windows에서 Gradle 홈이 한글 경로일 때(Gradle 9.2.0, JDK 21) 남는 보고서에서 스택을 줄였다. 까닭
/// (`GradleWorkerMain`을 찾지 못함)은 보고서에 없고 Gradle의 출력에만 나온다.
const GRADLE_EXECUTOR_EXITED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="Gradle Test Run :app:test" tests="1" skipped="0" failures="1" errors="0">
  <properties/>
  <testcase name="failed to execute tests" classname="Gradle Test Run :app:test" time="0.514">
    <failure message="org.gradle.process.ProcessExecutionException: Process 'Gradle Test Executor 1' finished with non-zero exit value 1" type="org.gradle.process.ProcessExecutionException">org.gradle.process.ProcessExecutionException: Process 'Gradle Test Executor 1' finished with non-zero exit value 1
	at org.gradle.process.internal.DefaultExecHandle$ExecResultImpl.assertNormalExitValue(DefaultExecHandle.java:450)
</failure>
  </testcase>
  <system-out><![CDATA[]]></system-out>
  <system-err><![CDATA[]]></system-err>
</testsuite>"#;

#[test]
fn 한글_경로에서_gradle이_테스트_클래스를_찾지_못한_것을_알아본다() {
    let cases = parse_junit(GRADLE_CLASS_NOT_FOUND).unwrap();
    let korean = Path::new(r"C:\Users\me\자바 게시판");
    let home = Path::new(r"C:\Users\me\.gradle");
    let problem =
        |tool, dir: &Path, cases: &[Case]| gradle_path_problem(tool, dir, home, cases, "");
    assert_eq!(
        problem(Tool::Gradle, korean, &cases),
        cfg!(windows).then_some(GradlePathProblem::Project),
        "Windows에서만 생긴다"
    );
    assert_eq!(
        problem(Tool::Gradle, Path::new(r"C:\work\board"), &cases),
        None
    );
    assert_eq!(problem(Tool::Maven, korean, &cases), None);
    // 테스트가 실제로 실패한 것은 아니다.
    let ordinary = parse_junit(GRADLE).unwrap();
    assert_eq!(problem(Tool::Gradle, korean, &ordinary), None);
}

#[test]
fn 한글_gradle_홈에서_테스트_jvm이_뜨지_못한_것을_알아본다() {
    let cases = parse_junit(GRADLE_EXECUTOR_EXITED).unwrap();
    let work = Path::new(r"C:\work\board");
    let korean_home = Path::new(r"C:\Users\홍길동\.gradle");
    // 테스트 JVM의 오류(한국어 Windows의 java가 949로 낸 글에서 클래스 이름은 그대로다).
    let output = "오류: java.lang.ClassNotFoundException: worker.org.gradle.process.internal.worker.GradleWorkerMain\n";
    assert_eq!(
        gradle_path_problem(Tool::Gradle, work, korean_home, &cases, output),
        cfg!(windows).then(|| GradlePathProblem::Home(korean_home.to_path_buf())),
        "Windows에서만 생긴다"
    );
    // 프로젝트 경로도 한글이면 홈을 먼저 알린다(홈이 깨지면 테스트 JVM부터 뜨지 못한다).
    assert_eq!(
        gradle_path_problem(
            Tool::Gradle,
            Path::new(r"C:\자바"),
            korean_home,
            &cases,
            output
        ),
        cfg!(windows).then(|| GradlePathProblem::Home(korean_home.to_path_buf()))
    );
    // 홈이 영문이거나, 출력에 까닭이 없으면(다른 까닭으로 테스트 JVM이 끝났다) 아니다.
    let home = Path::new(r"C:\Users\me\.gradle");
    assert_eq!(
        gradle_path_problem(Tool::Gradle, work, home, &cases, output),
        None
    );
    assert_eq!(
        gradle_path_problem(Tool::Gradle, work, korean_home, &cases, "Killed\n"),
        None
    );

    // Gradle 홈: 실행 구성의 환경 변수가 먼저다.
    let env = std::collections::BTreeMap::from([(
        "GRADLE_USER_HOME".to_owned(),
        r"D:\그레이들".to_owned(),
    )]);
    assert_eq!(gradle_user_home(&env), Some(PathBuf::from(r"D:\그레이들")));
    let default = gradle_user_home(&std::collections::BTreeMap::new()).unwrap();
    assert!(
        std::env::var_os("GRADLE_USER_HOME").is_some() || default.ends_with(".gradle"),
        "{}",
        default.display()
    );
}

#[test]
fn 종료_코드로_보고서에_없는_실패를_알아본다() {
    // Maven은 테스트 실패로 멈추지 않게 돌리므로 0이 아니면 빌드 오류다(실패한 테스트가 있어도).
    assert!(unreported_failure(Tool::Maven, Some(1), 1));
    assert!(!unreported_failure(Tool::Maven, Some(0), 1));
    // 다른 도구는 테스트가 실패해도 0이 아니다.
    assert!(!unreported_failure(Tool::Gradle, Some(1), 1));
    assert!(unreported_failure(Tool::Gradle, Some(1), 0));
    assert!(
        !unreported_failure(Tool::Pytest, None, 0),
        "끊긴 것은 따로 알린다"
    );
}

#[test]
fn egov_pom의_skip_tests를_찾아_지운다() {
    // egov 웹 템플릿(v5.0.7)의 surefire 설정 꼴. 관리 블록의 다른 플러그인은 건드리지 않는다.
    let pom = "<project>\r\n  <build>\r\n    <plugins>\r\n      <plugin>\r\n        <artifactId>maven-failsafe-plugin</artifactId>\r\n        <configuration><skipTests>true</skipTests></configuration>\r\n      </plugin>\r\n      <plugin>\r\n        <groupId>org.apache.maven.plugins</groupId>\r\n        <artifactId>maven-surefire-plugin</artifactId>\r\n        <configuration>\r\n          <skipTests>true</skipTests>\r\n          <reportFormat>xml</reportFormat>\r\n        </configuration>\r\n      </plugin>\r\n    </plugins>\r\n  </build>\r\n</project>\r\n";
    let range = pom_skips_tests(pom).unwrap();
    assert_eq!(&pom[range], "<skipTests>true</skipTests>");
    let enabled = enable_tests(pom).unwrap();
    assert_eq!(
        enabled,
        pom.replace("          <skipTests>true</skipTests>\r\n", "")
    );
    assert_eq!(pom_skips_tests(&enabled), None);
    assert!(enabled.contains("<configuration><skipTests>true</skipTests></configuration>"));
    // 한 줄에 다른 것과 같이 있으면 그것만 지운다.
    let inline = "<project><build><plugins><plugin><artifactId>maven-surefire-plugin</artifactId><configuration><skipTests>true</skipTests></configuration></plugin></plugins></build></project>";
    assert_eq!(
        enable_tests(inline).unwrap(),
        inline.replace("<skipTests>true</skipTests>", "")
    );
    assert_eq!(pom_skips_tests("<project/>"), None);
    assert_eq!(enable_tests("<project/>"), None);
}
