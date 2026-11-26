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
    // 이름이 test로 끝나는 작업과 check. `-x test`는 테스트를 빼는 것이다.
    assert_eq!(Tool::of("gradle integrationTest"), Some(Tool::Gradle));
    assert_eq!(
        Tool::of("./gradlew :app:test --offline"),
        Some(Tool::Gradle)
    );
    assert_eq!(Tool::of("gradle build"), None, "테스트를 거치기만 한다");
    assert_eq!(Tool::of("gradle check"), Some(Tool::Gradle));
    assert_eq!(Tool::of("gradle assemble -x test"), None);
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
    // 모듈별 결과를 찍는 로거는 늘 되살린다(`-q`는 `.mvn/maven.config`·`MAVEN_ARGS`에도 적을 수 있다).
    let events = "-Dorg.slf4j.simpleLogger.log.org.apache.maven.cli.event.ExecutionEventLogger=info \
         -Dmaven.logger.log.org.apache.maven.cling.event.ExecutionEventLogger=info";
    for command in ["mvn test", "mvn -B -q test", "mvn --quiet test"] {
        assert_eq!(
            with_report(Tool::Maven, command, dir),
            format!("{command} -fae -Dmaven.test.failure.ignore=true {events}")
        );
    }
    assert_eq!(
        with_report(
            Tool::Maven,
            "mvn -ff test -Dmaven.test.failure.ignore=false",
            dir
        ),
        format!("mvn -ff test -Dmaven.test.failure.ignore=false {events}"),
        "사용자가 고른 실패 방식은 그대로"
    );
    let again = with_report(Tool::Maven, "mvn -q test", dir);
    assert_eq!(
        with_report(Tool::Maven, &again, dir),
        again,
        "두 번 붙이지 않는다"
    );
    // 작업 줄을 남기는 옵션(`gradle.properties`의 출력 설정을 덮는다)을 끝에 붙인다.
    let lines = "--console=plain -Dorg.gradle.logging.level=lifecycle";
    assert_eq!(
        with_report(Tool::Gradle, "gradle test", dir),
        format!("gradle test --continue {lines}")
    );
    assert_eq!(
        with_report(Tool::Gradle, "gradle test --continue", dir),
        format!("gradle test --continue {lines}")
    );
    // 건너뛴 작업을 알아보는 작업 줄을 숨기는 옵션은 뺀다(값 안의 빈칸과 다른 옵션은 그대로).
    for quiet in [
        "gradle -q test",
        "gradle --quiet test",
        "gradle -w test",
        "gradle --warn test",
        "gradle --console=rich test",
        "gradle --console rich test",
        "gradle -Dorg.gradle.logging.level=quiet test",
    ] {
        assert_eq!(
            with_report(Tool::Gradle, quiet, dir),
            format!("gradle test --continue {lines}"),
            "{quiet}"
        );
    }
    assert_eq!(
        with_report(
            Tool::Gradle,
            "gradle test -q --tests \"a  b\" -Pq=-q --info --continue",
            dir
        ),
        format!("gradle test --tests \"a  b\" -Pq=-q --info --continue {lines}")
    );
    let pytest = with_report(Tool::Pytest, "python -m pytest", dir);
    assert!(
        pytest.starts_with("python -m pytest \"--junitxml=")
            && pytest.ends_with("pytest.xml\" --continue-on-collection-errors"),
        "{pytest}"
    );
    assert_eq!(
        with_report(
            Tool::Pytest,
            "pytest --junitxml=x.xml --continue-on-collection-errors",
            dir
        ),
        "pytest --junitxml=x.xml --continue-on-collection-errors"
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
fn gradle_보고서의_작업을_읽고_실패한_작업마다_다시_돌린다() {
    // `<프로젝트>/build/test-results/<작업>`: 프로젝트 경로는 settings.gradle이 있는 루트에서 센다.
    let root = std::env::temp_dir().join(format!("ferrite-gradle-tasks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("settings.gradle.kts"), "include(\"app\")").unwrap();
    let write = |dir: &Path, suite: &str, names: &[&str]| {
        std::fs::create_dir_all(dir).unwrap();
        let cases: String = names
            .iter()
            .map(|name| {
                format!(
                    "<testcase name=\"{name}()\" classname=\"com.example.board.{suite}\"><failure message=\"틀림\"/></testcase>"
                )
            })
            .collect();
        let file = dir.join(format!("TEST-com.example.board.{suite}.xml"));
        std::fs::write(
            &file,
            format!("<testsuite name=\"com.example.board.{suite}\">{cases}</testsuite>"),
        )
        .unwrap();
        read_report(&file).unwrap()
    };
    let unit = write(
        &root.join("app/build/test-results/test"),
        "CalcTest",
        &["addsWrong"],
    );
    let it = write(
        &root.join("app/build/test-results/integrationTest"),
        "BoardIT",
        &["savesWrong", "listsWrong"],
    );
    let top = write(
        &root.join("build/test-results/test"),
        "RootTest",
        &["checksWrong"],
    );
    assert!(unit.iter().all(|c| c.task.as_deref() == Some(":app:test")));
    assert!(
        it.iter()
            .all(|c| c.task.as_deref() == Some(":app:integrationTest"))
    );
    assert_eq!(top[0].task.as_deref(), Some(":test"));
    // Maven의 보고서는 그 모듈의 pom이 있으면 모듈 이름(`<name>`, 없거나 식이면 `<artifactId>`)을 적는다.
    let maven = write(&root.join("target/surefire-reports"), "MavenTest", &["x"]);
    assert_eq!(maven[0].task, None);
    let module = root.join("web");
    let pom = |name: &str| {
        std::fs::write(
            module.join("pom.xml"),
            format!(
                "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">
  <parent><artifactId>board-parent</artifactId><name>부모</name></parent>
  <artifactId>web</artifactId>
{name}</project>
"
            ),
        )
        .unwrap();
    };
    let read = |dir: &str| {
        write(&module.join("target").join(dir), "WebTest", &["x"])[0]
            .task
            .clone()
    };
    std::fs::create_dir_all(&module).unwrap();
    pom("");
    assert_eq!(read("surefire-reports").as_deref(), Some("web"));
    pom("  <name>게시판 화면</name>
");
    assert_eq!(read("failsafe-reports").as_deref(), Some("게시판 화면"));
    pom("  <name>${project.artifactId}</name>
");
    assert_eq!(read("surefire-reports").as_deref(), Some("web"));
    std::fs::remove_dir_all(&root).unwrap();

    // `check`는 `--tests`를 받지 못한다. 작업 이름과 옛 `--tests`는 빼고 옵션은 둔 채 작업마다 붙인다.
    let both: Vec<&Case> = unit.iter().chain(&it).collect();
    assert_eq!(
        rerun(
            Tool::Gradle,
            "gradlew --offline check -x lint --tests Old --continue",
            &both
        )
        .unwrap(),
        "gradlew --offline -x lint --continue :app:test --tests \"com.example.board.CalcTest.addsWrong\" :app:integrationTest --tests \"com.example.board.BoardIT.savesWrong\" --tests \"com.example.board.BoardIT.listsWrong\""
    );
    let only_it: Vec<&Case> = it.iter().collect();
    assert_eq!(
        rerun(Tool::Gradle, "gradle -p backend check", &only_it).unwrap(),
        "gradle -p backend :app:integrationTest --tests \"com.example.board.BoardIT.savesWrong\" --tests \"com.example.board.BoardIT.listsWrong\""
    );
    // 작업을 모르는 것이 섞이면 예전대로 명령에 붙인다.
    let gradle = parse_junit(GRADLE).unwrap();
    let mixed: Vec<&Case> = vec![&it[0], &gradle[1]];
    assert_eq!(
        rerun(Tool::Gradle, "gradle test", &mixed).unwrap(),
        "gradle test --tests \"com.example.board.BoardIT.savesWrong\" --tests \"com.example.board.CalcTest.addsWrong\""
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
        task: None,
        previous: false,
        label: None,
        suite_label: None,
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
    // `@Nested` 안쪽 클래스의 묶음(`CalcTest$Inner`)도 바깥 클래스의 프레임으로 간다.
    let nested = failed(
        "demo.CalcTest$Inner",
        "org.opentest4j.AssertionFailedError: expected: <5> but was: <3>\n\tat org.junit.jupiter.api.AssertionUtils.fail(AssertionUtils.java:55)\n\tat demo.CalcTest$Inner.inside(CalcTest.java:29)\n",
    );
    let found = frames(&nested);
    assert_eq!(test_frame(&nested, &found), Some(1));
    assert_eq!(found[1].line, 29);
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
fn gradle_설정이_폴더를_적은_프로젝트와_품은_빌드는_그_경로로_작업을_적는다() {
    let root = std::env::temp_dir().join(format!("ferrite-gradle-dirs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("lib")).unwrap();
    // 바깥 빌드(Kotlin DSL): app은 modules/application에 있고 lib를 품는다. 주석으로 막은 줄은 보지 않는다.
    std::fs::write(
        root.join("settings.gradle.kts"),
        "rootProject.name = \"board\"\ninclude(\"app\", \"web\")\nproject(\":app\").projectDir = file(\"modules/application\")\n  // project(\":web\").projectDir = file(\"modules/web\")\n/* includeBuild(\"old\")\n */\nincludeBuild(\"lib\")\n",
    )
    .unwrap();
    // 품은 빌드(Groovy DSL, MS949 주석): core는 폴더 이름 그대로, api는 다른 폴더.
    let mut groovy = b"// \xb8\xf0\xb5\xe2\n".to_vec();
    groovy.extend_from_slice(
        b"rootProject.name = 'libroot'\ninclude 'core', 'api'\nproject('api').projectDir = new File(settingsDir, 'services/../services/api')\n",
    );
    std::fs::write(root.join("lib/settings.gradle"), groovy).unwrap();
    std::fs::create_dir_all(root.join("old")).unwrap();
    std::fs::write(root.join("old/settings.gradle"), "").unwrap();
    let task = |dir: &str| {
        let report = root.join(dir).join("build/test-results/test");
        std::fs::create_dir_all(&report).unwrap();
        let file = report.join("TEST-demo.T.xml");
        std::fs::write(
            &file,
            "<testsuite name=\"demo.T\"><testcase name=\"t\" classname=\"demo.T\"/></testsuite>",
        )
        .unwrap();
        read_report(&file).unwrap()[0]
            .task
            .clone()
            .unwrap_or_default()
    };
    assert_eq!(task("modules/application"), ":app:test");
    assert_eq!(task("web"), ":web:test");
    assert_eq!(
        task("modules/web"),
        ":modules:web:test",
        "주석의 폴더는 보지 않는다"
    );
    assert_eq!(task("lib/core"), ":lib:core:test");
    assert_eq!(task("lib/services/api"), ":lib:api:test");
    assert_eq!(task("lib"), ":lib:test");
    assert_eq!(task("old"), ":test", "주석으로 막은 품은 빌드는 따로 돈다");
    std::fs::remove_dir_all(&root).unwrap();
}

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
    // 영문 Windows(코드 페이지 1252)는 한글을 나타내지 못해 홈 아래의 인자 파일부터 열지 못한다(CI에서 본 출력).
    let cp1252 = "Could not write standard input to Gradle Test Executor 1.\nError: could not open `C:\\Users\\runneradmin\\AppData\\Local\\Temp\\ferrite-test-tools-???? ?-7368\\.tmp\\gradle-worker-classpath10517670469663812345txt'\n";
    assert_eq!(
        gradle_path_problem(Tool::Gradle, work, korean_home, &cases, cp1252),
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

#[test]
fn gradle이_건너뛴_작업의_보고서만_고른다() {
    let root = std::env::temp_dir().join(format!("ferrite-gradle-skipped-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let report = |project: &str, task: &str| {
        let dir = root.join(project).join("build/test-results").join(task);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("TEST-demo.CalcTest.xml");
        std::fs::write(&file, "<testsuite/>").unwrap();
        file
    };
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("settings.gradle"), "include 'app', 'lib'\n").unwrap();
    let app = report("app", "test");
    let app_it = report("app", "integrationTest");
    let lib = report("lib", "test");
    let top = report("", "test");
    let output = "> Task :app:compileTestJava UP-TO-DATE\r\n> Task :app:test UP-TO-DATE\r\n> Task :app:integrationTest\r\n> Task :lib:test FAILED\r\n> Task :test FROM-CACHE\r\n";
    let mut found = gradle_skipped_reports(&root, output);
    found.sort();
    let mut expected = vec![app, top];
    expected.sort();
    assert_eq!(found, expected);
    assert!(!found.contains(&app_it) && !found.contains(&lib));
    assert!(gradle_skipped_reports(&root, "BUILD SUCCESSFUL").is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn gradle의_실패만_다시가_테스트를_찾지_못한_것을_알아본다() {
    let output = "> No tests found for given includes: [*AllTests](filter.includeTestsMatching)";
    assert!(gradle_rerun_found_nothing(Tool::Gradle, output));
    assert!(!gradle_rerun_found_nothing(Tool::Maven, output));
    assert!(!gradle_rerun_found_nothing(
        Tool::Gradle,
        "2 tests completed, 2 failed"
    ));
}

#[test]
fn 실패만_다시가_다시_돌리지_않은_지난_실패는_지운_것으로_보고_뺀다() {
    let case = |name: &str, outcome, task: Option<&str>| Case {
        suite: "demo.CalcTest".to_owned(),
        name: name.to_owned(),
        outcome,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration: false,
        task: task.map(str::to_owned),
        previous: false,
        label: None,
        suite_label: None,
    };
    let names = |cases: &[Case]| {
        cases
            .iter()
            .map(|c| (c.name.clone(), c.outcome))
            .collect::<Vec<_>>()
    };
    let pass = |name: &str| (name.to_owned(), Outcome::Passed);
    let fail = |name: &str| (name.to_owned(), Outcome::Failed);
    // 실패 둘 가운데 `gone`은 지웠고 `still`만 다시 돌았다.
    let before = |task| {
        vec![
            case("ok", Outcome::Passed, task),
            case("gone", Outcome::Failed, task),
            case("still", Outcome::Failed, task),
        ]
    };
    let apply_out = |tool, code, output: &str, task, found: Vec<Case>| {
        let mut cases = before(task);
        apply_rerun(tool, code, output, &mut cases, found);
        names(&cases)
    };
    let apply = |tool, code, task, found| apply_out(tool, code, "", task, found);
    let still = |task| vec![case("still", Outcome::Failed, task)];
    for (tool, code) in [
        (Tool::Maven, Some(0)),
        (Tool::Pytest, Some(1)),
        (Tool::Ctest, Some(8)),
        (Tool::Go, Some(1)),
    ] {
        assert_eq!(
            apply(tool, code, None, still(None)),
            [pass("ok"), fail("still")],
            "{tool:?}"
        );
    }
    let task = Some(":app:test");
    assert_eq!(
        apply(Tool::Gradle, Some(1), task, still(task)),
        [pass("ok"), fail("still")]
    );
    // 빌드 오류면 돌지 못한 것이라 남긴다: Maven이 0이 아니게 끝났다, Gradle의 그 작업이 결과를 내지 않았다,
    // Go의 그 패키지가 빌드에 실패했다.
    let kept = [pass("ok"), fail("gone"), fail("still")];
    assert_eq!(apply(Tool::Maven, Some(1), None, still(None)), kept);
    // Maven 다중 모듈: 0이 아니게 끝나도 모듈별 결과에서 그 모듈이 성공했으면 돌았던 것이라 뺀다. 그 모듈이
    // 깨졌거나 건너뛰었거나, 결과가 없거나(`-q`), 모듈을 모르면 남긴다.
    let summary = |common: &str| {
        format!(
            "[ERROR] COMPILATION ERROR :
[INFO] ------------------------------------------------------------------------
[INFO] Reactor Summary for board-parent 1.0:
[INFO] 
[INFO] board-parent ....................................... SUCCESS [  0.004 s]
[INFO] common ............................................. {common} [  1.234 s]
[INFO] web ................................................ SKIPPED
[INFO] batch .............................................. FAILURE [  0.512 s]
[INFO] ------------------------------------------------------------------------
[INFO] BUILD FAILURE
"
        )
    };
    let common = Some("common");
    let ok = summary("SUCCESS");
    assert_eq!(
        apply_out(Tool::Maven, Some(1), &ok, common, still(common)),
        [pass("ok"), fail("still")]
    );
    for broken in ["FAILURE", "SKIPPED"] {
        assert_eq!(
            apply_out(
                Tool::Maven,
                Some(1),
                &summary(broken),
                common,
                still(common)
            ),
            kept
        );
    }
    for module in [Some("web"), Some("batch"), Some("comm"), None] {
        assert_eq!(
            apply_out(Tool::Maven, Some(1), &ok, module, still(module)),
            kept,
            "{module:?}"
        );
    }
    // 색이 든 출력과 판이 붙은 이름(모듈들의 판이 다를 때)도 읽는다.
    let colored = ok
        .replace(" SUCCESS [  1.234", " [1;32mSUCCESS[m [  1.234")
        .replace("common ...", "common 2.0 ...");
    assert_eq!(
        apply_out(Tool::Maven, Some(1), &colored, common, still(common)),
        [pass("ok"), fail("still")]
    );
    assert_eq!(
        apply(Tool::Gradle, Some(1), task, still(Some(":lib:test"))),
        [pass("ok"), fail("gone"), fail("still"), fail("still")]
    );
    let mut built = still(None);
    built.push(case(PACKAGE_CASE, Outcome::Failed, None));
    assert_eq!(
        apply(Tool::Go, Some(1), None, built),
        [pass("ok"), fail("gone"), fail("still"), fail(PACKAGE_CASE)]
    );
    // 실패한 것은 돌지 않고 다른 것만 돌았다(pytest `--lf`가 모두 돌림, CTest가 같은 번호의 다른 테스트를 돌림).
    assert_eq!(
        apply(
            Tool::Ctest,
            Some(0),
            None,
            vec![case("fixed", Outcome::Passed, None)]
        ),
        [pass("ok"), pass("fixed")]
    );
    // 남긴 지난 실패는 앞 결과로 적는다(Maven 모듈이 깨짐). 다시 돈 결과는 앞 결과가 아니다.
    let mut cases = before(common);
    apply_rerun(
        Tool::Maven,
        Some(1),
        &summary("FAILURE"),
        &mut cases,
        still(common),
    );
    let previous: Vec<(&str, bool)> = cases
        .iter()
        .map(|c| (c.name.as_str(), c.previous))
        .collect();
    assert_eq!(previous, [("ok", false), ("gone", true), ("still", false)]);
    // 다시 돈 실패가 이제 통과하면 덮어 쓴다.
    assert_eq!(
        apply(
            Tool::Pytest,
            Some(0),
            None,
            vec![
                case("gone", Outcome::Passed, None),
                case("still", Outcome::Passed, None)
            ]
        ),
        [pass("ok"), pass("gone"), pass("still")]
    );
}

#[test]
fn 같은_테스트가_둘이면_실패한_것_하나로_모은다() {
    let case = |suite: &str, name: &str, outcome, integration| Case {
        suite: suite.to_owned(),
        name: name.to_owned(),
        outcome,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration,
        task: None,
        previous: false,
        label: None,
        suite_label: None,
    };
    let found = dedup(vec![
        case("CalcTest", "adds", Outcome::Passed, false),
        case("CalcTest", "addsWrong", Outcome::Passed, false),
        case("CalcTest", "adds", Outcome::Passed, false),
        case("CalcTest", "addsWrong", Outcome::Failed, false),
        // 돌린 곳(Failsafe)이 다르면 다른 테스트다.
        case("CalcTest", "adds", Outcome::Failed, true),
    ]);
    assert_eq!(
        found
            .iter()
            .map(|c| (c.name.as_str(), c.outcome, c.integration))
            .collect::<Vec<_>>(),
        [
            ("adds", Outcome::Passed, false),
            ("addsWrong", Outcome::Failed, false),
            ("adds", Outcome::Failed, true),
        ]
    );
}

#[test]
fn 실패만_다시에서_넘어간_전체_실행은_결과를_내지_못한_gradle_작업의_앞_결과를_남긴다() {
    let case = |name: &str, outcome, task: Option<&str>| Case {
        suite: "demo.CalcTest".to_owned(),
        name: name.to_owned(),
        outcome,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration: false,
        task: task.map(str::to_owned),
        previous: false,
        label: None,
        suite_label: None,
    };
    fn names(cases: &[Case]) -> Vec<(&str, Outcome)> {
        cases.iter().map(|c| (c.name.as_str(), c.outcome)).collect()
    }
    let (app, lib) = (Some(":app:test"), Some(":lib:test"));
    let before = || {
        vec![
            case("adds", Outcome::Passed, app),
            case("addsWrong", Outcome::Failed, app),
            case("subtractsWrong", Outcome::Failed, lib),
            case("loose", Outcome::Failed, None),
        ]
    };
    // app만 돌았다(lib은 컴파일이 깨졌다): app의 결과는 바꾸고 lib의 것은 남긴다. 작업을 모르는 것은 뺀다.
    let found = || {
        vec![
            case("adds", Outcome::Passed, app),
            case("addsFixed", Outcome::Passed, app),
        ]
    };
    let mut cases = before();
    apply_whole_rerun(Tool::Gradle, &mut cases, found());
    assert_eq!(
        names(&cases),
        [
            ("subtractsWrong", Outcome::Failed),
            ("adds", Outcome::Passed),
            ("addsFixed", Outcome::Passed),
        ]
    );
    // 남긴 것만 앞 결과다.
    assert_eq!(
        cases.iter().map(|c| c.previous).collect::<Vec<_>>(),
        [true, false, false]
    );
    // 다른 도구는 모두 바꾼다.
    let mut cases = before();
    apply_whole_rerun(Tool::Maven, &mut cases, found());
    assert_eq!(
        names(&cases),
        [("adds", Outcome::Passed), ("addsFixed", Outcome::Passed)]
    );
}

#[test]
fn pytest가_모으지_못한_파일의_지난_실패는_앞_결과로_남긴다() {
    let case = |suite: &str, name: &str, outcome| Case {
        suite: suite.to_owned(),
        name: name.to_owned(),
        outcome,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration: false,
        task: None,
        previous: false,
        label: None,
        suite_label: None,
    };
    let mut cases = vec![
        case("tests.test_a", "test_add", Outcome::Passed),
        case("tests.test_a", "test_wrong", Outcome::Failed),
        case("tests.test_b", "test_b_wrong", Outcome::Failed),
        case("tests.test_b.TestJob", "test_counts", Outcome::Failed),
        case("tests.test_bb", "test_other", Outcome::Failed),
    ];
    // test_b를 모으지 못했다. test_a의 지난 실패는 돌았는데 없으니 지운 것이다. 이름이 겹치는 test_bb는 다른 파일이다.
    let found = vec![
        case("", "tests.test_b", Outcome::Failed),
        case("tests.test_a", "test_add", Outcome::Passed),
    ];
    apply_rerun(Tool::Pytest, Some(1), "", &mut cases, found);
    let seen: Vec<(&str, bool)> = cases
        .iter()
        .map(|c| (c.name.as_str(), c.previous))
        .collect();
    assert_eq!(
        seen,
        [
            ("test_add", false),
            ("test_b_wrong", true),
            ("test_counts", true),
            ("tests.test_b", false),
        ]
    );
}

#[test]
fn 고른_테스트만_돌리는_명령을_만든다() {
    let case = |suite: &str, name: &str| Case {
        suite: suite.to_owned(),
        name: name.to_owned(),
        outcome: Outcome::Passed,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration: false,
        task: None,
        previous: false,
        label: None,
        suite_label: None,
    };
    let dir = std::env::temp_dir().join(format!("ferrite-run-only-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(dir.join("tests/test_a.py"), "").unwrap();
    let add = case("tests.test_a", "test_add");
    let param = case("tests.test_a.TestCalc", "test_sum[1-2]");
    // pytest: 파일을 찾아 노드 ID로, 클래스와 매개변수도 그대로.
    assert_eq!(
        run_only(Tool::Pytest, "pytest -q", &[&add, &param, &add], &dir).unwrap(),
        "pytest -q \"tests/test_a.py::test_add\" \"tests/test_a.py::TestCalc::test_sum[1-2]\""
    );
    // 파일을 못 찾으면 이름으로 거른다(매개변수는 뺀다).
    let lost = case("pkg.test_b", "test_b[x]");
    assert_eq!(
        run_only(Tool::Pytest, "pytest", &[&add, &lost], &dir).unwrap(),
        "pytest -k \"test_add or test_b\""
    );
    // CTest: 이름을 정규식으로(특수 글자는 이스케이프).
    let (ok, plus) = (case("ok", "ok"), case("a+b", "a+b"));
    assert_eq!(
        run_only(Tool::Ctest, "ctest --test-dir build", &[&ok, &plus], &dir).unwrap(),
        "ctest --test-dir build -R \"^(ok|a\\+b)$\""
    );
    // Maven은 실패만 다시와 같은 거르기다.
    let calc = case("demo.CalcTest", "adds");
    assert_eq!(
        run_only(Tool::Maven, "mvn test", &[&calc], &dir),
        rerun(Tool::Maven, "mvn test", &[&calc])
    );
    assert_eq!(run_only(Tool::Maven, "mvn test", &[], &dir), None);
    // 이름이 빈 것은 묶음 전체다.
    let whole = case("demo.CalcTest", "");
    assert!(
        run_only(Tool::Maven, "mvn test", &[&whole, &calc], &dir)
            .unwrap()
            .contains("-Dtest=CalcTest ")
    );
    assert_eq!(
        run_only(Tool::Gradle, "gradle test", &[&whole], &dir).unwrap(),
        "gradle test --tests \"demo.CalcTest\""
    );
    let module = case("tests.test_a", "");
    let class = case("tests.test_a.TestCalc", "");
    assert_eq!(
        run_only(Tool::Pytest, "pytest", &[&module, &class], &dir).unwrap(),
        "pytest \"tests/test_a.py\" \"tests/test_a.py::TestCalc\""
    );
    assert_eq!(
        run_only(Tool::Pytest, "pytest", &[&case("pkg.test_b", "")], &dir).unwrap(),
        "pytest -k \"test_b\""
    );
    assert_eq!(
        run_only(
            Tool::Go,
            "go test ./...",
            &[&case("example.com/board/calc", "")],
            &dir
        )
        .unwrap(),
        "go test -json example.com/board/calc"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn 고른_테스트만_돌린_결과는_고른_것만_지운_것으로_본다() {
    let case = |name: &str, outcome| Case {
        suite: "demo.CalcTest".to_owned(),
        name: name.to_owned(),
        outcome,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration: false,
        task: None,
        previous: false,
        label: None,
        suite_label: None,
    };
    let mut cases = vec![
        case("adds", Outcome::Passed),
        case("gone", Outcome::Passed),
        case("wrong", Outcome::Failed),
        case("other", Outcome::Failed),
    ];
    // adds·gone을 골라 돌렸다: gone은 결과에 없어 지운 것이다. 고르지 않은 실패(wrong·other)는 그대로 둔다.
    let targets = vec![case("adds", Outcome::Passed), case("gone", Outcome::Passed)];
    apply_selected(
        Tool::Pytest,
        Some(0),
        "",
        &mut cases,
        vec![case("adds", Outcome::Failed)],
        &targets,
    );
    let seen: Vec<(&str, Outcome)> = cases.iter().map(|c| (c.name.as_str(), c.outcome)).collect();
    assert_eq!(
        seen,
        [
            ("adds", Outcome::Failed),
            ("wrong", Outcome::Failed),
            ("other", Outcome::Failed)
        ]
    );
    // 클래스 전체(이름이 빈 것)를 돌리면 그 클래스의 결과에 없는 것은 모두 지운 것이다. 돌린 곳을 모르는 고름은
    // 돌린 곳(Maven 모듈)이 적힌 결과에도 든다.
    let mut cases = vec![
        Case {
            task: Some("common".to_owned()),
            ..case("adds", Outcome::Passed)
        },
        case("wrong", Outcome::Failed),
        Case {
            suite: "demo.OtherTest".to_owned(),
            ..case("other", Outcome::Failed)
        },
    ];
    let whole = vec![case("", Outcome::Passed)];
    let found = vec![Case {
        task: Some("common".to_owned()),
        ..case("adds", Outcome::Passed)
    }];
    apply_selected(Tool::Maven, Some(0), "", &mut cases, found, &whole);
    let seen: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(seen, ["adds", "other"]);
}

#[test]
fn 테스트_소스_파일의_gradle_테스트_작업() {
    let root = std::env::temp_dir().join(format!("ferrite-gradle-task-at-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let file = |path: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        path
    };
    file("settings.gradle.kts");
    std::fs::write(root.join("settings.gradle.kts"), "include(\"app\")\n").unwrap();
    file("build.gradle.kts");
    file("app/build.gradle.kts");
    assert_eq!(
        gradle_test_task(&file("app/src/test/java/demo/CalcTest.java")).as_deref(),
        Some(":app:test")
    );
    assert_eq!(
        gradle_test_task(&file("app/src/integrationTest/kotlin/demo/SaveIT.kt")).as_deref(),
        Some(":app:integrationTest")
    );
    assert_eq!(
        gradle_test_task(&file("src/test/java/RootTest.java")).as_deref(),
        Some(":test")
    );
    // `src` 밖이나 빌드 파일 없는 곳은 모른다.
    assert_eq!(gradle_test_task(&file("app/scripts/Tool.java")), None);
    assert_eq!(
        gradle_test_task(&std::env::temp_dir().join("Loose.java")),
        None
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn 한글_windows의_maven이_cp949로_낸_모듈별_결과도_읽는다() {
    // 한글 Windows의 Maven(JDK 8·21 모두)이 `<name>게시판 공통</name>` 모듈을 이 바이트로 냈다(2026-11-23).
    // 테스트 창은 줄마다 UTF-8이 아니면 CP949로 푼다(`text_file::decode`).
    let lines: [&[u8]; 3] = [
        b"[INFO] Reactor Summary for board-parent 1.0:",
        b"[INFO] board-parent ....................................... SUCCESS [  0.003 s]",
        b"[INFO] \xb0\xd4\xbd\xc3\xc6\xc7 \xb0\xf8\xc5\xeb ............................................. SUCCESS [  0.000 s]",
    ];
    let output: String = lines
        .iter()
        .map(|line| crate::text_file::decode(line).unwrap().text + "\n")
        .collect();
    assert_eq!(
        maven_built_modules(&output).unwrap(),
        ["board-parent", "게시판 공통"]
    );
}

#[test]
fn 커서의_jvm_테스트를_디버거를_기다리게_띄운다() {
    let case = Case {
        suite: "com.example.CalcTest".into(),
        name: "adds".into(),
        outcome: Outcome::Passed,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration: false,
        task: Some(":app:test".into()),
        previous: false,
        label: None,
        suite_label: None,
    };
    let dir = Path::new("/p");
    let maven = run_only(Tool::Maven, "mvn -B test", &[&case], dir).unwrap();
    assert_eq!(
        with_jvm_debug(Tool::Maven, &maven).unwrap(),
        format!("{maven} -Dmaven.surefire.debug")
    );
    // `--debug-jvm`은 앞 작업(`:app:test`)에 붙는다.
    let gradle = run_only(Tool::Gradle, "gradle test", &[&case], dir).unwrap();
    assert_eq!(
        with_jvm_debug(Tool::Gradle, &gradle).unwrap(),
        "gradle :app:test --tests \"com.example.CalcTest.adds\" --debug-jvm --rerun-tasks"
    );
    let integration = Case {
        integration: true,
        ..case.clone()
    };
    let failsafe = run_only(Tool::Maven, "mvn -B verify", &[&integration], dir).unwrap();
    assert_eq!(
        with_jvm_debug(Tool::Maven, &failsafe).unwrap(),
        format!("{failsafe} -Dmaven.failsafe.debug")
    );
    assert_eq!(with_jvm_debug(Tool::Pytest, "pytest"), None);
}

#[test]
fn junit5_nested_안쪽_클래스만_거른다() {
    let case = Case {
        suite: "demo.CalcTest$Inner".into(),
        name: "adds()".into(),
        outcome: Outcome::Failed,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration: false,
        task: Some(":app:test".into()),
        previous: false,
        label: None,
        suite_label: None,
    };
    // `$`를 떼면 바깥 클래스의 같은 이름 테스트가 돈다. sh가 `$Inner`를 풀지 않게 `*`로 적는다.
    assert_eq!(
        rerun(Tool::Maven, "mvn test", &[&case]).unwrap(),
        "mvn test -Dtest=CalcTest*Inner#adds -Dsurefire.failIfNoSpecifiedTests=false"
    );
    assert_eq!(
        rerun(Tool::Gradle, "gradle test", &[&case]).unwrap(),
        "gradle :app:test --tests \"demo.CalcTest*Inner.adds\""
    );
}

#[test]
fn kotlin_백틱_이름은_인자째_감싼다() {
    let case = |name: &str, integration: bool| Case {
        suite: "demo.CalcTest".into(),
        name: name.into(),
        outcome: Outcome::Failed,
        seconds: None,
        message: None,
        detail: String::new(),
        file: None,
        line: None,
        integration,
        task: Some(":test".into()),
        previous: false,
        label: None,
        suite_label: None,
    };
    // Surefire는 `,`를 클래스 사이로 읽으므로 한 글자 `?`로 적는다(`*`는 `adds_two_numbers`까지 고른다).
    let unit = case("adds two numbers, again", false);
    assert_eq!(
        rerun(Tool::Maven, "mvn test", &[&unit]).unwrap(),
        "mvn test \"-Dtest=CalcTest#adds two numbers? again\" -Dsurefire.failIfNoSpecifiedTests=false"
    );
    let it = case("reads a b", true);
    let line = rerun(Tool::Maven, "mvn verify", &[&it]).unwrap();
    assert_eq!(
        line,
        "mvn verify -Dtest=NoUnitTestsToRerun -Dsurefire.failIfNoSpecifiedTests=false \"-Dit.test=CalcTest#reads a b\" -Dit.failIfNoSpecifiedTests=false"
    );
    assert_eq!(
        with_jvm_debug(Tool::Maven, &line).unwrap(),
        format!("{line} -Dmaven.failsafe.debug")
    );
    // Gradle은 큰따옴표 안의 이름을 그대로 고른다(보고서는 `()`를 붙인다).
    assert_eq!(
        rerun(
            Tool::Gradle,
            "gradle test",
            &[&case("adds two numbers, again()", false)]
        )
        .unwrap(),
        "gradle :test --tests \"demo.CalcTest.adds two numbers, again\""
    );
}

/// Windows·JDK 21에서 `@DisplayName`을 붙인 예제를 실제로 돌린 보고서(Gradle 9.2.0·Surefire 3.5.4)를 줄였다.
/// Gradle은 테스트 이름에, Surefire는 클래스 이름에 표시 이름을 적는다.
#[test]
fn 표시_이름은_소스에서_진짜_이름으로_되돌려_다시_돌린다() {
    let root = std::env::temp_dir().join(format!("ferrite-display-name-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let source = root.join("src/test/java/demo");
    let gradle = root.join("build/test-results/test");
    let maven = root.join("target/surefire-reports");
    for dir in [&source, &gradle, &maven] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(root.join("settings.gradle"), "rootProject.name = 'calc'\n").unwrap();
    std::fs::write(
        source.join("CalcTest.java"),
        "package demo;\n\n@DisplayName(\"계산기\")\nclass CalcTest {\n    @Test\n    @DisplayName(\"두 수를 더한다\")\n    void adds() {}\n\n    @Test\n    void plain() {}\n\n    @ParameterizedTest\n    @DisplayName(\"다섯\")\n    @ValueSource(ints = {5})\n    void bare(int n) {}\n\n    @Nested\n    @DisplayName(\"안쪽\")\n    class Inner {\n        @Test\n        @DisplayName(\"안에서 더한다\")\n        void inside() {}\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        gradle.join("TEST-demo.CalcTest.xml"),
        r#"<testsuite name="계산기" tests="3"><testcase name="두 수를 더한다" classname="demo.CalcTest"/><testcase name="[1] 5" classname="demo.CalcTest"/><testcase name="plain()" classname="demo.CalcTest"/></testsuite>"#,
    )
    .unwrap();
    std::fs::write(
        gradle.join("TEST-demo.CalcTest$Inner.xml"),
        r#"<testsuite name="안쪽" tests="1"><testcase name="안에서 더한다" classname="demo.CalcTest$Inner"/></testsuite>"#,
    )
    .unwrap();
    std::fs::write(
        maven.join("TEST-demo.CalcTest.xml"),
        r#"<testsuite name="demo.CalcTest" tests="4"><testcase name="adds" classname="계산기"/><testcase name="bare(int)[1]" classname="계산기"/><testcase name="plain" classname="계산기"/><testcase name="inside" classname="안쪽"/></testsuite>"#,
    )
    .unwrap();
    let ids = |cases: &[Case]| -> Vec<(String, String)> {
        cases
            .iter()
            .map(|c| (c.suite.clone(), c.name.clone()))
            .collect()
    };
    let id = |s: &str, n: &str| (s.to_owned(), n.to_owned());
    // 결과 트리에 보일 (묶음, 테스트) 이름. 도구와 상관없이 소스의 표시 이름이다.
    let labels = |cases: &[Case]| -> Vec<(Option<String>, Option<String>)> {
        cases
            .iter()
            .map(|c| (c.suite_label.clone(), c.label.clone()))
            .collect()
    };
    let label = |s: &str, n: Option<&str>| (Some(s.to_owned()), n.map(str::to_owned));

    let mut found = read_report(&gradle.join("TEST-demo.CalcTest.xml")).unwrap();
    found.extend(read_report(&gradle.join("TEST-demo.CalcTest$Inner.xml")).unwrap());
    assert_eq!(
        ids(&found),
        [
            id("demo.CalcTest", "adds()"),
            id("demo.CalcTest", "[1] 5"),
            id("demo.CalcTest", "plain()"),
            id("demo.CalcTest$Inner", "inside()"),
        ]
    );
    assert_eq!(
        labels(&found),
        [
            label("계산기", Some("두 수를 더한다")),
            label("계산기", None),
            label("계산기", None),
            label("안쪽", Some("안에서 더한다")),
        ],
        "Gradle의 매개변수 실행(`[1] 5`)은 메서드를 몰라 표시 이름이 없다"
    );
    // 메서드를 모르는 매개변수 실행은 클래스째 돈다.
    let all: Vec<&Case> = found.iter().collect();
    assert_eq!(
        rerun(Tool::Gradle, "gradle test", &all).unwrap(),
        "gradle :test --tests \"demo.CalcTest.adds\" --tests \"demo.CalcTest\" --tests \"demo.CalcTest.plain\" --tests \"demo.CalcTest*Inner.inside\""
    );

    let found = read_report(&maven.join("TEST-demo.CalcTest.xml")).unwrap();
    assert_eq!(
        ids(&found),
        [
            id("demo.CalcTest", "adds"),
            id("demo.CalcTest", "bare(int)[1]"),
            id("demo.CalcTest", "plain"),
            id("demo.CalcTest$Inner", "inside"),
        ]
    );
    assert_eq!(
        labels(&found),
        [
            label("계산기", Some("두 수를 더한다")),
            label("계산기", Some("다섯 [1]")),
            label("계산기", None),
            label("안쪽", Some("안에서 더한다")),
        ]
    );
    let all: Vec<&Case> = found.iter().collect();
    assert_eq!(
        rerun(Tool::Maven, "mvn test", &all).unwrap(),
        "mvn test -Dtest=CalcTest#adds+bare+plain,CalcTest*Inner#inside -Dsurefire.failIfNoSpecifiedTests=false"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
