//! 디버그 구성. 사용자가 `.ferrite/debug.toml`에 적는 구성과, 열린 파일을 보고 만드는 구성.
//! 인자 안의 `${file}`(열린 파일), `${fileDirname}`(그 폴더), `${workspaceFolder}`(프로젝트 루트)는
//! 시작할 때 바꾼다. 그 밖의 `${…}`는 어댑터가 바꾸도록 그대로 둔다(php-debug의 `${port}`).

use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

/// 사용자 구성 파일(프로젝트 루트 기준).
pub const CUSTOM_FILE: &str = ".ferrite/debug.toml";

/// 사용자 구성 파일을 처음 만들 때의 내용.
pub const CUSTOM_TEMPLATE: &str = r#"# Ferrite 디버그 구성. [[debug]]마다 하나씩 적는다.
#   name      목록에 보일 이름
#   adapter   디버그 어댑터(debugpy)
#   request   launch(띄우기, 기본) 또는 attach(붙기)
#   arguments 어댑터에 그대로 넘기는 인자. ${file}은 열린 파일, ${fileDirname}은 그 폴더,
#             ${workspaceFolder}는 프로젝트 루트
#   build     시작하기 전에 셸로 돌릴 빌드 명령(선택). 실패하면 시작하지 않는다
#
# [[debug]]
# name = "app.py 디버그"
# adapter = "debugpy"
# arguments = { program = "${workspaceFolder}/app.py", args = ["--port", "8000"], justMyCode = true }
#
# [[debug]]
# name = "서버(Rust)"
# adapter = "codelldb"
# build = "cargo build --bin server"
# arguments = { program = "${workspaceFolder}/target/debug/server", args = [] }
"#;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebugConfig {
    pub name: String,
    pub adapter: String,
    #[serde(default = "default_request")]
    pub request: String,
    #[serde(default = "empty_object")]
    pub arguments: Value,
    /// 시작하기 전에 셸로 돌릴 빌드 명령. 실패하면 시작하지 않는다.
    #[serde(default)]
    pub build: Option<String>,
}

fn default_request() -> String {
    "launch".into()
}

fn empty_object() -> Value {
    json!({})
}

impl DebugConfig {
    pub fn attach(&self) -> bool {
        self.request == "attach"
    }

    /// 변수를 바꾼 인자. 열린 파일이 없는데 `${file}`을 쓰면 오류.
    pub fn resolved_arguments(&self, root: &Path, file: Option<&Path>) -> Result<Value, String> {
        resolve(&self.arguments, root, file)
    }
}

