//! java-debug 시작 준비. java-debug는 jdt.ls에 실린 번들이라, jdt.ls에 `workspace/executeCommand`로
//! 메인 클래스·클래스패스·Java 실행 파일을 묻고(launch에서 사용자가 적지 않은 것만), 마지막으로 디버그
//! 세션을 열게 해 받은 포트로 붙는다. 이 모듈은 순서와 응답 해석만 맡고, 보내기는 부르는 쪽(LSP 허브)이 한다.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

pub const RESOLVE_MAIN_CLASS: &str = "vscode.java.resolveMainClass";
pub const RESOLVE_CLASSPATH: &str = "vscode.java.resolveClasspath";
pub const RESOLVE_JAVA_EXECUTABLE: &str = "vscode.java.resolveJavaExecutable";
pub const BUILD_WORKSPACE: &str = "vscode.java.buildWorkspace";
pub const START_DEBUG_SESSION: &str = "vscode.java.startDebugSession";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    MainClass,
    Classpath,
    JavaExec,
    /// 띄우기 전에 고친 소스를 컴파일해 둔다(자동 빌드가 아직 돌지 않았을 수 있다).
    Build,
    Port,
}

#[derive(Debug, PartialEq)]
pub enum Next {
    /// jdt.ls에 보낼 `workspace/executeCommand`(명령, 인자).
    Command(&'static str, Vec<Value>),
    /// 이 포트로 붙어 이 인자로 launch/attach 한다.
    Ready {
        port: u16,
        arguments: Value,
    },
    Failed(String),
}

pub struct JavaStart {
    arguments: Map<String, Value>,
    /// launch에서 메인 클래스를 고를 때 이 파일의 것을 먼저 고른다.
    file: Option<PathBuf>,
    step: Step,
}

impl JavaStart {
    /// `arguments`는 구성의 인자(변수를 바꾼 것). attach면 포트만 받는다.
    pub fn new(arguments: Value, attach: bool, file: Option<&Path>) -> Self {
        let arguments = match arguments {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let mut start = Self {
            arguments,
            file: file.map(Path::to_path_buf),
            step: Step::Port,
        };
        if !attach {
            start.step = start.first_missing(Step::MainClass);
        }
        start
    }

    /// 지금 단계에서 보낼 명령.
    pub fn command(&self) -> Next {
        let text = |key: &str| {
            json!(
                self.arguments
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or("")
            )
        };
        match self.step {
            Step::MainClass => Next::Command(RESOLVE_MAIN_CLASS, Vec::new()),
            Step::Classpath => Next::Command(
                RESOLVE_CLASSPATH,
                vec![text("mainClass"), text("projectName")],
            ),
            Step::JavaExec => Next::Command(
                RESOLVE_JAVA_EXECUTABLE,
                vec![text("mainClass"), text("projectName")],
            ),
            // 인자는 JSON 문자열 하나다.
            Step::Build => Next::Command(
                BUILD_WORKSPACE,
                vec![json!(
                    json!({
                        "mainClass": text("mainClass"),
                        "projectName": text("projectName"),
                        "isFullBuild": false,
                    })
                    .to_string()
                )],
            ),
            Step::Port => Next::Command(START_DEBUG_SESSION, Vec::new()),
        }
    }

    /// 지금 단계의 응답을 받아 다음 할 일을 돌려준다.
    pub fn answer(&mut self, result: Result<Value, String>) -> Next {
        let value = match result {
            Ok(value) => value,
            Err(e) if e.contains("No delegateCommandHandler") => {
                return Next::Failed(
                    "java-debug가 jdt.ls에 실리지 않았습니다. 도구 > 언어 서버에서 java-debug를 \
                     설치하세요(설치하면 jdt.ls가 다시 뜹니다)"
                        .into(),
                );
            }
            Err(e) => return Next::Failed(format!("jdt.ls 오류: {e}")),
        };
        match self.step {
            Step::MainClass => {
                let Some(found) = self.pick_main_class(&value) else {
                    return Next::Failed(
                        "메인 클래스를 찾지 못했습니다. jdt.ls가 프로젝트를 다 읽고 빌드했는지 \
                         확인하세요"
                            .into(),
                    );
                };
                for key in ["mainClass", "projectName"] {
                    if let Some(text) = found.get(key).and_then(Value::as_str)
                        && !text.is_empty()
                    {
                        self.arguments.insert(key.into(), json!(text));
                    }
                }
                self.step = self.first_missing(Step::Classpath);
            }
            Step::Classpath => {
                // [모듈 경로 목록, 클래스 경로 목록]
                let paths = |i: usize| value.get(i).cloned().unwrap_or_else(|| json!([]));
                self.arguments.insert("modulePaths".into(), paths(0));
                self.arguments.insert("classPaths".into(), paths(1));
                self.step = self.first_missing(Step::JavaExec);
            }
            Step::JavaExec => {
                if let Some(java) = value.as_str().filter(|j| !j.is_empty()) {
                    self.arguments.insert("javaExec".into(), json!(java));
                }
                self.step = Step::Build;
            }
            // 0 실패, 1 성공, 2 컴파일 오류가 있으나 끝남, 3 취소. 오류가 있어도 다른 파일 문제일 수 있어 띄운다.
            Step::Build => {
                if matches!(value.as_u64(), Some(0 | 3)) {
                    return Next::Failed("Java 빌드에 실패했습니다. 문제 목록을 확인하세요".into());
                }
                self.step = Step::Port;
            }
            Step::Port => {
                let Some(port) = value.as_u64().and_then(|p| u16::try_from(p).ok()) else {
                    return Next::Failed(format!(
                        "jdt.ls가 디버그 포트를 주지 않았습니다: {value}"
                    ));
                };
                return Next::Ready {
                    port,
                    arguments: Value::Object(self.arguments.clone()),
                };
            }
        }
        self.command()
    }

    /// `from`부터 사용자가 적지 않은 첫 단계.
    fn first_missing(&self, from: Step) -> Step {
        let has = |key: &str| self.arguments.contains_key(key);
        [
            (Step::MainClass, has("mainClass")),
            (Step::Classpath, has("classPaths")),
            (Step::JavaExec, has("javaExec")),
        ]
        .into_iter()
        .skip_while(|(step, _)| *step != from)
        .find(|(_, given)| !given)
        .map_or(Step::Build, |(step, _)| step)
    }

    /// 열린 파일의 메인 클래스, 없으면 첫 메인 클래스.
    fn pick_main_class<'a>(&self, value: &'a Value) -> Option<&'a Map<String, Value>> {
        let found: Vec<&Map<String, Value>> = value
            .as_array()?
            .iter()
            .filter_map(Value::as_object)
            .filter(|m| m.get("mainClass").and_then(Value::as_str).is_some())
            .collect();
        let in_file = self.file.as_deref().and_then(|file| {
            found.iter().copied().find(|m| {
                m.get("filePath")
                    .and_then(Value::as_str)
                    .is_some_and(|p| same_path(Path::new(p), file))
            })
        });
        in_file.or_else(|| found.first().copied())
    }
}

