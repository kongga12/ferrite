//! JSP 출력의 XSS 점검. 이스케이프 없이 내보내는 EL(`${…}`)과 표현식(`<%= %>`)을 찾는다.
//! 요청 값(param·header·cookie 등)을 그대로 내보내면 경고, 그 밖의 값은 정보로 알린다.
//!
//! 값의 타입은 모르므로 식의 모양으로 가린다. 비교·산술·`empty`처럼 결과가 참거짓·숫자인 식,
//! 글자 그대로만 내보내는 삼항식, `fn:escapeXml`로 감싼 식은 안전하다고 본다.

use ferrite_core::jsp::{self, Context, Kind, Region};

use crate::mapper::{Finding, Severity};

/// 출력하는 값의 출처. 삼항식은 두 갈래 중 더 위험한 쪽을 따른다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Safe,
    Data,
    Request,
}

/// 요청에서 바로 오는 EL 내장 객체.
const REQUEST_OBJECTS: [&str; 5] = ["param", "paramValues", "header", "headerValues", "cookie"];
/// EL 예약어(이름이 아니다).
const KEYWORDS: [&str; 16] = [
    "true",
    "false",
    "null",
    "empty",
    "not",
    "and",
    "or",
    "eq",
    "ne",
    "lt",
    "gt",
    "le",
    "ge",
    "div",
    "mod",
    "instanceof",
];
/// 결과가 참거짓·숫자이거나 이스케이프된 글자인 JSTL 함수.
const SAFE_FUNCTIONS: [&str; 7] = [
    "escapeXml",
    "length",
    "contains",
    "containsIgnoreCase",
    "startsWith",
    "endsWith",
    "indexOf",
];
/// `<c:forEach varStatus="st">`의 `st.index` 같은 반복 상태 중 숫자·참거짓인 것.
const STATUS_FIELDS: [&str; 7] = ["index", "count", "first", "last", "begin", "end", "step"];
/// 결과가 참거짓·숫자인 연산자(최상위에 있으면 식 전체가 글자를 내보내지 않는다).
const OPERATORS: [&str; 11] = ["==", "!=", "<", ">", "&&", "||", "!", "-", "*", "/", "%"];

pub fn diagnose(text: &str) -> Vec<Finding> {
    let regions = jsp::regions(text);
    let el_ignored = regions
        .iter()
        .any(|r| r.kind == Kind::Directive && ignores_el(&text[r.inner()]));
    // 이 파일의 `<c:forEach varStatus="…">` 이름(같은 태그의 EL마다 나오므로 겹친다).
    let status: Vec<&str> = regions
        .iter()
        .filter_map(|r| match &r.context {
            Context::Attribute { name, tag } if name.eq_ignore_ascii_case("c:forEach") => {
                attribute(&text[tag.clone()], "varStatus")
            }
            _ => None,
        })
        .collect();
    regions
        .iter()
        .filter(|r| r.closed)
        .filter_map(|r| {
            let inner = &text[r.inner()];
            let source = match r.kind {
                // `#{}`는 JSP가 출력하지 않는다(지연 평가식).
                Kind::El if !el_ignored && text[r.range.start..].starts_with('$') => {
                    if !el_outputs(text, r) {
                        return None;
                    }
                    el_source(inner, &status)
                }
                Kind::Expression => java_source(inner),
                _ => return None,
            };
            finding(r, source)
        })
        .collect()
}

/// `<%@ page isELIgnored="true" %>`
fn ignores_el(directive: &str) -> bool {
    let compact: String = directive
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    compact.starts_with("page")
        && (compact.contains("iselignored=\"true\"") || compact.contains("iselignored='true'"))
}

/// 이 EL이 응답에 그대로 쓰이는가. 사용자 정의 태그(`c:if`, `c:forEach` 등)의 속성값은 태그가
/// 받아 쓰므로 출력이 아니다. `<c:out … escapeXml="false">`만 그대로 출력한다.
fn el_outputs(text: &str, region: &Region) -> bool {
    match &region.context {
        Context::Attribute { name, tag } if name.contains(':') => {
            name.eq_ignore_ascii_case("c:out")
                && attribute(&text[tag.clone()], "escapeXml").is_some_and(|v| {
                    let v = v.trim();
                    v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("${false}")
                })
        }
        _ => true,
    }
}

/// 태그 텍스트에서 속성값(따옴표 안). 이름은 대소문자를 가리지 않는다.
pub(crate) fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    attribute_range(tag, name).map(|range| &tag[range])
}

