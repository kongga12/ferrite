//! 테스트 결과 보고서: JUnit XML(Maven·Gradle·pytest·CTest)과 `go test -json`을 읽어 테스트 하나하나의
//! 결과로 바꾼다. 테스트 명령에 보고서 옵션을 붙이고, 이번 실행이 남긴 보고서를 찾고, 실패한 것만 다시
//! 돌리는 명령을 만들고, 실패 스택에서 프로젝트의 소스 줄을 찾는다.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use regex::Regex;
use serde_json::Value;

/// 결과를 읽을 수 있는 테스트 도구.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Maven,
    Gradle,
    Pytest,
    Ctest,
    Go,
}

impl Tool {
    /// 명령 줄(실행 구성)로 도구를 알아낸다. 테스트를 돌리는 명령이 아니면 None.
    pub fn of(command: &str) -> Option<Tool> {
        let words: Vec<String> = command
            .split_whitespace()
            .map(|w| w.trim_matches('"').to_ascii_lowercase())
            .collect();
        let program = words.first().map(|w| {
            let base = w.rsplit(['/', '\\']).next().unwrap_or(w);
            base.trim_end_matches(".cmd")
                .trim_end_matches(".bat")
                .trim_end_matches(".exe")
                .to_owned()
        })?;
        let has = |word: &str| words.iter().any(|w| w == word);
        match program.as_str() {
            "mvn" | "mvnw" if has("test") || has("verify") => Some(Tool::Maven),
            "gradle" | "gradlew" if gradle_runs_tests(&words) => Some(Tool::Gradle),
            "pytest" => Some(Tool::Pytest),
            _ if words.windows(2).any(|w| w[0] == "-m" && w[1] == "pytest") => Some(Tool::Pytest),
            "ctest" => Some(Tool::Ctest),
            "go" if words.get(1).is_some_and(|w| w == "test") => Some(Tool::Go),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    /// 실패와 오류(예외)를 함께 센다.
    Failed,
    Skipped,
}

/// 테스트 하나의 결과.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    /// 묶음: Java 클래스, pytest 모듈, Go 패키지, CTest 이름.
    pub suite: String,
    pub name: String,
    pub outcome: Outcome,
    pub seconds: Option<f64>,
    /// 실패·건너뜀 사유(첫 줄).
    pub message: Option<String>,
    /// 스택과 출력.
    pub detail: String,
    /// 보고서가 알려 준 소스 위치(pytest 일부 형식).
    pub file: Option<String>,
    pub line: Option<u32>,
    /// Maven Failsafe가 돌린 통합 테스트(`failsafe-reports`의 보고서). 실패만 다시 돌릴 때 Surefire의
    /// `-Dtest`가 아니라 `-Dit.test`로 거른다.
    pub integration: bool,
    /// 이 테스트를 돌린 곳. 보고서 폴더에서 읽는다.
    /// - Gradle: 테스트 작업(`:app:test`, `:app:integrationTest`, `<프로젝트>/build/test-results/<작업>`). 실패만
    ///   다시 돌릴 때 그 작업에 `--tests`를 붙인다.
    /// - Maven: 모듈 이름(`<모듈>/target/surefire-reports`의 pom, `maven_module`). 실패만 다시에서 빌드가 깨진
    ///   모듈의 실패를 남길 때 쓴다.
    pub task: Option<String>,
    /// 이번 실행에서 돌지 못해(그 모듈·작업의 빌드가 깨짐) 앞 실행의 결과를 남긴 것(`apply_rerun`,
    /// `apply_whole_rerun`). 보고서에서 읽은 결과는 거짓이다. 같은 테스트인지에는 들지 않는다.
    pub previous: bool,
}

impl Case {
    /// 같은 테스트인지: 묶음·이름과 돌린 곳(Failsafe, Gradle 작업, Maven 모듈)이 같다.
    pub fn same_test(&self, other: &Case) -> bool {
        self.suite == other.suite
            && self.name == other.name
            && self.integration == other.integration
            && self.task == other.task
    }
}

/// 한 번 돌린 결과에 같은 테스트가 둘 이상이면 하나로 모은다. JUnit 4 `Suite`로 묶은 클래스(`AllTests`)는
/// Surefire 기본 includes(`*Tests`)에도 들어, 묶음과 각 클래스로 두 번 돌아 두 보고서에 남는다(묶음의 보고서도
/// 테스트마다 실제 클래스를 적는다). 하나라도 실패했으면 실패한 것을 남긴다.
// ponytail: 선형 탐색이라 O(n²). 테스트가 수만 개라 느려지면 (묶음, 이름) 키의 HashMap으로.
pub fn dedup(cases: Vec<Case>) -> Vec<Case> {
    let mut kept: Vec<Case> = Vec::with_capacity(cases.len());
    for case in cases {
        match kept.iter_mut().find(|k| k.same_test(&case)) {
            Some(old) if old.outcome != Outcome::Failed && case.outcome == Outcome::Failed => {
                *old = case
            }
            Some(_) => {}
            None => kept.push(case),
        }
    }
    kept
}

/// 다시 돌린 결과를 앞 결과에 덮어 쓴다(없던 것은 더한다). 같은 테스트도 돌린 곳이 다르면 따로다: Maven은
/// 단위 테스트 단계(Surefire)와 통합 테스트 단계(Failsafe)가 같은 통합 테스트를 둘 다 돌릴 수 있고(pom이 Surefire의
/// `<includes>`에 넣었을 때), Gradle은 작업마다 따로 돈다.
pub fn merge_rerun(cases: &mut Vec<Case>, found: Vec<Case>) {
    for case in found {
        match cases.iter_mut().find(|c| c.same_test(&case)) {
            Some(old) => *old = case,
            None => cases.push(case),
        }
    }
}

/// 실패만 다시의 결과(`found`, 끝난 종료 코드 `code`)를 앞 결과에 넣는다. 다시 돌린 지난 실패가 결과에 없으면
/// 지우거나 이름을 바꾼 것으로 보고 뺀다. 다만 그 테스트가 돌 자리가 돌지 못했으면(빌드 오류) 남긴다.
/// - Maven: 테스트 실패로는 0으로 끝나게 돌리므로(`with_report`) 0이 아니면 어느 모듈의 빌드 오류다. 출력
///   (`output`)의 모듈별 결과(Reactor Summary)에서 성공한 모듈의 실패는 뺀다. 결과가 없으면(모듈 하나) 모두
///   남긴다. `-q`여도 `with_report`가 그 결과를 되살린다.
/// - Gradle: 그 테스트의 작업이 결과를 냈다(컴파일이 깨지면 작업이 돌지 않는다).
/// - Go: 그 패키지가 빌드에 실패하지 않았다(`PACKAGE_CASE`).
/// - pytest: 그 테스트의 파일을 모으지 못하지 않았다(문법 오류 등, `pytest_collect_error`). 다른 파일은
///   `with_report`가 붙인 `--continue-on-collection-errors`로 돈다. `--lf`는 지난 실패를 하나도 찾지 못하면
///   모두 돌린다.
/// - CTest: 늘 돈 것으로 본다. `--rerun-failed`는 번호로 골라 이름을 바꾼 뒤 같은 번호의 다른 테스트를 돌린다.
pub fn apply_rerun(
    tool: Tool,
    code: Option<i32>,
    output: &str,
    cases: &mut Vec<Case>,
    found: Vec<Case>,
) {
    let targets: Vec<Case> = cases
        .iter()
        .filter(|c| c.outcome == Outcome::Failed)
        .cloned()
        .collect();
    apply_selected(tool, code, output, cases, found, &targets);
}

/// `case`가 돌려 달라고 한 `target`에 드는지. 이름이 빈 것은 그 묶음(클래스와 그 안쪽 클래스, 모듈과 그 안의
/// 클래스, 패키지) 전체다. 돌린 곳(Gradle 작업·Maven 모듈)을 모르는 것(에디터에서 고름)은 돌린 곳을 따지지 않는다.
fn targeted(target: &Case, case: &Case) -> bool {
    let place = target.task.is_none() || target.task == case.task;
    if target.name.is_empty() {
        place
            && case
                .suite
                .strip_prefix(target.suite.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '$']))
    } else {
        target.same_test(case)
            || (target.task.is_none() && target.suite == case.suite && target.name == case.name)
    }
}

