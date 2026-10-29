//! 세션 동안 쓰는 쿠키 저장소. egov 로그인(`JSESSIONID`) 뒤 이어 보내는 요청에 쿠키를 붙이는 정도로
//! 단순하다: 도메인·경로·Secure·Max-Age를 따르고, `Expires`는 지난 날짜(삭제)만 알아본다.
// ponytail: Expires 날짜를 풀지 않는다(1970년 같은 지운다는 표시만 본다). 만료 시각이 중요해지면 날짜를 푼다.

use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// 소문자 호스트. `host_only`면 그 호스트에만, 아니면 하위 도메인에도 보낸다.
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    expires: Option<Instant>,
}

#[derive(Clone, Debug, Default)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

/// 요청 주소에서 (scheme, 소문자 호스트, 경로).
fn parts(url: &str) -> Option<(String, String, String)> {
    let uri: ureq::http::Uri = url.parse().ok()?;
    Some((
        uri.scheme_str().unwrap_or("http").to_ascii_lowercase(),
        uri.host()?.to_ascii_lowercase(),
        match uri.path() {
            "" => "/".to_owned(),
            p => p.to_owned(),
        },
    ))
}

impl CookieJar {
    pub fn len(&self) -> usize {
        self.cookies.iter().filter(|c| !c.expired()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
    }

    /// 응답의 `Set-Cookie` 하나를 담는다.
    pub fn store(&mut self, url: &str, header: &str) {
        let Some((_, host, path)) = parts(url) else {
            return;
        };
        let mut attributes = header.split(';');
        let Some((name, value)) = attributes.next().and_then(|nv| nv.split_once('=')) else {
            return;
        };
        let mut cookie = Cookie {
            name: name.trim().to_owned(),
            value: value.trim().trim_matches('"').to_owned(),
            domain: host.clone(),
            host_only: true,
            // 기본 경로: 요청 경로의 마지막 `/`까지.
            path: match path.rfind('/') {
                Some(0) | None => "/".to_owned(),
                Some(i) => path[..i].to_owned(),
            },
            secure: false,
            expires: None,
        };
        let mut delete = false;
        for attribute in attributes {
            let (key, value) = attribute.split_once('=').unwrap_or((attribute, ""));
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "domain" if !value.is_empty() => {
                    let domain = value.trim_start_matches('.').to_ascii_lowercase();
                    // 다른 사이트의 쿠키는 받지 않는다.
                    if host != domain && !host.ends_with(&format!(".{domain}")) {
                        return;
                    }
                    cookie.domain = domain;
                    cookie.host_only = false;
                }
                "path" if value.starts_with('/') => cookie.path = value.to_owned(),
                "secure" => cookie.secure = true,
                "max-age" => match value.parse::<i64>() {
                    Ok(secs) if secs <= 0 => delete = true,
                    Ok(secs) => {
                        cookie.expires = Some(Instant::now() + Duration::from_secs(secs as u64))
                    }
                    Err(_) => {}
                },
                "expires" if value.contains("1970") => delete = true,
                _ => {}
            }
        }
        // 같은 쿠키를 바꿀 때는 자리를 지킨다(같은 길이 경로끼리는 먼저 만든 것이 앞이다).
        let same = self.cookies.iter().position(|c| {
            c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path
        });
        match (same, delete) {
            (Some(i), true) => {
                self.cookies.remove(i);
            }
            (Some(i), false) => self.cookies[i] = cookie,
            (None, true) => {}
            (None, false) => self.cookies.push(cookie),
        }
    }

    /// 이 주소로 보낼 `Cookie` 헤더 값. 긴 경로의 쿠키가 먼저다.
    pub fn header(&self, url: &str) -> Option<String> {
        let (scheme, host, path) = parts(url)?;
        let mut matched: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| !c.expired())
            .filter(|c| !c.secure || scheme == "https")
            .filter(|c| {
                c.domain == host || (!c.host_only && host.ends_with(&format!(".{}", c.domain)))
            })
            .filter(|c| {
                path == c.path
                    || path.starts_with(&c.path)
                        && (c.path.ends_with('/') || path[c.path.len()..].starts_with('/'))
            })
            .collect();
        matched.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        (!matched.is_empty()).then(|| {
            matched
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }
}

impl Cookie {
    fn expired(&self) -> bool {
        self.expires.is_some_and(|at| Instant::now() >= at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 도메인과_경로와_secure를_따르고_지우기를_안다() {
        let mut jar = CookieJar::default();
        jar.store(
            "http://dev.local:8080/app/login.do",
            "JSESSIONID=abc; Path=/app; HttpOnly",
        );
        jar.store("http://dev.local:8080/app/login.do", "lang=ko");
        jar.store("http://dev.local/", "tok=1; Domain=.local; Secure");
        jar.store("http://dev.local/", "evil=1; Domain=other.com");
        assert_eq!(jar.len(), 3);
        assert_eq!(
            jar.header("http://dev.local:8080/app/board/list.do")
                .as_deref(),
            Some("JSESSIONID=abc; lang=ko"),
            "기본 경로는 /app, Secure 쿠키는 http로 보내지 않는다"
        );
        assert_eq!(
            jar.header("http://dev.local/application").as_deref(),
            None,
            "/app는 /application의 경로가 아니다"
        );
        assert_eq!(
            jar.header("https://a.local/").as_deref(),
            Some("tok=1"),
            "하위 도메인"
        );
        // 같은 이름·도메인·경로는 바꾸고, Max-Age=0은 지운다.
        jar.store("http://dev.local/app/x", "JSESSIONID=new; Path=/app");
        assert_eq!(
            jar.header("http://dev.local/app/").as_deref(),
            Some("JSESSIONID=new; lang=ko")
        );
        jar.store(
            "http://dev.local/app/x",
            "JSESSIONID=; Path=/app; Max-Age=0",
        );
        jar.store(
            "http://dev.local/app/x",
            "lang=; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        );
        assert_eq!(jar.header("http://dev.local/app/"), None);
        jar.clear();
        assert!(jar.is_empty());
    }
}
