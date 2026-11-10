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

    // 두 하위 프로젝트 모두 실패가 있어야 한다. `--tests`는 모든 하위 프로젝트의 test 작업에 걸려, 하나도
    // 맞지 않는 작업은 "No tests found"로 실패한다.
    let since = SystemTime::now();
    let rerun = test_report::rerun(Tool::Gradle, command, &failed(&cases)).unwrap();
    let (ok, _) = run(&root, &rerun);
    assert!(!ok);
    let again = junit(Tool::Gradle, &root, &root, since);
    assert_eq!(
        outcomes(&again),
        [
            ("adds wrong numbers()".to_owned(), Outcome::Failed),
            ("addsWrong()".to_owned(), Outcome::Failed),
        ],
        "실패한 것만 다시"
    );
    let _ = std::fs::remove_dir_all(&root);
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