/// 고른 테스트(`targets`)만 돌린 결과를 앞 결과에 넣는다(`run_only`). 실패만 다시(`apply_rerun`)와 같되, 결과에
/// 없으면 지운 것으로 볼 테스트가 지난 실패가 아니라 고른 테스트다. 고르지 않은 테스트는 그대로 둔다.
pub fn apply_selected(
    tool: Tool,
    code: Option<i32>,
    output: &str,
    cases: &mut Vec<Case>,
    found: Vec<Case>,
    targets: &[Case],
) {
    let built = match tool {
        Tool::Maven if code != Some(0) => maven_built_modules(output).unwrap_or_default(),
        _ => Vec::new(),
    };
    let ran_there = |c: &Case| match tool {
        Tool::Maven => {
            code == Some(0)
                || c.task.as_deref().is_some_and(|module| {
                    // 모듈들의 판이 다르면 이름 뒤에 판을 붙인다(`common 1.0`).
                    built.iter().any(|b| {
                        b.strip_prefix(module)
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
                    })
                })
        }
        Tool::Gradle => c.task.is_some() && found.iter().any(|f| f.task == c.task),
        Tool::Go => !found
            .iter()
            .any(|f| f.suite == c.suite && f.name == PACKAGE_CASE),
        Tool::Pytest => !found.iter().any(|f| pytest_collect_error(f, c)),
        Tool::Ctest => true,
    };
    cases.retain_mut(|c| {
        if !targets.iter().any(|t| targeted(t, c)) || found.iter().any(|f| f.same_test(c)) {
            return true;
        }
        // 다시 돌린 테스트가 결과에 없다: 돌 자리가 돌았으면 지운 것이라 빼고, 아니면 앞 결과로 남긴다.
        c.previous = !ran_there(c);
        c.previous
    });
    merge_rerun(cases, found);
}

/// 실패만 다시가 돌 테스트를 찾지 못해 전체를 다시 돌린 결과(`found`)로 앞 결과를 바꾼다. Gradle은 결과를 낸
/// 작업(이번에 돈 것과 최신이라 건너뛴 것)의 앞 결과만 바꾸고, 결과를 내지 못한 작업(컴파일이 깨진 하위 프로젝트)의
/// 앞 결과는 남긴다. 실패만 다시를 누른 사람에게 그 실패가 사라지면 고친 뒤 다시 돌릴 수 없다. Gradle의 "No tests
/// found"는 다른 하위 프로젝트의 빌드가 깨져도 나므로 이렇게 넘어온다. 다른 도구는 실패만 다시가 0으로 끝났을
/// 때만 넘어와(빌드가 깨지지 않았다) 모두 바꾼다.
pub fn apply_whole_rerun(tool: Tool, cases: &mut Vec<Case>, found: Vec<Case>) {
    if tool == Tool::Gradle {
        cases.retain_mut(|c| {
            c.previous = c.task.is_some() && !found.iter().any(|f| f.task == c.task);
            c.previous
        });
    } else {
        cases.clear();
    }
    cases.extend(found);
}

/// Maven 3의 실행 사건(모듈 머리 줄과 모듈별 결과)을 찍는 로거의 수준 속성. `-q`(모두 ERROR)여도 이 로거만
/// INFO로 두면 받기 기록이나 플러그인 출력은 감춘 채 모듈별 결과가 남는다(Maven 3.10에서 확인). `-q`가 없으면
/// 그 로거는 원래 INFO라 바뀌는 것이 없다.
const MAVEN_EVENT_LOGGER: &str =
    "org.slf4j.simpleLogger.log.org.apache.maven.cli.event.ExecutionEventLogger";

/// Maven 4에서 같은 로거의 수준 속성. 로거가 `org.apache.maven.cling.event`로 옮겨졌고 속성 접두어가
/// `maven.logger.`가 되었다(옛 접두어도 읽지만 경고를 낸다. 4.0.0-rc-7의 소스와 CI에서 확인). Maven 3은 모르는
/// 시스템 속성이라 무시한다.
const MAVEN4_EVENT_LOGGER: &str =
    "maven.logger.log.org.apache.maven.cling.event.ExecutionEventLogger";

/// Maven 출력 끝의 모듈별 결과(Reactor Summary) 줄(`[INFO] common ........ SUCCESS [  1.2 s]`).
static MAVEN_REACTOR_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[INFO\] (.+?) \.*\s*(SUCCESS|FAILURE|SKIPPED)(?:\s|$)").unwrap()
});

/// Maven 출력의 모듈별 결과에서 빌드에 성공한 모듈(`-fae`로 실패한 모듈과 그것에 기대 건너뛴 모듈은 뺀다).
/// 이름은 pom의 `<name>`(없으면 `<artifactId>`)이고, 모듈들의 판이 다르면 뒤에 판이 붙는다. 결과가 없으면
/// (`-q`, 모듈 하나) None.
pub fn maven_built_modules(output: &str) -> Option<Vec<String>> {
    static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());
    let output = ANSI.replace_all(output, "");
    let summary = output.rfind("Reactor Summary")?;
    Some(
        output[summary..]
            .lines()
            .skip(1)
            .filter_map(|line| MAVEN_REACTOR_LINE.captures(line.trim_end()))
            .filter(|c| &c[2] == "SUCCESS")
            .map(|c| c[1].trim().to_owned())
            .collect(),
    )
}

