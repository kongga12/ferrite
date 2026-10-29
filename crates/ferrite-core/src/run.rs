//! 실행 구성. 프로젝트 루트의 표지 파일(Cargo.toml, pom.xml 등)을 보고 만드는 구성과, 사용자가
//! `.ferrite/run.toml`에 적는 구성. 명령 줄은 플랫폼 셸(Windows `cmd`, 그 밖에는 사용자 셸)이 실행한다.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// 사용자 구성 파일(프로젝트 루트 기준).
pub const CUSTOM_FILE: &str = ".ferrite/run.toml";

/// 디버그 대기 실행이 여는 포트. 디버그의 "5005 포트에 붙기" 구성이 여기로 붙는다.
pub const JDWP_PORT: u16 = 5005;

/// JVM을 디버거가 붙을 수 있게 띄우는 옵션. 멈추지 않고 뜨며 이 컴퓨터에서만 붙을 수 있다(Java 9 이상).
pub fn jdwp_agent() -> String {
    format!("-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address={JDWP_PORT}")
}

/// 사용자 구성 파일을 처음 만들 때의 내용.
pub const CUSTOM_TEMPLATE: &str = r#"# Ferrite 실행 구성. [[run]]마다 하나씩 적는다.
#   name    목록에 보일 이름
#   command 셸이 실행할 명령 줄(Windows는 cmd, 그 밖에는 사용자 셸)
#   cwd     작업 폴더(선택). 상대 경로면 프로젝트 루트 기준
#   env     더할 환경 변수(선택)
#
# [[run]]
# name = "톰캣으로 실행(JDK 8)"
# command = "mvn tomcat7:run"
# env = { JAVA_HOME = "C:/Program Files/Java/jdk1.8.0_202" }
"#;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    pub name: String,
    /// 셸이 실행할 명령 줄.
    pub command: String,
    /// 작업 폴더. 상대 경로면 프로젝트 루트 기준, 없으면 루트.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// 더할 환경 변수.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl RunConfig {
    /// 이름이 곧 명령 줄인 구성(자동 감지).
    fn command(command: String) -> Self {
        Self {
            name: command.clone(),
            command,
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    pub fn working_directory(&self, root: &Path) -> PathBuf {
        match &self.cwd {
            Some(cwd) => root.join(cwd),
            None => root.to_path_buf(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomFile {
    #[serde(default)]
    run: Vec<RunConfig>,
}

/// 사용자 구성을 읽는다. 파일이 없으면 빈 목록, 잘못 적었으면 사용자에게 보일 오류.
pub fn load_custom(root: &Path) -> Result<Vec<RunConfig>, String> {
    let text = match std::fs::read_to_string(root.join(CUSTOM_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{CUSTOM_FILE}을 읽지 못했습니다: {e}")),
    };
    parse_custom(&text)
}

fn parse_custom(text: &str) -> Result<Vec<RunConfig>, String> {
    let file: CustomFile = toml::from_str(text).map_err(|e| format!("{CUSTOM_FILE}: {e}"))?;
    if let Some(bad) = file
        .run
        .iter()
        .find(|c| c.name.trim().is_empty() || c.command.trim().is_empty())
    {
        return Err(format!(
            "{CUSTOM_FILE}: name과 command는 비울 수 없습니다(name = {:?})",
            bad.name
        ));
    }
    Ok(file.run)
}

/// 프로젝트 루트의 표지 파일을 보고 구성을 만든다. 하위 폴더는 보지 않는다(필요하면 사용자 구성의 `cwd`).
pub fn detect(root: &Path) -> Vec<RunConfig> {
    let has = |name: &str| root.join(name).is_file();
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap_or_default();
    let mut commands: Vec<String> = Vec::new();
    // 디버거가 붙을 수 있게 띄우는 실행. 같은 명령의 보통 실행 바로 뒤에 둔다.
    let mut debug_runs: Vec<RunConfig> = Vec::new();

    if has("Cargo.toml") {
        commands.extend(["cargo run", "cargo test", "cargo build"].map(String::from));
    }
    if has("go.mod") {
        commands.extend(["go run .", "go test ./...", "go build ./..."].map(String::from));
    }
    if has("pom.xml") {
        let mvn = wrapper(root, "mvnw", "mvn");
        let pom = read("pom.xml");
        // egov 웹 프로젝트는 대개 tomcat7 플러그인, egov 4.x 이후 Boot 템플릿은 spring-boot 플러그인을 쓴다.
        for (plugin, goal) in [
            ("tomcat7-maven-plugin", "tomcat7:run"),
            ("spring-boot-maven-plugin", "spring-boot:run"),
            ("jetty-maven-plugin", "jetty:run"),
        ] {
            if pom.contains(plugin) {
                commands.push(format!("{mvn} {goal}"));
            }
        }
        let debug_name = |goal: &str| format!("{mvn} {goal} (디버그 {JDWP_PORT})");
        // tomcat7:run은 톰캣을 Maven JVM 안에서 띄우므로 Maven JVM에 옵션을 준다(사용자 MAVEN_OPTS 뒤에).
        if pom.contains("tomcat7-maven-plugin") {
            let opts = std::env::var("MAVEN_OPTS")
                .map(|o| format!("{o} {}", jdwp_agent()))
                .unwrap_or_else(|_| jdwp_agent());
            debug_runs.push(RunConfig {
                name: debug_name("tomcat7:run"),
                command: format!("{mvn} tomcat7:run"),
                cwd: None,
                env: BTreeMap::from([("MAVEN_OPTS".to_owned(), opts)]),
            });
        }
        // spring-boot:run은 앱을 따로 띄우므로 그 JVM에 준다. 쉼표·등호가 있어 인자 전체를 따옴표로 묶는다.
        if pom.contains("spring-boot-maven-plugin") {
            debug_runs.push(RunConfig::command(format!(
                "{mvn} spring-boot:run \"-Dspring-boot.run.jvmArguments={}\"",
                jdwp_agent()
            )));
            if let Some(last) = debug_runs.last_mut() {
                last.name = debug_name("spring-boot:run");
            }
        }
        commands.extend(["package", "test"].map(|goal| format!("{mvn} {goal}")));
    }
    let gradle_file = ["build.gradle", "build.gradle.kts"]
        .into_iter()
        .find(|f| has(f));
    if let Some(file) = gradle_file {
        let gradle = wrapper(root, "gradlew", "gradle");
        let script = read(file);
        if script.contains("org.springframework.boot") {
            commands.push(format!("{gradle} bootRun"));
        }
        let application = script.contains("'application'")
            || script.contains("\"application\"")
            || script.lines().any(|l| l.trim() == "application");
        if application {
            commands.push(format!("{gradle} run"));
        }
        commands.extend(["build", "test"].map(|task| format!("{gradle} {task}")));
    }
    if has("CMakeLists.txt") {
        commands.extend(
            [
                "cmake -S . -B build",
                "cmake --build build",
                // 다중 구성 빌드(Visual Studio·Xcode)는 구성을 골라야 테스트를 돌린다. 한 구성 빌드에는 상관없다.
                "ctest --test-dir build -C Debug",
            ]
            .map(String::from),
        );
    }
    let python_project = ["pyproject.toml", "setup.py", "requirements.txt"]
        .into_iter()
        .any(&has);
    let python = if cfg!(windows) { "python" } else { "python3" };
    for entry in ["main.py", "app.py"] {
        if has(entry) {
            commands.push(format!("{python} {entry}"));
        }
    }
    if has("manage.py") {
        commands.push(format!("{python} manage.py runserver"));
    }
    if python_project
        && (has("pytest.ini")
            || root.join("tests").is_dir()
            || read("pyproject.toml").contains("pytest"))
    {
        commands.push(format!("{python} -m pytest"));
    }
    if has("composer.json") {
        commands.push("composer install".into());
        commands.extend(
            composer_scripts(&read("composer.json"))
                .into_iter()
                .map(|s| format!("composer run-script {s}")),
        );
        if has("artisan") {
            commands.push("php artisan serve".into());
        }
    }
    let mut configs: Vec<RunConfig> = commands.into_iter().map(RunConfig::command).collect();
    for debug in debug_runs {
        let base = debug
            .command
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        let at = configs
            .iter()
            .position(|c| c.command == base)
            .map_or(configs.len(), |i| i + 1);
        configs.insert(at, debug);
    }
    configs
}

/// composer.json의 스크립트 이름. 다른 단계 앞뒤에 저절로 도는 것(`pre-`·`post-`)은 뺀다.
fn composer_scripts(json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value
        .get("scripts")
        .and_then(|s| s.as_object())
        .map(|scripts| {
            scripts
                .keys()
                .filter(|k| !k.starts_with("pre-") && !k.starts_with("post-"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 프로젝트에 들어 있는 래퍼 스크립트(mvnw, gradlew)가 있으면 그것, 없으면 설치된 도구.
/// Windows는 `cmd`가 작업 폴더의 `.cmd`·`.bat`를 먼저 찾으므로 이름만 쓴다.
fn wrapper(root: &Path, script: &str, tool: &str) -> String {
    if cfg!(windows) {
        if ["cmd", "bat"]
            .iter()
            .any(|ext| root.join(format!("{script}.{ext}")).is_file())
        {
            return script.to_owned();
        }
    } else if root.join(script).is_file() {
        return format!("./{script}");
    }
    tool.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferrite-run-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(configs: &[RunConfig]) -> Vec<&str> {
        configs.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn egov_maven_프로젝트는_톰캣_실행이_먼저() {
        let pom = "<project><build><plugins><plugin>\
                   <artifactId>tomcat7-maven-plugin</artifactId>\
                   </plugin></plugins></build></project>";
        let dir = project("maven", &[("pom.xml", pom)]);
        assert_eq!(
            names(&detect(&dir)),
            [
                "mvn tomcat7:run",
                "mvn tomcat7:run (디버그 5005)",
                "mvn package",
                "mvn test"
            ]
        );
        let debug = &detect(&dir)[1];
        assert_eq!(debug.command, "mvn tomcat7:run");
        assert!(debug.env["MAVEN_OPTS"].ends_with(&jdwp_agent()));

        // 래퍼가 있으면 래퍼를 쓴다.
        let wrapper = if cfg!(windows) { "mvnw.cmd" } else { "mvnw" };
        std::fs::write(dir.join(wrapper), "").unwrap();
        let expected = if cfg!(windows) { "mvnw" } else { "./mvnw" };
        assert_eq!(detect(&dir)[0].command, format!("{expected} tomcat7:run"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 여러_종류를_함께_감지한다() {
        let dir = project(
            "mixed",
            &[
                ("Cargo.toml", "[package]"),
                (
                    "build.gradle.kts",
                    "plugins {\n    application\n    id(\"org.springframework.boot\")\n}",
                ),
                ("pyproject.toml", "[tool.pytest.ini_options]"),
                ("main.py", ""),
                (
                    "composer.json",
                    r#"{"scripts": {"test": "phpunit", "post-install-cmd": "x", "lint": "phpcs"}}"#,
                ),
            ],
        );
        let configs = detect(&dir);
        let names = names(&configs);
        assert!(names.starts_with(&["cargo run", "cargo test", "cargo build"]));
        assert!(names.contains(&"gradle bootRun") && names.contains(&"gradle run"));
        let python = if cfg!(windows) { "python" } else { "python3" };
        assert!(names.contains(&format!("{python} main.py").as_str()));
        assert!(names.contains(&format!("{python} -m pytest").as_str()));
        assert!(names.contains(&"composer run-script test"));
        assert!(names.contains(&"composer run-script lint"));
        assert!(!names.iter().any(|n| n.contains("post-install")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 아무것도_없으면_빈_목록() {
        let dir = project("empty", &[("README.md", "")]);
        assert!(detect(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 사용자_구성을_읽고_잘못_적으면_알린다() {
        let configs = parse_custom(
            r#"
            [[run]]
            name = "톰캣"
            command = "mvn tomcat7:run"
            cwd = "web"
            env = { JAVA_HOME = "C:/jdk8" }
            "#,
        )
        .unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].env["JAVA_HOME"], "C:/jdk8");
        let root = Path::new("project");
        assert_eq!(configs[0].working_directory(root), root.join("web"));

        assert!(
            parse_custom(CUSTOM_TEMPLATE).unwrap().is_empty(),
            "견본은 모두 주석"
        );
        assert!(
            parse_custom("[[run]]\nname = \"a\"").is_err(),
            "command 빠짐"
        );
        assert!(parse_custom("[[run]]\nname = \" \"\ncommand = \"x\"").is_err());
        assert!(
            parse_custom("[[run]]\nname = \"a\"\ncommand = \"x\"\nargs = []").is_err(),
            "모르는 키"
        );
        assert!(load_custom(Path::new("없는-폴더")).unwrap().is_empty());
    }
}
