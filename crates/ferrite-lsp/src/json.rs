//! JSON 언어 서버(`vscode-json-language-server`)에 보낼 설정과 스키마.
//! - 앱에 넣은 SchemaStore 스키마(`schemas/`, Apache-2.0)는 설정의 `json.schemas`에 내용째 넣어 망 없이 검사한다.
//!   주소는 SchemaStore의 것을 그대로 써서, 다른 스키마가 그 주소를 가리켜도(`$ref`) 받지 않고 푼다.
//! - SchemaStore 목록(약 1,400종, 파일 패턴 약 3,000개 → 스키마 주소)은 망이 되면 받아 `json/schemaAssociations`로
//!   알린다. 스키마 내용은 서버가 https로 직접 받는다. 목록을 못 받으면 내장 스키마만 쓴다.
//! - 사용자가 settings.toml의 `[[json_schema]]`로 이은 것(파일 패턴 → 주소나 프로젝트 안 경로).
//!
//! 서버는 설정을 묻지 않고 `workspace/didChangeConfiguration`으로 받는다. 설정에 `json.validate.enable`이
//! 없으면 검사를 끄므로 늘 켜서 보낸다.

use std::path::Path;

use serde_json::{Value, json};

use crate::uri::path_to_uri;

/// 카탈로그의 JSON 서버 이름.
pub const SERVER: &str = "vscode-json";
/// SchemaStore 목록.
pub const CATALOG_URL: &str = "https://www.schemastore.org/api/json/catalog.json";

/// 앱에 넣은 스키마: (SchemaStore 주소, 맞출 파일 이름, 내용). 파일 이름은 SchemaStore 목록의 것 중
/// JSON인 것이다.
const BUILTIN: [(&str, &[&str], &str); 6] = [
    (
        "https://www.schemastore.org/package.json",
        &["package.json"],
        include_str!("../schemas/package.json"),
    ),
    (
        "https://www.schemastore.org/tsconfig.json",
        &["tsconfig*.json"],
        include_str!("../schemas/tsconfig.json"),
    ),
    (
        "https://www.schemastore.org/jsconfig.json",
        &["jsconfig.json"],
        include_str!("../schemas/jsconfig.json"),
    ),
    (
        "https://www.schemastore.org/eslintrc.json",
        &[".eslintrc", ".eslintrc.json"],
        include_str!("../schemas/eslintrc.json"),
    ),
    (
        "https://www.schemastore.org/prettierrc.json",
        &[".prettierrc", ".prettierrc.json"],
        include_str!("../schemas/prettierrc.json"),
    ),
    (
        "https://www.schemastore.org/babelrc.json",
        &[".babelrc", ".babelrc.json", "babel.config.json"],
        include_str!("../schemas/babelrc.json"),
    ),
];

/// 사용자가 이은 스키마(`[[json_schema]]`): 맞출 파일 패턴과 스키마.
pub struct UserSchema<'a> {
    pub files: &'a [String],
    /// `https://…`·`file://…` 주소, 또는 프로젝트 루트 기준 경로(절대 경로도 된다).
    pub url: &'a str,
}

/// `workspace/didChangeConfiguration`의 `settings`. `root`는 프로젝트 루트(사용자 패턴과 경로의 기준).
/// `offline`이면(SchemaStore 목록을 못 받았으면) 내장 스키마에서 바깥 스키마를 가리키는 것을 뺀다:
/// 서버는 받지 못한 참조가 하나라도 있으면 그 스키마로 검사하지 않고 "Unable to load schema"만 알리는데,
/// 내장 package.json 스키마도 바깥 스키마(stylelintrc 등) 여럿을 가리킨다. 빼면 그 부분만 검사하지 않는다.
pub fn settings(root: Option<&Path>, user: &[UserSchema], offline: bool) -> Value {
    let mut schemas: Vec<Value> = BUILTIN
        .iter()
        .filter_map(|(url, files, text)| {
            let mut schema: Value = serde_json::from_str(text).ok()?;
            if offline {
                drop_external_refs(&mut schema, url);
            }
            Some(json!({ "url": url, "fileMatch": files, "schema": schema }))
        })
        .collect();
    let folder = root.and_then(|r| path_to_uri(r).ok());
    for schema in user {
        let Some(url) = resolve(schema.url, root) else {
            continue;
        };
        let mut entry = json!({ "url": url, "fileMatch": schema.files });
        // 패턴을 프로젝트 기준으로 맞춘다(`config/*.json`).
        if let Some(folder) = &folder {
            entry["folderUri"] = json!(folder.as_str());
        }
        schemas.push(entry);
    }
    json!({
        "json": {
            "validate": { "enable": true },
            "format": { "enable": true },
            "schemas": schemas,
        },
    })
}