/// `error`가 `case`의 파일을 모으지 못한 오류인지. pytest의 JUnit 보고서는 모으기 오류를 `classname`이 비고
/// `name`이 모듈(`tests.test_b`)인 실패로 적는다. 그 파일의 테스트는 `classname`이 그 모듈(클래스 안이면
/// `tests.test_b.TestX`)이다.
fn pytest_collect_error(error: &Case, case: &Case) -> bool {
    error.suite.is_empty()
        && error.outcome == Outcome::Failed
        && case
            .suite
            .strip_prefix(error.name.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// Go 패키지 자체가 실패했을 때(빌드 실패 등)의 이름.
pub const PACKAGE_CASE: &str = "(패키지)";

/// JUnit XML(`testsuites`·`testsuite`·`testcase`)을 읽는다.
pub fn parse_junit(xml: &str) -> Result<Vec<Case>, String> {
    let document = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut cases = Vec::new();
    for node in document
        .descendants()
        .filter(|n| n.has_tag_name("testcase"))
    {
        let suite = node
            .attribute("classname")
            .map(str::to_owned)
            .or_else(|| {
                node.ancestors()
                    .find(|a| a.has_tag_name("testsuite"))
                    .and_then(|s| s.attribute("name"))
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let child = |tag: &str| node.children().find(|c| c.has_tag_name(tag));
        let text = |n: roxmltree::Node| -> String {
            n.children().filter_map(|c| c.text()).collect::<String>()
        };
        let problem = child("failure").or_else(|| child("error"));
        let skipped = child("skipped");
        // CTest는 결과를 status로 알린다. disabled는 건너뜀이고, notrun(`<skipped message="사유">`)은
        // 돌리지 못한 것(다중 구성 빌드에 `-C` 빠짐, 실행 파일 없음)이라 CTest처럼 실패로 센다. 다만
        // SKIP_RETURN_CODE·SKIP_REGULAR_EXPRESSION으로 건너뛴 것은 건너뜀이다.
        let status = node.attribute("status");
        let reason = skipped.and_then(|s| s.attribute("message"));
        let not_run = status == Some("notrun")
            && !reason.is_some_and(|r| r.to_ascii_uppercase().contains("SKIP"));
        let (outcome, message, mut detail) = match (problem, skipped) {
            (Some(p), _) => (
                Outcome::Failed,
                p.attribute("message").map(str::to_owned),
                text(p),
            ),
            _ if not_run => (
                Outcome::Failed,
                Some(format!("돌리지 못함: {}", reason.unwrap_or("Not Run"))),
                String::new(),
            ),
            (None, Some(s)) => (
                Outcome::Skipped,
                s.attribute("message").map(str::to_owned),
                text(s),
            ),
            (None, None) if status == Some("disabled") => (Outcome::Skipped, None, String::new()),
            (None, None) => (Outcome::Passed, None, String::new()),
        };
        // 실패한 테스트의 출력(CTest는 여기에만 있다).
        if outcome == Outcome::Failed {
            for tag in ["system-out", "system-err"] {
                if let Some(out) = child(tag).map(text).filter(|t| !t.trim().is_empty()) {
                    if !detail.is_empty() && !detail.ends_with('\n') {
                        detail.push('\n');
                    }
                    detail.push_str(&out);
                }
            }
        }
        let message = message
            .or_else(|| {
                detail
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_owned)
            })
            .map(|m| m.lines().next().unwrap_or_default().trim().to_owned())
            .filter(|m| !m.is_empty());
        cases.push(Case {
            suite,
            name: node.attribute("name").unwrap_or_default().to_owned(),
            outcome,
            seconds: node.attribute("time").and_then(|t| t.parse().ok()),
            message,
            detail,
            file: node.attribute("file").map(str::to_owned),
            line: node.attribute("line").and_then(|l| l.parse().ok()),
            integration: false,
            task: None,
            previous: false,
        });
    }
    Ok(cases)
}

/// `go test -json`(test2json)의 출력을 읽는다. JSON이 아닌 줄은 건너뛴다.
pub fn parse_go_json(output: &str) -> Vec<Case> {
    struct Running {
        package: String,
        name: String,
        output: String,
    }
    let mut running: Vec<Running> = Vec::new();
    let mut packages: Vec<(String, String)> = Vec::new();
    let mut cases = Vec::new();
    for line in output.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let text = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or_default();
        let (action, package, test) = (text("Action"), text("Package"), text("Test"));
        // Go 1.24부터 빌드 출력은 따로 온다(`ImportPath`는 "패키지 [패키지.test]").
        if action == "build-output" {
            let package = text("ImportPath").split(' ').next().unwrap_or_default();
            match packages.iter_mut().find(|(p, _)| p == package) {
                Some((_, out)) => out.push_str(text("Output")),
                None => packages.push((package.to_owned(), text("Output").to_owned())),
            }
            continue;
        }
        if test.is_empty() {
            match action {
                "output" => match packages.iter_mut().find(|(p, _)| p == package) {
                    Some((_, out)) => out.push_str(text("Output")),
                    None => packages.push((package.to_owned(), text("Output").to_owned())),
                },
                // 패키지가 실패했는데 실패한 테스트가 없으면 빌드·초기화 실패다.
                "fail"
                    if !cases
                        .iter()
                        .any(|c: &Case| c.suite == package && c.outcome == Outcome::Failed) =>
                {
                    let detail = packages
                        .iter()
                        .find(|(p, _)| p == package)
                        .map(|(_, o)| o.clone())
                        .unwrap_or_default();
                    cases.push(Case {
                        suite: package.to_owned(),
                        name: PACKAGE_CASE.to_owned(),
                        outcome: Outcome::Failed,
                        seconds: event.get("Elapsed").and_then(Value::as_f64),
                        message: detail
                            .lines()
                            .find(|l| !l.trim().is_empty())
                            .map(|l| l.trim().to_owned()),
                        detail,
                        file: None,
                        line: None,
                        integration: false,
                        task: None,
                        previous: false,
                    });
                }
                _ => {}
            }
            continue;
        }
        let index = running
            .iter()
            .position(|r| r.package == package && r.name == test);
        match action {
            "run" if index.is_none() => running.push(Running {
                package: package.to_owned(),
                name: test.to_owned(),
                output: String::new(),
            }),
            "output" => {
                let out = text("Output");
                // 실행 표시 줄은 뺀다.
                let noise = ["=== RUN", "=== PAUSE", "=== CONT", "=== NAME"]
                    .iter()
                    .any(|n| out.starts_with(n));
                match index {
                    Some(i) if !noise => running[i].output.push_str(out),
                    _ => {}
                }
            }
            "pass" | "fail" | "skip" => {
                let output = index.map(|i| running.remove(i).output).unwrap_or_default();
                let outcome = match action {
                    "pass" => Outcome::Passed,
                    "fail" => Outcome::Failed,
                    _ => Outcome::Skipped,
                };
                // t.Error의 글(`    calc_test.go:9: 글`)을 사유로 쓴다.
                let message = output.lines().find_map(|l| {
                    GO_FRAME
                        .captures(l)
                        .map(|c| l[c.get(0).unwrap().end()..].trim().to_owned())
                        .filter(|m| !m.is_empty())
                });
                cases.push(Case {
                    suite: package.to_owned(),
                    name: test.to_owned(),
                    outcome,
                    seconds: event.get("Elapsed").and_then(Value::as_f64),
                    message: (outcome != Outcome::Passed).then_some(message).flatten(),
                    detail: output,
                    file: None,
                    line: None,
                    integration: false,
                    task: None,
                    previous: false,
                });
            }
            _ => {}
        }
    }
    cases
}

/// 보고서를 남기게 고친 명령. pytest·CTest는 `dir`에 JUnit XML을 쓰게 하고, Go는 `-json`을 붙인다.
/// Maven·Gradle은 원래 보고서를 남긴다. Gradle은 첫 실패에서 남은 작업을 멈추므로 `--continue`를 붙여
/// 한 하위 프로젝트의 테스트가 실패해도 다른 하위 프로젝트의 테스트를 돌린다(실패만 다시 돌릴 때 걸러 낸
/// 테스트가 없는 하위 프로젝트도 "No tests found"로 실패한다). Maven도 첫 모듈의 실패에서 멈춘다.
/// `-fae`만으로는 실패한 모듈에 기대는 모듈을 건너뛰므로, 테스트 실패로는 모듈을 실패시키지 않게 하고
/// (`maven.test.failure.ignore`, 결과는 보고서로 가린다) 빌드 오류에도 상관없는 모듈은 돌게 `-fae`를 붙인다.
/// Maven의 `-q`는 모듈별 결과(Reactor Summary)도 감추므로 그 줄을 찍는 로거만 되살린다(`MAVEN_EVENT_LOGGER`).
/// `-q`는 명령 줄 말고도 `.mvn/maven.config`·`MAVEN_ARGS`에 적을 수 있어 늘 붙인다.
pub fn with_report(tool: Tool, command: &str, dir: &Path) -> String {
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let has = |flags: &[&str]| command.split_whitespace().any(|w| flags.contains(&w));
    match tool {
        Tool::Maven => {
            let mut line = command.to_owned();
            if !has(&[
                "-fae",
                "--fail-at-end",
                "-ff",
                "--fail-fast",
                "-fn",
                "--fail-never",
            ]) {
                line.push_str(" -fae");
            }
            if !command.contains("maven.test.failure.ignore") {
                line.push_str(" -Dmaven.test.failure.ignore=true");
            }
            for logger in [MAVEN_EVENT_LOGGER, MAVEN4_EVENT_LOGGER] {
                if !command.contains(logger) {
                    line.push_str(&format!(" -D{logger}=info"));
                }
            }
            line
        }
        Tool::Pytest => {
            let mut line = command.to_owned();
            if !command.contains("--junitxml") {
                line.push_str(&format!(" \"--junitxml={}\"", path("pytest.xml")));
            }
            // 한 파일을 모으지 못하면(문법 오류 등) 기본은 세션을 멈춰 아무 테스트도 돌지 않는다. 다른 파일은 돌게 한다.
            if !has(&["--continue-on-collection-errors"]) {
                line.push_str(" --continue-on-collection-errors");
            }
            line
        }
        Tool::Ctest if !command.contains("--output-junit") => {
            format!("{command} --output-junit \"{}\"", path("ctest.xml"))
        }
        Tool::Go if !has(&["-json"]) => command.replacen("test", "test -json", 1),
        Tool::Gradle => {
            let mut line = gradle_with_task_lines(command);
            if !has(&["--continue"]) {
                line.push_str(" --continue");
            }
            line.push(' ');
            line.push_str(GRADLE_TASK_LINES);
            line
        }
        _ => command.to_owned(),
    }
}

/// 건너뛴 작업을 알려 주는 작업 줄(`> Task :app:test UP-TO-DATE`)을 남기는 옵션. `gradle.properties`(프로젝트·
/// Gradle 홈)의 `org.gradle.logging.level=quiet`·`org.gradle.console=rich`를 덮는다. 명령 줄의 `-q`·`--console`은
/// 이것보다 앞서므로 `gradle_with_task_lines`가 뺀다(Gradle 9.2.0으로 확인).
const GRADLE_TASK_LINES: &str = "--console=plain -Dorg.gradle.logging.level=lifecycle";

/// 건너뛴 작업을 알려 주는 작업 줄을 숨기는 옵션을 뺀다. `-q`·`-w`는 줄을 없애고, `--console`은 꼴을 바꾼다.
/// 로그 수준 속성은 `GRADLE_TASK_LINES`가 다시 준다. 값 안의 빈칸을 그대로 두려고 한 칸씩 나눈다.
fn gradle_with_task_lines(command: &str) -> String {
    let mut kept = Vec::new();
    let mut words = command.split(' ');
    while let Some(word) = words.next() {
        match word {
            "-q" | "--quiet" | "-w" | "--warn" => {}
            "--console" => {
                words.next();
            }
            _ if word.starts_with("--console=")
                || word.starts_with("-Dorg.gradle.logging.level=") => {}
            _ => kept.push(word),
        }
    }
    kept.join(" ")
}

/// 종료 코드로 보아 보고서에 없는 실패(빌드 오류 등)가 있는지. Maven은 테스트 실패로는 빌드를 멈추지 않게
/// 돌리므로(`with_report`) 0이 아니면 빌드 오류다. 다른 도구는 실패한 테스트가 없는데 0이 아닐 때다.
/// `-fae`·`--continue`로 다른 모듈의 결과가 보여도 컴파일이 깨진 모듈은 결과에 없다.
pub fn unreported_failure(tool: Tool, code: Option<i32>, failed: usize) -> bool {
    code.is_some_and(|c| c != 0) && (tool == Tool::Maven || failed == 0)
}

/// Gradle의 실패만 다시(`--tests`)가 돌릴 테스트를 찾지 못했는지. 빌드 설정이 묶음만 돌리면(`filter`의
/// `includeTestsMatching("*AllTests")`, `include("**/AllTests.class")`) 그 거르기와 `--tests`를 둘 다 만족하는
/// 테스트가 없다. 보고서에는 묶음 안의 실제 클래스가 적혀 있어 그 이름으로 거르기 때문이다.
pub fn gradle_rerun_found_nothing(tool: Tool, output: &str) -> bool {
    tool == Tool::Gradle && output.contains("No tests found for given includes")
}

/// Windows의 Gradle(JDK 18 이상)이 경로에 든 한글 등 ASCII가 아닌 글자 때문에 테스트를 돌리지 못한 곳.
/// Gradle은 테스트 JVM의 클래스 경로를 인자 파일(`@…gradle-worker-classpath…txt`)로 넘기는데, JDK 18부터
/// 기본 문자셋이 UTF-8이라(JEP 400) 그 파일을 UTF-8로 쓰고 Windows의 java는 시스템 코드 페이지(한국어
/// 949)로 읽는다. 그러면 테스트마다가 아니라 "Gradle Test …"의 실패 하나로 온다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GradlePathProblem {
    /// 프로젝트 경로: 테스트 클래스를 찾지 못했다("Could not execute test class"와 `ClassNotFoundException`).
    Project,
    /// Gradle 홈(`GRADLE_USER_HOME`, 기본은 사용자 폴더의 `.gradle`): 테스트 JVM이 뜨지 못했다("Gradle Test
    /// Executor"가 0이 아닌 코드로 끝났다). 그 글자를 시스템 코드 페이지가 나타내면(한국어 Windows) 인자 파일을
    /// 열되 받아 둔 Gradle의 클래스(`GradleWorkerMain`)를 찾지 못하고, 나타내지 못하면(영문 Windows) 홈 아래의
    /// 인자 파일(`gradle-worker-classpath…`)부터 열지 못한다. 까닭은 출력에만 나온다. Windows 사용자 이름이
    /// 한글이면 기본 홈이 그렇다.
    Home(PathBuf),
}

/// 테스트가 경로의 한글 때문에 돌지 못했는지(`GradlePathProblem`). `dir`은 돌린 폴더, `home`은 Gradle 홈,
/// `output`은 Gradle의 출력(테스트 JVM이 뜨지 못한 까닭은 보고서가 아니라 출력에만 나온다).
pub fn gradle_path_problem(
    tool: Tool,
    dir: &Path,
    home: &Path,
    cases: &[Case],
    output: &str,
) -> Option<GradlePathProblem> {
    if !cfg!(windows) || tool != Tool::Gradle {
        return None;
    }
    let ascii = |path: &Path| path.to_string_lossy().is_ascii();
    let failed = |message: &str| {
        cases.iter().any(|c| {
            c.outcome == Outcome::Failed
                && c.message.as_deref().is_some_and(|m| m.contains(message))
        })
    };
    let worker_failed =
        output.contains("GradleWorkerMain") || output.contains("gradle-worker-classpath");
    if !ascii(home) && failed("Gradle Test Executor") && worker_failed {
        return Some(GradlePathProblem::Home(home.to_path_buf()));
    }
    let class_missing = cases.iter().any(|c| {
        c.outcome == Outcome::Failed
            && c.message
                .as_deref()
                .is_some_and(|m| m.contains("Could not execute test class"))
            && c.detail.contains("ClassNotFoundException")
    });
    (!ascii(dir) && class_missing).then_some(GradlePathProblem::Project)
}

/// Gradle 홈: 테스트를 돌린 환경(`env`, 실행 구성의 환경 변수)의 `GRADLE_USER_HOME`, 없으면 이 프로세스의 것,
/// 그것도 없으면 사용자 폴더의 `.gradle`(Gradle의 기본).
pub fn gradle_user_home(env: &std::collections::BTreeMap<String, String>) -> Option<PathBuf> {
    if let Some(home) = env.get("GRADLE_USER_HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(home) = std::env::var_os("GRADLE_USER_HOME") {
        return Some(PathBuf::from(home));
    }
    let user = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(user).map(|home| PathBuf::from(home).join(".gradle"))
}

/// pom.xml의 maven-surefire-plugin 설정에 박힌 `<skipTests>true</skipTests>`의 바이트 범위. egov 템플릿의
/// 기본값이고, 설정에 직접 적은 값이라 명령 줄(`-DskipTests=false`)로는 켤 수 없다.
pub fn pom_skips_tests(pom: &str) -> Option<std::ops::Range<usize>> {
    let doc = roxmltree::Document::parse(pom).ok()?;
    let is = |n: roxmltree::Node, value: &str| n.text().map(str::trim) == Some(value);
    doc.descendants()
        .filter(|n| n.tag_name().name() == "plugin")
        .filter(|p| {
            p.children()
                .any(|c| c.tag_name().name() == "artifactId" && is(c, "maven-surefire-plugin"))
        })
        .flat_map(|p| p.descendants())
        .find(|n| n.tag_name().name() == "skipTests" && is(*n, "true"))
        .map(|n| n.range())
}

/// 그 설정을 지운 pom(그 줄에 그것만 있으면 줄째). 그 뒤로는 Maven 기본대로 테스트를 돌리고,
/// 건너뛰려면 `-DskipTests`를 쓴다.
pub fn enable_tests(pom: &str) -> Option<String> {
    let range = pom_skips_tests(pom)?;
    let line_start = pom[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = pom[range.end..]
        .find('\n')
        .map_or(pom.len(), |i| range.end + i + 1);
    let alone = pom[line_start..range.start].trim().is_empty()
        && pom[range.end..line_end].trim().is_empty();
    let cut = if alone { line_start..line_end } else { range };
    Some(format!("{}{}", &pom[..cut.start], &pom[cut.end..]))
}

/// 보고서 파일 하나(JUnit XML)를 읽는다. Maven Failsafe의 보고서(`failsafe-reports`)면 통합 테스트로 적고,
/// Gradle의 보고서(`<프로젝트>/build/test-results/<작업>`)면 그것을 돌린 작업을, Maven의 보고서면 모듈을 적는다.
pub fn read_report(path: &Path) -> Result<Vec<Case>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut cases = parse_junit(&String::from_utf8_lossy(&bytes))?;
    let integration = path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|dir| dir == "failsafe-reports");
    let task = gradle_task(path).or_else(|| maven_module(path));
    for case in &mut cases {
        case.integration = integration;
        case.task.clone_from(&task);
    }
    undo_display_names(path, gradle_task(path).is_some(), &mut cases);
    Ok(cases)
}

