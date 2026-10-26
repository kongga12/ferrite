//! 디버그 구성. 사용자가 `.ferrite/debug.toml`에 적는 구성과, 열린 파일을 보고 만드는 구성.
//! 인자 안의 `${file}`(열린 파일)과 `${workspaceFolder}`(프로젝트 루트)는 시작할 때 바꾼다.

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
#   arguments 어댑터에 그대로 넘기는 인자. ${file}은 열린 파일, ${workspaceFolder}는 프로젝트 루트
#
# [[debug]]
# name = "app.py 디버그"
# adapter = "debugpy"
# arguments = { program = "${workspaceFolder}/app.py", args = ["--port", "8000"], justMyCode = true }
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
            if text.contains("${file}") {
                let file = file.ok_or("${file}을 쓰는 구성은 파일을 연 상태에서 시작하세요")?;
                text = text.replace("${file}", &file.to_string_lossy());
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

/// 열린 파일로 만드는 구성.
pub fn detect(file: Option<&Path>) -> Vec<DebugConfig> {
    let extension = file
        .and_then(Path::extension)
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("py") => vec![DebugConfig {
            name: "Python: 현재 파일".into(),
            adapter: "debugpy".into(),
            request: default_request(),
            arguments: json!({
                "program": "${file}",
                "cwd": "${workspaceFolder}",
                "console": "internalConsole",
                "justMyCode": true,
            }),
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 열린_파이썬_파일로_구성을_만들고_변수를_바꾼다() {
        let file = Path::new("/p/src/main.py");
        let configs = detect(Some(file));
        assert_eq!(configs.len(), 1);
        let arguments = configs[0]
            .resolved_arguments(Path::new("/p"), Some(file))
            .unwrap();
        assert_eq!(arguments["program"], json!(file.to_string_lossy()));
        assert_eq!(arguments["cwd"], "/p");
        assert_eq!(arguments["justMyCode"], true);
        assert!(detect(Some(Path::new("a.rs"))).is_empty());
        assert!(detect(None).is_empty());
        assert!(
            configs[0]
                .resolved_arguments(Path::new("/p"), None)
                .is_err(),
            "열린 파일 없이 ${{file}}"
        );
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
