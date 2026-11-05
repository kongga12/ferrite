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

# 저장 시 작업. 저장한 파일이 pattern에 맞으면 command를 창 없이 돌리고 결과를 상태바에 알린다.
#   pattern 프로젝트 루트 기준 글롭. `/`가 없으면 어느 폴더든(`*.scss`)
#   command ${file} ${relativeFile} ${fileDirname} ${fileBasename} ${fileBasenameNoExtension}
#           ${fileExtname} ${workspaceFolder}를 저장한 파일로 바꾼다. 공백이 든 경로는 "${file}"처럼 감싼다
#   cwd·env 실행 구성과 같다
#
# [[on_save]]
# name = "Sass"
# pattern = "src/main/webapp/**/*.scss"
# command = "sass \"${file}\" \"${fileDirname}/${fileBasenameNoExtension}.css\""
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
    #[serde(default)]
    on_save: Vec<SaveTask>,
}

/// 저장 시 작업(`[[on_save]]`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveTask {
    pub name: String,
    /// 프로젝트 루트 기준 글롭. `/`가 없으면 어느 폴더든.
    pub pattern: String,
    /// 셸이 실행할 명령 줄. `${file}` 등을 저장한 파일로 바꾼다.
    pub command: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl SaveTask {
    /// 저장한 파일이 패턴에 맞는지(프로젝트 밖 파일은 맞지 않는다).
    pub fn matches(&self, root: &Path, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(root) else {
            return false;
        };
        let relative: Vec<String> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let pattern = if self.pattern.contains('/') {
            self.pattern.trim_start_matches('/').to_owned()
        } else {
            format!("**/{}", self.pattern)
        };
        globset::GlobBuilder::new(&pattern)
            .literal_separator(true)
            .build()
            .is_ok_and(|g| g.compile_matcher().is_match(relative.join("/")))
    }

    /// 저장한 파일로 바꾼 명령 줄(이름은 VS Code의 작업 변수와 같다).
    pub fn command_for(&self, root: &Path, path: &Path) -> String {
        let text = |p: &Path| p.to_string_lossy().into_owned();
        let name = |p: Option<&std::ffi::OsStr>| p.map(|n| n.to_string_lossy().into_owned());
        let relative = path.strip_prefix(root).unwrap_or(path);
        let extension = name(path.extension()).map(|e| format!(".{e}"));
        [
            ("${workspaceFolder}", text(root)),
            ("${relativeFile}", text(relative)),
            (
                "${fileDirname}",
                path.parent().map(text).unwrap_or_default(),
            ),
            (
                "${fileBasenameNoExtension}",
                name(path.file_stem()).unwrap_or_default(),
            ),
            (
                "${fileBasename}",
                name(path.file_name()).unwrap_or_default(),
            ),
            ("${fileExtname}", extension.unwrap_or_default()),
            ("${file}", text(path)),
        ]
        .iter()
        .fold(self.command.clone(), |line, (key, value)| {
            line.replace(key, value)
        })
    }

    pub fn working_directory(&self, root: &Path) -> PathBuf {
        match &self.cwd {
            Some(cwd) => root.join(cwd),
            None => root.to_path_buf(),
        }
    }
}