fn resolve(value: &Value, root: &Path, file: Option<&Path>) -> Result<Value, String> {
    Ok(match value {
        Value::String(text) => {
            let mut text = text.replace("${workspaceFolder}", &root.to_string_lossy());
            if text.contains("${file}") || text.contains("${fileDirname}") {
                let file = file.ok_or("${file}을 쓰는 구성은 파일을 연 상태에서 시작하세요")?;
                let dir = file.parent().unwrap_or(root);
                text = text
                    .replace("${fileDirname}", &dir.to_string_lossy())
                    .replace("${file}", &file.to_string_lossy());
            }
            Value::String(text)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve(v, root, file))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), resolve(v, root, file)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomFile {
    #[serde(default)]
    debug: Vec<DebugConfig>,
}

/// 사용자 구성을 읽는다. 파일이 없으면 빈 목록, 잘못 적었으면 사용자에게 보일 오류.
pub fn load_custom(root: &Path) -> Result<Vec<DebugConfig>, String> {
    let text = match std::fs::read_to_string(root.join(CUSTOM_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{CUSTOM_FILE}을 읽지 못했습니다: {e}")),
    };
    parse_custom(&text)
}

fn parse_custom(text: &str) -> Result<Vec<DebugConfig>, String> {
    let file: CustomFile = toml::from_str(text).map_err(|e| format!("{CUSTOM_FILE}: {e}"))?;
    for config in &file.debug {
        if config.name.trim().is_empty() || config.adapter.trim().is_empty() {
            return Err(format!("{CUSTOM_FILE}: name과 adapter는 비울 수 없습니다"));
        }
        if !matches!(config.request.as_str(), "launch" | "attach") {
            return Err(format!(
                "{CUSTOM_FILE}: request는 launch 또는 attach입니다({:?})",
                config.name
            ));
        }
        if !config.arguments.is_object() {
            return Err(format!(
                "{CUSTOM_FILE}: arguments는 표여야 합니다({:?})",
                config.name
            ));
        }
    }
    Ok(file.debug)
}

/// 열린 파일(과 프로젝트 루트)로 만드는 구성.
pub fn detect(file: Option<&Path>, root: &Path) -> Vec<DebugConfig> {
    let extension = file
        .and_then(Path::extension)
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let config = |name: String, adapter: &str, arguments: Value| DebugConfig {
        name,
        adapter: adapter.into(),
        request: default_request(),
        arguments,
        build: None,
    };
    match extension.as_deref() {
        Some("py") => vec![config(
            "Python: 현재 파일".into(),
            "debugpy",
            json!({
                "program": "${file}",
                "cwd": "${workspaceFolder}",
                "console": "internalConsole",
                "justMyCode": true,
            }),
        )],
        // cargo가 만드는 기본 실행 파일(패키지 이름)을 빌드한 뒤 띄운다.
        Some("rs") => cargo_package(root)
            .map(|package| {
                let program = format!(
                    "${{workspaceFolder}}/target/debug/{package}{}",
                    std::env::consts::EXE_SUFFIX
                );
                DebugConfig {
                    build: Some("cargo build".into()),
                    ..config(
                        format!("Rust: {package} (cargo build 후)"),
                        "codelldb",
                        json!({
                            "program": program,
                            "args": [],
                            "cwd": "${workspaceFolder}",
                            "sourceLanguages": ["rust"],
                        }),
                    )
                }
            })
            .into_iter()
            .collect(),
        // java-debug(jdt.ls 번들): 메인 클래스·클래스패스는 시작할 때 jdt.ls에 묻는다. 톰캣·Boot는 실행의
        // "디버그 5005"로 띄운 뒤 붙는다.
        Some("java") => vec![
            config(
                "Java: 현재 파일(main)".into(),
                "java-debug",
                json!({ "cwd": "${workspaceFolder}", "console": "internalConsole" }),
            ),
            DebugConfig {
                request: "attach".into(),
                ..config(
                    format!(
                        "Java: {} 포트에 붙기(실행의 \"디버그 {}\")",
                        crate::run::JDWP_PORT,
                        crate::run::JDWP_PORT
                    ),
                    "java-debug",
                    json!({ "hostName": "localhost", "port": crate::run::JDWP_PORT }),
                )
            },
        ],
        // delve가 그 폴더의 패키지를 디버그용으로 빌드해 띄운다.
        Some("go") => vec![config(
            "Go: 현재 패키지".into(),
            "delve",
            json!({ "mode": "debug", "program": "${fileDirname}" }),
        )],
        // php-debug가 빈 포트로 Xdebug를 기다리고, 그 포트(${port})를 PHP에 알려 띄운다.
        Some("php") => vec![config(
            "PHP: 현재 파일(Xdebug)".into(),
            "php-debug",
            json!({
                "program": "${file}",
                "cwd": "${fileDirname}",
                "port": 0,
                "runtimeArgs": ["-dxdebug.start_with_request=yes"],
                "env": { "XDEBUG_MODE": "debug,develop", "XDEBUG_CONFIG": "client_port=${port}" },
            }),
        )],
        // js-debug가 브라우저를 띄우거나 이미 뜬 브라우저에 붙어, 화면 URL의 스크립트를 webapp 폴더의
        // 파일과 맞춘다. JSP의 인라인 스크립트는 디버그 창의 "화면 스크립트"로 연다.
        Some("js" | "mjs" | "jsx" | "ts" | "tsx" | "html" | "htm" | "jsp") => browser_configs(root),
        _ => Vec::new(),
    }
}

/// 브라우저 디버그 구성: 띄우기(따로 둔 프로필)와 9222 포트로 연 브라우저에 붙기. 주소는 [`base_url`],
/// 스크립트는 webapp 폴더(`src/main/webapp`·`WebContent`·`web`, 없으면 루트)의 파일과 맞춘다. 주소에
/// 컨텍스트 경로(`/app`)가 있으면 그 아래 URL을 webapp 폴더로 맞춘다.
fn browser_configs(root: &Path) -> Vec<DebugConfig> {
    let url = base_url(root).unwrap_or_else(|| "http://localhost:8080".into());
    let web_root = ["src/main/webapp", "WebContent", "web"]
        .iter()
        .find(|dir| root.join(dir).is_dir())
        .map_or_else(
            || "${workspaceFolder}".to_owned(),
            |dir| format!("${{workspaceFolder}}/{dir}"),
        );
    let path = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    let context = path
        .find('/')
        .and_then(|i| path[i + 1..].split('/').next())
        .filter(|segment| !segment.is_empty());
    let (kind, name) = browser();
    let mut common = json!({ "type": kind, "webRoot": web_root });
    if let Some(context) = context {
        common["pathMapping"] = json!({ format!("/{context}/"): format!("{web_root}/") });
    }
    let mut launch = common.clone();
    launch["url"] = json!(url);
    let mut attach = common;
    attach["port"] = json!(9222);
    attach["urlFilter"] = json!(format!("{url}*"));
    let config = |name: String, request: &str, arguments: Value| DebugConfig {
        name,
        adapter: "js-debug".into(),
        request: request.into(),
        arguments,
        build: None,
    };
    vec![
        config(format!("브라우저: {name}로 {url} 열기"), "launch", launch),
        config(
            format!("브라우저: 9222 포트로 연 {name}에 붙기(--remote-debugging-port=9222)"),
            "attach",
            attach,
        ),
    ]
}

/// 화면 주소: HTTP 요청의 환경 파일(루트나 `http/`의 `http-client.env.json`)에서 `dev`(없으면 첫) 환경의
/// `baseUrl`.
fn base_url(root: &Path) -> Option<String> {
    ["http-client.env.json", "http/http-client.env.json"]
        .iter()
        .find_map(|name| {
            let text = std::fs::read_to_string(root.join(name)).ok()?;
            let envs: serde_json::Map<String, Value> =
                serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
            let env = envs.get("dev").or_else(|| envs.values().next())?;
            let url = env.get("baseUrl")?.as_str()?.trim_end_matches('/');
            (!url.is_empty()).then(|| url.to_owned())
        })
}

/// js-debug의 브라우저 종류와 이름. Chrome이 있으면 Chrome, 없고 Edge가 있으면(Windows·macOS) Edge.
/// Linux는 Chrome으로 둔다(js-debug가 Chrome·Chromium을 찾는다).
fn browser() -> (&'static str, &'static str) {
    let env = |key: &str| std::env::var_os(key).map(std::path::PathBuf::from);
    let (chrome, edge) = if cfg!(windows) {
        (
            ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
                .iter()
                .filter_map(|key| env(key))
                .any(|dir| dir.join("Google/Chrome/Application/chrome.exe").exists()),
            env("ProgramFiles(x86)")
                .is_some_and(|dir| dir.join("Microsoft/Edge/Application/msedge.exe").exists()),
        )
    } else if cfg!(target_os = "macos") {
        (
            Path::new("/Applications/Google Chrome.app").exists(),
            Path::new("/Applications/Microsoft Edge.app").exists(),
        )
    } else {
        (true, false)
    };
    if !chrome && edge {
        ("pwa-msedge", "Edge")
    } else {
        ("pwa-chrome", "Chrome")
    }
}

/// Cargo.toml의 패키지 이름. 패키지가 없는 작업 공간이면 None.
fn cargo_package(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let manifest: toml::Table = toml::from_str(&text).ok()?;
    manifest
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 열린_파이썬_파일로_구성을_만들고_변수를_바꾼다() {
        let file = Path::new("/p/src/main.py");
        let configs = detect(Some(file), Path::new("/p"));
        assert_eq!(configs.len(), 1);
        let arguments = configs[0]
            .resolved_arguments(Path::new("/p"), Some(file))
            .unwrap();
        assert_eq!(arguments["program"], json!(file.to_string_lossy()));
        assert_eq!(arguments["cwd"], "/p");
        assert_eq!(arguments["justMyCode"], true);
        assert!(detect(Some(Path::new("a.txt")), Path::new("/p")).is_empty());
        assert!(detect(None, Path::new("/p")).is_empty());
        assert!(
            configs[0]
                .resolved_arguments(Path::new("/p"), None)
                .is_err(),
            "열린 파일 없이 ${{file}}"
        );
    }

    #[test]
    fn rust_go_php_파일로_구성을_만든다() {
        let root =
            std::env::temp_dir().join(format!("ferrite-debug-detect-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // 패키지가 없으면(작업 공간 Cargo.toml만) Rust 구성은 만들지 않는다.
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        assert!(detect(Some(&root.join("src/main.rs")), &root).is_empty());
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"my-app\"\n").unwrap();
        let rust = &detect(Some(&root.join("src/main.rs")), &root)[0];
        assert_eq!(rust.adapter, "codelldb");
        assert_eq!(rust.build.as_deref(), Some("cargo build"));
        let program = rust.resolved_arguments(&root, None).unwrap()["program"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            program.ends_with(&format!(
                "target/debug/my-app{}",
                std::env::consts::EXE_SUFFIX
            )),
            "{program}"
        );

        let main_go = root.join("cmd").join("main.go");
        let go = &detect(Some(&main_go), &root)[0];
        let arguments = go.resolved_arguments(&root, Some(&main_go)).unwrap();
        assert_eq!(
            arguments["program"],
            json!(root.join("cmd").to_string_lossy())
        );
        assert_eq!(arguments["mode"], "debug");

        let java = detect(Some(&root.join("src/Main.java")), &root);
        assert_eq!(java.len(), 2);
        assert!(!java[0].attach() && java[1].attach());
        assert_eq!(java[1].arguments["port"], 5005);

        let php = &detect(Some(&root.join("index.php")), &root)[0];
        let arguments = php
            .resolved_arguments(&root, Some(&root.join("index.php")))
            .unwrap();
        assert_eq!(
            arguments["env"]["XDEBUG_CONFIG"], "client_port=${port}",
            "어댑터가 바꿀 변수는 그대로 둔다"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn 화면_파일로_브라우저_구성을_만들고_주소는_환경_파일에서_읽는다() {
        let root =
            std::env::temp_dir().join(format!("ferrite-debug-browser-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // 환경 파일도 webapp 폴더도 없으면 기본 주소와 루트.
        let configs = detect(Some(&root.join("index.html")), &root);
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].adapter, "js-debug");
        assert!(!configs[0].attach() && configs[1].attach());
        let launch = configs[0].resolved_arguments(&root, None).unwrap();
        assert_eq!(launch["url"], "http://localhost:8080");
        assert_eq!(launch["webRoot"], json!(root.to_string_lossy()));
        assert!(launch["type"].as_str().unwrap().starts_with("pwa-"));
        assert!(launch.get("pathMapping").is_none());

        // egov: 환경 파일의 dev baseUrl(컨텍스트 경로 /app), src/main/webapp.
        std::fs::create_dir_all(root.join("http")).unwrap();
        std::fs::create_dir_all(root.join("src/main/webapp")).unwrap();
        std::fs::write(
            root.join("http/http-client.env.json"),
            "\u{feff}{\"prod\": {\"baseUrl\": \"https://a\"}, \"dev\": {\"baseUrl\": \"http://localhost:8080/app/\"}}",
        )
        .unwrap();
        let configs = detect(Some(&root.join("WEB-INF/jsp/list.jsp")), &root);
        let web_root = format!("{}/src/main/webapp", root.display());
        let launch = configs[0].resolved_arguments(&root, None).unwrap();
        assert_eq!(launch["url"], "http://localhost:8080/app");
        assert_eq!(launch["webRoot"], json!(web_root));
        assert_eq!(
            launch["pathMapping"],
            json!({ "/app/": format!("{web_root}/") })
        );
        let attach = configs[1].resolved_arguments(&root, None).unwrap();
        assert_eq!(
            (&attach["port"], &attach["urlFilter"]),
            (&json!(9222), &json!("http://localhost:8080/app*"))
        );
        assert!(configs[0].name.contains("http://localhost:8080/app"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn 사용자_구성을_읽고_잘못_적으면_알린다() {
        let configs = parse_custom(
            r#"
            [[debug]]
            name = "앱"
            adapter = "debugpy"
            arguments = { program = "${workspaceFolder}/app.py", args = ["--port", "8000"] }

            [[debug]]
            name = "붙기"
            adapter = "debugpy"
            request = "attach"
            arguments = { connect = { host = "127.0.0.1", port = 5678 } }
            "#,
        )
        .unwrap();
        assert!(!configs[0].attach() && configs[1].attach());
        let arguments = configs[0]
            .resolved_arguments(Path::new("/w"), None)
            .unwrap();
        assert_eq!(arguments["program"], "/w/app.py");
        assert_eq!(arguments["args"], json!(["--port", "8000"]));
        assert_eq!(configs[1].arguments["connect"]["port"], 5678);

        assert!(
            parse_custom(CUSTOM_TEMPLATE).unwrap().is_empty(),
            "견본은 모두 주석"
        );
        assert!(
            parse_custom("[[debug]]\nname = \"a\"").is_err(),
            "adapter 빠짐"
        );
        assert!(
            parse_custom("[[debug]]\nname = \"a\"\nadapter = \"x\"\nrequest = \"run\"").is_err()
        );
        assert!(parse_custom("[[debug]]\nname = \"a\"\nadapter = \"x\"\narguments = 3").is_err());
        assert!(load_custom(Path::new("없는-폴더")).unwrap().is_empty());
    }
}