/// JUnit 5 `@DisplayName`의 표시 이름을 소스에서 찾아 진짜 이름으로 되돌린다. 그래야 다시 돌리기·디버그가 그
/// 테스트를 거른다. Gradle은 테스트 이름에(`두 수를 더한다` → `adds()`), Surefire는 클래스 이름에(`계산기` →
/// `demo.CalcTest`, `@Nested`는 `demo.CalcTest$Inner`) 표시 이름을 적는다. 보고서 파일 이름
/// (`TEST-demo.CalcTest.xml`)의 클래스로 소스를 찾는다. 못 찾은 것(매개변수 실행의 `[1] 5`)은 그대로 둔다.
fn undo_display_names(path: &Path, gradle: bool, cases: &mut [Case]) {
    let Some(class) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("TEST-")?.strip_suffix(".xml"))
    else {
        return;
    };
    let top = class.split('$').next().unwrap_or(class);
    // 이 보고서의 클래스나 그 안쪽 클래스.
    let in_report = |suite: &str| suite == top || suite.starts_with(&format!("{top}$"));
    let name_display = |case: &Case| gradle && !case.name.is_empty() && !case.name.ends_with(')');
    let is_display = |case: &Case| !in_report(&case.suite) || name_display(case);
    if !cases.iter().any(is_display) {
        return;
    }
    let Some(project) = path
        .ancestors()
        .find(|a| a.file_name().is_some_and(|n| n == "build" || n == "target"))
        .and_then(Path::parent)
    else {
        return;
    };
    let simple = top.rsplit('.').next().unwrap_or(top);
    let Some(text) = ["java", "kt"].iter().find_map(|ext| {
        let frame = Frame {
            text: String::new(),
            file: format!("{simple}.{ext}"),
            line: 0,
            class: Some(top.to_owned()),
        };
        std::fs::read_to_string(resolve(project, project, &frame)?).ok()
    }) else {
        return;
    };
    let names = crate::test_at::jvm_display_names(&text);
    for case in cases.iter_mut().filter(|c| is_display(c)) {
        // 표시 이름이 겹치면 이 보고서의 클래스의 것을 고른다.
        if !in_report(&case.suite)
            && let Some((_, at)) = names
                .iter()
                .find(|(d, at)| *d == case.suite && at.name.is_empty() && in_report(&at.suite))
        {
            case.suite.clone_from(&at.suite);
        }
        if name_display(case)
            && let Some((_, at)) = names
                .iter()
                .find(|(d, at)| *d == case.name && at.suite == case.suite && !at.name.is_empty())
        {
            case.name = format!("{}()", at.name);
        }
    }
}

