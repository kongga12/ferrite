//! 실제 HTTP로 서버에 붙어 MCP 순서(initialize → initialized → tools/list → tools/call)를 확인한다.

use std::sync::Arc;

use ferrite_mcp::{Active, Debug, Diagnostic, Document, Frame, Server, Snapshot, Variable};
use serde_json::{Value, json};

const TOKEN: &str = "test-token";

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

/// (상태 코드, Mcp-Session-Id, 본문 JSON)
fn post(
    server: &Server,
    token: Option<&str>,
    origin: Option<&str>,
    body: &Value,
) -> (u16, Option<String>, Value) {
    let url = format!("http://{}/mcp", server.address());
    let mut request = agent()
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if let Some(token) = token {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    if let Some(origin) = origin {
        request = request.header("Origin", origin);
    }
    let mut response = request.send(body.to_string()).unwrap();
    let session = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let text = response.body_mut().read_to_string().unwrap();
    let value = serde_json::from_str(&text).unwrap_or(Value::Null);
    (response.status().as_u16(), session, value)
}

fn call(server: &Server, id: i64, name: &str, arguments: Value) -> Value {
    let (status, _, body) = post(
        server,
        Some(TOKEN),
        None,
        &json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": arguments } }),
    );
    assert_eq!(status, 200, "{body}");
    body
}

fn text(result: &Value) -> &str {
    result["result"]["content"][0]["text"].as_str().unwrap()
}

fn snapshot() -> Snapshot {
    Snapshot {
        root: Some("/p".into()),
        active: Some(Active {
            path: "/p/src/A.java".into(),
            line: 2,
            column: 5,
            selection: "int x".into(),
        }),
        documents: vec![Document {
            path: "/p/src/A.java".into(),
            language: "Java".into(),
            dirty: true,
            active: true,
            lines: 3,
            text: Arc::from("class A {\n    int x; // 저장 안 함\n}"),
        }],
        diagnostics: vec![Diagnostic {
            path: "/p/src/A.java".into(),
            line: 2,
            column: 9,
            severity: "warning".into(),
            message: "x를 쓰지 않음".into(),
            source: "jdtls".into(),
        }],
        debug: Some(Debug {
            name: "앱".into(),
            adapter: "java-debug".into(),
            state: "stopped".into(),
            frames: vec![Frame {
                name: "A.main".into(),
                path: Some("/p/src/A.java".into()),
                line: 2,
            }],
            variables: vec![Variable {
                scope: "Locals".into(),
                name: "x".into(),
                value: "3".into(),
                type_name: Some("int".into()),
            }],
        }),
    }
}

#[test]
fn 토큰과_origin을_확인하고_도구로_열린_파일과_진단과_디버거를_읽는다() {
    let server = Server::start(0, TOKEN).unwrap();
    server.set_snapshot(snapshot());
    let initialize = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } },
    });
    // 토큰이 없거나 틀리면, 브라우저(다른 Origin)에서 왔으면 받지 않는다.
    assert_eq!(post(&server, None, None, &initialize).0, 401);
    assert_eq!(post(&server, Some("wrong"), None, &initialize).0, 401);
    assert_eq!(
        post(
            &server,
            Some(TOKEN),
            Some("http://evil.example"),
            &initialize
        )
        .0,
        403
    );

    let (status, session, body) = post(
        &server,
        Some(TOKEN),
        Some("http://localhost:6274"),
        &initialize,
    );
    assert_eq!(status, 200);
    assert!(session.is_some_and(|s| s.len() == 64));
    assert_eq!(
        body["result"]["protocolVersion"], "2025-06-18",
        "클라이언트의 판을 따른다"
    );
    assert_eq!(body["result"]["serverInfo"]["name"], "ferrite");
    let (status, _, body) = post(
        &server,
        Some(TOKEN),
        None,
        &json!({ "jsonrpc": "2.0", "id": 9, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } }),
    );
    assert_eq!(
        (status, body["result"]["protocolVersion"].as_str()),
        (200, Some("2025-11-25"))
    );
    // 알림은 202로 받기만 한다.
    let (status, _, _) = post(
        &server,
        Some(TOKEN),
        None,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    assert_eq!(status, 202);

    let (_, _, list) = post(
        &server,
        Some(TOKEN),
        None,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_open_files",
            "read_open_file",
            "get_active_editor",
            "get_diagnostics",
            "get_debug_state"
        ]
    );
    assert!(
        list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["annotations"]["readOnlyHint"] == true)
    );

    let files: Value =
        serde_json::from_str(text(&call(&server, 3, "list_open_files", json!({})))).unwrap();
    assert_eq!(files["files"][0]["dirty"], true);
    assert!(
        files["files"][0].get("text").is_none(),
        "목록에는 내용을 싣지 않는다"
    );
    // 저장 안 한 내용, 경로 끝으로 찾기, 줄 범위
    assert_eq!(
        text(&call(
            &server,
            4,
            "read_open_file",
            json!({ "path": "src/A.java", "start_line": 2, "end_line": 2 })
        )),
        "    int x; // 저장 안 함"
    );
    let missing = call(&server, 5, "read_open_file", json!({ "path": "B.java" }));
    assert_eq!(missing["result"]["isError"], true);
    assert!(text(&missing).contains("열린 파일이 아닙니다"));
    let active: Value =
        serde_json::from_str(text(&call(&server, 6, "get_active_editor", json!({})))).unwrap();
    assert_eq!(
        (active["line"].as_i64(), active["selection"].as_str()),
        (Some(2), Some("int x"))
    );
    let diagnostics: Value = serde_json::from_str(text(&call(
        &server,
        7,
        "get_diagnostics",
        json!({ "path": "A.java" }),
    )))
    .unwrap();
    assert_eq!(diagnostics[0]["message"], "x를 쓰지 않음");
    let none: Value = serde_json::from_str(text(&call(
        &server,
        8,
        "get_diagnostics",
        json!({ "path": "B.java" }),
    )))
    .unwrap();
    assert_eq!(none, json!([]));
    let debug: Value =
        serde_json::from_str(text(&call(&server, 10, "get_debug_state", json!({})))).unwrap();
    assert_eq!(
        (
            debug["state"].as_str(),
            debug["variables"][0]["type"].as_str()
        ),
        (Some("stopped"), Some("int"))
    );

    // 모르는 도구·메서드는 JSON-RPC 오류, 다른 경로·GET은 HTTP로 거절한다.
    assert_eq!(
        call(&server, 11, "write_file", json!({}))["error"]["code"],
        -32602
    );
    let (_, _, body) = post(
        &server,
        Some(TOKEN),
        None,
        &json!({ "jsonrpc": "2.0", "id": 12, "method": "resources/list" }),
    );
    assert_eq!(body["error"]["code"], -32601);
    let url = format!("http://{}/mcp", server.address());
    let get = agent()
        .get(&url)
        .header("Authorization", &format!("Bearer {TOKEN}"))
        .call()
        .unwrap();
    assert_eq!(get.status().as_u16(), 405);
    let other = agent()
        .get(format!("http://{}/", server.address()))
        .call()
        .unwrap();
    assert_eq!(other.status().as_u16(), 404);

    // 버리면 더 듣지 않는다.
    let address = server.address();
    drop(server);
    assert!(
        std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(500))
            .is_err()
    );
}
