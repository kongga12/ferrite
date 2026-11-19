//! 실제 테스트 도구(Maven·Gradle·pytest·Go·CTest)를 돌려 보고서를 읽고, 실패 스택의 줄을 프로젝트에서
//! 찾고, 실패한 것만 다시 돌린다. 정적 분석기(SpotBugs·PMD·Semgrep·clang)가 낸 SARIF와 빌드 도구가 푼 의존성
//! 나무도 읽는다. 도구가 필요하고 Maven·Gradle은 의존성을 받으므로 기본으로는 돌지 않는다. CI의 3개 OS
//! 전체 작업이 `--ignored`로 돌린다. 도구가 없으면 로컬에서는 건너뛰고 CI에서는 실패한다.

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
    run_env(dir, line, &[])
}

fn run_env(dir: &Path, line: &str, env: &[(&str, &str)]) -> (bool, String) {
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
    let output = command
        .current_dir(dir)
        .envs(env.iter().copied())
        .output()
        .unwrap();
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
        .flat_map(|f| test_report::read_report(f).unwrap())
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
    assert!(
        ok,
        "테스트 실패로는 빌드를 멈추지 않는다(결과는 보고서로 가린다)"
    );
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
#[ignore = "Maven으로 다중 모듈의 JUnit 5 테스트를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_다중_모듈의_테스트를_모두_돌리고_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    // common이 실패해도 그것에 기대는 web과 상관없는 batch의 테스트까지 돈다.
    let root = temp_dir("maven-modules");
    let pom = |body: &str| {
        format!(
            "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n{body}</project>\n"
        )
    };
    std::fs::write(
        root.join("pom.xml"),
        pom(r#"  <groupId>com.example</groupId>
  <artifactId>board-parent</artifactId>
  <version>1.0</version>
  <packaging>pom</packaging>
  <modules>
    <module>common</module>
    <module>web</module>
    <module>batch</module>
  </modules>
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
"#),
    )
    .unwrap();
    let module = |name: &str, deps: &str, class: &str, tests: &str| {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("pom.xml"),
            pom(&format!("  <parent>\n    <groupId>com.example</groupId>\n    <artifactId>board-parent</artifactId>\n    <version>1.0</version>\n  </parent>\n  <artifactId>{name}</artifactId>\n  <dependencies>{deps}</dependencies>\n")),
        )
        .unwrap();
        let java = root.join(name).join("src/test/java/com/example").join(name);
        std::fs::create_dir_all(&java).unwrap();
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package com.example.{name};\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass {class} {{\n{tests}}}\n"),
        )
        .unwrap();
        java.join(format!("{class}.java"))
    };
    let text = module(
        "common",
        "",
        "TextTest",
        "    @Test\n    void trims() {\n        assertEquals(\"a\", \" a \".trim());\n    }\n\n    @Test\n    void joinsWrong() {\n        assertEquals(\"가나\", \"가\" + \"다\", \"잇기\");\n    }\n",
    );
    module(
        "web",
        "<dependency><groupId>com.example</groupId><artifactId>common</artifactId><version>1.0</version></dependency>",
        "PageTest",
        "    @Test\n    void pages() {\n        assertEquals(2, 11 / 10 + 1);\n    }\n",
    );
    let job = module(
        "batch",
        "",
        "JobTest",
        "    @Test\n    void runs() {\n        assertEquals(1, 1);\n    }\n\n    @Test\n    void countsWrong() {\n        assertEquals(3, 1 + 1, \"세기\");\n    }\n",
    );
    let report = root.join("reports");
    let command = "mvn -B -q test";
    let since = SystemTime::now();
    let (ok, _) = run(
        &root,
        &test_report::with_report(Tool::Maven, command, &report),
    );
    assert!(ok, "테스트 실패로는 빌드를 멈추지 않는다");
    let cases = junit(Tool::Maven, &root, &report, since);
    assert_eq!(
        outcomes(&cases),
        [
            ("countsWrong".to_owned(), Outcome::Failed),
            ("joinsWrong".to_owned(), Outcome::Failed),
            ("pages".to_owned(), Outcome::Passed),
            ("runs".to_owned(), Outcome::Passed),
            ("trims".to_owned(), Outcome::Passed),
        ]
    );
    let line = |name: &str| test_line(&root, &root, cases.iter().find(|c| c.name == name).unwrap());
    assert_eq!(line("joinsWrong"), (text, 15));
    assert_eq!(line("countsWrong"), (job, 15));

    // 보고서는 파일 시각이 `since`보다 2초 앞까지 이번 것으로 친다(거친 파일 시스템). 다시 돌리면 web은
    // 걸러 낸 테스트가 없어 보고서를 새로 쓰지 않으므로, 바로 돌리면 첫 실행의 것이 섞인다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let since = SystemTime::now();
    let rerun = test_report::rerun(Tool::Maven, command, &failed(&cases)).unwrap();
    run(
        &root,
        &test_report::with_report(Tool::Maven, &rerun, &report),
    );
    let again = junit(Tool::Maven, &root, &report, since);
    assert_eq!(
        outcomes(&again),
        [
            ("countsWrong".to_owned(), Outcome::Failed),
            ("joinsWrong".to_owned(), Outcome::Failed),
        ],
        "두 모듈의 실패만 다시"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Maven으로 JUnit 5 단위·통합 테스트(Surefire·Failsafe)를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_통합_테스트를_surefire와_failsafe가_둘_다_돌리면_둘로_읽고_실패만_다시도_둘_다_돈다() {
    if !require("mvn -v") {
        return;
    }
    // Surefire의 `<includes>`에 통합 테스트(`*IT`)를 넣고 Failsafe도 기본대로 둔다: 통합 테스트가 두 번 돈다.
    let root = temp_dir("maven-it-both");
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
        <configuration>
          <includes>
            <include>**/*Test.java</include>
            <include>**/*IT.java</include>
          </includes>
        </configuration>
      </plugin>
      <plugin>
        <artifactId>maven-failsafe-plugin</artifactId>
        <version>3.5.4</version>
        <executions>
          <execution>
            <goals>
              <goal>integration-test</goal>
              <goal>verify</goal>
            </goals>
          </execution>
        </executions>
      </plugin>
    </plugins>
  </build>
</project>
"#,
    )
    .unwrap();
    for (class, ok, wrong) in [
        ("CalcTest", "adds", "addsWrong"),
        ("BoardIT", "lists", "savesWrong"),
    ] {
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass {class} {{\n    @Test\n    void {ok}() {{\n        assertEquals(3, 1 + 2);\n    }}\n\n    @Test\n    void {wrong}() {{\n        assertEquals(4, 1 + 2);\n    }}\n}}\n"),
        )
        .unwrap();
    }
    let report = root.join("reports");
    let command = "mvn -B -q verify";
    let kinds = |cases: &[Case]| {
        let mut kinds: Vec<(String, Outcome, bool)> = cases
            .iter()
            .map(|c| (c.name.clone(), c.outcome, c.integration))
            .collect();
        kinds.sort_by(|a, b| (&a.0, a.2).cmp(&(&b.0, b.2)));
        kinds
    };
    let kind = |name: &str, outcome, integration| (name.to_owned(), outcome, integration);
    let since = SystemTime::now();
    let (ok, output) = run(
        &root,
        &test_report::with_report(Tool::Maven, command, &report),
    );
    assert!(ok, "{output}");
    let cases = junit(Tool::Maven, &root, &report, since);
    // 통합 테스트는 두 플러그인의 보고서에 하나씩 남아 둘로 읽는다(단위 테스트 단계에서 돈 것과 통합 테스트 단계에서 돈 것).
    assert_eq!(
        kinds(&cases),
        [
            kind("adds", Outcome::Passed, false),
            kind("addsWrong", Outcome::Failed, false),
            kind("lists", Outcome::Passed, false),
            kind("lists", Outcome::Passed, true),
            kind("savesWrong", Outcome::Failed, false),
            kind("savesWrong", Outcome::Failed, true),
        ]
    );
    // 실패만 다시: 실패한 넷(통합 테스트는 두 번)을 다시 돌리면 두 플러그인이 각자 그것만 돌린다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let since = SystemTime::now();
    let line = test_report::rerun(Tool::Maven, command, &failed(&cases)).unwrap();
    let (ok, output) = run(
        &root,
        &test_report::with_report(Tool::Maven, &line, &report),
    );
    assert!(ok, "{line}\n{output}");
    assert_eq!(
        kinds(&junit(Tool::Maven, &root, &report, since)),
        [
            kind("addsWrong", Outcome::Failed, false),
            kind("savesWrong", Outcome::Failed, false),
            kind("savesWrong", Outcome::Failed, true),
        ],
        "{line}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Maven으로 TestNG 단위·통합 테스트(Surefire·Failsafe)를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_testng_테스트도_한_번씩_읽고_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    // Surefire·Failsafe가 TestNG를 돌리면 JUnit 꼴 보고서(`TEST-*.xml`) 말고도 TestNG의 보고서
    // (`testng-results.xml`, `junitreports/TEST-*.xml` 등)를 같은 폴더에 남긴다.
    let root = temp_dir("maven-testng");
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
      <groupId>org.testng</groupId>
      <artifactId>testng</artifactId>
      <version>7.10.2</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.4</version>
      </plugin>
      <plugin>
        <artifactId>maven-failsafe-plugin</artifactId>
        <version>3.5.4</version>
        <executions>
          <execution>
            <goals>
              <goal>integration-test</goal>
              <goal>verify</goal>
            </goals>
          </execution>
        </executions>
      </plugin>
    </plugins>
  </build>
</project>
"#,
    )
    .unwrap();
    for (class, ok, wrong) in [
        ("CalcTest", "adds", "addsWrong"),
        ("BoardIT", "lists", "savesWrong"),
    ] {
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package com.example.board;\n\nimport static org.testng.Assert.assertEquals;\n\nimport org.testng.annotations.Test;\n\npublic class {class} {{\n    @Test\n    public void {ok}() {{\n        assertEquals(1 + 2, 3);\n    }}\n\n    @Test\n    public void {wrong}() {{\n        assertEquals(1 + 2, 4);\n    }}\n}}\n"),
        )
        .unwrap();
    }
    let report = root.join("reports");
    let command = "mvn -B -q verify";
    let kinds = |cases: &[Case]| {
        let mut kinds: Vec<(String, Outcome, bool)> = cases
            .iter()
            .map(|c| (c.name.clone(), c.outcome, c.integration))
            .collect();
        kinds.sort_by(|a, b| a.0.cmp(&b.0));
        kinds
    };
    let kind = |name: &str, outcome, integration| (name.to_owned(), outcome, integration);
    let since = SystemTime::now();
    let (ok, output) = run(
        &root,
        &test_report::with_report(Tool::Maven, command, &report),
    );
    assert!(ok, "{output}");
    let cases = junit(Tool::Maven, &root, &report, since);
    // 테스트마다 한 번씩만 읽는다(TestNG의 보고서로 겹치지 않는다).
    assert_eq!(
        kinds(&cases),
        [
            kind("adds", Outcome::Passed, false),
            kind("addsWrong", Outcome::Failed, false),
            kind("lists", Outcome::Passed, true),
            kind("savesWrong", Outcome::Failed, true),
        ]
    );
    // TestNG의 실패 스택에서도 테스트 줄(`assertEquals`, 15줄)로 간다.
    let saves = cases.iter().find(|c| c.name == "savesWrong").unwrap();
    assert_eq!(
        test_line(&root, &root, saves),
        (java.join("BoardIT.java"), 15)
    );
    // 실패만 다시: TestNG도 `-Dtest`·`-Dit.test`의 `클래스#메서드`로 거른다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let since = SystemTime::now();
    let line = test_report::rerun(Tool::Maven, command, &failed(&cases)).unwrap();
    let (ok, output) = run(
        &root,
        &test_report::with_report(Tool::Maven, &line, &report),
    );
    assert!(ok, "{line}\n{output}");
    assert_eq!(
        kinds(&junit(Tool::Maven, &root, &report, since)),
        [
            kind("addsWrong", Outcome::Failed, false),
            kind("savesWrong", Outcome::Failed, true),
        ],
        "{line}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// TestNG 매개변수 테스트(`@DataProvider`, 두 줄 중 둘째가 틀림)와 그냥 실패하는 테스트. 테스트 줄은 16·21줄.
fn write_testng_data_provider(java: &Path) {
    std::fs::create_dir_all(java).unwrap();
    std::fs::write(
        java.join("CalcTest.java"),
        "package com.example.board;\n\nimport static org.testng.Assert.assertEquals;\n\nimport org.testng.annotations.DataProvider;\nimport org.testng.annotations.Test;\n\npublic class CalcTest {\n    @DataProvider\n    public Object[][] sums() {\n        return new Object[][] {{1, 2, 3}, {2, 2, 5}};\n    }\n\n    @Test(dataProvider = \"sums\")\n    public void adds(int a, int b, int sum) {\n        assertEquals(a + b, sum);\n    }\n\n    @Test\n    public void subtractsWrong() {\n        assertEquals(3 - 1, 1);\n    }\n}\n",
    )
    .unwrap();
}

/// TestNG 매개변수 테스트를 돌려 읽고, 줄을 찾고, 실패만 다시 돌린다. 도구마다 매개변수 줄의 이름 꼴만 다르다.
fn check_testng_data_provider(
    tool: Tool,
    root: &Path,
    report: &Path,
    command: &str,
    rows: [&str; 2],
) {
    let java = root.join(match tool {
        Tool::Gradle => "app/src/test/java/com/example/board",
        _ => "src/test/java/com/example/board",
    });
    write_testng_data_provider(&java);
    let run_tool = |line: &str| {
        let since = SystemTime::now();
        run(root, &test_report::with_report(tool, line, report));
        outcomes(&test_report::dedup(junit(tool, root, report, since)))
    };
    let all = |first| {
        vec![
            (rows[0].to_owned(), first),
            (rows[1].to_owned(), Outcome::Failed),
            ("subtractsWrong".to_owned(), Outcome::Failed),
        ]
    };
    // 매개변수 줄마다 이름이 달라 `dedup`이 하나로 모으지 않는다.
    let since = SystemTime::now();
    run(root, &test_report::with_report(tool, command, report));
    let cases = junit(tool, root, report, since);
    assert_eq!(
        outcomes(&test_report::dedup(cases.clone())),
        all(Outcome::Passed)
    );
    let row = cases.iter().find(|c| c.name == rows[1]).unwrap();
    assert_eq!(test_line(root, root, row), (java.join("CalcTest.java"), 16));
    // 실패만 다시: 매개변수 줄 하나만 고를 수는 없어 그 메서드(`adds`)의 줄을 모두 돈다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let line = test_report::rerun(tool, command, &failed(&cases)).unwrap();
    assert_eq!(run_tool(&line), all(Outcome::Passed), "{line}");
}

#[test]
#[ignore = "Maven으로 TestNG 매개변수 테스트를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_testng_매개변수_테스트를_줄마다_읽고_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    let root = temp_dir("maven-testng-data-provider");
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
      <groupId>org.testng</groupId>
      <artifactId>testng</artifactId>
      <version>7.10.2</version>
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
    // Surefire는 값을 먼저, 줄 번호(1부터)를 뒤에 적는다.
    check_testng_data_provider(
        Tool::Maven,
        &root,
        &root.join("reports"),
        "mvn -B -q test",
        ["adds[1, 2, 3](1)", "adds[2, 2, 5](2)"],
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// TestNG 묶음 파일(`src/test/resources/testng.xml`)이 고르는 두 클래스를 쓰고 돌려 읽고, 줄을 찾고, 실패만
/// 다시 돌린다. `root`는 명령을 돌리는 폴더, `project`는 테스트가 든 (하위) 프로젝트 폴더.
fn check_testng_xml(tool: Tool, root: &Path, project: &Path, report: &Path, command: &str) {
    let java = project.join("src/test/java/com/example/board");
    std::fs::create_dir_all(&java).unwrap();
    std::fs::create_dir_all(project.join("src/test/resources")).unwrap();
    std::fs::write(
        project.join("src/test/resources/testng.xml"),
        "<!DOCTYPE suite SYSTEM \"https://testng.org/testng-1.0.dtd\">\n<suite name=\"board\">\n  <test name=\"all\">\n    <classes>\n      <class name=\"com.example.board.CalcTest\"/>\n      <class name=\"com.example.board.BoardTest\"/>\n    </classes>\n  </test>\n</suite>\n",
    )
    .unwrap();
    for (class, ok, wrong) in [
        ("CalcTest", "adds", "addsWrong"),
        ("BoardTest", "lists", "savesWrong"),
    ] {
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package com.example.board;\n\nimport static org.testng.Assert.assertEquals;\n\nimport org.testng.annotations.Test;\n\npublic class {class} {{\n    @Test\n    public void {ok}() {{\n        assertEquals(1 + 2, 3);\n    }}\n\n    @Test\n    public void {wrong}() {{\n        assertEquals(1 + 2, 4);\n    }}\n}}\n"),
        )
        .unwrap();
    }
    let run_tool = |line: &str| {
        let since = SystemTime::now();
        run(root, &test_report::with_report(tool, line, report));
        test_report::dedup(junit(tool, root, report, since))
    };
    let cases = run_tool(command);
    assert_eq!(
        suite_tests(&cases),
        [
            suite_test("BoardTest", "lists", Outcome::Passed),
            suite_test("BoardTest", "savesWrong", Outcome::Failed),
            suite_test("CalcTest", "adds", Outcome::Passed),
            suite_test("CalcTest", "addsWrong", Outcome::Failed),
        ]
    );
    let saves = cases.iter().find(|c| c.name == "savesWrong").unwrap();
    assert_eq!(
        test_line(root, root, saves),
        (java.join("BoardTest.java"), 15)
    );
    // 실패만 다시: 거르기(Maven `-Dtest`, Gradle `--tests`)가 묶음 파일보다 앞서 실패한 메서드만 돈다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let line = test_report::rerun(tool, command, &failed(&cases)).unwrap();
    assert_eq!(
        suite_tests(&run_tool(&line)),
        [
            suite_test("BoardTest", "savesWrong", Outcome::Failed),
            suite_test("CalcTest", "addsWrong", Outcome::Failed),
        ],
        "{line}"
    );
}

#[test]
#[ignore = "Maven으로 TestNG 묶음 파일(testng.xml)의 테스트를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_testng_xml로_돌린_테스트를_읽고_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    // Surefire는 묶음 파일로 돌면 보고서 하나(`TEST-TestSuite.xml`)에 테스트마다 실제 클래스를 적는다. TestNG가
    // 따로 남기는 보고서(`junitreports/`, 묶음 이름 폴더)는 읽지 않는다.
    let root = temp_dir("maven-testng-xml");
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
      <groupId>org.testng</groupId>
      <artifactId>testng</artifactId>
      <version>7.10.2</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.4</version>
        <configuration>
          <suiteXmlFiles>
            <suiteXmlFile>src/test/resources/testng.xml</suiteXmlFile>
          </suiteXmlFiles>
        </configuration>
      </plugin>
    </plugins>
  </build>
</project>
"#,
    )
    .unwrap();
    check_testng_xml(
        Tool::Maven,
        &root,
        &root,
        &root.join("reports"),
        "mvn -B -q test",
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 두 번째 메서드가 실패하는 `CalcTest`(`name`). `ok`면 통과하게 쓴다.
fn write_calc_test(java: &Path, name: &str, ok: bool) {
    std::fs::create_dir_all(java).unwrap();
    let expected = if ok { 3 } else { 4 };
    std::fs::write(
        java.join("CalcTest.java"),
        format!("package demo;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass CalcTest {{\n    @Test\n    void adds() {{\n        assertEquals(3, 1 + 2);\n    }}\n\n    @Test\n    void {name}() {{\n        assertEquals({expected}, 1 + 2);\n    }}\n}}\n"),
    )
    .unwrap();
}

#[test]
#[ignore = "Maven으로 JUnit 5 테스트를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_실패한_테스트의_이름을_바꾸면_실패만_다시는_아무것도_돌지_않는다() {
    if !require("mvn -v") {
        return;
    }
    let root = temp_dir("maven-renamed");
    std::fs::write(
        root.join("pom.xml"),
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>demo</groupId>
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
    let java = root.join("src/test/java/demo");
    write_calc_test(&java, "addsWrong", false);
    let report = root.join("reports");
    let command = "mvn -B -q test";
    let since = SystemTime::now();
    run(
        &root,
        &test_report::with_report(Tool::Maven, command, &report),
    );
    let cases = junit(Tool::Maven, &root, &report, since);
    // 실패한 메서드의 이름을 바꿔 고쳤다.
    write_calc_test(&java, "addsFixed", true);
    std::thread::sleep(std::time::Duration::from_secs(3));
    let since = SystemTime::now();
    let line = test_report::rerun(Tool::Maven, command, &failed(&cases)).unwrap();
    let (ok, output) = run(
        &root,
        &test_report::with_report(Tool::Maven, &line, &report),
    );
    // 아무것도 돌지 않고 0으로 끝나 보고서가 없다. 테스트 창은 전체를 다시 돌린다.
    assert!(ok, "{line}\n{output}");
    assert!(test_report::report_files(Tool::Maven, &root, &report, since).is_empty());
    let since = SystemTime::now();
    run(
        &root,
        &test_report::with_report(Tool::Maven, command, &report),
    );
    assert_eq!(
        outcomes(&junit(Tool::Maven, &root, &report, since)),
        [
            ("adds".to_owned(), Outcome::Passed),
            ("addsFixed".to_owned(), Outcome::Passed),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Maven으로 JUnit 4 Suite로 묶은 테스트를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_junit4_suite로_묶어_돌려도_실제_클래스로_읽고_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    // 오래된 프로젝트에 흔한 JUnit 4 묶음: `AllTests`가 `@RunWith(Suite.class)`로 클래스들을 묶고, pom은 그 묶음만
    // 돌린다(`<includes>`). 보고서 파일은 묶음 이름(`TEST-…AllTests.xml`)이지만 테스트마다 실제 클래스를 적는다.
    let root = temp_dir("maven-junit4-suite");
    let java = root.join("src/test/java/com/example/board");
    std::fs::create_dir_all(&java).unwrap();
    let pom = |configuration: &str| {
        format!(
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
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>4.13.2</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.4</version>{configuration}
      </plugin>
    </plugins>
  </build>
</project>
"#
        )
    };
    std::fs::write(
        root.join("pom.xml"),
        pom("
        <configuration>
          <includes>
            <include>**/AllTests.java</include>
          </includes>
        </configuration>"),
    )
    .unwrap();
    std::fs::write(
        java.join("AllTests.java"),
        "package com.example.board;\n\nimport org.junit.runner.RunWith;\nimport org.junit.runners.Suite;\n\n@RunWith(Suite.class)\n@Suite.SuiteClasses({CalcTest.class, BoardTest.class})\npublic class AllTests {\n}\n",
    )
    .unwrap();
    for (class, ok, wrong) in [
        ("CalcTest", "adds", "addsWrong"),
        ("BoardTest", "lists", "savesWrong"),
    ] {
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package com.example.board;\n\nimport static org.junit.Assert.assertEquals;\n\nimport org.junit.Test;\n\npublic class {class} {{\n    @Test\n    public void {ok}() {{\n        assertEquals(3, 1 + 2);\n    }}\n\n    @Test\n    public void {wrong}() {{\n        assertEquals(4, 1 + 2);\n    }}\n}}\n"),
        )
        .unwrap();
    }
    let report = root.join("reports");
    let command = "mvn -B -q test";
    let tests = |cases: &[Case]| {
        let mut tests: Vec<(String, String, Outcome)> = cases
            .iter()
            .map(|c| (c.suite.clone(), c.name.clone(), c.outcome))
            .collect();
        tests.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        tests
    };
    let test = |class: &str, name: &str, outcome| {
        (
            format!("com.example.board.{class}"),
            name.to_owned(),
            outcome,
        )
    };
    let all = [
        test("BoardTest", "lists", Outcome::Passed),
        test("BoardTest", "savesWrong", Outcome::Failed),
        test("CalcTest", "adds", Outcome::Passed),
        test("CalcTest", "addsWrong", Outcome::Failed),
    ];
    // 실패가 있어 mvn은 실패로 끝난다.
    let run_maven = |line: &str| {
        let since = SystemTime::now();
        run(&root, &test_report::with_report(Tool::Maven, line, &report));
        junit(Tool::Maven, &root, &report, since)
    };
    let cases = run_maven(command);
    // 묶음 이름이 아니라 실제 클래스로 읽는다.
    assert_eq!(tests(&cases), all);
    let saves = cases.iter().find(|c| c.name == "savesWrong").unwrap();
    assert_eq!(
        test_line(&root, &root, saves),
        (java.join("BoardTest.java"), 15)
    );
    // 실패만 다시: `-Dtest`가 includes를 덮어 묶음 없이 실패한 클래스의 메서드만 돈다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let line = test_report::rerun(Tool::Maven, command, &failed(&cases)).unwrap();
    assert_eq!(
        tests(&run_maven(&line)),
        [
            test("BoardTest", "savesWrong", Outcome::Failed),
            test("CalcTest", "addsWrong", Outcome::Failed),
        ],
        "{line}"
    );
    // includes를 두지 않으면 묶음(`AllTests`)도 기본 includes(`*Tests`)에 들어, 테스트마다 묶음과 제 클래스로 두 번
    // 돌아 두 보고서에 남는다. 테스트 창은 하나로 모은다.
    std::fs::write(root.join("pom.xml"), pom("")).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(3));
    let twice = run_maven(command);
    assert_eq!(twice.len(), 8);
    assert_eq!(tests(&test_report::dedup(twice)), all);
    let _ = std::fs::remove_dir_all(&root);
}

/// JUnit 5 묶음(`@Suite`, JUnit Platform Suite)과 그것이 고르는 두 클래스. 테스트 줄은 15줄.
fn write_junit5_suite(java: &Path) {
    std::fs::create_dir_all(java).unwrap();
    std::fs::write(
        java.join("AllTests.java"),
        "package com.example.board;\n\nimport org.junit.platform.suite.api.SelectClasses;\nimport org.junit.platform.suite.api.Suite;\n\n@Suite\n@SelectClasses({CalcTest.class, BoardTest.class})\nclass AllTests {\n}\n",
    )
    .unwrap();
    for (class, ok, wrong) in [
        ("CalcTest", "adds", "addsWrong"),
        ("BoardTest", "lists", "savesWrong"),
    ] {
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass {class} {{\n    @Test\n    void {ok}() {{\n        assertEquals(3, 1 + 2);\n    }}\n\n    @Test\n    void {wrong}() {{\n        assertEquals(4, 1 + 2);\n    }}\n}}\n"),
        )
        .unwrap();
    }
}

/// (클래스, 이름, 결과)를 이름 순으로.
fn suite_tests(cases: &[Case]) -> Vec<(String, String, Outcome)> {
    let mut tests: Vec<(String, String, Outcome)> = cases
        .iter()
        .map(|c| {
            let class = c.suite.rsplit('.').next().unwrap_or(&c.suite);
            (class.to_owned(), c.name.clone(), c.outcome)
        })
        .collect();
    tests.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    tests
}

fn suite_test(class: &str, name: &str, outcome: Outcome) -> (String, String, Outcome) {
    (class.to_owned(), name.to_owned(), outcome)
}

#[test]
#[ignore = "Maven으로 JUnit 5 Suite로 묶은 테스트를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_junit5_suite가_고른_클래스와_두_번_돈_테스트를_하나로_읽고_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    // `@Suite`인 `AllTests`도 Surefire 기본 includes(`*Tests`)에 들어, 테스트마다 묶음(suite 엔진)과 제
    // 클래스(jupiter 엔진)로 두 번 돈다. 묶음의 보고서(`TEST-…AllTests.xml`)도 테스트마다 실제 클래스를 적는다.
    let root = temp_dir("maven-junit5-suite");
    let java = root.join("src/test/java/com/example/board");
    write_junit5_suite(&java);
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
    <dependency>
      <groupId>org.junit.platform</groupId>
      <artifactId>junit-platform-suite</artifactId>
      <version>1.12.2</version>
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
    let report = root.join("reports");
    let command = "mvn -B -q test";
    let run_maven = |line: &str| {
        let since = SystemTime::now();
        run(&root, &test_report::with_report(Tool::Maven, line, &report));
        junit(Tool::Maven, &root, &report, since)
    };
    let twice = run_maven(command);
    assert_eq!(twice.len(), 8, "묶음과 제 클래스로 두 번");
    let cases = test_report::dedup(twice);
    assert_eq!(
        suite_tests(&cases),
        [
            suite_test("BoardTest", "lists", Outcome::Passed),
            suite_test("BoardTest", "savesWrong", Outcome::Failed),
            suite_test("CalcTest", "adds", Outcome::Passed),
            suite_test("CalcTest", "addsWrong", Outcome::Failed),
        ]
    );
    let saves = cases.iter().find(|c| c.name == "savesWrong").unwrap();
    assert_eq!(
        test_line(&root, &root, saves),
        (java.join("BoardTest.java"), 15)
    );
    // 실패만 다시: `-Dtest=CalcTest#addsWrong,…`에 묶음 이름이 없어 묶음은 돌지 않는다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let line = test_report::rerun(Tool::Maven, command, &failed(&cases)).unwrap();
    assert_eq!(
        suite_tests(&run_maven(&line)),
        [
            suite_test("BoardTest", "savesWrong", Outcome::Failed),
            suite_test("CalcTest", "addsWrong", Outcome::Failed),
        ],
        "{line}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Maven으로 JUnit 4 단위·통합 테스트(Surefire·Failsafe)를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_junit4_분류로_가른_통합_테스트도_보고서_폴더로_가려_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    // egov에 흔한 JUnit 4: 통합 테스트는 이름이 아니라 분류(`@Category`)로 표시하고, Surefire는 그 분류를 빼고
    // (`<excludedGroups>`) Failsafe는 그 분류만 돌린다(`<groups>`, 이름은 모두).
    let root = temp_dir("maven-junit4-groups");
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
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>4.13.2</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.4</version>
        <configuration>
          <excludedGroups>com.example.board.IntegrationTests</excludedGroups>
        </configuration>
      </plugin>
      <plugin>
        <artifactId>maven-failsafe-plugin</artifactId>
        <version>3.5.4</version>
        <configuration>
          <groups>com.example.board.IntegrationTests</groups>
          <includes>
            <include>**/*Test.java</include>
          </includes>
        </configuration>
        <executions>
          <execution>
            <goals>
              <goal>integration-test</goal>
              <goal>verify</goal>
            </goals>
          </execution>
        </executions>
      </plugin>
    </plugins>
  </build>
</project>
"#,
    )
    .unwrap();
    std::fs::write(
        java.join("IntegrationTests.java"),
        "package com.example.board;\n\n/** 통합 테스트 분류. */\npublic interface IntegrationTests {\n}\n",
    )
    .unwrap();
    for (class, category, ok, wrong) in [
        ("CalcTest", "", "adds", "addsWrong"),
        (
            "BoardTest",
            "@org.junit.experimental.categories.Category(IntegrationTests.class)\n",
            "lists",
            "savesWrong",
        ),
    ] {
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package com.example.board;\n\nimport static org.junit.Assert.assertEquals;\n\nimport org.junit.Test;\n\n{category}public class {class} {{\n    @Test\n    public void {ok}() {{\n        assertEquals(3, 1 + 2);\n    }}\n\n    @Test\n    public void {wrong}() {{\n        assertEquals(4, 1 + 2);\n    }}\n}}\n"),
        )
        .unwrap();
    }
    let report = root.join("reports");
    let command = "mvn -B -q verify";
    let kinds = |cases: &[Case]| {
        let mut kinds: Vec<(String, Outcome, bool)> = cases
            .iter()
            .map(|c| (c.name.clone(), c.outcome, c.integration))
            .collect();
        kinds.sort_by(|a, b| a.0.cmp(&b.0));
        kinds
    };
    let kind = |name: &str, outcome, integration| (name.to_owned(), outcome, integration);
    let since = SystemTime::now();
    let (ok, output) = run(
        &root,
        &test_report::with_report(Tool::Maven, command, &report),
    );
    assert!(ok, "{output}");
    let cases = junit(Tool::Maven, &root, &report, since);
    // 분류로 가른 통합 테스트(`BoardTest`, 이름은 단위 테스트 꼴)도 Failsafe의 보고서에 남아 통합 테스트로 읽는다.
    assert_eq!(
        kinds(&cases),
        [
            kind("adds", Outcome::Passed, false),
            kind("addsWrong", Outcome::Failed, false),
            kind("lists", Outcome::Passed, true),
            kind("savesWrong", Outcome::Failed, true),
        ]
    );
    // JUnit 4의 실패 스택에서도 테스트 줄(`assertEquals`, 16줄)로 간다.
    let saves = cases.iter().find(|c| c.name == "savesWrong").unwrap();
    assert_eq!(
        test_line(&root, &root, saves),
        (java.join("BoardTest.java"), 16)
    );
    // 실패만 다시: `-Dtest`·`-Dit.test`로 걸러도 분류(`groups`·`excludedGroups`)는 그대로 먹어, 각 플러그인이 제
    // 분류에서 실패한 것만 돈다.
    let rerun_failed = |failed: &[&Case]| {
        // 보고서는 파일 시각이 `since`보다 2초 앞까지 이번 것으로 친다(거친 파일 시스템).
        std::thread::sleep(std::time::Duration::from_secs(3));
        let since = SystemTime::now();
        let line = test_report::rerun(Tool::Maven, command, failed).unwrap();
        let (ok, output) = run(
            &root,
            &test_report::with_report(Tool::Maven, &line, &report),
        );
        assert!(ok, "{line}\n{output}");
        kinds(&junit(Tool::Maven, &root, &report, since))
    };
    assert_eq!(
        rerun_failed(&failed(&cases)),
        [
            kind("addsWrong", Outcome::Failed, false),
            kind("savesWrong", Outcome::Failed, true),
        ]
    );
    let only_it: Vec<&Case> = failed(&cases)
        .into_iter()
        .filter(|c| c.integration)
        .collect();
    assert_eq!(
        rerun_failed(&only_it),
        [kind("savesWrong", Outcome::Failed, true)]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Maven으로 JUnit 5 단위·통합 테스트(Surefire·Failsafe)를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_includes를_바꾼_pom도_보고서_폴더로_가려_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    // (이름, 플러그인 설정, 통합 테스트 클래스, 그 클래스를 Failsafe가 돌리는지)
    let layouts = [
        // 통합 테스트도 Surefire의 `<includes>`에 넣어 단위 테스트 단계에서 돌린다(Failsafe 없음).
        (
            "maven-includes-surefire",
            r#"      <plugin>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.4</version>
        <configuration>
          <includes>
            <include>**/*Test.java</include>
            <include>**/*IT.java</include>
          </includes>
        </configuration>
      </plugin>"#,
            "BoardIT",
            false,
        ),
        // Failsafe가 `*IntegrationTest`를 맡고, Surefire는 그것을 뺀다(기본 `*Test`에 걸리므로).
        (
            "maven-includes-failsafe",
            r#"      <plugin>
        <artifactId>maven-surefire-plugin</artifactId>
        <version>3.5.4</version>
        <configuration>
          <excludes>
            <exclude>**/*IntegrationTest.java</exclude>
          </excludes>
        </configuration>
      </plugin>
      <plugin>
        <artifactId>maven-failsafe-plugin</artifactId>
        <version>3.5.4</version>
        <configuration>
          <includes>
            <include>**/*IntegrationTest.java</include>
          </includes>
        </configuration>
        <executions>
          <execution>
            <goals>
              <goal>integration-test</goal>
              <goal>verify</goal>
            </goals>
          </execution>
        </executions>
      </plugin>"#,
            "BoardIntegrationTest",
            true,
        ),
    ];
    for (name, plugins, board, integration) in layouts {
        let root = temp_dir(name);
        let java = root.join("src/test/java/com/example/board");
        std::fs::create_dir_all(&java).unwrap();
        std::fs::write(
            root.join("pom.xml"),
            format!(
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
{plugins}
    </plugins>
  </build>
</project>
"#
            ),
        )
        .unwrap();
        for (class, ok, wrong) in [
            ("CalcTest", "adds", "addsWrong"),
            (board, "lists", "savesWrong"),
        ] {
            std::fs::write(
                java.join(format!("{class}.java")),
                format!("package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass {class} {{\n    @Test\n    void {ok}() {{\n        assertEquals(3, 1 + 2);\n    }}\n\n    @Test\n    void {wrong}() {{\n        assertEquals(4, 1 + 2);\n    }}\n}}\n"),
            )
            .unwrap();
        }
        let report = root.join("reports");
        let command = "mvn -B -q verify";
        let since = SystemTime::now();
        let (ok, output) = run(
            &root,
            &test_report::with_report(Tool::Maven, command, &report),
        );
        assert!(ok, "{name}: {output}");
        let kinds = |cases: &[Case]| {
            let mut kinds: Vec<(String, Outcome, bool)> = cases
                .iter()
                .map(|c| (c.name.clone(), c.outcome, c.integration))
                .collect();
            kinds.sort_by(|a, b| a.0.cmp(&b.0));
            kinds
        };
        let kind = |name: &str, outcome, integration| (name.to_owned(), outcome, integration);
        let cases = junit(Tool::Maven, &root, &report, since);
        // 단위·통합은 어느 플러그인이 돌렸는지(보고서 폴더)로 가린다. 클래스 이름과는 상관없다.
        assert_eq!(
            kinds(&cases),
            [
                kind("adds", Outcome::Passed, false),
                kind("addsWrong", Outcome::Failed, false),
                kind("lists", Outcome::Passed, integration),
                kind("savesWrong", Outcome::Failed, integration),
            ],
            "{name}"
        );
        // 실패만 다시: 그 플러그인으로 실패한 것만 돈다(`-Dtest`·`-Dit.test`는 pom의 includes·excludes보다 앞선다).
        let rerun_failed = |failed: &[&Case]| {
            // 보고서는 파일 시각이 `since`보다 2초 앞까지 이번 것으로 친다(거친 파일 시스템).
            std::thread::sleep(std::time::Duration::from_secs(3));
            let since = SystemTime::now();
            let line = test_report::rerun(Tool::Maven, command, failed).unwrap();
            let (ok, output) = run(
                &root,
                &test_report::with_report(Tool::Maven, &line, &report),
            );
            assert!(ok, "{name}: {line}\n{output}");
            kinds(&junit(Tool::Maven, &root, &report, since))
        };
        assert_eq!(
            rerun_failed(&failed(&cases)),
            [
                kind("addsWrong", Outcome::Failed, false),
                kind("savesWrong", Outcome::Failed, integration),
            ],
            "{name}"
        );
        let only_board: Vec<&Case> = failed(&cases)
            .into_iter()
            .filter(|c| c.name == "savesWrong")
            .collect();
        assert_eq!(
            rerun_failed(&only_board),
            [kind("savesWrong", Outcome::Failed, integration)],
            "{name}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
#[ignore = "Maven으로 JUnit 5 단위·통합 테스트(Surefire·Failsafe)를 돌린다(의존성을 받는다). Java 17 이상과 mvn이 필요하다"]
fn maven_failsafe_통합_테스트도_돌리고_실패만_다시() {
    if !require("mvn -v") {
        return;
    }
    let root = temp_dir("maven-failsafe");
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
      <plugin>
        <artifactId>maven-failsafe-plugin</artifactId>
        <version>3.5.4</version>
        <executions>
          <execution>
            <goals>
              <goal>integration-test</goal>
              <goal>verify</goal>
            </goals>
          </execution>
        </executions>
      </plugin>
    </plugins>
  </build>
</project>
"#,
    )
    .unwrap();
    let test = |class: &str, tests: &str| {
        let path = java.join(format!("{class}.java"));
        std::fs::write(
            &path,
            format!("package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass {class} {{\n{tests}}}\n"),
        )
        .unwrap();
        path
    };
    test(
        "CalcTest",
        "    @Test\n    void adds() {\n        assertEquals(3, 1 + 2);\n    }\n\n    @Test\n    void addsWrong() {\n        assertEquals(4, 1 + 2, \"더하기\");\n    }\n",
    );
    let board = test(
        "BoardIT",
        "    @Test\n    void lists() {\n        assertEquals(2, 1 + 1);\n    }\n\n    @Test\n    void savesWrong() {\n        assertEquals(\"저장\", \"삭제\", \"게시글 저장\");\n    }\n",
    );
    let report = root.join("reports");
    // 통합 테스트 단계까지 간다. 통합 테스트가 실패해도 Failsafe의 verify가 빌드를 멈추지 않는다.
    let command = "mvn -B -q verify";
    let since = SystemTime::now();
    let (ok, _) = run(
        &root,
        &test_report::with_report(Tool::Maven, command, &report),
    );
    assert!(ok, "통합 테스트 실패로도 빌드를 멈추지 않는다");
    let cases = junit(Tool::Maven, &root, &report, since);
    let kinds: Vec<(&str, Outcome, bool)> = {
        let mut kinds: Vec<_> = cases
            .iter()
            .map(|c| (c.name.as_str(), c.outcome, c.integration))
            .collect();
        kinds.sort_by(|a, b| a.0.cmp(b.0));
        kinds
    };
    assert_eq!(
        kinds,
        [
            ("adds", Outcome::Passed, false),
            ("addsWrong", Outcome::Failed, false),
            ("lists", Outcome::Passed, true),
            ("savesWrong", Outcome::Failed, true),
        ]
    );
    let saves = cases.iter().find(|c| c.name == "savesWrong").unwrap();
    assert_eq!(test_line(&root, &root, saves), (board, 15));

    // 실패만 다시: 단위 테스트는 Surefire로, 통합 테스트는 Failsafe로 하나씩만 돈다(통합 테스트를 단위 테스트
    // 단계에서 돌리지 않고, 통과한 통합 테스트는 다시 돌지 않는다).
    let rerun_failed = |failed: &[&Case]| {
        // 보고서는 파일 시각이 `since`보다 2초 앞까지 이번 것으로 친다(거친 파일 시스템).
        std::thread::sleep(std::time::Duration::from_secs(3));
        let since = SystemTime::now();
        let line = test_report::rerun(Tool::Maven, command, failed).unwrap();
        let (ok, _) = run(
            &root,
            &test_report::with_report(Tool::Maven, &line, &report),
        );
        assert!(ok, "{line}");
        let again = junit(Tool::Maven, &root, &report, since);
        let mut kinds: Vec<(String, bool)> = again
            .iter()
            .map(|c| (c.name.clone(), c.integration))
            .collect();
        kinds.sort();
        assert!(again.iter().all(|c| c.outcome == Outcome::Failed));
        kinds
    };
    assert_eq!(
        rerun_failed(&failed(&cases)),
        [
            ("addsWrong".to_owned(), false),
            ("savesWrong".to_owned(), true)
        ]
    );
    // 통합 테스트만 다시: 단위 테스트는 돌지 않는다.
    let only_it: Vec<&Case> = failed(&cases)
        .into_iter()
        .filter(|c| c.integration)
        .collect();
    assert_eq!(rerun_failed(&only_it), [("savesWrong".to_owned(), true)]);
    // 단위 테스트만 다시: 통합 테스트는 돌지 않는다.
    let only_unit: Vec<&Case> = failed(&cases)
        .into_iter()
        .filter(|c| !c.integration)
        .collect();
    assert_eq!(rerun_failed(&only_unit), [("addsWrong".to_owned(), false)]);
    let _ = std::fs::remove_dir_all(&root);
}

/// 시험용 Gradle 프로젝트 폴더. `gradle init`이 만드는 꼴로 루트에는 설정만 두고, 하위 프로젝트마다
/// Kotlin DSL 빌드 파일을 둔다. Gradle·Kotlin 컴파일러 데몬은 띄우지 않는다(로컬에서 시험 뒤에 남지 않게).
fn gradle_project(name: &str, projects: &[(&str, &str)]) -> PathBuf {
    let root = temp_dir(name);
    let names: Vec<String> = projects.iter().map(|(p, _)| format!("\"{p}\"")).collect();
    std::fs::write(
        root.join("settings.gradle.kts"),
        format!(
            "rootProject.name = \"board\"\ninclude({})\n",
            names.join(", ")
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("gradle.properties"),
        "org.gradle.daemon=false\nkotlin.compiler.execution.strategy=in-process\n",
    )
    .unwrap();
    for (project, build) in projects {
        std::fs::create_dir_all(root.join(project)).unwrap();
        std::fs::write(root.join(project).join("build.gradle.kts"), build).unwrap();
    }
    root
}

#[test]
#[ignore = "Gradle로 JUnit 5 테스트를 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_test_보고서와_실패만_다시() {
    if !require("gradle --version") {
        return;
    }
    let root = gradle_project(
        "gradle",
        &[(
            "app",
            "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.11.4\")\n    testRuntimeOnly(\"org.junit.platform:junit-platform-launcher\")\n}\n\ntasks.test {\n    useJUnitPlatform()\n}\n",
        )],
    );
    let java = root.join("app/src/test/java/com/example/board");
    std::fs::create_dir_all(&java).unwrap();
    std::fs::write(
        java.join("CalcTest.java"),
        "package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Disabled;\nimport org.junit.jupiter.api.Test;\n\nclass CalcTest {\n    @Test\n    void adds() {\n        assertEquals(3, 1 + 2);\n    }\n\n    @Test\n    void addsWrong() {\n        assertEquals(4, 1 + 2, \"더하기\");\n    }\n\n    @Test\n    void throwsError() {\n        throw new IllegalStateException(\"게시판 오류\");\n    }\n\n    @Disabled(\"나중에\")\n    @Test\n    void skipped() {\n    }\n}\n",
    )
    .unwrap();
    // 실행 구성이 만드는 명령과 같다. 루트에서 하위 프로젝트의 테스트를 다 돌리고, 보고서는
    // `app/build/test-results`에 남는다.
    let command = "gradle test";
    assert!(
        ferrite_core::run::detect(&root)
            .iter()
            .any(|c| c.command == command)
    );
    assert_eq!(Tool::of(command), Some(Tool::Gradle));
    let since = SystemTime::now();
    let (ok, _) = run(
        &root,
        &test_report::with_report(Tool::Gradle, command, &root),
    );
    assert!(!ok, "실패한 테스트가 있으면 gradle이 실패한다");
    let cases = junit(Tool::Gradle, &root, &root, since);
    assert_eq!(
        outcomes(&cases),
        [
            ("adds()".to_owned(), Outcome::Passed),
            ("addsWrong()".to_owned(), Outcome::Failed),
            ("skipped()".to_owned(), Outcome::Skipped),
            ("throwsError()".to_owned(), Outcome::Failed),
        ]
    );
    let wrong = cases.iter().find(|c| c.name == "addsWrong()").unwrap();
    assert_eq!(
        test_line(&root, &root, wrong),
        (java.join("CalcTest.java"), 16)
    );

    let since = SystemTime::now();
    let rerun = test_report::rerun(Tool::Gradle, command, &failed(&cases)).unwrap();
    let (ok, _) = run(&root, &rerun);
    assert!(!ok);
    let again = junit(Tool::Gradle, &root, &root, since);
    assert_eq!(
        outcomes(&again),
        [
            ("addsWrong()".to_owned(), Outcome::Failed),
            ("throwsError()".to_owned(), Outcome::Failed),
        ],
        "실패한 것만 다시"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 묶음만 돌리게 한 JUnit 5 테스트를 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_묶음만_돌리는_설정은_실패만_다시가_테스트를_찾지_못한_것을_알아본다() {
    if !require("gradle --version") {
        return;
    }
    for (name, rule) in [
        (
            "filter",
            "filter {\n        includeTestsMatching(\"*AllTests\")\n    }",
        ),
        ("include", "include(\"**/AllTests.class\")"),
    ] {
        let root = gradle_project(
            &format!("gradle-suite-only-{name}"),
            &[(
                "app",
                &format!(
                    "plugins {{\n    java\n}}\n\nrepositories {{\n    mavenCentral()\n}}\n\ndependencies {{\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.12.2\")\n    testImplementation(\"org.junit.platform:junit-platform-suite\")\n    testRuntimeOnly(\"org.junit.platform:junit-platform-launcher\")\n}}\n\ntasks.test {{\n    useJUnitPlatform()\n    {rule}\n}}\n"
                ),
            )],
        );
        write_junit5_suite(&root.join("app/src/test/java/com/example/board"));
        let command = "gradle test";
        let since = SystemTime::now();
        run(
            &root,
            &test_report::with_report(Tool::Gradle, command, &root),
        );
        let cases = junit(Tool::Gradle, &root, &root, since);
        // 묶음으로 한 번만 돌고, 보고서에는 실제 클래스가 적힌다.
        assert_eq!(
            suite_tests(&cases),
            [
                suite_test("BoardTest", "lists()", Outcome::Passed),
                suite_test("BoardTest", "savesWrong()", Outcome::Failed),
                suite_test("CalcTest", "adds()", Outcome::Passed),
                suite_test("CalcTest", "addsWrong()", Outcome::Failed),
            ],
            "{name}"
        );
        // 그 클래스 이름의 `--tests`는 빌드 설정의 거르기에 걸려 돌 것이 없다. 테스트 창은 전체를 다시 돌린다.
        let line = test_report::rerun(Tool::Gradle, command, &failed(&cases)).unwrap();
        let line = test_report::with_report(Tool::Gradle, &line, &root);
        let (ok, output) = run(&root, &format!("{line} 2>&1"));
        assert!(!ok, "{name}: {output}");
        assert!(
            test_report::gradle_rerun_found_nothing(Tool::Gradle, &output),
            "{name}: {output}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
#[ignore = "Gradle로 묶음만 도는 프로젝트와 보통 프로젝트를 함께 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_일부_프로젝트만_묶음만_돌아도_전체를_다시_돌린_결과에_통과한_프로젝트가_남는다() {
    if !require("gradle --version") {
        return;
    }
    let build = |rule: &str| {
        format!(
            "plugins {{\n    java\n}}\n\nrepositories {{\n    mavenCentral()\n}}\n\ndependencies {{\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.12.2\")\n    testImplementation(\"org.junit.platform:junit-platform-suite\")\n    testRuntimeOnly(\"org.junit.platform:junit-platform-launcher\")\n}}\n\ntasks.test {{\n    useJUnitPlatform()\n    {rule}\n}}\n"
        )
    };
    let root = gradle_project(
        "gradle-partial-suite",
        &[
            ("app", &build("")),
            (
                "lib",
                &build("filter {\n        includeTestsMatching(\"*AllTests\")\n    }"),
            ),
        ],
    );
    let app = root.join("app/src/test/java/com/example/board");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("AppTest.java"),
        "package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass AppTest {\n    @Test\n    void starts() {\n        assertEquals(3, 1 + 2);\n    }\n}\n",
    )
    .unwrap();
    write_junit5_suite(&root.join("lib/src/test/java/com/example/board"));
    let command = "gradle test";
    let since = SystemTime::now();
    run(
        &root,
        &test_report::with_report(Tool::Gradle, command, &root),
    );
    let cases = junit(Tool::Gradle, &root, &root, since);
    let all = [
        suite_test("AppTest", "starts()", Outcome::Passed),
        suite_test("BoardTest", "lists()", Outcome::Passed),
        suite_test("BoardTest", "savesWrong()", Outcome::Failed),
        suite_test("CalcTest", "adds()", Outcome::Passed),
        suite_test("CalcTest", "addsWrong()", Outcome::Failed),
    ];
    assert_eq!(suite_tests(&cases), all);
    // 실패는 lib에만 있고, lib의 거르기에 걸려 `--tests`로는 돌 것이 없다.
    let line = test_report::rerun(Tool::Gradle, command, &failed(&cases)).unwrap();
    let line = test_report::with_report(Tool::Gradle, &line, &root);
    let (ok, output) = run(&root, &format!("{line} 2>&1"));
    assert!(!ok, "{output}");
    assert!(
        test_report::gradle_rerun_found_nothing(Tool::Gradle, &output),
        "{output}"
    );
    // 전체를 다시 돌려도 통과한 app은 최신이라 건너뛰어 새 보고서가 없다. 건너뛴 작업의 보고서까지 읽어야 남는다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let since = SystemTime::now();
    let (_, output) = run(
        &root,
        &test_report::with_report(Tool::Gradle, command, &root),
    );
    let mut found = junit(Tool::Gradle, &root, &root, since);
    assert_eq!(suite_tests(&found), all[1..]);
    for file in test_report::gradle_skipped_reports(&root, &output) {
        found.extend(test_report::read_report(&file).unwrap());
    }
    assert_eq!(suite_tests(&found), all);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 두 프로젝트의 테스트를 두 번 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_최신으로_건너뛴_프로젝트의_보고서도_다시_읽는다() {
    if !require("gradle --version") {
        return;
    }
    let build = "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.12.2\")\n    testRuntimeOnly(\"org.junit.platform:junit-platform-launcher\")\n}\n\ntasks.test {\n    useJUnitPlatform()\n}\n";
    let root = gradle_project("gradle-up-to-date", &[("app", build), ("lib", build)]);
    for (project, class, body) in [
        ("app", "AppTest", "assertEquals(3, 1 + 2);"),
        ("lib", "LibTest", "assertEquals(4, 1 + 2);"),
    ] {
        let java = root.join(project).join("src/test/java/demo");
        std::fs::create_dir_all(&java).unwrap();
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package demo;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass {class} {{\n    @Test\n    void checks() {{\n        {body}\n    }}\n}}\n"),
        )
        .unwrap();
    }
    let command = test_report::with_report(Tool::Gradle, "gradle test", &root);
    let all = [
        suite_test("AppTest", "checks()", Outcome::Passed),
        suite_test("LibTest", "checks()", Outcome::Failed),
    ];
    let since = SystemTime::now();
    let (_, output) = run(&root, &command);
    assert_eq!(suite_tests(&junit(Tool::Gradle, &root, &root, since)), all);
    assert!(
        test_report::gradle_skipped_reports(&root, &output).is_empty(),
        "{output}"
    );
    // 다시 돌리면 통과한 app은 최신이라 건너뛰고 보고서를 새로 쓰지 않는다.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let since = SystemTime::now();
    let (_, output) = run(&root, &command);
    let fresh = junit(Tool::Gradle, &root, &root, since);
    assert_eq!(suite_tests(&fresh), all[1..], "{output}");
    let skipped = test_report::gradle_skipped_reports(&root, &output);
    assert_eq!(skipped.len(), 1, "{output}");
    let mut cases = fresh;
    cases.extend(test_report::read_report(&skipped[0]).unwrap());
    assert_eq!(suite_tests(&cases), all);
    assert_eq!(
        cases
            .iter()
            .find(|c| c.suite == "demo.AppTest")
            .unwrap()
            .task
            .as_deref(),
        Some(":app:test")
    );
    // 구성에 `-q`를 적어도 작업 줄이 남아 건너뛴 작업을 알아본다.
    let quiet = test_report::with_report(Tool::Gradle, "gradle test -q", &root);
    let (_, output) = run(&root, &quiet);
    assert_eq!(
        test_report::gradle_skipped_reports(&root, &output).len(),
        1,
        "{quiet}\n{output}"
    );
    // `gradle.properties`가 출력을 줄여도(로그 수준·콘솔) 작업 줄이 남는다.
    let properties = root.join("gradle.properties");
    let mut text = std::fs::read_to_string(&properties).unwrap();
    text.push_str("org.gradle.logging.level=quiet\norg.gradle.console=rich\n");
    std::fs::write(&properties, text).unwrap();
    let plain = test_report::with_report(Tool::Gradle, "gradle test", &root);
    let (_, output) = run(&root, &plain);
    assert_eq!(
        test_report::gradle_skipped_reports(&root, &output).len(),
        1,
        "{plain}\n{output}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 JUnit 5 테스트를 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_실패한_테스트의_이름을_바꾸면_실패만_다시가_테스트를_찾지_못한다() {
    if !require("gradle --version") {
        return;
    }
    let root = gradle_project(
        "gradle-renamed",
        &[(
            "app",
            "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.12.2\")\n    testRuntimeOnly(\"org.junit.platform:junit-platform-launcher\")\n}\n\ntasks.test {\n    useJUnitPlatform()\n}\n",
        )],
    );
    let java = root.join("app/src/test/java/demo");
    write_calc_test(&java, "addsWrong", false);
    let command = "gradle test";
    let since = SystemTime::now();
    run(
        &root,
        &test_report::with_report(Tool::Gradle, command, &root),
    );
    let cases = junit(Tool::Gradle, &root, &root, since);
    write_calc_test(&java, "addsFixed", true);
    let line = test_report::rerun(Tool::Gradle, command, &failed(&cases)).unwrap();
    let line = test_report::with_report(Tool::Gradle, &line, &root);
    let (ok, output) = run(&root, &format!("{line} 2>&1"));
    // 묶음만 도는 설정과 같은 오류로 끝난다. 테스트 창은 전체를 다시 돌려 앞 결과를 바꾼다.
    assert!(!ok, "{output}");
    assert!(
        test_report::gradle_rerun_found_nothing(Tool::Gradle, &output),
        "{output}"
    );
    std::thread::sleep(std::time::Duration::from_secs(3));
    let since = SystemTime::now();
    run(
        &root,
        &test_report::with_report(Tool::Gradle, command, &root),
    );
    assert_eq!(
        outcomes(&junit(Tool::Gradle, &root, &root, since)),
        [
            ("adds()".to_owned(), Outcome::Passed),
            ("addsFixed()".to_owned(), Outcome::Passed),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 TestNG 테스트(useTestNG)를 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_testng_매개변수_테스트를_줄마다_읽고_실패만_다시() {
    if !require("gradle --version") {
        return;
    }
    let root = gradle_project(
        "gradle-testng",
        &[(
            "app",
            "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(\"org.testng:testng:7.10.2\")\n}\n\ntasks.test {\n    useTestNG()\n}\n",
        )],
    );
    // Gradle은 줄 번호(0부터)를 먼저, 값을 뒤에 적는다.
    check_testng_data_provider(
        Tool::Gradle,
        &root,
        &root,
        "gradle test",
        ["adds[0](1, 2, 3)", "adds[1](2, 2, 5)"],
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 TestNG 묶음 파일(useTestNG의 suites)의 테스트를 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_testng_xml로_돌린_테스트를_읽고_실패만_다시() {
    if !require("gradle --version") {
        return;
    }
    let root = gradle_project(
        "gradle-testng-xml",
        &[(
            "app",
            "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(\"org.testng:testng:7.10.2\")\n}\n\ntasks.test {\n    useTestNG {\n        suites(\"src/test/resources/testng.xml\")\n    }\n}\n",
        )],
    );
    check_testng_xml(Tool::Gradle, &root, &root.join("app"), &root, "gradle test");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 JUnit 5 Suite로 묶은 테스트를 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_junit5_suite로_두_번_돈_테스트를_하나로_읽고_실패만_다시() {
    if !require("gradle --version") {
        return;
    }
    // Gradle은 `@Suite`인 `AllTests`와 그것이 고른 클래스를 모두 돌린다. 묶음으로 돈 것도 제 클래스의 보고서
    // (`TEST-…CalcTest.xml`)에 들어가, 한 보고서에 같은 테스트가 두 번 적힌다.
    let root = gradle_project(
        "gradle-junit5-suite",
        &[(
            "app",
            "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.12.2\")\n    testImplementation(\"org.junit.platform:junit-platform-suite\")\n    testRuntimeOnly(\"org.junit.platform:junit-platform-launcher\")\n}\n\ntasks.test {\n    useJUnitPlatform()\n}\n",
        )],
    );
    let java = root.join("app/src/test/java/com/example/board");
    write_junit5_suite(&java);
    let command = "gradle test";
    let run_gradle = |line: &str| {
        let since = SystemTime::now();
        run(&root, &test_report::with_report(Tool::Gradle, line, &root));
        junit(Tool::Gradle, &root, &root, since)
    };
    let twice = run_gradle(command);
    assert_eq!(twice.len(), 8, "묶음과 제 클래스로 두 번");
    let cases = test_report::dedup(twice);
    assert_eq!(
        suite_tests(&cases),
        [
            suite_test("BoardTest", "lists()", Outcome::Passed),
            suite_test("BoardTest", "savesWrong()", Outcome::Failed),
            suite_test("CalcTest", "adds()", Outcome::Passed),
            suite_test("CalcTest", "addsWrong()", Outcome::Failed),
        ]
    );
    let saves = cases.iter().find(|c| c.name == "savesWrong()").unwrap();
    assert_eq!(
        test_line(&root, &root, saves),
        (java.join("BoardTest.java"), 15)
    );
    // 실패만 다시: `--tests`가 고른 메서드만 돈다(묶음으로 다시 돌아도 하나로 모은다).
    let line = test_report::rerun(Tool::Gradle, command, &failed(&cases)).unwrap();
    assert_eq!(
        suite_tests(&test_report::dedup(run_gradle(&line))),
        [
            suite_test("BoardTest", "savesWrong()", Outcome::Failed),
            suite_test("CalcTest", "addsWrong()", Outcome::Failed),
        ],
        "{line}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 단위 테스트와 통합 테스트 작업(JVM Test Suite)을 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_통합_테스트_작업도_돌리고_작업마다_실패만_다시() {
    if !require("gradle --version") {
        return;
    }
    // `check`가 단위 테스트(`test`)와 통합 테스트(`integrationTest`) 작업을 다 돌린다.
    let root = gradle_project(
        "gradle-it",
        &[(
            "app",
            "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ntesting {\n    suites {\n        val test by getting(JvmTestSuite::class) {\n            useJUnitJupiter(\"5.11.4\")\n        }\n        register<JvmTestSuite>(\"integrationTest\") {\n            useJUnitJupiter(\"5.11.4\")\n            dependencies {\n                implementation(project())\n            }\n        }\n    }\n}\n\ntasks.named(\"check\") {\n    dependsOn(testing.suites.named(\"integrationTest\"))\n}\n",
        )],
    );
    let write = |dir: &str, class: &str, body: &str| {
        let java = root.join(dir).join("com/example/board");
        std::fs::create_dir_all(&java).unwrap();
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass {class} {{\n{body}}}\n"),
        )
        .unwrap();
    };
    write(
        "app/src/test/java",
        "CalcTest",
        "    @Test\n    void adds() {\n        assertEquals(3, 1 + 2);\n    }\n\n    @Test\n    void addsWrong() {\n        assertEquals(4, 1 + 2, \"더하기\");\n    }\n",
    );
    write(
        "app/src/integrationTest/java",
        "BoardIT",
        "    @Test\n    void lists() {\n        assertEquals(2, 1 + 1);\n    }\n\n    @Test\n    void savesWrong() {\n        assertEquals(\"저장\", \"삭제\", \"게시글 저장\");\n    }\n",
    );
    let command = "gradle check";
    assert_eq!(Tool::of(command), Some(Tool::Gradle));
    let since = SystemTime::now();
    let (ok, _) = run(
        &root,
        &test_report::with_report(Tool::Gradle, command, &root),
    );
    assert!(!ok);
    let cases = junit(Tool::Gradle, &root, &root, since);
    let tasks = |cases: &[Case]| {
        let mut found: Vec<(String, Outcome, String)> = cases
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.outcome,
                    c.task.clone().unwrap_or_default(),
                )
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    };
    let task = |name: &str, outcome, task: &str| (name.to_owned(), outcome, task.to_owned());
    assert_eq!(
        tasks(&cases),
        [
            task("adds()", Outcome::Passed, ":app:test"),
            task("addsWrong()", Outcome::Failed, ":app:test"),
            task("lists()", Outcome::Passed, ":app:integrationTest"),
            task("savesWrong()", Outcome::Failed, ":app:integrationTest"),
        ]
    );

    // `check`에는 `--tests`를 붙이지 못한다. 실패한 작업마다 그 작업의 실패만 다시 돌린다.
    let rerun_failed = |failed: &[&Case]| {
        // 보고서는 고친 시각으로 고르고 2초 앞당겨 보므로(FAT), 바로 앞 실행이 끝 무렵 남긴 보고서가 섞인다.
        // 돌지 않은 작업의 보고서가 남지 않게 지우고 돌린다.
        std::fs::remove_dir_all(root.join("app/build/test-results")).unwrap();
        let since = SystemTime::now();
        let line = test_report::rerun(Tool::Gradle, command, failed).unwrap();
        let (ok, output) = run(&root, &test_report::with_report(Tool::Gradle, &line, &root));
        assert!(!ok, "{line}\n{output}");
        tasks(&junit(Tool::Gradle, &root, &root, since))
    };
    assert_eq!(
        rerun_failed(&failed(&cases)),
        [
            task("addsWrong()", Outcome::Failed, ":app:test"),
            task("savesWrong()", Outcome::Failed, ":app:integrationTest"),
        ]
    );
    // 통합 테스트만 실패: 단위 테스트 작업은 돌지 않는다.
    let only_it: Vec<&Case> = failed(&cases)
        .into_iter()
        .filter(|c| c.task.as_deref() == Some(":app:integrationTest"))
        .collect();
    assert_eq!(
        rerun_failed(&only_it),
        [task(
            "savesWrong()",
            Outcome::Failed,
            ":app:integrationTest"
        )]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 JUnit 5 테스트를 돌린다(의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_설정이_폴더를_바꾼_프로젝트와_품은_빌드의_실패만_다시() {
    if !require("gradle --version") {
        return;
    }
    // app은 modules/application에 있고(설정이 폴더를 적었다), lib(품은 빌드, Groovy DSL)의 core도 루트에서 돌린다.
    let root = temp_dir("gradle-dirs");
    let build = "plugins {\n    id 'java'\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation 'org.junit.jupiter:junit-jupiter:5.11.4'\n    testRuntimeOnly 'org.junit.platform:junit-platform-launcher'\n}\n\ntest {\n    useJUnitPlatform()\n}\n";
    let files = [
        (
            "settings.gradle.kts",
            "rootProject.name = \"board\"\ninclude(\"app\")\nproject(\":app\").projectDir = file(\"modules/application\")\nincludeBuild(\"lib\")\n".to_owned(),
        ),
        ("gradle.properties", "org.gradle.daemon=false\n".to_owned()),
        ("modules/application/build.gradle", build.to_owned()),
        ("lib/settings.gradle", "rootProject.name = 'libroot'\ninclude 'core'\n".to_owned()),
        ("lib/core/build.gradle", build.to_owned()),
    ];
    for (file, text) in files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    for (dir, class) in [("modules/application", "AppTest"), ("lib/core", "CoreTest")] {
        let java = root.join(dir).join("src/test/java/demo");
        std::fs::create_dir_all(&java).unwrap();
        std::fs::write(
            java.join(format!("{class}.java")),
            format!("package demo;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass {class} {{\n    @Test\n    void ok() {{\n        assertEquals(3, 1 + 2);\n    }}\n\n    @Test\n    void wrong() {{\n        assertEquals(4, 1 + 2);\n    }}\n}}\n"),
        )
        .unwrap();
    }
    let command = "gradle test :lib:core:test --continue";
    assert_eq!(Tool::of(command), Some(Tool::Gradle));
    let since = SystemTime::now();
    let (ok, output) = run(
        &root,
        &test_report::with_report(Tool::Gradle, command, &root),
    );
    assert!(!ok, "{output}");
    let tasks = |cases: &[Case]| {
        let mut found: Vec<(String, String, Outcome, String)> = cases
            .iter()
            .map(|c| {
                let class = c.suite.rsplit('.').next().unwrap_or_default().to_owned();
                (
                    class,
                    c.name.clone(),
                    c.outcome,
                    c.task.clone().unwrap_or_default(),
                )
            })
            .collect();
        found.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        found
    };
    let case = |class: &str, name: &str, outcome, task: &str| {
        (class.to_owned(), name.to_owned(), outcome, task.to_owned())
    };
    let cases = junit(Tool::Gradle, &root, &root, since);
    assert_eq!(
        tasks(&cases),
        [
            case("AppTest", "ok()", Outcome::Passed, ":app:test"),
            case("AppTest", "wrong()", Outcome::Failed, ":app:test"),
            case("CoreTest", "ok()", Outcome::Passed, ":lib:core:test"),
            case("CoreTest", "wrong()", Outcome::Failed, ":lib:core:test"),
        ]
    );
    // 실패만 다시: 작업마다 그 작업의 실패만 돈다(폴더 이름으로 셈한 `:modules:application:test`·`:core:test`는
    // Gradle이 찾지 못한다).
    for dir in ["modules/application", "lib/core"] {
        std::fs::remove_dir_all(root.join(dir).join("build/test-results")).unwrap();
    }
    let since = SystemTime::now();
    let line = test_report::rerun(Tool::Gradle, command, &failed(&cases)).unwrap();
    let (ok, output) = run(&root, &test_report::with_report(Tool::Gradle, &line, &root));
    assert!(!ok, "{line}\n{output}");
    assert_eq!(
        tasks(&junit(Tool::Gradle, &root, &root, since)),
        [
            case("AppTest", "wrong()", Outcome::Failed, ":app:test"),
            case("CoreTest", "wrong()", Outcome::Failed, ":lib:core:test"),
        ],
        "{line}\n{output}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 Kotlin·Scala의 JUnit 5 테스트를 돌린다(컴파일러와 의존성을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_kotlin_scala_테스트의_줄과_실패만_다시() {
    if !require("gradle --version") {
        return;
    }
    let common = "\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.11.4\")\n    testRuntimeOnly(\"org.junit.platform:junit-platform-launcher\")\n}\n\ntasks.test {\n    useJUnitPlatform()\n}\n";
    let kotlin = format!("plugins {{\n    kotlin(\"jvm\") version \"2.3.0\"\n}}{common}");
    let scala = format!(
        "plugins {{\n    scala\n}}{common}\ndependencies {{\n    implementation(\"org.scala-lang:scala-library:2.13.16\")\n}}\n"
    );
    let root = gradle_project("gradle-jvm", &[("kt", &kotlin), ("sc", &scala)]);
    // Kotlin 관례대로 공통 루트 패키지(`com.example.board`)를 폴더에서 뺐다. 이름은 백틱으로 빈칸을 넣었다.
    let kt = root.join("kt/src/test/kotlin");
    std::fs::create_dir_all(&kt).unwrap();
    std::fs::write(
        kt.join("CalcTest.kt"),
        "package com.example.board\n\nimport org.junit.jupiter.api.Assertions.assertEquals\nimport org.junit.jupiter.api.Test\n\nclass CalcTest {\n    @Test\n    fun `adds numbers`() {\n        assertEquals(3, 1 + 2)\n    }\n\n    @Test\n    fun `adds wrong numbers`() {\n        assertEquals(4, 1 + 2, \"더하기\")\n    }\n}\n",
    )
    .unwrap();
    // Scala는 람다(`$anonfun$`) 안에서 실패한다.
    let sc = root.join("sc/src/test/scala/com/example/board");
    std::fs::create_dir_all(&sc).unwrap();
    std::fs::write(
        sc.join("CalcSuite.scala"),
        "package com.example.board\n\nimport org.junit.jupiter.api.Assertions.assertEquals\nimport org.junit.jupiter.api.Test\n\nclass CalcSuite {\n  @Test\n  def adds(): Unit = assertEquals(3, 1 + 2)\n\n  @Test\n  def addsWrong(): Unit =\n    List(1).foreach(n => assertEquals(4, n + 2, \"더하기\"))\n}\n",
    )
    .unwrap();
    let command = "gradle test";
    let since = SystemTime::now();
    let (ok, _) = run(
        &root,
        &test_report::with_report(Tool::Gradle, command, &root),
    );
    assert!(!ok, "실패한 테스트가 있으면 gradle이 실패한다");
    let cases = junit(Tool::Gradle, &root, &root, since);
    for case in failed(&cases) {
        eprintln!("[{} {}]\n{}", case.suite, case.name, case.detail);
    }
    assert_eq!(
        outcomes(&cases),
        [
            ("adds numbers()".to_owned(), Outcome::Passed),
            ("adds wrong numbers()".to_owned(), Outcome::Failed),
            ("adds()".to_owned(), Outcome::Passed),
            ("addsWrong()".to_owned(), Outcome::Failed),
        ]
    );
    let line = |name: &str| test_line(&root, &root, cases.iter().find(|c| c.name == name).unwrap());
    assert_eq!(line("adds wrong numbers()"), (kt.join("CalcTest.kt"), 14));
    assert_eq!(line("addsWrong()"), (sc.join("CalcSuite.scala"), 12));

    // 실패만 다시(앱처럼 보고서 옵션을 붙인다). 빈칸 든 이름도 `--tests`로 걸러진다.
    let again = |cases: &[&Case]| {
        // 보고서는 고친 시각으로 고르고 2초 앞당겨 보므로(FAT), 바로 앞 실행이 끝 무렵 남긴 다른 작업의
        // 보고서가 섞인다. 돌지 않은 작업의 보고서가 남지 않게 지우고 돌린다.
        for project in ["kt", "sc"] {
            let _ = std::fs::remove_dir_all(root.join(project).join("build/test-results"));
        }
        let since = SystemTime::now();
        let rerun = test_report::rerun(Tool::Gradle, command, cases).unwrap();
        let (ok, _) = run(
            &root,
            &test_report::with_report(Tool::Gradle, &rerun, &root),
        );
        assert!(!ok);
        outcomes(&junit(Tool::Gradle, &root, &root, since))
    };
    assert_eq!(
        again(&failed(&cases)),
        [
            ("adds wrong numbers()".to_owned(), Outcome::Failed),
            ("addsWrong()".to_owned(), Outcome::Failed),
        ],
        "실패한 것만 다시"
    );
    // 한 하위 프로젝트에만 실패가 있으면 그 작업(`:sc:test`)만 돈다.
    let scala: Vec<&Case> = failed(&cases)
        .into_iter()
        .filter(|c| c.name == "addsWrong()")
        .collect();
    assert_eq!(again(&scala), [("addsWrong()".to_owned(), Outcome::Failed)]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "Gradle로 JUnit 5 테스트를 한글 경로의 Gradle 홈으로 돌린다(그 홈에 의존성을 새로 받는다). Java 18 이상과 gradle이 필요하다"]
fn gradle_홈이_한글_경로면_windows에서_테스트_jvm이_뜨지_못한_것을_알아본다() {
    if !require("gradle --version") {
        return;
    }
    // 프로젝트 경로는 영문이고 Gradle 홈만 한글이다(Windows 사용자 이름이 한글이면 기본 홈이 그렇다).
    let root = gradle_project(
        "gradle-home",
        &[(
            "app",
            "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.11.4\")\n    testRuntimeOnly(\"org.junit.platform:junit-platform-launcher\")\n}\n\ntasks.test {\n    useJUnitPlatform()\n}\n",
        )],
    );
    let java = root.join("app/src/test/java/com/example/board");
    std::fs::create_dir_all(&java).unwrap();
    std::fs::write(
        java.join("CalcTest.java"),
        "package com.example.board;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass CalcTest {\n    @Test\n    void adds() {\n        assertEquals(3, 1 + 2);\n    }\n}\n",
    )
    .unwrap();
    let home = temp_dir("그레이들 홈");
    let env = std::collections::BTreeMap::from([(
        "GRADLE_USER_HOME".to_owned(),
        home.to_string_lossy().into_owned(),
    )]);
    assert_eq!(test_report::gradle_user_home(&env), Some(home.clone()));
    let since = SystemTime::now();
    // 테스트 창처럼 표준 출력과 오류를 함께 받는다(테스트 JVM이 뜨지 못한 까닭은 출력에만 나온다).
    let output = {
        #[cfg(windows)]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let mut command = Command::new("cmd.exe");
            command.args(["/d", "/s", "/c"]).raw_arg("\"gradle test\"");
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new("sh");
            command.args(["-c", "gradle test"]);
            command
        };
        let output = command
            .current_dir(&root)
            .env("GRADLE_USER_HOME", &home)
            .output()
            .unwrap();
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    };
    let cases = junit(Tool::Gradle, &root, &root, since);
    let problem = test_report::gradle_path_problem(Tool::Gradle, &root, &home, &cases, &output);
    if cfg!(windows) {
        // JDK 18 이상이 Gradle이 UTF-8로 적은 클래스 경로 파일을 시스템 코드 페이지로 읽어, Gradle 홈에 받아 둔
        // Gradle의 클래스부터 찾지 못한다.
        assert_eq!(
            problem,
            Some(test_report::GradlePathProblem::Home(home.clone())),
            "{output}"
        );
    } else {
        assert_eq!(problem, None);
        assert_eq!(outcomes(&cases), [("adds()".to_owned(), Outcome::Passed)]);
    }
    let _ = std::fs::remove_dir_all(&root);
    for _ in 0..20 {
        if !home.exists() || std::fs::remove_dir_all(&home).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[test]
#[ignore = "Gradle로 의존성 나무를 푼다(의존성의 POM을 받는다). Java 17 이상과 gradle이 필요하다"]
fn gradle_dependencies로_전이_의존과_고른_버전을_읽는다() {
    use ferrite_core::vuln::manifest;

    if !require("gradle --version") {
        return;
    }
    // app: log4j-slf4j-impl이 끌어오는 slf4j-api 1.7.x는 직접 의존(2.0.9)으로 올라간다(`-> 2.0.9`).
    // lib: BOM이 버전을 정하는 의존(`-> 5.11.4`)과 BOM 아래 버전 제약(`(c)`).
    let root = gradle_project(
        "gradle-deps",
        &[
            (
                "app",
                "plugins {\n    java\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    implementation(project(\":lib\"))\n    implementation(\"org.apache.logging.log4j:log4j-slf4j-impl:2.20.0\")\n    implementation(\"org.slf4j:slf4j-api:2.0.9\")\n    testImplementation(\"org.junit.jupiter:junit-jupiter:5.11.4\")\n}\n",
            ),
            (
                "lib",
                "plugins {\n    `java-library`\n}\n\nrepositories {\n    mavenCentral()\n}\n\ndependencies {\n    api(platform(\"org.junit:junit-bom:5.11.4\"))\n    api(\"org.junit.jupiter:junit-jupiter-api\")\n}\n",
            ),
        ],
    );
    // 루트에는 빌드 파일이 없다. 하위 프로젝트마다 그 폴더에서 돌린다(그 프로젝트의 나무만 낸다).
    let manifests = manifest::find(&root);
    assert_eq!(
        manifests.iter().map(|m| m.path.clone()).collect::<Vec<_>>(),
        [
            root.join("app").join("build.gradle.kts"),
            root.join("lib").join("build.gradle.kts")
        ]
    );
    let deps = |m: &manifest::Manifest| {
        let line = m.command(&root, &root.join("unused")).unwrap();
        let (ok, stdout) = run(m.path.parent().unwrap(), &line);
        assert!(ok, "의존성 나무");
        m.read(Some(&stdout)).unwrap()
    };
    let find = |deps: &[ferrite_core::vuln::Dependency], name: &str| {
        let mut found: Vec<(String, usize, Option<String>)> = deps
            .iter()
            .filter(|d| d.name == name)
            .map(|d| (d.version.clone(), d.line, d.via.clone()))
            .collect();
        found.sort();
        found
    };
    let at = |version: &str, line: usize, via: Option<&str>| {
        (version.to_owned(), line, via.map(str::to_owned))
    };

    let app = deps(&manifests[0]);
    let impl_ = Some("org.apache.logging.log4j:log4j-slf4j-impl");
    assert_eq!(
        find(&app, "org.apache.logging.log4j:log4j-core"),
        [at("2.20.0", 10, impl_)],
        "실행 때 끌어오는 전이 의존은 직접 의존 줄에"
    );
    assert_eq!(
        find(&app, "org.slf4j:slf4j-api"),
        [at("2.0.9", 10, impl_), at("2.0.9", 11, None)],
        "Gradle이 고른 버전"
    );
    assert_eq!(
        find(&app, "org.junit.jupiter:junit-jupiter-api"),
        [at("5.11.4", 9, Some("project :lib"))],
        "하위 프로젝트가 끌어온 것은 그 프로젝트 줄에(BOM의 제약은 빼고)"
    );
    assert!(
        find(&app, "org.junit.jupiter:junit-jupiter").is_empty(),
        "테스트 의존은 runtimeClasspath에 없다"
    );

    let lib = deps(&manifests[1]);
    assert_eq!(
        find(&lib, "org.junit.jupiter:junit-jupiter-api"),
        [at("5.11.4", 10, None)],
        "BOM이 정한 버전"
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

/// 의존 없는 Java 17 Maven 프로젝트.
const JAVA17_POM: &str = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>board</artifactId>
  <version>1.0</version>
  <properties>
    <maven.compiler.release>17</maven.compiler.release>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
  </properties>
</project>
"#;

/// 정적 분석 시험의 프로젝트와 그 Java 파일. 경로에 한글과 빈칸을 넣어 SARIF 주소의 인코딩을 본다
/// (한국어 Windows에서 흔한 경로다).
fn analyzed_project(name: &str) -> (PathBuf, PathBuf) {
    let root = temp_dir(name).join("게시판 프로젝트");
    let java = root.join("src/main/java/com/example/board");
    std::fs::create_dir_all(&java).unwrap();
    let board = java.join("Board.java");
    std::fs::write(
        &board,
        "package com.example.board;\n\nimport java.util.List;\n\npublic class Board {\n    private String unused;\n\n    public int parse(String s) {\n        try {\n            return Integer.parseInt(s);\n        } catch (NumberFormatException e) {\n        }\n        return 0;\n    }\n}\n",
    )
    .unwrap();
    (root, board)
}

/// 결과를 줄 순서로 (도구, 규칙, 수준, 시작 줄, 끝 줄). 파일은 모두 `file`이어야 한다.
fn sarif_lines(
    findings: &[ferrite_core::sarif::Finding],
    file: &Path,
) -> Vec<(String, String, ferrite_core::sarif::Level, u32, u32)> {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap();
    for finding in findings {
        let path = finding.path.as_deref().expect("프로젝트의 파일");
        assert_eq!(canonical(path), canonical(file), "{}", finding.uri);
    }
    let mut found: Vec<_> = findings
        .iter()
        .map(|f| (f.tool.clone(), f.rule.clone(), f.level, f.line, f.end_line))
        .collect();
    found.sort_by_key(|f| f.3);
    found
}

#[test]
#[ignore = "Maven으로 PMD(maven-pmd-plugin 3.28.0, PMD 7.17.0)를 받아 돌린다. Java 17 이상과 mvn이 필요하다"]
fn pmd가_낸_sarif를_읽고_원본_파일을_찾는다() {
    use ferrite_core::sarif::{self, Level};

    if !require("mvn -v") {
        return;
    }
    let (root, board) = analyzed_project("pmd");
    std::fs::write(root.join("pom.xml"), JAVA17_POM).unwrap();
    // maven-pmd-plugin은 SARIF를 짧은 이름으로 고르지 못해 PMD의 렌더러 클래스로 고른다. 결과는
    // `target/pmd.sarif.json`이고, 파일 주소는 절대 `file:` 주소다(빈칸은 `%20`, 한글은 그대로).
    let (ok, _) = run(
        &root,
        "mvn -B -q compile org.apache.maven.plugins:maven-pmd-plugin:3.28.0:pmd -Dformat=net.sourceforge.pmd.renderers.SarifRenderer",
    );
    assert!(ok, "PMD 분석");
    let report = root.join("target").join("pmd.sarif.json");
    assert_eq!(
        sarif::candidates(&root),
        std::slice::from_ref(&report),
        "프로젝트에서 찾는다"
    );
    let text = std::fs::read_to_string(&report).unwrap();
    let findings = sarif::parse(&text, &root, &root.join("target")).unwrap();
    let pmd = |rule: &str, level, line, end| ("PMD".to_owned(), rule.to_owned(), level, line, end);
    assert_eq!(
        sarif_lines(&findings, &board),
        [
            pmd("UnnecessaryImport", Level::Note, 3, 3),
            pmd("UnusedPrivateField", Level::Warning, 6, 6),
            pmd("EmptyCatchBlock", Level::Warning, 11, 12),
        ]
    );
    assert!(
        findings
            .iter()
            .any(|f| f.message == "Avoid unused private fields such as 'unused'.")
    );
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
#[ignore = "Maven으로 PMD(maven-pmd-plugin 3.28.0, PMD 7.17.0)를 받아 돌린다. Java 17 이상과 mvn이 필요하다"]
fn egov_룰셋의_44개_규칙이_모두_잡힌다() {
    use ferrite_core::egov_pmd::{self, PomEdit};
    use ferrite_core::sarif;

    if !require("mvn -v") {
        return;
    }
    let (root, _) = analyzed_project("egov-pmd");
    // 규칙마다 걸리게 만든 예제(공식 4.3 룰셋은 이 중 다섯 규칙을 잡지 못한다).
    let java = root.join("src/main/java/com/example/board");
    for (name, text) in [
        ("Board.java", include_str!("egov_pmd/Board.java")),
        ("Extra.java", include_str!("egov_pmd/Extra.java")),
        ("Sample.java", include_str!("egov_pmd/Sample.java")),
        ("Page.java", include_str!("egov_pmd/Page.java")),
    ] {
        std::fs::write(java.join(name), text).unwrap();
    }
    // [egov 룰셋 넣기]와 같다: 룰셋 파일을 쓰고 pom에 적는다.
    std::fs::write(root.join(egov_pmd::FILE), egov_pmd::RULESET).unwrap();
    let PomEdit::Changed(pom) = egov_pmd::add_to_pom(JAVA17_POM).unwrap() else {
        panic!("pom을 고치지 않음");
    };
    std::fs::write(root.join("pom.xml"), pom).unwrap();
    let (ok, _) = run(
        &root,
        "mvn -B -q compile pmd:pmd -Dformat=net.sourceforge.pmd.renderers.SarifRenderer",
    );
    assert!(ok, "PMD 분석");
    let target = root.join("target");
    let text = std::fs::read_to_string(target.join("pmd.sarif.json")).unwrap();
    let findings = sarif::parse(&text, &root, &target).unwrap();
    let fired: std::collections::BTreeSet<&str> =
        findings.iter().map(|f| f.rule.as_str()).collect();
    let rules: std::collections::BTreeSet<&str> = egov_pmd::RULESET
        .split("category/java/")
        .skip(1)
        .map(|s| s.split(['/', '"']).nth(1).unwrap())
        .collect();
    assert_eq!(rules.len(), 44);
    assert_eq!(fired, rules, "44개 규칙이 모두 잡히고 다른 규칙은 없다");
    let empty_catch = findings
        .iter()
        .find(|f| f.rule == "EmptyCatchBlock")
        .unwrap();
    // PMD는 실제 경로로 적는다(macOS `/private/var`, Windows 긴 사용자 폴더 이름). 끝부분만 견준다.
    let path = empty_catch.path.as_deref().expect("프로젝트의 파일");
    assert!(
        path.ends_with("src/main/java/com/example/board/Board.java"),
        "{}",
        path.display()
    );
    assert_eq!(
        (empty_catch.line, empty_catch.message.as_str()),
        (14, "내용이 없는 Catch Block이 존재, 오류 상황 대응 부재")
    );
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
#[ignore = "Semgrep이 필요하다(pip install semgrep)"]
fn semgrep이_낸_sarif를_읽고_원본_파일을_찾는다() {
    use ferrite_core::sarif::{self, Level};

    if !require("semgrep --version") {
        return;
    }
    let (root, board) = analyzed_project("semgrep");
    std::fs::write(
        root.join("rules.yml"),
        "rules:\n  - id: empty-catch\n    languages: [java]\n    severity: ERROR\n    message: 예외를 삼키는 빈 catch 블록\n    pattern: |\n      try { ... } catch ($E $V) { }\n  - id: parse-int\n    languages: [java]\n    severity: INFO\n    message: 숫자 변환 $X\n    pattern: Integer.parseInt($X)\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    // Semgrep(Python)은 규칙 파일을 시스템 로캘 인코딩으로 읽어, 한국어 Windows(cp949)에서는 한글 메시지가
    // 든 UTF-8 규칙을 읽다 멈춘다. UTF-8 모드로 돌린다.
    let (ok, _) = run_env(
        &root,
        "semgrep scan --config rules.yml --sarif --output reports/semgrep.sarif --metrics=off --disable-version-check .",
        &[("PYTHONUTF8", "1")],
    );
    assert!(ok, "Semgrep 분석");
    let report = root.join("reports").join("semgrep.sarif");
    assert_eq!(
        sarif::candidates(&root),
        std::slice::from_ref(&report),
        "프로젝트에서 찾는다"
    );
    // 파일 주소는 돌린 폴더 기준이고(`%SRCROOT%`의 경로는 주지 않는다) Windows에서는 역슬래시로 적는다.
    let text = std::fs::read_to_string(&report).unwrap();
    let findings = sarif::parse(&text, &root, &root.join("reports")).unwrap();
    let semgrep = |rule: &str, level, line, end| {
        ("Semgrep OSS".to_owned(), rule.to_owned(), level, line, end)
    };
    assert_eq!(
        sarif_lines(&findings, &board),
        [
            semgrep("empty-catch", Level::Error, 9, 12),
            semgrep("parse-int", Level::Note, 10, 10),
        ]
    );
    let mut messages: Vec<&str> = findings.iter().map(|f| f.message.as_str()).collect();
    messages.sort();
    assert_eq!(messages, ["숫자 변환 s", "예외를 삼키는 빈 catch 블록"]);
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
#[ignore = "clang이 필요하다(CI 러너에는 3개 OS 모두 있다)"]
fn clang_정적_분석기가_낸_sarif를_읽고_원본_파일을_찾는다() {
    use ferrite_core::sarif::{self, Level};

    if !require("clang --version") {
        return;
    }
    // 경로에 한글과 빈칸을 넣어 SARIF 주소의 인코딩을 본다. 헤더를 쓰지 않아 OS마다 표준 헤더가 없어도 된다.
    let root = temp_dir("clang").join("게시판 프로젝트");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    let board = root.join("src").join("board.c");
    std::fs::write(
        &board,
        "/* 게시판 분석 시험 */\nint deref(int *p) {\n    if (p == 0) {\n        return *p;\n    }\n    return 0;\n}\n\nint divide(int a) {\n    int zero = 0;\n    return a / zero;\n}\n",
    )
    .unwrap();
    let (ok, _) = run(
        &root,
        "clang --analyze --analyzer-output sarif -o reports/clang.sarif src/board.c",
    );
    assert!(ok, "clang 정적 분석");
    let report = root.join("reports").join("clang.sarif");
    let text = std::fs::read_to_string(&report).unwrap();
    // 판마다 꼴이 조금씩 달라 실패하면 원문을 본다.
    eprintln!("{text}");
    assert_eq!(
        sarif::candidates(&root),
        std::slice::from_ref(&report),
        "프로젝트에서 찾는다"
    );
    let findings = sarif::parse(&text, &root, &root.join("reports")).unwrap();
    let found: Vec<(String, Level, u32)> = sarif_lines(&findings, &board)
        .into_iter()
        .map(|(_, rule, level, line, _)| (rule, level, line))
        .collect();
    assert_eq!(
        found,
        [
            ("core.NullDereference".to_owned(), Level::Warning, 4),
            ("core.DivideZero".to_owned(), Level::Warning, 11),
        ]
    );
    assert!(
        findings
            .iter()
            .all(|f| f.tool.to_lowercase().contains("clang")),
        "{:?}",
        findings.iter().map(|f| &f.tool).collect::<Vec<_>>()
    );
    let messages: Vec<String> = findings.iter().map(|f| f.message.to_lowercase()).collect();
    assert!(
        messages[0].contains("null") && messages[1].contains("zero"),
        "{messages:?}"
    );
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
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
    std::fs::write(root.join("pom.xml"), JAVA17_POM).unwrap();
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

#[test]
#[ignore = "Maven으로 의존성 나무를 풀고(maven-dependency-plugin을 받는다) OSV에 온라인으로 묻고 Maven DB 묶음(약 10MB)을 받는다"]
fn 전이_의존의_log4shell을_온라인과_오프라인_db가_찾는다() {
    use ferrite_core::vuln::{Ecosystem, Finding, manifest, osv};

    if !require("mvn -v") {
        return;
    }
    let root = temp_dir("vuln");
    std::fs::write(
        root.join("pom.xml"),
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>board</artifactId>
  <version>1.0</version>
  <dependencies>
    <dependency>
      <groupId>org.apache.logging.log4j</groupId>
      <artifactId>log4j-slf4j-impl</artifactId>
      <version>2.14.1</version>
    </dependency>
  </dependencies>
</project>
"#,
    )
    .unwrap();
    let manifests = manifest::find(&root);
    assert_eq!(manifests.len(), 1);
    let tree = root.join("tree.txt");
    let line = manifests[0].command(&root, &tree).unwrap();
    assert!(run(&root, &line).0, "의존성 나무");
    let deps = manifests[0]
        .read(Some(&std::fs::read_to_string(&tree).unwrap()))
        .unwrap();
    let core = deps
        .iter()
        .find(|d| d.name == "org.apache.logging.log4j:log4j-core")
        .expect("전이 의존 log4j-core");
    assert_eq!(
        (core.version.as_str(), core.line, core.via.as_deref()),
        (
            "2.14.1",
            6,
            Some("org.apache.logging.log4j:log4j-slf4j-impl")
        )
    );
    let log4shell = |findings: &[Finding]| {
        findings
            .iter()
            .find(|f| f.title() == "CVE-2021-44228")
            .map(|f| {
                (
                    f.dependency.name.clone(),
                    f.fixed.clone(),
                    f.dependency.line,
                )
            })
    };
    let expected = Some((
        "org.apache.logging.log4j:log4j-core".to_owned(),
        Some("2.15.0".to_owned()),
        6,
    ));
    let online = osv::findings(&osv::query_online(&deps).unwrap(), &deps);
    assert_eq!(log4shell(&online), expected, "온라인");
    let zip = root.join("osv").join("all.zip");
    osv::download_database(Ecosystem::Maven, &zip, &mut |_, _| {}).unwrap();
    let offline = osv::findings(&osv::query_offline(&zip, &deps).unwrap(), &deps);
    assert_eq!(log4shell(&offline), expected, "오프라인");
    // 처음 검사가 만든 색인으로 다시 읽어도 같다(색인은 레코드의 자리로 묶음 목차 없이 푼다).
    assert!(osv::index_path(&zip).is_file());
    let again = osv::findings(&osv::query_offline(&zip, &deps).unwrap(), &deps);
    assert_eq!(again, offline, "색인으로 읽은 결과");
    let _ = std::fs::remove_dir_all(&root);
}
