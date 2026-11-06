//! `.http` 파일 해석. 널리 쓰는 꼴을 따른다.
//!
//! ```text
//! @host = {{baseUrl}}/api          ← 파일 변수(첫 요청 앞)
//! ### 게시판 목록                   ← 요청을 나눈다. 뒤의 글은 이름
//! # @name list                     ← 지시: @name, @insecure(자체 서명 인증서), @no-redirect, @no-cookie-jar
//! GET {{host}}/board/list.do?page=1
//!     &size=10                     ← 질의 이어 쓰기
//! Accept: application/json         ← 헤더(빈 줄까지)
//!
//! {"q": "본문"}                     ← 본문(다음 ### 까지)
//! < ./photo.png                    ← 본문 줄 대신 그 파일 내용(멀티파트의 파트에도 쓴다)
//! >> ./out.json                    ← 응답 본문을 저장(`>>!`는 덮어쓴다)
//! ```
//!
//!
//! 응답 처리 스크립트(`> {% … %}`, `> ./script.js`)는 본문에서 빼서 따로 둔다([`crate::script`]).
//! 요청 전 스크립트(요청 줄 앞의 `< {% … %}`, `< ./pre.js`)는 돌리지 않고 건너뛴다.

use std::ops::Range;

const METHODS: [&str; 9] = [
    "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE", "CONNECT",
];

/// 요청 하나. 글은 변수를 채우기 전 그대로다.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// 요청 줄(0부터).
    pub line: usize,
    /// 이 요청이 차지하는 줄(`###` 줄부터 다음 `###` 앞까지).
    pub lines: Range<usize>,
    pub name: Option<String>,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// 본문 글. `< 경로` 줄은 그대로 두고 보낼 때 파일 내용으로 바꾼다.
    pub body: Option<String>,
    /// `>> 경로`(덮어쓰면 `>>!`): 응답 본문을 저장할 파일(변수를 채우기 전).
    pub save: Option<Save>,
    /// 응답 처리 스크립트, 쓴 차례대로.
    pub handlers: Vec<Handler>,
    /// 요청 전 스크립트가 있었다(돌리지 않는다).
    pub pre_script: bool,
    /// 이 요청만 서버 인증서를 검증하지 않는다(개발 서버의 자체 서명 인증서).
    pub insecure: bool,
    pub no_redirect: bool,
    pub no_cookie_jar: bool,
}