/// 내장이 아닌 바깥 스키마를 가리키는 `$ref`를 뺀다(그 자리는 무엇이든 받는다). 같은 스키마 안(`#…`)과
/// 다른 내장 스키마를 가리키는 것은 둔다. 상대 주소는 스키마 주소(`base`)의 폴더를 기준으로 푼다.
fn drop_external_refs(value: &mut Value, base: &str) {
    match value {
        Value::Object(map) => {
            let external = map.get("$ref").and_then(Value::as_str).is_some_and(|r| {
                if r.starts_with('#') {
                    return false;
                }
                let target = r.split('#').next().unwrap_or(r);
                let target = if target.contains("://") {
                    target.to_owned()
                } else {
                    format!("{}{target}", &base[..base.rfind('/').map_or(0, |i| i + 1)])
                };
                !BUILTIN.iter().any(|(url, ..)| *url == target)
            });
            if external {
                map.remove("$ref");
            }
            for child in map.values_mut() {
                drop_external_refs(child, base);
            }
        }
        Value::Array(items) => {
            for item in items {
                drop_external_refs(item, base);
            }
        }
        _ => {}
    }
}

/// 사용자가 적은 주소를 서버가 받을 URI로. 주소(`scheme://`)는 그대로, 경로는 프로젝트 루트 기준.
fn resolve(url: &str, root: Option<&Path>) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if url.contains("://") {
        return Some(url.to_owned());
    }
    let path = Path::new(url);
    let path = match root {
        Some(root) if path.is_relative() => root.join(path),
        _ => path.to_path_buf(),
    };
    path_to_uri(&path).ok().map(|u| u.as_str().to_owned())
}

/// SchemaStore 목록(catalog.json)을 `json/schemaAssociations`의 객체 꼴(`{ 파일 패턴: [스키마 주소] }`)로.
/// 앱에 넣은 스키마는 뺀다. 배열 꼴(`[{ uri, fileMatch }]`)도 규격에 있지만, 서버의 JSON-RPC가 배열
/// `params`를 인자로 펼쳐 첫 항목만 받으므로 쓰지 않는다(VS Code 클라이언트는 감싸서 보낸다).
pub fn associations(catalog: &str) -> Result<Value, String> {
    let catalog: Value = serde_json::from_str(catalog)
        .map_err(|e| format!("SchemaStore 목록을 읽지 못했습니다: {e}"))?;
    let schemas = catalog["schemas"]
        .as_array()
        .ok_or("SchemaStore 목록에 schemas가 없습니다")?;
    let mut map = serde_json::Map::new();
    for schema in schemas {
        let Some(url) = schema["url"].as_str() else {
            continue;
        };
        if BUILTIN.iter().any(|(builtin, ..)| *builtin == url) {
            continue;
        }
        for pattern in schema["fileMatch"].as_array().into_iter().flatten() {
            let Some(pattern) = pattern.as_str() else {
                continue;
            };
            if let Value::Array(urls) = map.entry(pattern).or_insert_with(|| json!([])) {
                urls.push(json!(url));
            }
        }
    }
    Ok(Value::Object(map))
}

