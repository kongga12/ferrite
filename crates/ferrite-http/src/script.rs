//! 응답 처리 스크립트(`> {% … %}`, `> ./check.js`)를 돌린다. 널리 쓰는 API 중 흔히 쓰는 것을 둔다.
//!
//! - `client.global.set(이름, 값)`·`get`·`isEmpty`·`clear`·`clearAll`: 정한 값은 다음 요청의 `{{이름}}`에 쓴다.
//! - `client.test(이름, 함수)`: 함수가 던지면 실패. `client.assert(조건, 메시지)`, `client.log(…)`
//!   (`console.log`도 같다), `client.exit()`.
//! - `response.status`, `response.body`(JSON이면 객체, 아니면 글), `response.headers.valueOf(이름)`·
//!   `valuesOf(이름)`, `response.contentType.mimeType`·`charset`.
//!
//! 요청 전 스크립트(`< {% %}`)와 `request` 객체는 없다.

use std::collections::BTreeMap;
use std::path::Path;

use boa_engine::{Context, Source};

use crate::Response;

/// 한 함수 안의 반복을 모두 센 상한. 넘으면 스크립트를 멈춘다(응답 창이 다음 요청을 기다리지 않게).
// ponytail: 함수마다 따로 세므로 반복 안에서 반복하는 함수를 부르면 더 오래 돈다. 실제로 그런 스크립트가
// 문제가 되면 따로 스레드에서 돌리고 시간이 지나면 버린다.
const LOOP_LIMIT: u64 = 10_000_000;

/// `client.test` 하나의 결과.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResult {
    pub name: String,
    /// 실패한 까닭. 통과면 None.
    pub error: Option<String>,
}

/// 스크립트를 돌린 결과.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub tests: Vec<TestResult>,
    pub log: Vec<String>,
    /// 테스트 밖에서 던진 오류(문법 오류 포함). 오류가 난 스크립트에서 멈춘다.
    pub error: Option<String>,
}

/// API를 만드는 JS. `__data`(응답·변수)를 먼저 둔다.
const PRELUDE: &str = r#"
var __globals = __data.globals;
var __log = [];
var __tests = [];
var __exited = false;
var __text = function (value) {
  var text = typeof value === "string" ? value : JSON.stringify(value);
  return text === undefined ? String(value) : text;
};
var __message = function (e) {
  if (e instanceof Error) return e.name === "Error" ? e.message : e.name + ": " + e.message;
  return __text(e);
};
var client = {
  global: {
    set: function (name, value) { __globals[String(name)] = __text(value); },
    get: function (name) {
      var value = __globals[String(name)];
      return value === undefined ? null : value;
    },
    isEmpty: function () { return Object.keys(__globals).length === 0; },
    clear: function (name) { delete __globals[String(name)]; },
    clearAll: function () { __globals = {}; }
  },
  test: function (name, fn) {
    try {
      fn();
      __tests.push([String(name), null]);
    } catch (e) {
      if (__exited) throw e;
      __tests.push([String(name), __message(e)]);
    }
  },
  assert: function (condition, message) {
    if (!condition) throw new Error(message === undefined ? "Assertion failed" : String(message));
  },
  log: function () { __log.push(Array.prototype.map.call(arguments, __text).join(" ")); },
  exit: function () {
    __exited = true;
    throw new Error("client.exit()");
  }
};
var console = { log: client.log };
var response = (function () {
  var headers = __data.headers;
  var values = function (name) {
    name = String(name).toLowerCase();
    return headers
      .filter(function (h) { return h[0].toLowerCase() === name; })
      .map(function (h) { return h[1]; });
  };
  var body = __data.text;
  if (__data.json) {
    try { body = JSON.parse(body); } catch (e) {}
  }
  return {
    status: __data.status,
    body: body,
    headers: {
      valueOf: function (name) {
        var found = values(name);
        return found.length ? found[0] : null;
      },
      valuesOf: values
    },
    contentType: { mimeType: __data.mime, charset: __data.charset }
  };
})();
"#;

/// 스크립트 하나: 이름(오류에 보인다)과 내용. 요청 파일 안의 스크립트는 앞을 빈 줄로 채워 오류의 줄이
/// 그 파일의 줄과 같다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub name: String,
    pub code: String,
}