/// 응답 처리 스크립트.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handler {
    /// `> {% … %}`: `{%` 뒤부터 `%}` 앞까지. `line`은 `{%`가 든 줄(0부터)이다.
    Inline { line: usize, code: String },
    /// `> 경로`: 요청 파일의 폴더 기준 스크립트 파일.
    File(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub path: String,
    pub overwrite: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpFile {
    /// 파일 변수(`@이름 = 값`), 쓴 차례대로.
    pub variables: Vec<(String, String)>,
    pub requests: Vec<Request>,
}

impl HttpFile {
    /// 이 줄이 든 요청. 요청 사이 빈 곳이면 앞의 요청.
    pub fn request_at(&self, line: usize) -> Option<&Request> {
        self.requests.iter().rev().find(|r| r.lines.start <= line)
    }
}

enum State {
    Before,
    Headers,
    Body,
}

pub fn parse(text: &str) -> HttpFile {
    let lines: Vec<&str> = text.lines().collect();
    let mut file = HttpFile::default();
    let mut start = 0;
    while start <= lines.len() {
        // 블록: start(### 줄이면 그 다음)부터 다음 ### 앞까지.
        let mut name = None;
        let mut first = start;
        if let Some(rest) = lines
            .get(start)
            .and_then(|l| l.trim_start().strip_prefix("###"))
        {
            let rest = rest.trim();
            name = (!rest.is_empty()).then(|| rest.to_owned());
            first = start + 1;
        }
        let end = (first..lines.len())
            .find(|&i| lines[i].trim_start().starts_with("###"))
            .unwrap_or(lines.len());
        if let Some(mut request) = block(&lines, first..end, &mut file.variables) {
            request.name = request.name.or(name);
            // `###` 줄도 그 요청에 든다(그 줄에서 Ctrl+Enter).
            request.lines.start = start;
            file.requests.push(request);
        }
        if end >= lines.len() {
            break;
        }
        start = end;
    }
    file
}

fn block(
    lines: &[&str],
    range: Range<usize>,
    variables: &mut Vec<(String, String)>,
) -> Option<Request> {
    let mut request = Request {
        lines: range.clone(),
        ..Default::default()
    };
    let mut state = State::Before;
    let mut body: Vec<&str> = Vec::new();
    let mut found = false;
    // `> {%`부터 `%}`까지(응답 처리 스크립트) 안이면 그 시작 줄과 지금까지의 글.
    let mut handler: Option<(usize, String)> = None;
    // 요청 줄 앞의 `< {%`부터 `%}`까지(요청 전 스크립트) 안이다.
    let mut in_pre_script = false;
    for i in range {
        let line = lines[i];
        let trimmed = line.trim();
        // 본문 없이 헤더 바로 뒤에 온 `>> 경로`·`> 스크립트`도 받는다.
        if matches!(state, State::Headers) && trimmed.starts_with('>') {
            state = State::Body;
        }
        match state {
            State::Before => {
                if in_pre_script {
                    in_pre_script = !trimmed.contains("%}");
                    continue;
                }
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(rest) = trimmed.strip_prefix('<') {
                    request.pre_script = true;
                    let rest = rest.trim_start();
                    in_pre_script = rest.starts_with("{%") && !rest.contains("%}");
                    continue;
                }
                let comment = trimmed
                    .strip_prefix('#')
                    .or_else(|| trimmed.strip_prefix("//"));
                if let Some(comment) = comment {
                    let comment = comment.trim();
                    if let Some(name) = comment.strip_prefix("@name") {
                        request.name = Some(name.trim().to_owned()).filter(|n| !n.is_empty());
                    }
                    match comment {
                        "@insecure" => request.insecure = true,
                        "@no-redirect" => request.no_redirect = true,
                        "@no-cookie-jar" => request.no_cookie_jar = true,
                        _ => {}
                    }
                    continue;
                }
                if let Some(rest) = trimmed.strip_prefix('@')
                    && let Some((name, value)) = rest.split_once('=')
                {
                    variables.push((name.trim().to_owned(), value.trim().to_owned()));
                    continue;
                }
                let (method, rest) = match trimmed.split_once(char::is_whitespace) {
                    Some((m, rest)) if METHODS.contains(&m.to_ascii_uppercase().as_str()) => {
                        (m.to_ascii_uppercase(), rest.trim())
                    }
                    _ if METHODS.contains(&trimmed.to_ascii_uppercase().as_str()) => {
                        (trimmed.to_ascii_uppercase(), "")
                    }
                    _ => ("GET".to_owned(), trimmed),
                };
                // 끝의 `HTTP/1.1`은 뺀다.
                let url = match rest.rsplit_once(char::is_whitespace) {
                    Some((url, version)) if version.starts_with("HTTP/") => url.trim(),
                    _ => rest,
                };
                request.method = method;
                request.url = url.to_owned();
                request.line = i;
                found = true;
                state = State::Headers;
            }
            State::Headers => {
                if trimmed.is_empty() {
                    state = State::Body;
                } else if trimmed.starts_with('?') || trimmed.starts_with('&') {
                    request.url.push_str(trimmed);
                } else if trimmed.starts_with('#') || trimmed.starts_with("//") {
                    // 헤더 사이 주석
                } else if let Some((name, value)) = trimmed.split_once(':') {
                    request
                        .headers
                        .push((name.trim().to_owned(), value.trim().to_owned()));
                }
            }
            State::Body => {
                if let Some((start, mut code)) = handler.take() {
                    code.push('\n');
                    match line.split_once("%}") {
                        Some((last, _)) => {
                            code.push_str(last);
                            request.handlers.push(Handler::Inline { line: start, code });
                        }
                        None => {
                            code.push_str(line);
                            handler = Some((start, code));
                        }
                    }
                } else if let Some(rest) = trimmed.strip_prefix(">>") {
                    let (overwrite, path) = match rest.strip_prefix('!') {
                        Some(path) => (true, path),
                        None => (false, rest),
                    };
                    let path = path.trim();
                    if !path.is_empty() {
                        request.save = Some(Save {
                            path: path.to_owned(),
                            overwrite,
                        });
                    }
                } else if let Some(rest) = trimmed.strip_prefix("> ") {
                    let rest = rest.trim();
                    match rest.strip_prefix("{%") {
                        Some(code) => match code.split_once("%}") {
                            Some((code, _)) => request.handlers.push(Handler::Inline {
                                line: i,
                                code: code.to_owned(),
                            }),
                            None => handler = Some((i, code.to_owned())),
                        },
                        None if !rest.is_empty() => {
                            request.handlers.push(Handler::File(rest.to_owned()))
                        }
                        None => {}
                    }
                } else {
                    body.push(line);
                }
            }
        }
    }
    // 닫지 않은 스크립트도 둔다(돌리면 문법 오류로 알린다).
    if let Some((line, code)) = handler {
        request.handlers.push(Handler::Inline { line, code });
    }
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    if !body.is_empty() {
        request.body = Some(body.join("\n"));
    }
    found.then_some(request)
}

/// `{{이름}}`을 채운다. 채우지 못한 이름들을 오류로 돌려준다. `lookup`은 앞뒤 공백을 뺀 이름을 받는다.
pub fn substitute(
    text: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, Vec<String>> {
    let mut out = String::with_capacity(text.len());
    let mut missing = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open + 2..].find("}}") else {
            break;
        };
        out.push_str(&rest[..open]);
        let name = rest[open + 2..open + 2 + close].trim();
        match lookup(name) {
            Some(value) => out.push_str(&value),
            None => {
                if !missing.iter().any(|m| m == name) {
                    missing.push(name.to_owned());
                }
            }
        }
        rest = &rest[open + 2 + close + 2..];
    }
    out.push_str(rest);
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "@host = {{baseUrl}}/app\n\n### 목록\nGET {{host}}/list.do?page=1\n    &size=10\nAccept: application/json\n\n### 저장\n# @name save\n# @insecure\nPOST {{host}}/save.do HTTP/1.1\nContent-Type: application/x-www-form-urlencoded\n\ntitle=안녕&body=x\n\n\n###\n# 주석만 있는 블록\n\n### 기본은 GET\nhttp://localhost/ping\n";

    #[test]
    fn 요청을_나누고_이름_헤더_본문_지시를_읽는다() {
        let file = parse(FILE);
        assert_eq!(file.variables, [("host".into(), "{{baseUrl}}/app".into())]);
        assert_eq!(file.requests.len(), 3);
        let list = &file.requests[0];
        assert_eq!(
            (list.name.as_deref(), list.method.as_str()),
            (Some("목록"), "GET")
        );
        assert_eq!(list.url, "{{host}}/list.do?page=1&size=10");
        assert_eq!(list.headers, [("Accept".into(), "application/json".into())]);
        assert_eq!((list.line, list.body.as_ref()), (3, None));
        let save = &file.requests[1];
        assert_eq!(
            save.name.as_deref(),
            Some("save"),
            "@name이 ### 이름보다 앞선다"
        );
        assert!(save.insecure && !save.no_redirect);
        assert_eq!(save.url, "{{host}}/save.do");
        assert_eq!(save.body.as_deref(), Some("title=안녕&body=x"));
        assert_eq!(file.requests[2].method, "GET");
        assert_eq!(file.requests[2].url, "http://localhost/ping");
        // 줄 → 요청: 블록 안, 그리고 주석만 있는 블록은 앞의 요청
        assert_eq!(file.request_at(13).map(|r| r.line), Some(10));
        assert_eq!(
            file.request_at(7).map(|r| r.line),
            Some(10),
            "### 줄은 아래 요청"
        );
        assert_eq!(file.request_at(18).map(|r| r.line), Some(10));
        assert_eq!(file.request_at(0), None);
    }

    #[test]
    fn 파일_본문은_두고_저장_줄과_응답_스크립트는_뺀다() {
        let file = parse(
            "POST /up\nContent-Type: multipart/form-data; boundary=B\n\n--B\n\n< ./a.png\n--B--\n\n> {%\n  client.test(\"x\", () => {});\n%}\n>> {{dir}}/out.json\n### 둘\nGET /a\n\n> ./check.js\n>>! b.txt\n",
        );
        let up = &file.requests[0];
        assert_eq!(up.body.as_deref(), Some("--B\n\n< ./a.png\n--B--"));
        assert_eq!(
            up.save,
            Some(Save {
                path: "{{dir}}/out.json".into(),
                overwrite: false
            })
        );
        assert_eq!(
            up.handlers,
            [Handler::Inline {
                line: 8,
                code: "\n  client.test(\"x\", () => {});\n".into()
            }]
        );
        let get = &file.requests[1];
        assert_eq!(get.body, None);
        assert_eq!(get.handlers, [Handler::File("./check.js".into())]);
        assert!(
            get.save
                .as_ref()
                .is_some_and(|s| s.overwrite && s.path == "b.txt")
        );
        assert!(parse("GET /a\n").requests[0].handlers.is_empty());
        let direct = &parse("GET /a\nAccept: */*\n>> a.json\n").requests[0];
        assert_eq!(
            (
                direct.headers.len(),
                direct.body.as_ref(),
                direct.save.is_some()
            ),
            (1, None, true),
            "헤더 바로 뒤의 저장 줄"
        );
    }

    #[test]
    fn 한_줄_스크립트와_요청_전_스크립트() {
        let file = parse(
            "< {%\n  request.variables.set(\"a\", 1);\n%}\n< ./pre.js\nPOST /login\n\nid=a\n> {% client.global.set(\"t\", response.body.t); %}\n> {% client.log(1) %}\n",
        );
        let login = &file.requests[0];
        assert_eq!(
            (login.method.as_str(), login.url.as_str(), login.line),
            ("POST", "/login", 4),
            "요청 전 스크립트는 요청 줄이 아니다"
        );
        assert!(login.pre_script);
        assert_eq!(login.body.as_deref(), Some("id=a"));
        assert_eq!(
            login.handlers,
            [
                Handler::Inline {
                    line: 7,
                    code: " client.global.set(\"t\", response.body.t); ".into()
                },
                Handler::Inline {
                    line: 8,
                    code: " client.log(1) ".into()
                }
            ]
        );
        assert!(!parse("GET /a\n").requests[0].pre_script);
        let open = &parse("GET /a\n\n> {%\nclient.log(1)\n").requests[0];
        assert_eq!(
            open.handlers,
            [Handler::Inline {
                line: 2,
                code: "\nclient.log(1)".into()
            }],
            "닫지 않은 스크립트"
        );
    }

    #[test]
    fn 변수를_채우고_못_채운_이름을_모은다() {
        let lookup = |name: &str| (name == "a").then(|| "1".to_owned());
        assert_eq!(substitute("x{{ a }}y{{a}}", &lookup), Ok("x1y1".into()));
        assert_eq!(
            substitute("{{b}}{{c}}{{b}}", &lookup),
            Err(vec!["b".into(), "c".into()])
        );
        assert_eq!(substitute("열린 {{a", &lookup), Ok("열린 {{a".into()));
    }
}
