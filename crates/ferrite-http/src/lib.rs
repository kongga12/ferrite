//! HTTP 요청 파일(`.http`): 해석(`parse`), 환경 파일(`env`), 보내기(`send`). 쿠키는 세션 동안
//! `CookieJar`에 두고 이어 보내는 요청에 붙인다. 리다이렉트는 직접 따라가며 거치는 응답의 쿠키도 담는다
//! (로그인의 302 응답이 세션 쿠키를 준다).

pub mod cookies;
pub mod env;
pub mod parse;

use std::cell::RefCell;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ureq::Agent;
use ureq::http;
use ureq::tls::{RootCerts, TlsConfig};

pub use cookies::CookieJar;
pub use env::Environments;
pub use parse::{HttpFile, Request, parse};

/// 받는 본문의 상한.
const MAX_BODY: u64 = 20 * 1024 * 1024;
const MAX_REDIRECTS: usize = 10;

/// 변수를 채운 보낼 요청.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub insecure: bool,
    pub no_redirect: bool,
    pub no_cookie_jar: bool,
}

/// 요청의 `{{변수}}`를 채운다. 찾는 순서: 파일 변수(그 안의 변수도 채운다), 고른 환경, 동적 변수
/// (`$uuid`, `$timestamp`, `$randomInt`).
pub fn prepare(
    file: &HttpFile,
    request: &Request,
    env: Option<(&Environments, &str)>,
) -> Result<Prepared, String> {
    /// 파일 변수 안에서 못 채운 이름은 `nested`에 모은다(바깥의 파일 변수 이름보다 쓸모 있다).
    fn lookup(
        file: &HttpFile,
        env: Option<(&Environments, &str)>,
        name: &str,
        depth: u8,
        nested: &RefCell<Vec<String>>,
    ) -> Option<String> {
        if depth > 8 {
            return None;
        }
        if let Some((_, value)) = file.variables.iter().rev().find(|(n, _)| n == name) {
            return parse::substitute(value, &|n| lookup(file, env, n, depth + 1, nested))
                .map_err(|names| nested.borrow_mut().extend(names))
                .ok();
        }
        if let Some(value) = env.and_then(|(envs, selected)| envs.get(selected, name)) {
            return Some(value.to_owned());
        }
        dynamic(name)
    }
    let nested = RefCell::new(Vec::new());
    let fill = |text: &str| parse::substitute(text, &|n| lookup(file, env, n, 0, &nested));
    let mut missing = Vec::new();
    let mut take = |result: Result<String, Vec<String>>| {
        result.unwrap_or_else(|names| {
            for name in names {
                if !missing.contains(&name) {
                    missing.push(name);
                }
            }
            String::new()
        })
    };
    let url = take(fill(&request.url));
    let headers = request
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), take(fill(v))))
        .collect();
    let body = request.body.as_deref().map(|b| take(fill(b)));
    missing.retain(|name| !file.variables.iter().any(|(n, _)| n == name));
    for name in nested.into_inner() {
        if !missing.contains(&name) {
            missing.push(name);
        }
    }
    if !missing.is_empty() {
        let hint = match env {
            Some((_, selected)) => format!("(환경 {selected})"),
            None => "(환경을 고르지 않았다)".to_owned(),
        };
        return Err(format!("채우지 못한 변수: {} {hint}", missing.join(", ")));
    }
    Ok(Prepared {
        method: request.method.clone(),
        url,
        headers,
        body,
        insecure: request.insecure,
        no_redirect: request.no_redirect,
        no_cookie_jar: request.no_cookie_jar,
    })
}

fn random() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    hasher.finish()
}

/// 동적 변수.
fn dynamic(name: &str) -> Option<String> {
    match name {
        "$uuid" | "$random.uuid" => {
            let (a, b) = (random(), random());
            Some(format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                a >> 32,
                (a >> 16) & 0xffff,
                a & 0xfff,
                (b >> 48 & 0x3fff) | 0x8000,
                b & 0xffff_ffff_ffff
            ))
        }
        "$timestamp" => Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string(),
        ),
        "$randomInt" => Some((random() % 1000).to_string()),
        _ => None,
    }
}

/// 받은 응답.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// 문자 집합대로 푼 본문. JSON이면 들여 쓴다.
    pub text: String,
    /// 본문을 푼 문자 집합.
    pub charset: &'static str,
    /// 리다이렉트까지 모두 걸린 시간.
    pub elapsed: Duration,
    /// 마지막 주소.
    pub url: String,
    /// 거친 리다이렉트(상태, 주소).
    pub redirects: Vec<(u16, String)>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// 요청을 보내는 쪽. 인증서를 검증하는 것과 하지 않는 것(`# @insecure`) 두 가지를 둔다.
