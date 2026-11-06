//! 요청 전 스크립트(`< {% … %}`)와 응답 처리 스크립트(`> {% … %}`, `> ./check.js`)를 돌린다. 널리 쓰는
//! API 중 흔히 쓰는 것을 둔다.
//!
//! - `client.global.set(이름, 값)`·`get`·`isEmpty`·`clear`·`clearAll`: 정한 값은 다음 요청의 `{{이름}}`에 쓴다.
//! - `client.test(이름, 함수)`: 함수가 던지면 실패. `client.assert(조건, 메시지)`, `client.log(…)`
//!   (`console.log`도 같다), `client.exit()`.
//! - `request.variables.set·get`(이 요청에만 쓰는 변수, 채울 때 가장 먼저 찾는다), `request.environment.get`,
//!   `request.method`, `request.url`·`request.body`의 `getRaw()`·`tryGetSubstituted()`(아는 변수로 채운 것).
//! - `response.status`, `response.body`(JSON이면 객체, 아니면 글), `response.headers.valueOf(이름)`·
//!   `valuesOf(이름)`, `response.contentType.mimeType`·`charset`. 요청 전 스크립트에서는 null이다.
//! - `crypto.sha256()`·`sha512()`, `crypto.hmac.sha256()`·`sha512()`(`withTextSecret`·`withHexSecret`·
//!   `withBase64Secret`): `updateWithText`·`updateWithHex`·`updateWithBase64` → `digest()` → `toHex()`·
//!   `toBase64()`.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use boa_engine::{
    Context, JsArgs, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source, js_string,
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256, Sha512};

use crate::{Globals, Response};

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

/// 스크립트가 보는 요청(`request`): 메서드, 채우기 전 주소·본문, 변수를 찾을 곳.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestView {
    pub method: String,
    pub url: String,
    pub body: Option<String>,
    /// 파일 변수(`@이름 = 값`), 쓴 차례대로.
    pub file_variables: Vec<(String, String)>,
    /// 고른 환경의 변수(`request.environment`).
    pub environment: BTreeMap<String, String>,
    /// 요청 전 스크립트가 이 요청에 정한 변수(`request.variables`).
    pub variables: Globals,
}

/// API를 만드는 JS. `__data`(요청·응답·변수)와 `__digest`(해시)를 먼저 둔다.
const PRELUDE: &str = r#"
var __globals = __data.globals;
var __vars = __data.request.variables;
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
var __has = function (map, name) { return Object.prototype.hasOwnProperty.call(map, name); };
// 보낼 때와 같은 차례로 찾는다: 요청 변수, 파일 변수(그 안의 변수도), client.global, 환경. 못 찾으면 null.
var __lookup = function (name, depth) {
  if (depth > 8) return null;
  if (__has(__vars, name)) return __vars[name];
  var file = __data.request.file;
  for (var i = file.length - 1; i >= 0; i--) {
    if (file[i][0] === name) return __fill(file[i][1], depth + 1);
  }
  if (__has(__globals, name)) return __globals[name];
  if (__has(__data.request.environment, name)) return __data.request.environment[name];
  return null;
};
var __fill = function (text, depth) {
  if (text === null) return null;
  return String(text).replace(/\{\{\s*([^{}]+?)\s*\}\}/g, function (all, name) {
    var value = __lookup(name, depth || 0);
    return value === null ? all : value;
  });
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
var request = {
  method: __data.request.method,
  url: {
    getRaw: function () { return __data.request.url; },
    tryGetSubstituted: function () { return __fill(__data.request.url); }
  },
  body: {
    getRaw: function () { return __data.request.body; },
    tryGetSubstituted: function () { return __fill(__data.request.body); }
  },
  variables: {
    set: function (name, value) { __vars[String(name)] = __text(value); },
    get: function (name) { return __has(__vars, String(name)) ? __vars[String(name)] : null; }
  },
  environment: {
    get: function (name) {
      var env = __data.request.environment;
      return __has(env, String(name)) ? env[String(name)] : null;
    }
  }
};
var response = __data.response && (function () {
  var r = __data.response;
  var values = function (name) {
    name = String(name).toLowerCase();
    return r.headers
      .filter(function (h) { return h[0].toLowerCase() === name; })
      .map(function (h) { return h[1]; });
  };
  var body = r.text;
  if (r.json) {
    try { body = JSON.parse(body); } catch (e) {}
  }
  return {
    status: r.status,
    body: body,
    headers: {
      valueOf: function (name) {
        var found = values(name);
        return found.length ? found[0] : null;
      },
      valuesOf: values
    },
    contentType: { mimeType: r.mime, charset: r.charset }
  };
})();
var __hasher = function (algorithm, secret) {
  var parts = [];
  var self = {
    updateWithText: function (text) { parts.push(["text", String(text)]); return self; },
    updateWithHex: function (text) { parts.push(["hex", String(text)]); return self; },
    updateWithBase64: function (text) { parts.push(["base64", String(text)]); return self; },
    digest: function () {
      var out = JSON.parse(__digest(algorithm, JSON.stringify(secret), JSON.stringify(parts)));
      return {
        toHex: function () { return out.hex; },
        toBase64: function () { return out.base64; }
      };
    }
  };
  return self;
};
var __hmac = function (algorithm) {
  return {
    withTextSecret: function (s) { return __hasher(algorithm, ["text", String(s)]); },
    withHexSecret: function (s) { return __hasher(algorithm, ["hex", String(s)]); },
    withBase64Secret: function (s) { return __hasher(algorithm, ["base64", String(s)]); }
  };
};
var crypto = {
  sha256: function () { return __hasher("sha256", null); },
  sha512: function () { return __hasher("sha512", null); },
  hmac: {
    sha256: function () { return __hmac("sha256"); },
    sha512: function () { return __hmac("sha512"); }
  }
};
"#;

/// 스크립트 하나: 이름(오류에 보인다)과 내용. 요청 파일 안의 스크립트는 앞을 빈 줄로 채워 오류의 줄이
/// 그 파일의 줄과 같다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub name: String,
    pub code: String,
}