/// 같은 파일인지. Windows는 대소문자와 구분자를 가리지 않는다.
fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
        norm(a) == norm(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main_classes() -> Value {
        json!([
            { "mainClass": "demo.Other", "projectName": "demo", "filePath": "/p/src/demo/Other.java" },
            { "mainClass": "demo.Main", "projectName": "demo", "filePath": "/p/src/demo/Main.java" },
        ])
    }

    #[test]
    fn launch는_메인_클래스_클래스패스_java_포트_순으로_묻는다() {
        let file = Path::new("/p/src/demo/Main.java");
        let mut start = JavaStart::new(json!({ "cwd": "/p" }), false, Some(file));
        assert_eq!(start.command(), Next::Command(RESOLVE_MAIN_CLASS, vec![]));
        assert_eq!(
            start.answer(Ok(main_classes())),
            Next::Command(RESOLVE_CLASSPATH, vec![json!("demo.Main"), json!("demo")]),
            "열린 파일의 메인 클래스를 고른다"
        );
        assert_eq!(
            start.answer(Ok(json!([[], ["/p/bin"]]))),
            Next::Command(
                RESOLVE_JAVA_EXECUTABLE,
                vec![json!("demo.Main"), json!("demo")]
            )
        );
        let Next::Command(BUILD_WORKSPACE, build) = start.answer(Ok(json!("/jdk/bin/java"))) else {
            panic!("띄우기 전에 빌드한다");
        };
        let params: Value = serde_json::from_str(build[0].as_str().unwrap()).unwrap();
        assert_eq!(params["mainClass"], "demo.Main");
        assert_eq!(
            start.answer(Ok(json!(1))),
            Next::Command(START_DEBUG_SESSION, vec![])
        );
        let Next::Ready { port, arguments } = start.answer(Ok(json!(51234))) else {
            panic!("포트를 받아야 한다");
        };
        assert_eq!(port, 51234);
        assert_eq!(arguments["mainClass"], "demo.Main");
        assert_eq!(arguments["classPaths"], json!(["/p/bin"]));
        assert_eq!(arguments["javaExec"], "/jdk/bin/java");
        assert_eq!(arguments["cwd"], "/p");
    }

    #[test]
    fn 적은_것은_묻지_않고_attach는_포트만_받는다() {
        let start = JavaStart::new(
            json!({ "mainClass": "a.B", "classPaths": [], "javaExec": "java" }),
            false,
            None,
        );
        assert!(
            matches!(start.command(), Next::Command(BUILD_WORKSPACE, _)),
            "빌드는 묻는다"
        );
        let mut attach =
            JavaStart::new(json!({ "hostName": "localhost", "port": 5005 }), true, None);
        assert_eq!(attach.command(), Next::Command(START_DEBUG_SESSION, vec![]));
        let Next::Ready { arguments, .. } = attach.answer(Ok(json!(4000))) else {
            panic!()
        };
        assert_eq!(arguments["port"], 5005, "붙을 대상 포트는 그대로");
    }

    #[test]
    fn 실패를_알기_쉽게_알린다() {
        let mut start = JavaStart::new(json!({}), false, None);
        assert!(
            matches!(start.answer(Ok(json!([]))), Next::Failed(m) if m.contains("메인 클래스"))
        );
        let mut start = JavaStart::new(json!({}), true, None);
        assert!(matches!(
            start.answer(Err("No delegateCommandHandler for vscode.java.startDebugSession".into())),
            Next::Failed(m) if m.contains("java-debug를 설치")
        ));
        let mut start = JavaStart::new(json!({}), true, None);
        assert!(matches!(start.answer(Ok(Value::Null)), Next::Failed(_)));
        let mut start = JavaStart::new(
            json!({ "mainClass": "a.B", "classPaths": [], "javaExec": "java" }),
            false,
            None,
        );
        assert!(matches!(start.answer(Ok(json!(0))), Next::Failed(m) if m.contains("빌드")));
    }
}
