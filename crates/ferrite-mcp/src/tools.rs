//! JSON-RPC 처리와 도구. 도구는 앱이 넣어 둔 스냅샷만 읽는다.

use serde_json::{Value, json};

use crate::{PROTOCOL_VERSIONS, Snapshot};

/// 요청에 대한 응답. 알림(id 없음)과 응답은 None.
pub fn handle(message: &Value, snapshot: &Snapshot) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message["method"].as_str()?;
    let params = &message["params"];
    let result = match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list() })),
        "tools/call" => call(params, snapshot),
        _ => Err((-32601, format!("모르는 메서드입니다: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message },
        }),
    })
}

fn initialize(params: &Value) -> Value {
    // 클라이언트가 고른 판을 알면 그대로, 모르면 가장 새 판을 알린다(클라이언트가 이어 갈지 고른다).
    let asked = params["protocolVersion"].as_str().unwrap_or_default();
    let version = PROTOCOL_VERSIONS
        .iter()
        .find(|v| **v == asked)
        .unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "ferrite", "title": "Ferrite", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Ferrite 편집기에서 지금 열린 파일(저장하지 않은 내용 포함), 활성 에디터의 커서·선택, \
            진단(언어 서버·정적 분석·의존성 취약점), 디버거 상태를 읽는다. 파일을 바꾸지 않는다. \
            줄과 열은 1부터 센다.",
    })
}

fn tool(name: &str, title: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
        "annotations": { "readOnlyHint": true, "openWorldHint": false },
    })
}

fn tool_list() -> Vec<Value> {
    let path = json!({
        "type": "string",
        "description": "파일 경로. 절대 경로나 프로젝트 기준 상대 경로(경로 끝이 같으면 찾는다)",
    });
    vec![
        tool(
            "list_open_files",
            "열린 파일 목록",
            "Ferrite에 열린 파일과 언어, 저장하지 않은 변경이 있는지, 활성 파일인지, 프로젝트 루트를 돌려준다.",
            json!({}),
            &[],
        ),
        tool(
            "read_open_file",
            "열린 파일 읽기",
            "열린 파일의 지금 내용을 돌려준다(디스크에 저장하지 않은 편집 포함). 줄 범위를 주면 그 줄만.",
            json!({
                "path": path,
                "start_line": { "type": "integer", "minimum": 1, "description": "처음 줄(1부터, 선택)" },
                "end_line": { "type": "integer", "minimum": 1, "description": "끝 줄(포함, 선택)" },
            }),
            &["path"],
        ),
        tool(
            "get_active_editor",
            "활성 에디터",
            "활성 파일의 경로, 커서의 줄·열, 고른 글을 돌려준다.",
            json!({}),
            &[],
        ),
        tool(
            "get_diagnostics",
            "진단",
            "언어 서버 진단(열린 파일), 정적 분석(SARIF), 의존성 취약점을 파일·줄·열·심각도·메시지·출처로 돌려준다.",
            json!({ "path": path }),
            &[],
        ),
        tool(
            "get_debug_state",
            "디버거 상태",
            "디버그 세션의 구성·어댑터·상태, 멈춘 자리, 호출 스택, 불러온 변수(고른 프레임)를 돌려준다.",
            json!({}),
            &[],
        ),
    ]
}

type CallResult = Result<Value, (i64, String)>;

fn text(text: String, error: bool) -> CallResult {
    Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": error }))
}

fn pretty(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// 경로가 같거나 끝이 같은지(구분자는 `/`·`\` 모두).
fn same_path(path: &str, wanted: &str) -> bool {
    let norm = |p: &str| p.replace('\\', "/");
    let (path, wanted) = (norm(path), norm(wanted).trim_start_matches("./").to_owned());
    path == wanted || path.ends_with(&format!("/{wanted}"))
}

fn call(params: &Value, snapshot: &Snapshot) -> CallResult {
    let name = params["name"].as_str().unwrap_or_default();
    let args = &params["arguments"];
    match name {
        "list_open_files" => text(
            pretty(&json!({ "root": snapshot.root, "files": snapshot.documents })),
            false,
        ),
        "read_open_file" => {
            let Some(wanted) = args["path"].as_str() else {
                return Err((-32602, "path가 필요합니다".into()));
            };
            let Some(doc) = snapshot
                .documents
                .iter()
                .find(|d| same_path(&d.path, wanted))
            else {
                return text(
                    format!(
                        "열린 파일이 아닙니다: {wanted} (list_open_files로 열린 파일을 보세요)"
                    ),
                    true,
                );
            };
            let lines: Vec<&str> = doc.text.split('\n').collect();
            let line = |key: &str| args[key].as_u64().map(|n| n as usize);
            let start = line("start_line").unwrap_or(1).max(1);
            let end = line("end_line").unwrap_or(lines.len()).min(lines.len());
            if start > end {
                return text(
                    format!("줄 범위가 비었습니다(파일은 {}줄)", lines.len()),
                    true,
                );
            }
            text(lines[start - 1..end].join("\n"), false)
        }
        "get_active_editor" => match &snapshot.active {
            Some(active) => text(pretty(active), false),
            None => text("열린 파일이 없습니다".into(), false),
        },
        "get_diagnostics" => {
            let wanted = args["path"].as_str();
            let found: Vec<_> = snapshot
                .diagnostics
                .iter()
                .filter(|d| wanted.is_none_or(|w| same_path(&d.path, w)))
                .collect();
            text(pretty(&found), false)
        }
        "get_debug_state" => match &snapshot.debug {
            Some(debug) => text(pretty(debug), false),
            None => text("디버그 세션이 없습니다".into(), false),
        },
        _ => Err((-32602, format!("모르는 도구입니다: {name}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 경로는_끝이_같아도_찾는다() {
        assert!(same_path("C:\\p\\src\\A.java", "src/A.java"));
        assert!(same_path("/p/src/A.java", "./src/A.java"));
        assert!(same_path("/p/src/A.java", "/p/src/A.java"));
        assert!(!same_path("/p/src/BA.java", "A.java"));
    }
}
