//! JSP 구간 찾기: 주석 `<%-- --%>`, 지시자 `<%@ %>`, 선언 `<%! %>`, 표현식 `<%= %>`,
//! 스크립틀릿 `<% %>`, EL `${}`·`#{}`. 구문 강조(HTML·Java 층 가리기)와 egov 서버의 XSS 진단이
//! 함께 쓴다.
//!
//! HTML 구조는 EL·표현식이 어디로 출력되는지(본문, 어떤 태그의 속성값, `<script>`·`<style>` 안)를
//! 알 만큼만 따라간다. 완전한 HTML 파서가 아니다.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `<%-- --%>`
    Comment,
    /// `<%@ page … %>`
    Directive,
    /// `<%! … %>`
    Declaration,
    /// `<% … %>`
    Scriptlet,
    /// `<%= … %>`: 값을 그대로 출력한다.
    Expression,
    /// `${…}`·`#{…}`: 본문·HTML 속성값에서는 값을 그대로 출력한다.
    El,
}

/// EL·표현식이 놓인 자리.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Context {
    /// 태그 밖 본문(HTML 주석 안 포함: 주석도 그대로 내려간다).
    Text,
    /// 여는 태그의 속성값. `tag`는 그 태그(`<input …>` 전체)의 바이트 범위다.
    Attribute { name: String, tag: Range<usize> },
    /// `<script>` 안
    Script,
    /// `<style>` 안
    Style,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub range: Range<usize>,
    pub kind: Kind,
    pub context: Context,
    /// 닫는 구분자가 있는가(없으면 문서 끝까지다).
    pub closed: bool,
}

impl Region {
    /// 스크립틀릿 계열(안이 Java 코드)인가.
    pub fn is_java(&self) -> bool {
        matches!(
            self.kind,
            Kind::Declaration | Kind::Scriptlet | Kind::Expression
        )
    }

    /// 구분자를 뺀 안쪽 범위.
    pub fn inner(&self) -> Range<usize> {
        let (open, close) = match self.kind {
            Kind::Comment => (4, 4),
            Kind::Directive | Kind::Declaration | Kind::Expression => (3, 2),
            Kind::Scriptlet => (2, 2),
            Kind::El => (2, 1),
        };
        let start = (self.range.start + open).min(self.range.end);
        let end = if self.closed {
            self.range.end - close
        } else {
            self.range.end
        };
        start..end.max(start)
    }
}

/// 커서 자리에 열려 있는 여는 태그(`<c:out value="…" ` 처럼 아직 `>`가 오지 않은 것).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTag {
    /// `<`의 위치.
    pub start: usize,
    /// 태그 이름 전체(커서 뒤까지 이어진 부분 포함).
    pub name: String,
    /// 커서가 속성값 따옴표 안에 있다.
    pub in_value: bool,
}

/// 문서의 JSP 구간을 앞에서부터 찾는다. 닫히지 않은 구간은 문서 끝까지다.
pub fn regions(text: &str) -> Vec<Region> {
    scan(text, text.len()).regions
}

/// `offset`에 열려 있는 여는 태그. 커서가 JSP 구간(EL·스크립틀릿 등) 안이거나 태그 밖이면 None.
pub fn open_tag_at(text: &str, offset: usize) -> Option<OpenTag> {
    let state = scan(text, offset.min(text.len()));
    if state.inside_region {
        return None;
    }
    state.tag.map(|(start, name)| OpenTag {
        start,
        name,
        in_value: state.quote.is_some(),
    })
}

struct Scan {
    regions: Vec<Region>,
    /// 멈춘 자리에 열려 있는 여는 태그(시작 위치, 이름)와 그 안의 따옴표.
    tag: Option<(usize, String)>,
    quote: Option<u8>,
    /// 멈춘 자리가 JSP 구간 안이다.
    inside_region: bool,
}