/// 속성값(따옴표 안)의 바이트 범위. 닫는 따옴표가 없으면(입력 중) 태그 끝까지다.
pub(crate) fn attribute_range(tag: &str, name: &str) -> Option<std::ops::Range<usize>> {
    let lower = tag.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(&name) {
        let start = from + at;
        from = start + name.len();
        let before = lower[..start].chars().next_back();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let start = tag.len() - rest.len() + 1;
        let end = tag[start..]
            .find(quote)
            .map_or(tag.len(), |end| start + end);
        return Some(start..end);
    }
    None
}

/// EL 식이 내보내는 값의 출처. `status`는 반복 상태 변수 이름이다.
fn el_source(expr: &str, status: &[&str]) -> Source {
    let code = strip_strings(expr);
    let code = code.trim();
    if code.contains("fn:escapeXml(") {
        return Source::Safe;
    }
    if let Some((question, colon)) = ternary(code) {
        return el_source(&code[question + 1..colon], status)
            .max(el_source(&code[colon + 1..], status));
    }
    if safe_call(code) || has_operator(code) {
        return Source::Safe;
    }
    let code = code.replace("pageContext.request.contextPath", "");
    names(&code)
        .map(|(at, name)| {
            let after = &code[at + name.len()..];
            let field = after.strip_prefix('.').map(|f| {
                f.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or("")
            });
            if REQUEST_OBJECTS.contains(&name)
                || (name == "pageContext" && after.starts_with(".request"))
            {
                Source::Request
            } else if (is_constant(name) && field.is_none() && !after.starts_with('['))
                || (status.contains(&name) && field.is_some_and(|f| STATUS_FIELDS.contains(&f)))
            {
                Source::Safe
            } else {
                Source::Data
            }
        })
        .max()
        .unwrap_or(Source::Safe)
}

/// `<%= … %>` 안 Java 식의 출처. 이스케이프 함수를 거치면 안전하다고 본다.
fn java_source(expr: &str) -> Source {
    let code = expr.trim();
    let lower = code.to_ascii_lowercase();
    if ["escape", "xss", "encode"]
        .iter()
        .any(|w| lower.contains(w))
        || code == "request.getContextPath()"
    {
        return Source::Safe;
    }
    const REQUEST_CALLS: [&str; 7] = [
        "getParameter",
        "getHeader",
        "getQueryString",
        "getRequestURI",
        "getRequestURL",
        "getCookies",
        "getPathInfo",
    ];
    let constant = code
        .rsplit('.')
        .next()
        .is_some_and(|last| is_constant(last.trim()))
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
    if REQUEST_CALLS.iter().any(|call| code.contains(call)) {
        Source::Request
    } else if constant {
        Source::Safe
    } else {
        Source::Data
    }
}

/// 대문자 이름(`CONTEXT_PATH`, `Config.REG_LIST`)은 설정 상수로 본다.
// ponytail: 관례에 기댄 판단이라 사용자 입력을 대문자 이름에 담아 내보내면 놓친다. 오탐을 줄이려는
// 선택이다. 놓치는 사례가 나오면 같은 파일의 `<c:set var>` 값을 따라가 판단한다.
fn is_constant(name: &str) -> bool {
    name.len() > 1
        && name.bytes().any(|b| b.is_ascii_uppercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

fn finding(region: &Region, source: Source) -> Option<Finding> {
    let (severity, code, risk) = match source {
        Source::Safe => return None,
        Source::Request => (
            Severity::Warning,
            "xss-request",
            "요청 값(param·header·cookie 등)이라 반사형 XSS 위험이 큽니다",
        ),
        Source::Data => (
            Severity::Information,
            "xss-output",
            "값에 태그가 들어 있으면 저장형 XSS가 됩니다",
        ),
    };
    let place = match region.context {
        Context::Script => "<script> 안에",
        Context::Attribute { .. } => "태그 속성에",
        Context::Text | Context::Style => "HTML에",
    };
    let fix = match (&region.context, region.kind) {
        (Context::Script, _) => "자바스크립트 문자열이면 JS 이스케이프(예: JSON 직렬화)를 쓰세요",
        (_, Kind::Expression) => "출력 전에 HTML 이스케이프하세요",
        _ => "<c:out value=\"…\"/>이나 ${fn:escapeXml(…)}로 감싸세요",
    };
    Some(Finding {
        range: region.range.clone(),
        severity,
        code,
        message: format!("이스케이프 없이 {place} 출력합니다. {risk}. {fix}."),
    })
}

/// 문자열 리터럴의 내용을 지운다(따옴표는 남긴다). 길이는 그대로다.
fn strip_strings(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let mut quote = None;
    let mut escaped = false;
    for c in expr.chars() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                    out.push(c);
                    continue;
                }
                out.extend(std::iter::repeat_n(' ', c.len_utf8()));
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                }
                out.push(c);
            }
        }
    }
    out
}