/// SchemaStore 목록을 받는다(약 0.5MB). 망이 막혔으면 오류.
pub fn fetch_catalog() -> Result<String, String> {
    let mut response = crate::install::agent()
        .get(CATALOG_URL)
        .call()
        .map_err(|e| format!("SchemaStore 목록을 받지 못했습니다: {e}"))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("SchemaStore 목록을 받지 못했습니다: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 내장_스키마와_사용자_스키마로_설정을_만든다() {
        let root = std::env::temp_dir().join("ferrite-json");
        let files = ["config/*.json".to_owned()];
        let remote = ["*.app.json".to_owned()];
        let settings = settings(
            Some(&root),
            &[
                UserSchema {
                    files: &files,
                    url: "schemas/config.schema.json",
                },
                UserSchema {
                    files: &remote,
                    url: "https://example.com/app.schema.json",
                },
                UserSchema {
                    files: &remote,
                    url: "  ",
                },
            ],
            false,
        );
        assert_eq!(
            settings["json"]["validate"],
            json!({ "enable": true }),
            "없으면 검사를 끈다"
        );
        let schemas = settings["json"]["schemas"].as_array().unwrap();
        assert_eq!(schemas.len(), 8, "내장 6 + 사용자 2(빈 주소는 뺀다)");
        // 내장은 내용째, 다른 스키마가 가리키는 SchemaStore 주소로.
        let package = &schemas[0];
        assert_eq!(package["url"], "https://www.schemastore.org/package.json");
        assert_eq!(package["fileMatch"], json!(["package.json"]));
        assert_eq!(package["schema"]["properties"]["name"]["type"], "string");
        assert!(
            BUILTIN
                .iter()
                .all(|(_, _, text)| serde_json::from_str::<Value>(text).is_ok()),
            "내장 스키마는 모두 JSON이다"
        );
        // 사용자: 경로는 프로젝트 기준 file URI, 주소는 그대로, 패턴은 프로젝트 기준.
        let local = &schemas[6];
        assert_eq!(
            local["url"],
            path_to_uri(&root.join("schemas/config.schema.json"))
                .unwrap()
                .as_str()
        );
        assert_eq!(local["fileMatch"], json!(["config/*.json"]));
        assert_eq!(local["folderUri"], path_to_uri(&root).unwrap().as_str());
        assert_eq!(schemas[7]["url"], "https://example.com/app.schema.json");
        assert!(schemas[7].get("schema").is_none());
    }

    /// 스키마의 `$ref` 값을 모두 모은다.
    fn refs(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                    out.push(r.to_owned());
                }
                map.values().for_each(|v| refs(v, out));
            }
            Value::Array(items) => items.iter().for_each(|v| refs(v, out)),
            _ => {}
        }
    }

    #[test]
    fn 망이_막히면_내장_스키마의_바깥_참조를_뺀다() {
        let package = |offline: bool| {
            let mut found = Vec::new();
            refs(
                &settings(None, &[], offline)["json"]["schemas"][0]["schema"],
                &mut found,
            );
            found
        };
        let online = package(false);
        assert!(online.iter().any(|r| r == "stylelintrc.json"), "상대 주소");
        assert!(online.iter().any(|r| r.ends_with("/quikrun.json")));
        let offline = package(true);
        assert!(
            offline.iter().all(|r| r.starts_with('#')
                || r == "https://www.schemastore.org/prettierrc.json"
                || r == "eslintrc.json"),
            "같은 스키마 안과 다른 내장 스키마만 남는다: {offline:?}"
        );
        assert!(offline.iter().any(|r| r.ends_with("/prettierrc.json")));
        assert!(
            offline.iter().any(|r| r == "eslintrc.json"),
            "내장을 가리키는 상대 주소도 둔다"
        );
        assert!(offline.len() > 10, "안쪽 참조는 둔다");
    }

    #[test]
    fn schemastore_목록을_패턴별_연결로_바꾸고_내장은_뺀다() {
        let catalog = r#"{ "schemas": [
            { "name": "package.json", "fileMatch": ["package.json"], "url": "https://www.schemastore.org/package.json" },
            { "name": "Renovate", "fileMatch": ["renovate.json", ".renovaterc"], "url": "https://docs.renovatebot.com/renovate-schema.json" },
            { "name": "파일 이름 없음", "url": "https://example.com/a.json" },
            { "name": "빈 이름", "fileMatch": [], "url": "https://example.com/b.json" },
            { "name": "같은 패턴", "fileMatch": [".renovaterc"], "url": "https://example.com/c.json" }
        ] }"#;
        assert_eq!(
            associations(catalog).unwrap(),
            json!({
                "renovate.json": ["https://docs.renovatebot.com/renovate-schema.json"],
                ".renovaterc": [
                    "https://docs.renovatebot.com/renovate-schema.json",
                    "https://example.com/c.json"
                ],
            })
        );
        assert!(associations("{").is_err());
        assert!(associations("{}").is_err());
    }
}