/// `until`까지 앞에서부터 훑는다. 구간 찾기와 커서 자리 판단이 같은 규칙을 쓰게 한 곳에 둔다.
fn scan(text: &str, until: usize) -> Scan {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut tag: Option<(usize, String)> = None;
    let mut quote: Option<u8> = None;
    // `<script>`·`<style>` 안이면 그 이름.
    let mut raw: Option<&'static str> = None;
    let mut inside_region = false;
    let mut i = 0;
    while i < until {
        let rest = &text[i..];
        let context = || match (&tag, raw) {
            (Some((start, name)), _) => Context::Attribute {
                name: name.clone(),
                tag: *start..tag_end(text, *start),
            },
            (None, Some("script")) => Context::Script,
            (None, Some(_)) => Context::Style,
            (None, None) => Context::Text,
        };
        // JSP 구간은 태그·속성값·<script> 안 어디서든 먼저 본다.
        if let Some(after) = rest.strip_prefix("<%") {
            let (kind, close) = match after.as_bytes().first() {
                Some(b'-') if after.starts_with("--") => (Kind::Comment, "--%>"),
                Some(b'@') => (Kind::Directive, "%>"),
                Some(b'!') => (Kind::Declaration, "%>"),
                Some(b'=') => (Kind::Expression, "%>"),
                _ => (Kind::Scriptlet, "%>"),
            };
            let close_at = after.find(close);
            let end = close_at.map_or(text.len(), |at| i + 2 + at + close.len());
            found.push(Region {
                range: i..end,
                kind,
                context: context(),
                closed: close_at.is_some(),
            });
            inside_region = end > until || (end == until && close_at.is_none());
            i = end;
            continue;
        }
        if (rest.starts_with("${") || rest.starts_with("#{"))
            // `\${`는 글자 그대로다.
            && !(i > 0 && bytes[i - 1] == b'\\')
        {
            let end = el_end(text, i + 2);
            found.push(Region {
                range: i..end,
                kind: Kind::El,
                context: context(),
                closed: bytes.get(end.wrapping_sub(1)) == Some(&b'}'),
            });
            inside_region =
                end > until || (end == until && bytes.get(end.wrapping_sub(1)) != Some(&b'}'));
            i = end;
            continue;
        }
        let byte = bytes[i];
        if tag.is_some() {
            match (quote, byte) {
                (Some(q), b) if b == q => quote = None,
                (None, b'"' | b'\'') => quote = Some(byte),
                // 따옴표 밖의 `<이름`은 새 태그다(입력 중이라 앞 태그가 아직 닫히지 않은 경우).
                (None, b'<') if tag_name(&rest[1..]).is_some() => {
                    let name = tag_name(&rest[1..]).unwrap_or_default();
                    tag = Some((i, name.to_owned()));
                    i += 1 + name.len();
                    continue;
                }
                (None, b'>') => {
                    let (start, name) = tag.take().unwrap_or_default();
                    let self_closing = i > start && bytes[i - 1] == b'/';
                    raw = match name.to_ascii_lowercase().as_str() {
                        "script" if !self_closing => Some("script"),
                        "style" if !self_closing => Some("style"),
                        _ => raw,
                    };
                }
                _ => {}
            }
        } else if let Some(name) = raw {
            // 바이트로 비교한다(`</` 뒤가 한글이면 글자 중간에서 자르게 된다).
            if rest.starts_with("</")
                && rest.as_bytes()[2..]
                    .get(..name.len())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name.as_bytes()))
            {
                raw = None;
            }
        } else if byte == b'<'
            && let Some(name) = tag_name(&rest[1..])
        {
            tag = Some((i, name.to_owned()));
            quote = None;
            i += 1 + name.len();
            continue;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    Scan {
        regions: found,
        tag,
        quote,
        inside_region,
    }
}

/// 여는 태그의 이름(`<c:out` → `c:out`). 닫는 태그(`</…`)·선언(`<!…`)은 None.
fn tag_name(after_lt: &str) -> Option<&str> {
    let first = after_lt.chars().next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let end = after_lt
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.')))
        .unwrap_or(after_lt.len());
    Some(&after_lt[..end])
}

/// 태그의 끝(`>` 다음). 따옴표 안의 `>`와 JSP 구간은 건너뛴다. 없으면 문서 끝.
fn tag_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut quote = None;
    let mut i = start + 1;
    while i < bytes.len() {
        // 바이트로 본다: i는 한글 글자 중간일 수 있다(`<%`를 본 자리는 글자 경계다).
        if bytes[i..].starts_with(b"<%") {
            i = text[i..].find("%>").map_or(text.len(), |at| i + at + 2);
            continue;
        }
        match (quote, bytes[i]) {
            (Some(q), b) if b == q => quote = None,
            (None, b'"' | b'\'') => quote = Some(bytes[i]),
            (None, b'>') => return i + 1,
            _ => {}
        }
        i += 1;
    }
    text.len()
}

/// EL의 끝(닫는 `}` 다음). 안의 문자열과 중괄호(맵 리터럴 등)를 센다. 없으면 문서 끝.
fn el_end(text: &str, from: usize) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut i = from;
    while i < bytes.len() {
        let byte = bytes[i];
        match (quote, byte) {
            (Some(_), b'\\') => i += 1,
            (Some(q), b) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'{') => depth += 1,
            (None, b'}') if depth == 0 => return i + 1,
            (None, b'}') => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    text.len()
}

/// HTML 층: JSP 구간을 모두 공백으로 가린 사본(줄바꿈은 그대로, 길이도 같다). 위치가 원문과
/// 같아 가린 텍스트로 만든 트리의 위치를 그대로 쓸 수 있다.
pub fn html_text(text: &str, regions: &[Region]) -> String {
    blank(text, regions.iter().map(|r| r.range.clone()))
}

/// Java 층: 스크립틀릿·선언·표현식의 안쪽만 남기고 나머지를 가린 사본.
pub fn java_text(text: &str, regions: &[Region]) -> String {
    let mut hidden = Vec::new();
    let mut at = 0;
    for inner in regions.iter().filter(|r| r.is_java()).map(Region::inner) {
        hidden.push(at..inner.start);
        at = inner.end;
    }
    hidden.push(at..text.len());
    blank(text, hidden.into_iter())
}