/// 괄호 밖 바이트를 (위치, 바이트)로 돌려준다.
fn top_level(code: &str) -> impl Iterator<Item = (usize, u8)> + '_ {
    let mut depth = 0usize;
    code.bytes().enumerate().filter(move |&(_, b)| {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ => return depth == 0,
        }
        false
    })
}

/// 최상위 삼항식의 `?`와 짝인 `:` 위치. 함수 이름의 `fn:`은 건너뛴다.
fn ternary(code: &str) -> Option<(usize, usize)> {
    let question = top_level(code).find(|&(_, b)| b == b'?')?.0;
    let mut nested = 0;
    let bytes = code.as_bytes();
    let colon = top_level(code)
        .skip_while(|&(i, _)| i <= question)
        .find(|&(i, b)| {
            match b {
                b'?' => nested += 1,
                b':' if !is_namespace(bytes, i) => {
                    if nested == 0 {
                        return true;
                    }
                    nested -= 1;
                }
                _ => {}
            }
            false
        })?;
    Some((question, colon.0))
}

/// `fn:length(`의 `:`처럼 함수 접두사를 잇는 콜론인가(`접두사:이름(` 모양).
fn is_namespace(bytes: &[u8], at: usize) -> bool {
    let word = |b: &u8| b.is_ascii_alphanumeric() || *b == b'_';
    if at == 0 || !bytes.get(at - 1).is_some_and(word) {
        return false;
    }
    let rest = &bytes[(at + 1).min(bytes.len())..];
    let len = rest.iter().take_while(|b| word(b)).count();
    len > 0 && rest.get(len) == Some(&b'(')
}