/// Maven 보고서(`<모듈>/target/surefire-reports/TEST-….xml`, `failsafe-reports`)를 남긴 모듈의 이름: 그 pom의
/// `<name>`, 없으면 `<artifactId>`(Maven이 모듈별 결과에 적는 이름. `<name>`은 물려받지 않는다). 식(`${…}`)을
/// 쓴 `<name>`은 풀지 않고 `<artifactId>`로 갈음한다(그 모듈은 결과와 맞지 않아 실패를 남긴다).
fn maven_module(path: &Path) -> Option<String> {
    let reports = path.parent()?;
    if !matches!(
        reports.file_name()?.to_str()?,
        "surefire-reports" | "failsafe-reports"
    ) {
        return None;
    }
    let target = reports.parent()?;
    if target.file_name()? != "target" {
        return None;
    }
    let pom = std::fs::read_to_string(target.parent()?.join("pom.xml")).ok()?;
    let document = roxmltree::Document::parse(&pom).ok()?;
    let child = |tag: &str| {
        document
            .root_element()
            .children()
            .find(|c| c.has_tag_name(tag))
            .and_then(|c| c.text())
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    child("name")
        .filter(|name| !name.contains("${"))
        .or_else(|| child("artifactId"))
        .map(str::to_owned)
}

/// 테스트 소스 파일(`<프로젝트>/src/<소스 세트>/…`)을 돌리는 Gradle 테스트 작업의 경로(`:app:test`, 소스 세트가
/// `integrationTest`면 `:app:integrationTest`, 루트 프로젝트면 `:test`). 테스트 작업은 소스 세트 이름을 따른다고
/// 본다(java 플러그인의 `test`, `jvm-test-suite`의 스위트). 프로젝트는 위로 가장 가까운 `build.gradle(.kts)`가
/// 있는 폴더다. 그 아래 `src/<소스 세트>/` 안이 아니면 None.
pub fn gradle_test_task(file: &Path) -> Option<String> {
    let project = file
        .ancestors()
        .skip(1)
        .find(|d| d.join("build.gradle").is_file() || d.join("build.gradle.kts").is_file())?;
    let mut parts = file.strip_prefix(project).ok()?.components();
    if parts.next()?.as_os_str() != "src" {
        return None;
    }
    let set = parts.next()?.as_os_str().to_str()?.to_owned();
    parts.next()?;
    Some(format!("{}:{set}", gradle_project_path(project)?))
}

/// Gradle 보고서(`<프로젝트>/build/test-results/<작업>/TEST-….xml`)를 남긴 작업의 경로(`:app:test`, 루트
/// 프로젝트면 `:test`).
fn gradle_task(path: &Path) -> Option<String> {
    let task_dir = path.parent()?;
    let results = task_dir.parent()?;
    let build = results.parent()?;
    if results.file_name()? != "test-results" || build.file_name()? != "build" {
        return None;
    }
    let project = gradle_project_path(build.parent()?)?;
    Some(format!(
        "{project}:{}",
        task_dir.file_name()?.to_string_lossy()
    ))
}

/// 프로젝트 폴더의 Gradle 경로(`:app`, 루트 프로젝트는 빈 글). 빌드의 루트는 위로 가장 가까운
/// `settings.gradle(.kts)`가 있는 폴더(없으면 그 프로젝트)다.
/// - 설정이 폴더를 적은 프로젝트(`project(":app").projectDir = file("modules/application")`)는 그 경로다.
///   나머지는 루트에서 그 폴더까지의 폴더 이름이다.
/// - 그 루트를 바깥 빌드가 품으면(`includeBuild("lib")`) 품은 빌드의 이름(폴더 이름)을 앞에 붙인다. 바깥 루트에서
///   `:lib:core:test`로 돈다(Gradle 9.2.0으로 확인).
///
/// 설정은 글로만 읽는다. 반복문으로 정한 폴더, 폴더를 바꾼 프로젝트의 하위 프로젝트(Gradle은 `include`한 때의
/// 부모 폴더로 정한다), 이름을 바꾼 프로젝트·품은 빌드는 맞지 않을 수 있다(Gradle이 작업을 찾지 못한다고 알린다).
fn gradle_project_path(project_dir: &Path) -> Option<String> {
    let Some((root, settings)) = gradle_settings_above(project_dir) else {
        return Some(String::new());
    };
    let here = std::fs::canonicalize(project_dir).ok();
    let named = GRADLE_PROJECT_DIR
        .captures_iter(&settings)
        .find(|c| here.is_some() && std::fs::canonicalize(root.join(&c[2])).ok() == here);
    let project = match named {
        Some(c) if c[1].starts_with(':') => c[1].to_owned(),
        Some(c) => format!(":{}", &c[1]),
        None => project_dir
            .strip_prefix(&root)
            .ok()?
            .components()
            .map(|part| format!(":{}", part.as_os_str().to_string_lossy()))
            .collect(),
    };
    let included =
        root.parent()
            .and_then(gradle_settings_above)
            .is_some_and(|(outer, settings)| {
                let me = std::fs::canonicalize(&root).ok();
                GRADLE_INCLUDE_BUILD
                    .captures_iter(&settings)
                    .any(|c| me.is_some() && std::fs::canonicalize(outer.join(&c[1])).ok() == me)
            });
    Some(match (included, root.file_name()) {
        (true, Some(build)) => format!(":{}{project}", build.to_string_lossy()),
        _ => project,
    })
}

/// `dir`에서 위로 가장 가까운 Gradle 설정 파일의 폴더와 글(줄 주석·블록 주석은 뺀다). 설정은 MS949 주석이 있어도
/// 읽는다(깨진 글자는 바꾼다).
fn gradle_settings_above(dir: &Path) -> Option<(PathBuf, String)> {
    dir.ancestors().find_map(|dir| {
        ["settings.gradle", "settings.gradle.kts"]
            .iter()
            .find_map(|file| std::fs::read(dir.join(file)).ok())
            .map(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                (
                    dir.to_path_buf(),
                    GRADLE_COMMENT.replace_all(&text, "").into_owned(),
                )
            })
    })
}