/// 범위들을 공백으로 바꾼다(줄바꿈은 둔다). 범위 경계는 ASCII 구분자 자리라 글자 경계이고, 안쪽
/// 바이트는 모두 공백이 되므로 UTF-8이 깨지지 않는다.
fn blank(text: &str, ranges: impl Iterator<Item = Range<usize>>) -> String {
    let mut bytes = text.as_bytes().to_vec();
    for range in ranges {
        for b in &mut bytes[range] {
            if !matches!(*b, b'\n' | b'\r') {
                *b = b' ';
            }
        }
    }
    String::from_utf8(bytes).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<%@ page contentType="text/html; charset=UTF-8" %>
<%-- 목록 ${notOutput} --%>
<html><body>
<p>${param.keyword}</p>
<input type="text" name="q" value="${searchVO.searchKeyword}">
<c:out value="${result.title}"/>
<% String id = request.getParameter("id"); %>
<%= id %>
<script>var name = '${user.name}';</script>
\${literal} ${map['a}b']}
</body></html>
"#;

    fn found(text: &str) -> Vec<(Kind, &str)> {
        regions(text)
            .into_iter()
            .map(|r| (r.kind, &text[r.range]))
            .collect()
    }

    #[test]
    fn jsp_구간을_종류별로_찾는다() {
        let kinds = found(PAGE);
        assert_eq!(
            kinds,
            [
                (
                    Kind::Directive,
                    r#"<%@ page contentType="text/html; charset=UTF-8" %>"#
                ),
                (Kind::Comment, "<%-- 목록 ${notOutput} --%>"),
                (Kind::El, "${param.keyword}"),
                (Kind::El, "${searchVO.searchKeyword}"),
                (Kind::El, "${result.title}"),
                (
                    Kind::Scriptlet,
                    r#"<% String id = request.getParameter("id"); %>"#
                ),
                (Kind::Expression, "<%= id %>"),
                (Kind::El, "${user.name}"),
                (Kind::El, "${map['a}b']}"),
            ],
            "주석 안 EL과 \\${{는 빼고, 문자열 안 중괄호는 센다"
        );
    }

    #[test]
    fn el이_출력되는_자리를_안다() {
        let contexts: Vec<Context> = regions(PAGE)
            .into_iter()
            .filter(|r| r.kind == Kind::El)
            .map(|r| r.context)
            .collect();
        let tag_of = |c: &Context| match c {
            Context::Attribute { name, tag } => Some((name.clone(), PAGE[tag.clone()].to_owned())),
            _ => None,
        };
        assert_eq!(contexts[0], Context::Text);
        assert_eq!(
            tag_of(&contexts[1]),
            Some((
                "input".into(),
                r#"<input type="text" name="q" value="${searchVO.searchKeyword}">"#.into()
            ))
        );
        assert_eq!(tag_of(&contexts[2]).unwrap().0, "c:out");
        assert_eq!(contexts[3], Context::Script);
        assert_eq!(contexts[4], Context::Text, "</script> 다음은 다시 본문");

        let korean =
            r#"<input title="제목 입력" value="${vo.제목}"><script></스크립트></script>${a}"#;
        let found = regions(korean);
        assert_eq!(
            tag_of(&found[0].context).map(|(name, _)| name),
            Some("input".into()),
            "태그 안 한글에서 글자 중간을 자르지 않는다"
        );
        assert_eq!(found[1].context, Context::Text);
    }

    #[test]
    fn 커서_자리의_열린_태그를_안다() {
        let text = r#"<c:out value="${a}" esc <p>x</p> <form:input path="t" ${b}"#;
        let at = |needle: &str| text.find(needle).unwrap() + needle.len();
        let tag = open_tag_at(text, at("esc")).unwrap();
        assert_eq!(
            (tag.start, tag.name.as_str(), tag.in_value),
            (0, "c:out", false)
        );
        assert!(
            open_tag_at(text, at("value=\"$")).is_none(),
            "EL 안은 태그 자리가 아니다"
        );
        assert!(open_tag_at(text, at("value=\"")).unwrap().in_value);
        assert_eq!(
            open_tag_at(text, at("<form:in")).unwrap().name,
            "form:input"
        );
        assert!(open_tag_at(text, at("<p>")).is_none(), "닫힌 태그 뒤 본문");
        assert!(
            open_tag_at(text, text.len() - 1).is_none(),
            "닫히지 않은 EL 안"
        );
    }

    #[test]
    fn 가린_사본은_길이와_줄이_같다() {
        let regions = regions(PAGE);
        let html = html_text(PAGE, &regions);
        assert_eq!(html.len(), PAGE.len());
        assert_eq!(html.lines().count(), PAGE.lines().count());
        assert!(!html.contains("${param") && !html.contains("<%"));
        assert!(
            html.contains(r"\${literal}"),
            "역슬래시를 붙인 EL은 글자 그대로라 남는다"
        );
        assert!(html.contains("<input type=\"text\""), "HTML은 그대로");
        let java = java_text(PAGE, &regions);
        assert!(java.contains("request.getParameter"));
        assert!(!java.contains("<html>"));
        assert_eq!(
            &PAGE[regions[5].inner()],
            r#" String id = request.getParameter("id"); "#
        );
    }
}