/// 16진수 글을 바이트로.
fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    let bad = || format!("16진수가 아닙니다: {text}");
    if !text.len().is_multiple_of(2) {
        return Err(bad());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i + 2)
                .and_then(|b| u8::from_str_radix(b, 16).ok())
                .ok_or_else(bad)
        })
        .collect()
}

fn hash<D: Digest>(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut hasher = D::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().to_vec()
}

fn mac<M: Mac + KeyInit>(key: &[u8], parts: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let mut mac = <M as KeyInit>::new_from_slice(key).map_err(|e| e.to_string())?;
    for part in parts {
        mac.update(part);
    }
    Ok(mac.finalize().into_bytes().to_vec())
}

/// `crypto`의 해시: 알고리즘, 비밀 키(`["text"|"hex"|"base64", 값]`의 JSON, HMAC이 아니면 null), 넣은
/// 조각들(같은 꼴의 배열 JSON)을 받아 `{hex, base64}` JSON을 돌려준다.
fn digest(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let mut text = |i: usize| -> JsResult<String> {
        Ok(args
            .get_or_undefined(i)
            .to_string(ctx)?
            .to_std_string_escaped())
    };
    let (algorithm, secret, parts) = (text(0)?, text(1)?, text(2)?);
    let compute = || -> Result<Vec<u8>, String> {
        let bytes = |(kind, value): &(String, String)| match kind.as_str() {
            "hex" => from_hex(value),
            "base64" => STANDARD
                .decode(value.trim())
                .map_err(|e| format!("base64가 아닙니다: {e}")),
            _ => Ok(value.as_bytes().to_vec()),
        };
        let secret: Option<(String, String)> =
            serde_json::from_str(&secret).map_err(|e| e.to_string())?;
        let parts: Vec<(String, String)> =
            serde_json::from_str(&parts).map_err(|e| e.to_string())?;
        let parts = parts.iter().map(bytes).collect::<Result<Vec<_>, _>>()?;
        match (algorithm.as_str(), secret.as_ref().map(bytes).transpose()?) {
            ("sha256", None) => Ok(hash::<Sha256>(&parts)),
            ("sha512", None) => Ok(hash::<Sha512>(&parts)),
            ("sha256", Some(key)) => mac::<Hmac<Sha256>>(&key, &parts),
            ("sha512", Some(key)) => mac::<Hmac<Sha512>>(&key, &parts),
            _ => Err(format!("모르는 해시: {algorithm}")),
        }
    };
    let out = compute().map_err(|e| JsNativeError::typ().with_message(e))?;
    let hex: String = out.iter().map(|b| format!("{b:02x}")).collect();
    let json = serde_json::json!({ "hex": hex, "base64": STANDARD.encode(&out) }).to_string();
    Ok(JsValue::from(JsString::from(json.as_str())))
}