/// 식 전체가 결과가 안전한 JSTL 함수 호출 하나인가(`fn:length(list)`).
fn safe_call(code: &str) -> bool {
    let Some((_, rest)) = code.split_once(':') else {
        return false;
    };
    let Some(open) = rest.find('(') else {
        return false;
    };
    let name = &rest[..open];
    if !SAFE_FUNCTIONS.contains(&name) {
        return false;
    }
    // 첫 여는 괄호의 짝이 식의 끝이어야 호출 하나다.
    let start = code.len() - rest.len() + open;
    let mut depth = 0;
    for (i, b) in code.bytes().enumerate().skip(start) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i == code.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// 최상위에 결과가 참거짓·숫자인 연산자가 있는가. 문자열을 잇는 `+=`는 뺀다.
fn has_operator(code: &str) -> bool {
    let bytes = code.as_bytes();
    let symbol = top_level(code).any(|(i, b)| {
        // 바이트 단위로 본다(한글 이름 중간 위치에서 문자열을 자르지 않게).
        let rest = &bytes[i..];
        OPERATORS.iter().any(|op| rest.starts_with(op.as_bytes()))
            || (b == b'+' && bytes.get(i + 1) != Some(&b'='))
    });
    symbol
        || names_with_keywords(code)
            .any(|(at, name)| KEYWORDS[3..].contains(&name) && depth_at(code, at) == 0)
}

fn depth_at(code: &str, at: usize) -> usize {
    code[..at].bytes().fold(0usize, |d, b| match b {
        b'(' | b'[' | b'{' => d + 1,
        b')' | b']' | b'}' => d.saturating_sub(1),
        _ => d,
    })
}

/// 식이 참조하는 이름(속성 접근 `.name`, 예약어, 함수 이름은 뺀다).
fn names(code: &str) -> impl Iterator<Item = (usize, &str)> {
    names_with_keywords(code).filter(|(_, name)| !KEYWORDS.contains(name))
}

/// 식의 단어 중 앞에 `.`이 없는 것(위치, 단어). 함수 이름(`fn:length`)은 뺀다.
fn names_with_keywords(code: &str) -> impl Iterator<Item = (usize, &str)> {
    let bytes = code.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < bytes.len() {
            let start = i;
            if !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' || bytes[i] == b'$') {
                i += 1;
                continue;
            }
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let before = code[..start].trim_end().bytes().next_back();
            let function = bytes.get(i) == Some(&b':') && is_namespace(bytes, i)
                || before == Some(b':') && is_namespace(bytes, start - 1);
            if before != Some(b'.') && !function {
                return Some((start, &code[start..i]));
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(text: &str) -> Vec<(&str, &'static str)> {
        diagnose(text)
            .into_iter()
            .map(|f| (&text[f.range], f.code))
            .collect()
    }

    #[test]
    fn el_출처를_식의_모양으로_가린다() {
        let cases = [
            ("param.id", Source::Request),
            ("paramValues.ids[0]", Source::Request),
            ("header['User-Agent']", Source::Request),
            ("cookie.JSESSIONID.value", Source::Request),
            ("pageContext.request.queryString", Source::Request),
            ("pageContext.request.contextPath", Source::Safe),
            ("vo.title", Source::Data),
            ("result.param", Source::Data),
            ("fn:escapeXml(param.q)", Source::Safe),
            ("fn:length(list)", Source::Safe),
            ("fn:substring(vo.title, 0, 10)", Source::Data),
            ("status.index + 1", Source::Safe),
            ("empty list", Source::Safe),
            ("vo.useAt eq 'Y'", Source::Safe),
            ("vo.a == '1' ? 'selected' : ''", Source::Safe),
            ("vo.a == '1' ? vo.name : ''", Source::Data),
            ("empty param.q ? '' : param.q", Source::Request),
            ("x ? fn:length(a) : fn:trim(b)", Source::Data),
            ("'고정 글자'", Source::Safe),
            ("vo.first += vo.last", Source::Data),
            ("CONTEXT_PATH", Source::Safe),
            ("CODES.name", Source::Data),
            ("st.index", Source::Safe),
            ("st.current", Source::Data),
            ("status.index", Source::Data),
        ];
        for (expr, want) in cases {
            assert_eq!(el_source(expr, &["st"]), want, "{expr}");
        }
    }

    #[test]
    fn 자리별로_출력인지_가린다() {
        let page = r#"<%@ page contentType="text/html; charset=UTF-8" %>
<%@ taglib prefix="c" uri="http://java.sun.com/jsp/jstl/core" %>
<p>${vo.title}</p>
<input type="text" name="q" value="${param.q}">
<c:if test="${param.q ne ''}"><c:out value="${param.q}"/></c:if>
<c:out value="${vo.content}" escapeXml="false"/>
<a href="${pageContext.request.contextPath}/list.do">목록</a>
<script>var id = '${param.id}'; var n = ${fn:escapeXml(vo.n)};</script>
<span>\${literal} #{deferred}</span>
<%= request.getParameter("q") %> <%= EgovWebUtil.clearXSSMinimum(vo.getTitle()) %> <%= Config.REG_LIST %>
<c:forEach items="${list}" var="row" varStatus="i"><td>${i.count}</td></c:forEach>
<%-- ${param.commented} --%>
"#;
        assert_eq!(
            codes(page),
            vec![
                ("${vo.title}", "xss-output"),
                ("${param.q}", "xss-request"),
                ("${vo.content}", "xss-output"),
                ("${param.id}", "xss-request"),
                ("<%= request.getParameter(\"q\") %>", "xss-request"),
            ]
        );
        let script = diagnose(page)
            .into_iter()
            .find(|f| f.message.contains("<script>"))
            .unwrap();
        assert_eq!(script.severity, Severity::Warning);
        assert!(
            script.message.contains("JS 이스케이프"),
            "{}",
            script.message
        );
    }

    #[test]
    fn el을_끈_페이지는_el을_보지_않는다() {
        let page =
            "<%@ page isELIgnored=\"true\" %>\n<p>${param.q}</p><%= request.getParameter(\"q\") %>";
        assert_eq!(
            codes(page),
            vec![("<%= request.getParameter(\"q\") %>", "xss-request")]
        );
    }

    #[test]
    fn 태그_속성값을_찾는다() {
        let tag = r#"<c:out value="${a}" escapeXML = 'false' />"#;
        assert_eq!(attribute(tag, "escapeXml"), Some("false"));
        assert_eq!(attribute(tag, "value"), Some("${a}"));
        assert_eq!(attribute(r#"<c:out xvalue="1">"#, "value"), None);
    }
}