/// 사용자 구성을 읽는다. 파일이 없으면 빈 목록, 잘못 적었으면 사용자에게 보일 오류.
pub fn load_custom(root: &Path) -> Result<Vec<RunConfig>, String> {
    let text = match std::fs::read_to_string(root.join(CUSTOM_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{CUSTOM_FILE}을 읽지 못했습니다: {e}")),
    };
    parse_custom(&text).map(|f| f.run)
}

/// 저장 시 작업을 읽는다. 파일이 없으면 빈 목록, 잘못 적었으면 사용자에게 보일 오류.
pub fn load_on_save(root: &Path) -> Result<Vec<SaveTask>, String> {
    let text = match std::fs::read_to_string(root.join(CUSTOM_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{CUSTOM_FILE}을 읽지 못했습니다: {e}")),
    };
    parse_custom(&text).map(|f| f.on_save)
}

fn parse_custom(text: &str) -> Result<CustomFile, String> {
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
    if let Some(bad) = file.on_save.iter().find(|t| {
        t.name.trim().is_empty() || t.pattern.trim().is_empty() || t.command.trim().is_empty()
    }) {
        return Err(format!(
            "{CUSTOM_FILE}: on_save의 name·pattern·command는 비울 수 없습니다(name = {:?})",
            bad.name
        ));
    }
    Ok(file)
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
    const GRADLE_BUILDS: [&str; 2] = ["build.gradle", "build.gradle.kts"];
    // 하위 프로젝트로 나눈 빌드(`gradle init`의 꼴)는 루트에 설정 파일만 두고 빌드 파일은 `app/` 등에
    // 둔다. 루트에서 돌린 작업은 그 작업이 있는 하위 프로젝트에서 다 돈다.
    if GRADLE_BUILDS
        .into_iter()
        .chain(["settings.gradle", "settings.gradle.kts"])
        .any(&has)
    {
        let gradle = wrapper(root, "gradlew", "gradle");
        // 루트와 한 단계 아래 폴더의 빌드 파일.
        let subdirs = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir());
        let script = std::iter::once(root.to_path_buf())
            .chain(subdirs)
            .flat_map(|dir| GRADLE_BUILDS.map(|f| dir.join(f)))
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .collect::<Vec<_>>()
            .join("\n");
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
    fn gradle_init의_하위_프로젝트_빌드도_감지한다() {
        let dir = project(
            "gradle-init",
            &[
                ("settings.gradle.kts", "include(\"app\")"),
                ("app/build.gradle.kts", "plugins {\n    application\n}"),
            ],
        );
        assert_eq!(
            names(&detect(&dir)),
            ["gradle run", "gradle build", "gradle test"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
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
        .unwrap()
        .run;
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].env["JAVA_HOME"], "C:/jdk8");
        let root = Path::new("project");
        assert_eq!(configs[0].working_directory(root), root.join("web"));

        assert!(
            parse_custom(CUSTOM_TEMPLATE).is_ok_and(|f| f.run.is_empty() && f.on_save.is_empty()),
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

    #[test]
    fn 저장_시_작업의_패턴과_변수() {
        let file = parse_custom(
            r#"
            [[on_save]]
            name = "Sass"
            pattern = "src/main/webapp/**/*.scss"
            command = "sass \"${file}\" \"${fileDirname}/${fileBasenameNoExtension}.css\""

            [[on_save]]
            name = "prettier"
            pattern = "*.{js,jsp}"
            command = "prettier --write ${relativeFile} # ${fileBasename} ${fileExtname} ${workspaceFolder}"
            cwd = "web"
            "#,
        )
        .unwrap();
        let [sass, prettier] = &file.on_save[..] else {
            panic!("둘이어야 한다");
        };
        let root = Path::new("/p");
        let scss = root.join("src/main/webapp/css/site.scss");
        assert!(sass.matches(root, &scss));
        assert!(
            !sass.matches(root, &root.join("site.scss")),
            "/가 있으면 루트 기준"
        );
        assert!(!sass.matches(Path::new("/other"), &scss), "프로젝트 밖");
        let dir = scss.parent().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            sass.command_for(root, &scss),
            format!("sass \"{}\" \"{dir}/site.css\"", scss.display())
        );
        let jsp = root.join("web/WEB-INF/jsp/list.jsp");
        assert!(prettier.matches(root, &jsp), "/가 없으면 어느 폴더든");
        assert!(!prettier.matches(root, &root.join("a.java")));
        assert_eq!(
            prettier.command_for(root, &jsp),
            format!(
                "prettier --write {} # list.jsp .jsp {}",
                Path::new("web/WEB-INF/jsp/list.jsp").display(),
                root.display()
            )
        );
        assert_eq!(prettier.working_directory(root), root.join("web"));
        assert!(
            parse_custom("[[on_save]]\nname = \"a\"\npattern = \"\"\ncommand = \"x\"").is_err()
        );
        assert!(
            parse_custom("[[on_save]]\nname = \"a\"\ncommand = \"x\"").is_err(),
            "pattern 빠짐"
        );
    }
}
