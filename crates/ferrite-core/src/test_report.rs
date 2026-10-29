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
            "gradle" | "gradlew" if has("test") || has("check") => Some(Tool::Gradle),
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
                });
            }
            _ => {}
        }
    }
    cases
}

/// 보고서를 남기게 고친 명령. pytest·CTest는 `dir`에 JUnit XML을 쓰게 하고, Go는 `-json`을 붙인다.
/// Maven·Gradle은 원래 보고서를 남긴다.
pub fn with_report(tool: Tool, command: &str, dir: &Path) -> String {
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    match tool {
        Tool::Pytest if !command.contains("--junitxml") => {
            format!("{command} \"--junitxml={}\"", path("pytest.xml"))
        }
        Tool::Ctest if !command.contains("--output-junit") => {
            format!("{command} --output-junit \"{}\"", path("ctest.xml"))
        }
        Tool::Go if !command.split_whitespace().any(|w| w == "-json") => {
            command.replacen("test", "test -json", 1)
        }
        _ => command.to_owned(),
    }
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

/// 실패한 것만 다시 돌리는 명령: Maven `-Dtest`, Gradle `--tests`, pytest `--lf`, CTest
/// `--rerun-failed`, Go `-run`.
pub fn rerun(tool: Tool, command: &str, failed: &[&Case]) -> Option<String> {
    if failed.is_empty() {
        return None;
    }
    // JUnit 5의 매개변수 테스트 이름(`adds(int)[1]`)과 Gradle의 `()`를 뺀다.
    let method = |name: &str| name.split(['(', '[']).next().unwrap_or(name).to_owned();
    Some(match tool {
        Tool::Maven => {
            let mut classes: Vec<(String, Vec<String>)> = Vec::new();
            for case in failed {
                let class = case.suite.rsplit('.').next().unwrap_or(&case.suite);
                let class = class.split('$').next().unwrap_or(class).to_owned();
                let name = method(&case.name);
                match classes.iter_mut().find(|(c, _)| *c == class) {
                    Some((_, names)) if !names.contains(&name) => names.push(name),
                    Some(_) => {}
                    None => classes.push((class, vec![name])),
                }
            }
            let list: Vec<String> = classes
                .iter()
                .map(|(class, names)| format!("{class}#{}", names.join("+")))
                .collect();
            format!(
                "{command} -Dtest={} -Dsurefire.failIfNoSpecifiedTests=false",
                list.join(",")
            )
        }
        Tool::Gradle => {
            let mut line = command.to_owned();
            for case in failed {
                line.push_str(&format!(
                    " --tests \"{}.{}\"",
                    case.suite,
                    method(&case.name)
                ));
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
                if case.name != PACKAGE_CASE && !names.contains(&top) {
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
    // 모듈·로더 앞붙이(`java.base/`, Gradle의 `app//`)는 건너뛴다.
    Regex::new(r"^\s*at\s+(?:[\w.$@-]*/+)?([\w$.]+)\.[\w$<>]+\(([^:()]+\.(?:java|kt|groovy|scala)):(\d+)\)")
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

/// 프레임의 파일을 프로젝트에서 찾는다. Java는 패키지 경로(`com/example/CalcTest.java`)로, 그 밖은
/// `dir`(테스트를 돌린 폴더) 기준 경로, 없으면 경로 끝이 같은 파일.
pub fn resolve(root: &Path, dir: &Path, frame: &Frame) -> Option<PathBuf> {
    let relative = match &frame.class {
        Some(class) => {
            let package: Vec<&str> = class.split('.').collect();
            let package = &package[..package.len().saturating_sub(1)];
            let mut path = package.join("/");
            if !path.is_empty() {
                path.push('/');
            }
            path + &frame.file
        }
        None => {
            let direct = dir.join(&frame.file);
            if direct.is_file() {
                return Some(direct);
            }
            frame.file.replace('\\', "/")
        }
    };
    crate::files::find_suffix(root, &relative)
}

#[cfg(test)]
mod tests;