pub struct Client {
    secure: Agent,
    insecure: Agent,
}

impl Default for Client {
    fn default() -> Self {
        let agent = |verify: bool| -> Agent {
            let tls = if verify {
                // 회사망의 TLS 검사 프록시처럼 OS에 추가된 루트 인증서도 믿는다.
                TlsConfig::builder().root_certs(RootCerts::PlatformVerifier)
            } else {
                TlsConfig::builder().disable_verification(true)
            };
            Agent::config_builder()
                .tls_config(tls.build())
                .http_status_as_error(false)
                // 리다이렉트는 직접 따라간다(거치는 응답의 쿠키를 담으려고).
                .max_redirects(0)
                .timeout_connect(Some(Duration::from_secs(15)))
                .timeout_global(Some(Duration::from_secs(120)))
                .user_agent(concat!("ferrite/", env!("CARGO_PKG_VERSION")))
                .build()
                .into()
        };
        Self {
            secure: agent(true),
            insecure: agent(false),
        }
    }
}

impl Client {
    /// 보낸다. 쿠키는 `jar`에서 붙이고 받은 것을 담는다(`# @no-cookie-jar`면 둘 다 하지 않는다).
    pub fn send(&self, request: &Prepared, jar: &mut CookieJar) -> Result<Response, String> {
        let agent = if request.insecure {
            &self.insecure
        } else {
            &self.secure
        };
        let started = Instant::now();
        let mut method = request.method.clone();
        let mut url = request.url.clone();
        let mut body = request.body.clone();
        let mut headers = request.headers.clone();
        let mut redirects = Vec::new();
        loop {
            let mut builder = http::Request::builder()
                .method(method.as_str())
                .uri(url.as_str());
            for (k, v) in &headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            if !request.no_cookie_jar
                && !headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("cookie"))
                && let Some(cookie) = jar.header(&url)
            {
                builder = builder.header("Cookie", cookie);
            }
            let result = match &body {
                Some(b) => builder
                    .body(b.clone().into_bytes())
                    .map_err(|e| e.to_string())
                    .and_then(|r| agent.run(r).map_err(describe)),
                None => builder
                    .body(())
                    .map_err(|e| e.to_string())
                    .and_then(|r| agent.run(r).map_err(describe)),
            };
            let mut response = result?;
            let status = response.status().as_u16();
            let response_headers: Vec<(String, String)> = response
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_owned(),
                        String::from_utf8_lossy(v.as_bytes()).into_owned(),
                    )
                })
                .collect();
            if !request.no_cookie_jar {
                for (_, value) in response_headers
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                {
                    jar.store(&url, value);
                }
            }
            let location = response_headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                .map(|(_, v)| v.clone());
            if !request.no_redirect
                && matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(location) = location
                && redirects.len() < MAX_REDIRECTS
            {
                let next = resolve(&url, &location);
                redirects.push((status, next.clone()));
                // 303과 POST의 301·302는 GET으로 바꾸고 본문을 뺀다(브라우저와 같다).
                if status == 303
                    || (matches!(status, 301 | 302) && method != "GET" && method != "HEAD")
                {
                    method = "GET".into();
                    body = None;
                    headers.retain(|(k, _)| {
                        !k.eq_ignore_ascii_case("content-type")
                            && !k.eq_ignore_ascii_case("content-length")
                    });
                }
                url = next;
                continue;
            }
            let bytes = response
                .body_mut()
                .with_config()
                .limit(MAX_BODY)
                .read_to_vec()
                .map_err(describe)?;
            let content_type = response_headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v.as_str())
                .unwrap_or("");
            let (text, charset) = decode(&bytes, content_type);
            return Ok(Response {
                status,
                reason: response
                    .status()
                    .canonical_reason()
                    .unwrap_or("")
                    .to_owned(),
                headers: response_headers.clone(),
                text: pretty_json(&text, content_type).unwrap_or(text),
                charset,
                body: bytes,
                elapsed: started.elapsed(),
                url,
                redirects,
            });
        }
    }
}

fn describe(e: ureq::Error) -> String {
    let text = e.to_string();
    if text.to_lowercase().contains("certificate") || text.contains("UnknownIssuer") {
        format!("{text} (개발 서버의 자체 서명 인증서면 요청 위에 `# @insecure`를 적는다)")
    } else {
        text
    }
}