/// 스크립트들을 한 JS 환경에서 차례로 돌린다. 요청 전 스크립트면 `response`가 없다. `globals`
/// (`client.global`)와 `request.variables`는 스크립트가 바꾼 대로 고친다(오류가 나도 그 앞에서 정한 값은
/// 남는다).
pub fn run(
    scripts: &[Script],
    request: &mut RequestView,
    response: Option<&Response>,
    globals: &mut Globals,
) -> Outcome {
    let response = response.map(|r| {
        let mime = r
            .header("content-type")
            .and_then(|t| t.split(';').next())
            .unwrap_or("")
            .trim()
            .to_owned();
        serde_json::json!({
            "status": r.status,
            "headers": r.headers,
            "text": r.text,
            "json": mime.to_ascii_lowercase().contains("json"),
            "mime": mime,
            "charset": r.charset,
        })
    });
    let data = serde_json::json!({
        "globals": globals,
        "request": {
            "method": request.method,
            "url": request.url,
            "body": request.body,
            "file": request.file_variables,
            "environment": request.environment,
            "variables": request.variables,
        },
        "response": response,
    })
    .to_string()
    // JSON은 이 두 글자를 그대로 두지만 예전 JS 문법에서는 줄 바꿈이다.
    .replace('\u{2028}', "\\u2028")
    .replace('\u{2029}', "\\u2029");
    let mut ctx = Context::default();
    ctx.runtime_limits_mut()
        .set_loop_iteration_limit(LOOP_LIMIT);
    let mut outcome = Outcome::default();
    let prelude = ctx
        .register_global_builtin_callable(
            js_string!("__digest"),
            3,
            NativeFunction::from_fn_ptr(digest),
        )
        .and_then(|()| {
            ctx.eval(Source::from_bytes(&format!(
                "var __data = {data};\n{PRELUDE}"
            )))
        });
    if let Err(e) = prelude {
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
            "JSON.stringify({ globals: __globals, variables: __vars, log: __log, tests: __tests })",
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
    let strings = |value: &serde_json::Value| -> Option<Globals> {
        value.as_object().map(|map| {
            map.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                .collect()
        })
    };
    if let Some(map) = strings(&state["globals"]) {
        *globals = map;
    }
    if let Some(map) = strings(&state["variables"]) {
        request.variables = map;
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

    /// 응답 스크립트로 돌린다.
    fn run(scripts: &[Script], response: &Response, globals: &mut Globals) -> Outcome {
        super::run(
            scripts,
            &mut RequestView::default(),
            Some(response),
            globals,
        )
    }

    #[test]
    fn 요청_전_스크립트는_요청_변수를_정하고_아는_변수로_채운_주소를_본다() {
        let mut request = RequestView {
            method: "POST".into(),
            url: "{{host}}/api/{{path}}?t={{ts}}&x={{none}}".into(),
            body: Some("{\"user\": \"{{user}}\"}".into()),
            file_variables: vec![("host".into(), "{{base}}/app".into())],
            environment: BTreeMap::from([
                ("base".to_owned(), "http://h".to_owned()),
                ("secret".to_owned(), "key".to_owned()),
            ]),
            variables: Globals::new(),
        };
        let mut globals = Globals::from([("user".to_owned(), "홍길동".to_owned())]);
        let outcome = super::run(
            &[script(
                r#"
request.variables.set("path", "board");
request.variables.set("ts", 1700000000);
client.log(request.method, request.url.getRaw());
client.log(request.url.tryGetSubstituted());
client.log(request.body.tryGetSubstituted(), request.environment.get("secret"), request.variables.get("none"));
client.log(response === null);
"#,
            )],
            &mut request,
            None,
            &mut globals,
        );
        assert_eq!(outcome.error, None);
        assert_eq!(
            request.variables,
            Globals::from([
                ("path".to_owned(), "board".to_owned()),
                ("ts".to_owned(), "1700000000".to_owned())
            ])
        );
        assert_eq!(
            outcome.log,
            [
                "POST {{host}}/api/{{path}}?t={{ts}}&x={{none}}",
                "http://h/app/api/board?t=1700000000&x={{none}}",
                "{\"user\": \"홍길동\"} key null",
                "true"
            ],
            "모르는 변수는 그대로 둔다"
        );
    }

    #[test]
    fn 해시와_hmac은_알려진_값과_같다() {
        let mut globals = Globals::new();
        let outcome = super::run(
            &[script(
                r#"
client.log(crypto.sha256().updateWithText("abc").digest().toHex());
client.log(crypto.sha512().updateWithText("a").updateWithText("bc").digest().toHex().slice(0, 16));
client.log(crypto.hmac.sha256().withTextSecret("key").updateWithText("The quick brown fox jumps over the lazy dog").digest().toHex());
client.log(crypto.hmac.sha256().withHexSecret("6b6579").updateWithBase64("VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZw==").digest().toBase64());
client.log(crypto.sha256().updateWithText("한").digest().toHex() === crypto.sha256().updateWithHex("ED959C").digest().toHex());
crypto.sha256().updateWithHex("zz").digest();
"#,
            )],
            &mut RequestView::default(),
            None,
            &mut globals,
        );
        assert_eq!(
            outcome.log,
            [
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "ddaf35a193617aba",
                "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",
                "97yD9DBThCSxMpjmqm+xQ+9NWaFJRhdZl0edvC0aPNg=",
                "true"
            ]
        );
        let error = outcome.error.unwrap();
        assert!(error.contains("16진수가 아닙니다"), "{error}");
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