/// 설정의 주석: 블록 주석과 줄 전체가 주석인 줄(문자열 속 `//`는 건드리지 않게 줄 끝 주석은 둔다).
static GRADLE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?ms)/\*.*?\*/|^[ \t]*//[^\n]*").unwrap());
/// 프로젝트 폴더를 적은 줄: `project(":app").projectDir = file("modules/app")`,
/// `project(':app').projectDir = new File(settingsDir, 'modules/app')`(`rootDir`·`File(…)`도). (경로, 폴더).
static GRADLE_PROJECT_DIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"project\s*\(\s*["']([^"']+)["']\s*\)\s*\.\s*projectDir\s*=\s*(?:new\s+)?(?:file|File)\s*\(\s*(?:(?:settingsDir|rootDir|rootProject\.projectDir)\s*,\s*)?["']([^"'$]+)["']\s*\)"#).unwrap()
});
/// 품은 빌드: `includeBuild("lib")`, `includeBuild 'lib'`. (폴더).
static GRADLE_INCLUDE_BUILD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"includeBuild\s*\(?\s*["']([^"'$]+)["']"#).unwrap());

/// Gradle 명령이 테스트를 돌리는지: `check`, 또는 이름이 `test`로 끝나는 작업
/// (`integrationTest`, `:app:test`). 옵션의 값(`-x test`)은 작업이 아니다. `build`는 테스트를 거치기만 하므로 Maven의
/// `install`처럼 보지 않는다(자동 감지한 `gradle build`가 테스트 창의 기본 구성이 되지 않게).
fn gradle_runs_tests(words: &[String]) -> bool {
    gradle_tasks(words).any(|task| {
        let name = task.rsplit(':').next().unwrap_or(task);
        name == "check" || name.ends_with("test")
    })
}

/// 값을 다음 낱말로 받는 Gradle 옵션.
const GRADLE_OPTIONS_WITH_VALUE: &[&str] = &[
    "-p",
    "--project-dir",
    "-c",
    "--settings-file",
    "-I",
    "--init-script",
    "-g",
    "--gradle-user-home",
    "-x",
    "--exclude-task",
    "--console",
    "--warning-mode",
    "--include-build",
    "--project-cache-dir",
    "--priority",
    "--tests",
];

/// 명령의 낱말(첫 낱말은 프로그램)에서 작업 이름들. 옵션과 그 값은 뺀다.
fn gradle_tasks(words: &[String]) -> impl Iterator<Item = &str> {
    let mut takes_value = false;
    words.iter().skip(1).filter_map(move |word| {
        let word = word.as_str();
        if std::mem::take(&mut takes_value) {
            return None;
        }
        if word.starts_with('-') {
            takes_value = GRADLE_OPTIONS_WITH_VALUE.contains(&word);
            return None;
        }
        Some(word)
    })
}

/// Gradle 명령에서 작업 이름과 이미 있던 `--tests`(그 값까지)를 뺀 것: 프로그램과 옵션.
fn gradle_without_tasks(command: &str) -> String {
    let mut kept = Vec::new();
    let mut words = command.split_whitespace();
    if let Some(program) = words.next() {
        kept.push(program);
    }
    let mut value_of: Option<&str> = None;
    for word in words {
        if let Some(option) = value_of.take() {
            if option != "--tests" {
                kept.push(word);
            }
            continue;
        }
        if word.starts_with('-') {
            if GRADLE_OPTIONS_WITH_VALUE.contains(&word) {
                value_of = Some(word);
            }
            if word != "--tests" && !word.starts_with("--tests=") {
                kept.push(word);
            }
        }
    }
    kept.join(" ")
}

/// 이번 실행이 남긴 보고서 파일(`since` 뒤에 고친 것). Maven은 `target/surefire-reports`·
/// `failsafe-reports`, Gradle은 `build/test-results`, pytest·CTest는 `dir`. Go는 표준 출력으로 준다.
pub fn report_files(tool: Tool, root: &Path, dir: &Path, since: SystemTime) -> Vec<PathBuf> {
    // 파일 시각의 단위가 거친 파일 시스템(FAT 2초)도 있어 조금 앞당긴다.
    let since = since.checked_sub(Duration::from_secs(2)).unwrap_or(since);
    let fresh = |path: &Path| {
        path.extension().is_some_and(|e| e == "xml")
            && std::fs::metadata(path)
                .and_then(|m| m.modified())
                .is_ok_and(|m| m >= since)
    };
    let mut files = match tool {
        Tool::Maven | Tool::Gradle => {
            let mut found = Vec::new();
            crate::files::walk(root, 12, &mut |path, is_dir| {
                if is_dir {
                    return true;
                }
                let parent = |n: usize| {
                    path.ancestors()
                        .nth(n)
                        .and_then(Path::file_name)
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_owned()
                };
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let report = match tool {
                    Tool::Maven => {
                        matches!(parent(1).as_str(), "surefire-reports" | "failsafe-reports")
                            && name.starts_with("TEST-")
                    }
                    _ => path.ancestors().any(|a| {
                        a.file_name().is_some_and(|n| n == "test-results")
                            && a.parent()
                                .and_then(Path::file_name)
                                .is_some_and(|n| n == "build")
                    }),
                };
                if report && fresh(path) {
                    found.push(path.to_path_buf());
                }
                true
            });
            found
        }
        Tool::Pytest | Tool::Ctest => std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| fresh(p))
            .collect(),
        Tool::Go => Vec::new(),
    };
    files.sort();
    files
}

/// 테스트 JVM이 시작에서 멈춰 디버거를 기다리게(JDWP, `run::JDWP_PORT`) 고친 명령(`line`은 `run_only`의 것).
/// Maven은 Surefire의 `maven.surefire.debug`(통합 테스트면 Failsafe의 `maven.failsafe.debug`), Gradle은 테스트 작업 옵션 `--debug-jvm`이고 둘 다 기본이 5005에서
/// 기다리는 것이다. `--debug-jvm`은 앞 작업에 붙으므로 작업과 `--tests` 뒤인 끝에 둔다. Gradle은 같은 거르기로 앞서
/// 통과한 작업을 최신으로 건너뛰어 JVM을 띄우지 않으므로 `--rerun-tasks`도 붙인다. 다른 도구는 None.
// ponytail: --rerun-tasks는 그 작업이 기대는 컴파일까지 다시 한다. Gradle 7.6+만 받는 작업 옵션 --rerun이면 테스트만.
pub fn with_jvm_debug(tool: Tool, line: &str) -> Option<String> {
    match tool {
        // 통합 테스트(`-Dit.test`, 결과 트리에서 고른 것)는 Failsafe가 띄운다. 그때 Surefire에는 걸러 돌 것이 없다.
        Tool::Maven if line.contains("-Dit.test=") => {
            Some(format!("{line} -Dmaven.failsafe.debug"))
        }
        Tool::Maven => Some(format!("{line} -Dmaven.surefire.debug")),
        Tool::Gradle => Some(format!("{line} --debug-jvm --rerun-tasks")),
        _ => None,
    }
}