/// 스크립트들을 차례로 돌린다. `globals`(`client.global`)는 스크립트가 바꾼 대로 고친다(오류가 나도
/// 그 앞에서 정한 값은 남는다).
pub fn run(
    scripts: &[Script],
    response: &Response,
    globals: &mut BTreeMap<String, String>,
) -> Outcome {
    let mime = response
        .header("content-type")
        .and_then(|t| t.split(';').next())
        .unwrap_or("")
        .trim()
        .to_owned();
    let data = serde_json::json!({
        "globals": globals,
        "status": response.status,
        "headers": response.headers,
        "text": response.text,
        "json": mime.to_ascii_lowercase().contains("json"),
        "mime": mime,
        "charset": response.charset,
    })
    .to_string()
    // JSON은 이 두 글자를 그대로 두지만 예전 JS 문법에서는 줄 바꿈이다.
    .replace('\u{2028}', "\\u2028")
    .replace('\u{2029}', "\\u2029");
    let mut ctx = Context::default();
    ctx.runtime_limits_mut()
        .set_loop_iteration_limit(LOOP_LIMIT);
    let mut outcome = Outcome::default();
    if let Err(e) = ctx.eval(Source::from_bytes(&format!(
        "var __data = {data};\n{PRELUDE}"
    ))) {
        outcome.error = Some(format!("스크립트 환경을 만들지 못했습니다: {e}"));
        return outcome;
    }
    for script in scripts {
        // 경로를 주면 오류에 `(이름:줄:칸)`이 붙는다. 문법 오류는 붙지 않아 이름을 앞에 단다.
        let source = Source::from_bytes(&script.code).with_path(Path::new(&script.name));
        if let Err(e) = ctx.eval(source) {
            let exited = ctx
                .eval(Source::from_bytes("__exited"))
                .is_ok_and(|v| v.to_boolean());
            if !exited {
                let e = e.to_string();
                outcome.error = Some(if e.contains(&script.name) {
                    e
                } else {
                    format!("{}: {e}", script.name)
                });
            }
            break;
        }
    }
    let state = ctx
        .eval(Source::from_bytes(
            "JSON.stringify({ globals: __globals, log: __log, tests: __tests })",
        ))
        .ok()
        .and_then(|v| v.as_string().map(|s| s.to_std_string_escaped()))
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let Some(state) = state else {
        outcome
            .error
            .get_or_insert_with(|| "스크립트의 결과를 읽지 못했습니다".to_owned());
        return outcome;
    };
    if let Some(map) = state["globals"].as_object() {
        *globals = map
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
            .collect();
    }
    outcome.log = state["log"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    outcome.tests = state["tests"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| TestResult {
            name: t[0].as_str().unwrap_or_default().to_owned(),
            error: t[1].as_str().map(str::to_owned),
        })
        .collect();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn response(content_type: &str, text: &str) -> Response {
        Response {
            status: 200,
            reason: "OK".into(),
            headers: vec![
                ("Content-Type".into(), content_type.into()),
                ("Set-Cookie".into(), "a=1".into()),
                ("set-cookie".into(), "b=2".into()),
            ],
            body: text.as_bytes().to_vec(),
            text: text.into(),
            charset: "UTF-8",
            elapsed: Duration::ZERO,
            url: "http://h/".into(),
            redirects: Vec::new(),
        }
    }

    fn script(code: &str) -> Script {
        Script {
            name: "s".into(),
            code: code.into(),
        }
    }

    #[test]
    fn 응답에서_값을_꺼내_변수로_두고_테스트를_센다() {
        let r = response(
            "application/json; charset=UTF-8",
            "{\"token\": \"t-1\", \"user\": {\"id\": 7, \"name\": \"홍길동\\u2028\"}}",
        );
        let mut globals = BTreeMap::from([("old".to_owned(), "x".to_owned())]);
        let outcome = run(
            &[script(
                r#"
client.global.set("token", response.body.token);
client.global.set("id", response.body.user.id);
client.global.set("user", response.body.user);
client.global.clear("old");
client.test("상태 200", function () { client.assert(response.status === 200, "상태가 " + response.status); });
client.test("이름", () => { client.assert(response.body.user.name === "없음", "이름이 다름"); });
client.test("오타", function () { response.body.nothing.x; });
client.log("쿠키", response.headers.valuesOf("SET-COOKIE"), response.headers.valueOf("content-type"));
console.log(response.contentType.mimeType, response.contentType.charset, client.global.get("none"));
"#,
            )],
            &r,
            &mut globals,
        );
        assert_eq!(outcome.error, None);
        assert_eq!(globals["token"], "t-1");
        assert_eq!(globals["id"], "7", "숫자는 글로");
        assert_eq!(globals["user"], "{\"id\":7,\"name\":\"홍길동\u{2028}\"}");
        assert!(!globals.contains_key("old"));
        assert_eq!(
            outcome.tests[..2],
            [
                TestResult {
                    name: "상태 200".into(),
                    error: None
                },
                TestResult {
                    name: "이름".into(),
                    error: Some("이름이 다름".into())
                }
            ]
        );
        let typo = outcome.tests[2].error.as_deref().unwrap();
        assert!(typo.starts_with("TypeError"), "{typo}");
        assert_eq!(
            outcome.log,
            [
                "쿠키 [\"a=1\",\"b=2\"] application/json; charset=UTF-8",
                "application/json UTF-8 null"
            ]
        );
    }

    #[test]
    fn 글_본문_오류_멈춤_반복_상한() {
        let r = response("text/html", "<p>{\"a\":1}</p>");
        let mut globals = BTreeMap::new();
        let outcome = run(
            &[
                script("client.global.set('body', typeof response.body + response.body.length);"),
                script("client.global.set('a', 1);\n\nnothing();\nclient.global.set('b', 2);"),
                script("client.global.set('c', 3);"),
            ],
            &r,
            &mut globals,
        );
        assert_eq!(globals.get("body").map(String::as_str), Some("string14"));
        assert_eq!(
            globals.get("a").map(String::as_str),
            Some("1"),
            "오류 앞은 남는다"
        );
        assert!(
            !globals.contains_key("b") && !globals.contains_key("c"),
            "오류에서 멈춘다"
        );
        let error = outcome.error.unwrap();
        assert!(error.contains("nothing"), "{error}");

        let outcome = run(
            &[
                script("client.global.set('x', 1); client.exit(); client.global.set('y', 2);"),
                script("client.global.set('z', 3);"),
            ],
            &r,
            &mut globals,
        );
        assert_eq!(outcome.error, None, "exit는 오류가 아니다");
        assert!(
            globals.contains_key("x") && !globals.contains_key("y") && !globals.contains_key("z")
        );

        let outcome = run(&[script("while (true) {}")], &r, &mut globals);
        assert!(outcome.error.is_some(), "끝없는 반복은 멈춘다");
        let outcome = run(
            &[script("client.test('a', function () {")],
            &r,
            &mut globals,
        );
        assert!(
            outcome
                .error
                .is_some_and(|e| e.starts_with("s: SyntaxError")),
            "문법 오류에는 이름을 단다"
        );
    }
}
