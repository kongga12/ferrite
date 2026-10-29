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
//! ```

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
    pub body: Option<String>,
    /// 이 요청만 서버 인증서를 검증하지 않는다(개발 서버의 자체 서명 인증서).
    pub insecure: bool,
    pub no_redirect: bool,
    pub no_cookie_jar: bool,
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
    for i in range {
        let line = lines[i];
        let trimmed = line.trim();
        match state {
            State::Before => {
                if trimmed.is_empty() {
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
            State::Body => body.push(line),
        }
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