/// Gradle 출력의 건너뛴 작업 줄(`> Task :app:test UP-TO-DATE`).
static GRADLE_SKIPPED_TASK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^> Task (\S+) (?:UP-TO-DATE|FROM-CACHE)\s*$").unwrap());

/// Gradle이 최신으로 보고 건너뛴 테스트 작업의 보고서. 앞선 실행이 남긴 그대로라 시각으로 고르는
/// `report_files`에 들지 않는다. 작업 줄은 `with_report`가 남긴다.
pub fn gradle_skipped_reports(root: &Path, output: &str) -> Vec<PathBuf> {
    let skipped: Vec<&str> = GRADLE_SKIPPED_TASK
        .captures_iter(output)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect();
    if skipped.is_empty() {
        return Vec::new();
    }
    report_files(Tool::Gradle, root, root, SystemTime::UNIX_EPOCH)
        .into_iter()
        .filter(|path| gradle_task(path).is_some_and(|task| skipped.contains(&task.as_str())))
        .collect()
}

/// 고른 테스트(`cases`)만 돌리는 명령. Maven·Gradle·Go는 실패만 다시(`rerun`)와 같은 거르기다. pytest는 노드
/// ID(`파일::클래스::이름`)로 고른다. 결과의 묶음(`classname`, `tests.test_a.TestX`)에서 돌린 폴더(`dir`) 아래의
/// 파일(`tests/test_a.py`)을 찾고, 못 찾으면 이름으로 거른다(`-k`). CTest는 이름의 정규식(`-R`)이다.
pub fn run_only(tool: Tool, command: &str, cases: &[&Case], dir: &Path) -> Option<String> {
    if cases.is_empty() {
        return None;
    }
    match tool {
        Tool::Pytest => {
            let ids: Option<Vec<String>> = cases.iter().map(|c| pytest_node_id(c, dir)).collect();
            Some(match ids {
                Some(ids) => {
                    let mut line = command.to_owned();
                    for id in dedup_strings(ids) {
                        line.push_str(&format!(" \"{id}\""));
                    }
                    line
                }
                None => {
                    // 이름이 빈 것(묶음 전체)은 묶음의 끝 마디(모듈·클래스 이름)로 거른다.
                    let names: Vec<String> = cases
                        .iter()
                        .map(|c| match c.name.split('[').next().unwrap_or(&c.name) {
                            "" => c.suite.rsplit('.').next().unwrap_or(&c.suite).to_owned(),
                            name => name.to_owned(),
                        })
                        .collect();
                    format!("{command} -k \"{}\"", dedup_strings(names).join(" or "))
                }
            })
        }
        Tool::Ctest => {
            let names: Vec<String> = cases.iter().map(|c| regex::escape(&c.name)).collect();
            Some(format!(
                "{command} -R \"^({})$\"",
                dedup_strings(names).join("|")
            ))
        }
        _ => rerun(tool, command, cases),
    }
}

/// 순서를 지키며 겹친 것을 뺀다.
fn dedup_strings(items: Vec<String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        if !kept.contains(&item) {
            kept.push(item);
        }
    }
    kept
}

/// pytest 결과의 노드 ID. 묶음(`tests.test_a`, 클래스 안이면 `tests.test_a.TestX`)의 앞쪽을 `dir` 아래 파일로
/// 맞춰 본다(가장 긴 것부터). 파일이 없으면 None.
fn pytest_node_id(case: &Case, dir: &Path) -> Option<String> {
    let parts: Vec<&str> = case.suite.split('.').collect();
    (1..=parts.len()).rev().find_map(|k| {
        let file = format!("{}.py", parts[..k].join("/"));
        dir.join(&file).is_file().then(|| {
            let mut id = file;
            for class in &parts[k..] {
                id.push_str("::");
                id.push_str(class);
            }
            // 이름이 빈 것은 그 파일·클래스 전체다.
            if !case.name.is_empty() {
                id.push_str("::");
                id.push_str(&case.name);
            }
            id
        })
    })
}