/// 리다이렉트 주소를 지금 주소 기준으로 푼다.
fn resolve(base: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_owned();
    }
    let Ok(uri) = base.parse::<http::Uri>() else {
        return location.to_owned();
    };
    let scheme = uri.scheme_str().unwrap_or("http");
    let authority = uri.authority().map(|a| a.as_str()).unwrap_or("");
    if let Some(rest) = location.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    if location.starts_with('/') {
        return format!("{scheme}://{authority}{location}");
    }
    let dir = uri.path().rsplit_once('/').map_or("", |(d, _)| d);
    format!("{scheme}://{authority}{dir}/{location}")
}

/// 본문을 문자 집합대로 푼다: `Content-Type`의 charset, 없으면 UTF-8, UTF-8이 아니면 EUC-KR(CP949,
/// 예전 egov 화면).
pub fn decode(bytes: &[u8], content_type: &str) -> (String, &'static str) {
    let label = content_type
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, v)| v.trim().trim_matches('"'));
    if let Some(encoding) = label.and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes())) {
        let (text, _, _) = encoding.decode(bytes);
        return (text.into_owned(), encoding.name());
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), "UTF-8"),
        Err(_) => {
            let (text, _, _) = encoding_rs::EUC_KR.decode(bytes);
            (text.into_owned(), encoding_rs::EUC_KR.name())
        }
    }
}

/// JSON이면 들여 쓴다. 다시 쓰지 않고 글을 그대로 들여 써서 서버가 보낸 키 차례와 숫자 표기
/// (`1.10`, 큰 정수)가 바뀌지 않는다.
fn pretty_json(text: &str, content_type: &str) -> Option<String> {
    let looks = content_type.to_ascii_lowercase().contains("json")
        || text.trim_start().starts_with(['{', '[']);
    if !looks || serde_json::from_str::<serde_json::Value>(text).is_err() {
        return None;
    }
    let mut out = String::with_capacity(text.len() * 2);
    let mut depth = 0usize;
    let (mut in_string, mut escaped) = (false, false);
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n("  ", depth));
    };
    let mut chars = text.trim().chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
                // 빈 것은 `{}` 그대로.
                if let Some(close) = chars.next_if(|&n| n == '}' || n == ']') {
                    out.push(close);
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 변수는_파일_환경_동적_순서로_채운다() {
        let file = parse(
            "@path = {{base}}/app\n@id = 7\n###\nGET {{path}}/x?id={{id}}&t={{$timestamp}}\nX-Req: {{$uuid}}\n",
        );
        let mut envs = Environments::default();
        envs.values
            .entry("dev".into())
            .or_default()
            .insert("base".into(), "http://h".into());
        let prepared = prepare(&file, &file.requests[0], Some((&envs, "dev"))).unwrap();
        assert!(
            prepared.url.starts_with("http://h/app/x?id=7&t="),
            "{}",
            prepared.url
        );
        let uuid = &prepared.headers[0].1;
        assert_eq!((uuid.len(), uuid.as_bytes()[14]), (36, b'4'));
        let err = prepare(&file, &file.requests[0], None).unwrap_err();
        assert!(
            err.contains("base") && err.contains("환경을 고르지 않았다"),
            "{err}"
        );
    }

    #[test]
    fn 문자_집합과_json과_리다이렉트_주소() {
        let (euc, _, _) = encoding_rs::EUC_KR.encode("한글");
        assert_eq!(
            decode(&euc, "text/html; charset=euc-kr"),
            ("한글".into(), "EUC-KR")
        );
        assert_eq!(
            decode(&euc, "text/html").1,
            "EUC-KR",
            "UTF-8이 아니면 EUC-KR"
        );
        assert_eq!(decode("한글".as_bytes(), "").0, "한글");
        assert_eq!(
            pretty_json(
                r#"{"z":"한 \"{,:\\","b":[1.10, {}],"a":[ ]}"#,
                "application/json"
            )
            .unwrap(),
            "{\n  \"z\": \"한 \\\"{,:\\\\\",\n  \"b\": [\n    1.10,\n    {}\n  ],\n  \"a\": []\n}",
            "키 차례·숫자 표기·문자열 속 기호를 그대로 둔다"
        );
        assert_eq!(pretty_json("<html>", "text/html"), None);
        assert_eq!(
            resolve("http://h:8080/app/login.do", "/app/main.do"),
            "http://h:8080/app/main.do"
        );
        assert_eq!(
            resolve("http://h:8080/app/login.do", "main.do"),
            "http://h:8080/app/main.do"
        );
        assert_eq!(resolve("https://h/a", "//cdn/x"), "https://cdn/x");
        assert_eq!(resolve("http://h/a", "https://o/b"), "https://o/b");
    }
}
