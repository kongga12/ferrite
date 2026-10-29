//! 어댑터가 돌려주는 본문 중 화면에 쓰는 것만 골라 읽는다. 모르는 필드는 버린다.

use std::path::PathBuf;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    pub supports_configuration_done_request: bool,
    pub supports_conditional_breakpoints: bool,
    pub supports_terminate_request: bool,
    pub supports_loaded_sources_request: bool,
    pub exception_breakpoint_filters: Option<Vec<Value>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Source {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// 0보다 크면 어댑터에 `source`로 내용을 물어야 한다(로컬 파일이 없는 페이지 스크립트 등).
    #[serde(default, rename = "sourceReference")]
    pub source_reference: i64,
}

impl Source {
    /// 어댑터에 다시 보낼 때의 꼴(중단점, `source` 요청).
    pub fn to_json(&self) -> Value {
        let mut source = serde_json::Map::new();
        if let Some(name) = &self.name {
            source.insert("name".into(), Value::from(name.as_str()));
        }
        if let Some(path) = &self.path {
            source.insert("path".into(), Value::from(path.to_string_lossy()));
        }
        if self.source_reference > 0 {
            source.insert("sourceReference".into(), Value::from(self.source_reference));
        }
        Value::Object(source)
    }
}

/// 줄·열은 1부터 센다(초기화 때 그렇게 요청한다).
#[derive(Clone, Debug, Deserialize)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub source: Option<Source>,
    pub line: i64,
    #[serde(default)]
    pub column: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub name: String,
    pub variables_reference: i64,
    #[serde(default)]
    pub expensive: bool,
}

/// `variables_reference`가 0이 아니면 펼칠 수 있다(그 번호로 `variables`를 다시 묻는다).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(default, rename = "type")]
    pub type_name: Option<String>,
    #[serde(default)]
    pub variables_reference: i64,
}

/// 어댑터가 확인한 중단점. 실제로 걸 수 있는 줄로 옮겼으면 `line`이 바뀐다.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct BreakpointState {
    /// 어댑터가 붙인 번호. 나중에 `breakpoint` 이벤트로 이 번호의 확인 상태를 바꾼다.
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub message: Option<String>,
}

/// 응답 본문의 목록 필드를 읽는다. 읽을 수 없는 항목은 건너뛴다.
fn list<T: DeserializeOwned>(body: &Value, key: &str) -> Vec<T> {
    body.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// `threads` 응답.
pub fn threads(body: &Value) -> Vec<Thread> {
    list(body, "threads")
}

/// `stackTrace` 응답.
pub fn stack_frames(body: &Value) -> Vec<StackFrame> {
    list(body, "stackFrames")
}

/// `scopes` 응답.
pub fn scopes(body: &Value) -> Vec<Scope> {
    list(body, "scopes")
}

/// `loadedSources` 응답.
pub fn sources(body: &Value) -> Vec<Source> {
    list(body, "sources")
}

/// `variables` 응답.
pub fn variables(body: &Value) -> Vec<Variable> {
    list(body, "variables")
}

pub(crate) fn breakpoint_states(body: &Value) -> Vec<BreakpointState> {
    list(body, "breakpoints")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn 응답_본문을_읽고_모르는_필드와_깨진_항목은_넘긴다() {
        let body = json!({ "stackFrames": [
            { "id": 1, "name": "main", "line": 3, "column": 1,
              "source": { "path": "/p/a.py", "sourceReference": 0 }, "presentationHint": "normal" },
            { "id": "깨짐" },
        ]});
        let frames = stack_frames(&body);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].source.as_ref().unwrap().path.as_deref(),
            Some(std::path::Path::new("/p/a.py"))
        );
        let vars = variables(&json!({ "variables": [
            { "name": "x", "value": "42", "type": "int", "variablesReference": 0 },
            { "name": "items", "value": "[1, 2]", "variablesReference": 5 },
        ]}));
        assert_eq!(vars[0].type_name.as_deref(), Some("int"));
        assert_eq!(vars[1].variables_reference, 5);
        assert!(threads(&json!({})).is_empty());
    }
}