/// 실패한 것만 다시 돌리는 명령: Maven `-Dtest`(통합 테스트는 `-Dit.test`), Gradle `--tests`, pytest `--lf`,
/// CTest `--rerun-failed`, Go `-run`.
pub fn rerun(tool: Tool, command: &str, failed: &[&Case]) -> Option<String> {
    if failed.is_empty() {
        return None;
    }
    // JUnit 5의 매개변수 테스트 이름(`adds(int)[1]`)과 Gradle의 `()`를 뺀다.
    let method = |name: &str| name.split(['(', '[']).next().unwrap_or(name).to_owned();
    // JUnit 5 `@Nested` 안쪽 클래스(`CalcTest$Inner`)는 `$`까지 적어야 그것만 거른다(떼면 바깥 클래스의 같은
    // 이름 테스트가 돈다). 명령은 셸로 띄우고 sh는 큰따옴표 안의 `$Inner`를 변수로 풀므로 `*`로 적는다(Maven
    // Surefire 3.5.4·Gradle 9.2가 `CalcTest*Inner`로 안쪽 클래스만 골랐다).
    let nested = |class: &str| class.replace('$', "*");
    // Gradle `--tests`의 거르기. 이름이 빈 것(묶음 전체, `run_only`)은 클래스만. 메서드를 모르는 이름(Gradle이
    // 표시 이름만 적는 매개변수 실행 `[1] 5`)도 클래스째 돌린다.
    let gradle_test = |case: &Case| {
        if method(&case.name).is_empty() {
            nested(&case.suite)
        } else {
            format!("{}.{}", nested(&case.suite), method(&case.name))
        }
    };
    Some(match tool {
        Tool::Maven => {
            // `클래스#메서드+메서드,…`(Surefire·Failsafe의 거르기 꼴).
            let filter = |cases: &[&&Case]| {
                let mut classes: Vec<(String, Vec<String>)> = Vec::new();
                for case in cases {
                    let class = nested(case.suite.rsplit('.').next().unwrap_or(&case.suite));
                    // Kotlin 백틱 이름(`adds two numbers, again`): Surefire는 `,`(클래스 사이)·`+`(메서드
                    // 사이)로 가르므로 아무 한 글자인 `?`로 적는다.
                    let name = method(&case.name).replace([',', '+'], "?");
                    match classes.iter_mut().find(|(c, _)| *c == class) {
                        Some((_, names)) if !names.contains(&name) => names.push(name),
                        Some(_) => {}
                        None => classes.push((class, vec![name])),
                    }
                }
                // 이름이 빈 것(클래스 전체, `run_only`)이 있으면 그 클래스를 통째로.
                classes
                    .iter()
                    .map(|(class, names)| {
                        if names.iter().any(String::is_empty) {
                            class.clone()
                        } else {
                            format!("{class}#{}", names.join("+"))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            };
            // 단위 테스트(Surefire)와 통합 테스트(Failsafe)는 거르는 속성이 다르다. `-Dtest`에 통합 테스트를
            // 넣으면 Surefire가 그것을 단위 테스트 단계에서 돌리고, Failsafe는 통합 테스트를 모두 돈다.
            let (integration, unit): (Vec<&&Case>, Vec<&&Case>) =
                failed.iter().partition(|case| case.integration);
            // 실패한 단위 테스트가 없으면 Surefire는 아무것도 돌리지 않는다(맞는 클래스가 없는 이름). `-Dtest=!*`는
            // 지연된 변수 확장을 켠 cmd에서 `!`가 사라진다.
            let unit = if unit.is_empty() {
                MAVEN_NO_UNIT_TESTS.to_owned()
            } else {
                filter(&unit)
            };
            // 공백이 든 거르기(Kotlin 백틱 이름)는 셸이 가르지 않게 인자째 감싼다.
            let property = |key: &str, filter: String| {
                if filter.contains(' ') {
                    format!("\"-D{key}={filter}\"")
                } else {
                    format!("-D{key}={filter}")
                }
            };
            let mut line = format!(
                "{command} {} -Dsurefire.failIfNoSpecifiedTests=false",
                property("test", unit)
            );
            if !integration.is_empty() {
                // 다중 모듈이면 그 통합 테스트가 없는 모듈도 있다.
                line.push_str(&format!(
                    " {} -Dit.failIfNoSpecifiedTests=false",
                    property("it.test", filter(&integration))
                ));
            } else if runs_integration_tests(command) {
                line.push_str(" -DskipITs");
            }
            line
        }
        // 보고서가 알려 준 작업마다 `--tests`를 붙인다. `check`·`build`는 테스트 작업이 아니라 `--tests`를 받지
        // 못하고("Unknown command-line option"), 작업 이름만으로는 다른 테스트 작업(`integrationTest`)의 것을 거르지
        // 못한다. 명령의 작업 이름은 빼고 옵션은 둔다. 작업을 모르는 것이 있으면 명령에 그대로 붙인다.
        Tool::Gradle if failed.iter().all(|case| case.task.is_some()) => {
            let mut tasks: Vec<(&str, Vec<String>)> = Vec::new();
            for case in failed {
                let task = case.task.as_deref().unwrap_or_default();
                let test = gradle_test(case);
                match tasks.iter_mut().find(|(t, _)| *t == task) {
                    Some((_, tests)) if !tests.contains(&test) => tests.push(test),
                    Some(_) => {}
                    None => tasks.push((task, vec![test])),
                }
            }
            let mut line = gradle_without_tasks(command);
            for (task, tests) in tasks {
                line.push_str(&format!(" {task}"));
                for test in tests {
                    line.push_str(&format!(" --tests \"{test}\""));
                }
            }
            line
        }
        Tool::Gradle => {
            let mut line = command.to_owned();
            for case in failed {
                line.push_str(&format!(" --tests \"{}\"", gradle_test(case)));
            }
            line
        }
        Tool::Pytest => format!("{command} --lf"),
        Tool::Ctest => format!("{command} --rerun-failed"),
        Tool::Go => {
            let mut packages: Vec<&str> = Vec::new();
            let mut names: Vec<&str> = Vec::new();
            for case in failed {
                if !packages.contains(&case.suite.as_str()) {
                    packages.push(&case.suite);
                }
                let top = case.name.split('/').next().unwrap_or(&case.name);
                // 이름이 빈 것(패키지 전체, `run_only`)은 거르지 않는다.
                if case.name != PACKAGE_CASE && !top.is_empty() && !names.contains(&top) {
                    names.push(top);
                }
            }
            let run = if names.is_empty() {
                String::new()
            } else {
                format!(" -run \"^({})$\"", names.join("|"))
            };
            format!("go test -json{run} {}", packages.join(" "))
        }
    })
}

/// 실패한 통합 테스트만 다시 돌릴 때 Surefire에 주는 거르기(맞는 단위 테스트 클래스가 없는 이름).
const MAVEN_NO_UNIT_TESTS: &str = "NoUnitTestsToRerun";

/// Maven 명령이 통합 테스트 단계(`integration-test`)까지 가는지(Failsafe가 돈다).
fn runs_integration_tests(command: &str) -> bool {
    command.split_whitespace().any(|word| {
        matches!(
            word.trim_matches('"'),
            "integration-test" | "post-integration-test" | "verify" | "install" | "deploy"
        )
    })
}

/// 스택·출력에서 찾은 소스 줄.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// 보일 글(스택의 그 줄).
    pub text: String,
    /// 스택에 적힌 파일(이름이나 상대 경로).
    pub file: String,
    pub line: u32,
    /// Java 프레임의 클래스(`com.example.CalcTest`, 안쪽 클래스는 바깥 클래스).
    pub class: Option<String>,
}

static JAVA_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    // 모듈·로더 앞붙이(`java.base/`, Gradle의 `app//`)는 건너뛴다. Kotlin의 백틱 이름(`adds numbers`)은
    // 빈칸이 들고, 그 안의 람다는 클래스 이름(`CalcTest$adds numbers$1`)에도 든다.
    Regex::new(
        r"^\s*at\s+(?:[\w.$@-]*/+)?([^\s()/][^()/]*)\.[^.()/]+\(([^:()]+\.(?:java|kt|groovy|scala)):(\d+)\)",
    )
    .unwrap()
});
static PYTHON_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:File "([^"]+\.py)", line (\d+)|([^\s:"]+\.py):(\d+):)"#).unwrap()
});
static GO_FRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s+([\w./-]+\.go):(\d+):").unwrap());

/// 실패 스택·출력의 소스 줄들(보고서가 위치를 알려 주면 그것이 먼저).
pub fn frames(case: &Case) -> Vec<Frame> {
    let mut frames = Vec::new();
    if let (Some(file), Some(line)) = (&case.file, case.line) {
        frames.push(Frame {
            text: format!("{file}:{line}"),
            file: file.clone(),
            line,
            class: None,
        });
    }
    for text in case.detail.lines() {
        let frame = if let Some(c) = JAVA_FRAME.captures(text) {
            let class = c[1].split('$').next().unwrap_or(&c[1]).to_owned();
            Some((c[2].to_owned(), c[3].parse().ok(), Some(class)))
        } else if let Some(c) = PYTHON_FRAME.captures(text) {
            let (file, line) = match c.get(1) {
                Some(file) => (file.as_str(), &c[2]),
                None => (&c[3], &c[4]),
            };
            Some((file.to_owned(), line.parse().ok(), None))
        } else {
            GO_FRAME
                .captures(text)
                .map(|c| (c[1].to_owned(), c[2].parse().ok(), None))
        };
        if let Some((file, Some(line), class)) = frame {
            frames.push(Frame {
                text: text.trim().to_owned(),
                file,
                line,
                class,
            });
        }
    }
    frames
}

/// 실패한 테스트에서 먼저 보일 프레임: Java는 테스트 클래스의 첫 프레임, pytest는 마지막 프레임(테스트
/// 줄), 그 밖은 첫 프레임.
pub fn test_frame(case: &Case, frames: &[Frame]) -> Option<usize> {
    if let Some(i) = frames
        .iter()
        .position(|f| f.class.as_deref() == Some(case.suite.as_str()))
    {
        return Some(i);
    }
    if frames.iter().any(|f| f.file.ends_with(".py")) && case.file.is_none() {
        return frames.len().checked_sub(1);
    }
    (!frames.is_empty()).then_some(0)
}

/// 프레임의 파일을 프로젝트에서 찾는다. JVM 프레임은 패키지 경로(`com/example/CalcTest.java`)로, 그 밖은
/// `dir`(테스트를 돌린 폴더) 기준 경로, 없으면 경로 끝이 같은 파일.
pub fn resolve(root: &Path, dir: &Path, frame: &Frame) -> Option<PathBuf> {
    if let Some(class) = &frame.class {
        let mut package: Vec<&str> = class.split('.').collect();
        package.pop();
        // Java는 폴더가 패키지를 따른다. Kotlin·Scala·Groovy는 달라도 되고, Kotlin 관례는 공통 루트
        // 패키지를 폴더에서 빼는 것이라(`com.example.board` → `src/test/kotlin/CalcTest.kt`) 패키지 앞을
        // 줄여 가며 찾는다.
        let skips = if frame.file.ends_with(".java") {
            0
        } else {
            package.len()
        };
        return (0..=skips).find_map(|skip| {
            let mut parts = package[skip..].to_vec();
            parts.push(&frame.file);
            crate::files::find_suffix(root, &parts.join("/"))
        });
    }
    let direct = dir.join(&frame.file);
    if direct.is_file() {
        return Some(direct);
    }
    crate::files::find_suffix(root, &frame.file.replace('\\', "/"))
}

#[cfg(test)]
mod tests;
