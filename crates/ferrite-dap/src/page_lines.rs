//! 화면 스크립트 ↔ JSP 줄 맞추기. js-debug는 JSP가 만든 화면(HTML)의 인라인 스크립트를 화면의 줄로 준다.
//! 스크립트 내용으로 프로젝트의 JSP 파일과 그 줄을 찾는다. JSP의 EL(`${…}`)·스크립틀릿(`<% %>`)·
//! 태그(`<c:url …/>`)는 화면에서 다른 글로 바뀌므로 그 앞뒤 글자로 맞추고, 화면에 없는 줄(태그만 있는
//! 줄, 지시자)과 화면에만 있는 줄(반복으로 늘어난 줄)은 줄 맞춤(LCS)으로 건너뛴다.

use std::path::{Path, PathBuf};

/// 맞춰 볼 파일의 확장자(JSP와 그 조각).
pub const EXTENSIONS: [&str; 4] = ["jsp", "jspf", "jspx", "tag"];

/// 건너뛸 폴더(빌드 결과·의존성).
const SKIP: [&str; 5] = ["target", "build", "node_modules", ".git", "out"];

/// 화면 스크립트 하나와 JSP 파일의 줄 대응.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageLines {
    pub file: PathBuf,
    /// (화면 줄, 파일 줄), 0부터. 둘 다 커지는 순서다.
    pairs: Vec<(usize, usize)>,
}

impl PageLines {
    /// 화면 줄(0부터)의 파일 줄. 맞춘 줄이 아니면 바로 앞에서 맞춘 줄부터 센다(다음 맞춘 줄은 넘지 않는다).
    pub fn to_file(&self, page: usize) -> usize {
        let after = self.pairs.partition_point(|&(p, _)| p <= page);
        match (
            after.checked_sub(1).map(|i| self.pairs[i]),
            self.pairs.get(after),
        ) {
            (Some((p, f)), next) => {
                let guess = f + (page - p);
                next.map_or(guess, |&(_, next_file)| {
                    guess.min(next_file.saturating_sub(1).max(f))
                })
            }
            (None, Some(&(p, f))) => f.saturating_sub(p - page),
            (None, None) => page,
        }
    }

    /// 파일 줄(0부터)의 화면 줄. 화면에 그대로 나온 줄만.
    pub fn to_page(&self, file: usize) -> Option<usize> {
        self.pairs
            .iter()
            .find(|&&(_, f)| f == file)
            .map(|&(p, _)| p)
    }
}

/// 화면 스크립트 블록 여럿(레이아웃·`<jsp:include>`·Tiles로 합친 화면, JSP 하나의 `<script>` 여럿)의
/// JSP와 줄. 블록마다 그것을 만든 JSP를 따로 찾는다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageMap {
    /// (화면 시작 줄, 끝 줄(그 줄까지), 줄 대응), 0부터.
    blocks: Vec<(usize, usize, PageLines)>,
}

impl PageMap {
    /// 화면 전체를 한 JSP에 맞춘 것.
    pub fn single(lines: PageLines) -> Self {
        Self {
            blocks: vec![(0, usize::MAX, lines)],
        }
    }

    /// 스크립트 블록들((시작 줄, 칸)은 1부터, 글)마다 JSP를 찾는다. 찾지 못한 블록은 뺀다. 하나도 못
    /// 찾으면 None.
    pub fn find(
        blocks: &[(usize, usize, &str)],
        files: &[(PathBuf, String)],
        hint: &str,
    ) -> Option<Self> {
        let blocks: Vec<(usize, usize, PageLines)> = blocks
            .iter()
            .filter_map(|&(line, _, text)| {
                let start = line.saturating_sub(1);
                let script = format!("{}{text}", "\n".repeat(start));
                let found = find(&script, files, hint)?;
                Some((start, start + text.lines().count().max(1) - 1, found))
            })
            .collect();
        (!blocks.is_empty()).then_some(Self { blocks })
    }

    /// 화면 줄(0부터)이 든 블록의 JSP와 그 줄. 어느 블록에도 들지 않으면 None.
    pub fn to_file(&self, page: usize) -> Option<(&Path, usize)> {
        self.blocks
            .iter()
            .find(|(start, end, _)| (*start..=*end).contains(&page))
            .map(|(_, _, lines)| (lines.file.as_path(), lines.to_file(page)))
    }

    /// 이 JSP 줄(0부터)의 화면 줄들. 그 줄을 화면에 그대로 낸 블록마다 하나(`<c:forEach>` 안의
    /// 스크립트처럼 한 JSP 블록이 여러 번 나오면 여럿).
    pub fn to_pages(&self, file: &Path, line: usize) -> Vec<usize> {
        self.blocks
            .iter()
            .filter(|(_, _, lines)| lines.file == file)
            .filter_map(|(_, _, lines)| lines.to_page(line))
            .collect()
    }

    /// 맞춘 JSP들(화면에 나온 차례, 겹치지 않게).
    pub fn files(&self) -> Vec<&Path> {
        let mut files: Vec<&Path> = Vec::new();
        for (_, _, lines) in &self.blocks {
            if !files.contains(&lines.file.as_path()) {
                files.push(&lines.file);
            }
        }
        files
    }
}

/// 화면의 인라인 스크립트 글들(문서 차례)을 JSON 배열로 내는 식(원래 HTML을 받지 못할 때 쓴다). 브라우저가
/// 돌리는 스크립트만 센다(`<script type="text/template">` 같은 템플릿은 js-debug가 알리지 않는다). 화면에는
/// 스크립트가 끼워 넣은 것(jQuery `.load()`·`.html()`의 조각 등)도 있어, 알려 온 블록과 수가 같을 때만
/// 차례로 짝짓는다. js-debug의 `evaluate`에 `clipboard` 문맥으로 물으면 따옴표를 붙인 JSON 문자열로 답한다
/// (`page_scripts`로 푼다).
pub const PAGE_SCRIPTS: &str = "JSON.stringify(Array.from(document.scripts).filter(s => !s.src && (!s.type || /(java|ecma)script|module/i.test(s.type))).map(s => s.text))";

/// 화면의 원래 HTML에서 블록 자리((줄, 칸)은 1부터, 칸은 글자 수)마다 인라인 스크립트의 글을 잘라 낸다:
/// 그 자리부터 `</script` 앞까지(HTML 파서가 스크립트를 끝내는 곳). 자리가 HTML 밖이면 None(화면을 다시
/// 불러 HTML이 바뀌었다).
pub fn scripts_at<'a>(
    html: &'a str,
    starts: &[(usize, usize)],
) -> Option<Vec<(usize, usize, &'a str)>> {
    let mut lines = vec![0];
    lines.extend(html.match_indices('\n').map(|(i, _)| i + 1));
    starts
        .iter()
        .map(|&(line, column)| {
            let from = *lines.get(line.checked_sub(1)?)?;
            // 칸은 그 줄(줄 끝 글자까지) 안에서 센다.
            let to = lines.get(line).copied().unwrap_or(html.len());
            let (at, _) = html[from..to].char_indices().nth(column.checked_sub(1)?)?;
            let at = from + at;
            let text = &html[at..];
            let end = text
                .as_bytes()
                .windows(8)
                .position(|w| w.eq_ignore_ascii_case(b"</script"))
                .unwrap_or(text.len());
            Some((line, column, &text[..end]))
        })
        .collect()
}

/// `PAGE_SCRIPTS`의 답(`evaluate`의 `result`)을 글 목록으로 푼다.
pub fn page_scripts(result: &str) -> Option<Vec<String>> {
    let json: String = serde_json::from_str(result).ok()?;
    serde_json::from_str(&json).ok()
}

/// js-debug가 인라인 스크립트의 이름 끝에 붙이는 `꞉줄:칸`(1부터, `꞉`는 U+A789)을 뗀다: (화면, 줄, 칸).
/// js-debug는 같은 화면 주소의 스크립트를 하나만 남기므로(새것이 오면 앞의 것을 지운다) 화면으로 묶는다.
pub fn inline_script(name: &str) -> Option<(&str, usize, usize)> {
    let (page, at) = name.rsplit_once('\u{a789}')?;
    let (line, column) = at.split_once(':')?;
    let line = line.parse().ok().filter(|&l| l > 0)?;
    Some((page, line, column.parse().ok()?))
}

/// 인라인 스크립트들((시작 줄, 칸)은 1부터, 글)을 화면 줄에 놓은 글. 스크립트 밖은 빈 줄이다(js-debug가
/// 스크립트 하나를 주는 꼴과 같다).
pub fn assemble(blocks: &[(usize, usize, &str)]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for &(line, column, text) in blocks {
        for (k, part) in text.split('\n').enumerate() {
            let at = line.saturating_sub(1) + k;
            if lines.len() <= at {
                lines.resize(at + 1, String::new());
            }
            let part = part.strip_suffix('\r').unwrap_or(part);
            lines[at] = if k == 0 {
                format!("{}{part}", " ".repeat(column.saturating_sub(1)))
            } else {
                part.to_owned()
            };
        }
    }
    lines.join("\n")
}

/// 프로젝트의 JSP 파일들.
pub fn candidates(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, 12, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if depth > 0 && !SKIP.contains(&name.as_ref()) {
                walk(&path, depth - 1, found);
            }
        } else if is_page_file(&path) {
            found.push(path);
        }
    }
}

/// JSP와 그 조각인지(확장자로).
pub fn is_page_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// 화면 스크립트(`script`, 화면 줄에 맞춰 앞이 빈 줄)를 만든 JSP와 그 줄을 찾는다. `files`는 (경로, 내용),
/// `hint`는 화면 주소(같은 스크립트가 여러 JSP에 있을 때 경로가 닮은 것을 고른다). 스크립트의 의미 있는
/// 줄을 절반 넘게 맞춘 파일이 없으면 None.
pub fn find(script: &str, files: &[(PathBuf, String)], hint: &str) -> Option<PageLines> {
    let page: Vec<(usize, &str)> = script
        .lines()
        .enumerate()
        .map(|(i, l)| (i, l.trim()))
        .filter(|(_, l)| !l.is_empty())
        .collect();
    let wanted = page.iter().filter(|(_, l)| meaningful(l)).count();
    if wanted == 0 {
        return None;
    }
    // 내용에 그대로 든 긴 줄이 하나도 없으면 줄 맞춤을 하지 않는다(파일이 많아도 빠르게).
    let mut anchors: Vec<&str> = page
        .iter()
        .map(|(_, l)| *l)
        .filter(|l| meaningful(l))
        .collect();
    anchors.sort_by_key(|l| std::cmp::Reverse(l.len()));
    anchors.truncate(5);
    let hint = words(hint);
    let mut best: Option<((usize, usize, usize), PageLines)> = None;
    for (path, text) in files {
        if !anchors.iter().any(|a| text.contains(a)) {
            continue;
        }
        let file: Vec<&str> = text.lines().map(str::trim).collect();
        let pairs = align(&page, &file);
        let matched = pairs
            .iter()
            .filter(|&&(i, _)| meaningful(page[i].1))
            .count();
        if matched * 2 <= wanted {
            continue;
        }
        let path_words = words(&path.to_string_lossy());
        let near = hint.iter().filter(|w| path_words.contains(w)).count();
        let score = (matched, pairs.len(), near);
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            let pairs = pairs.into_iter().map(|(i, f)| (page[i].0, f)).collect();
            best = Some((
                score,
                PageLines {
                    file: path.clone(),
                    pairs,
                },
            ));
        }
    }
    best.map(|(_, lines)| lines)
}

/// 맞출 만한 줄: 글자·숫자가 셋 이상(`});` 같은 줄은 어디에나 있다).
fn meaningful(line: &str) -> bool {
    line.chars().filter(|c| c.is_alphanumeric()).count() >= 3
}

/// 경로·주소의 낱말(소문자).
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(str::to_lowercase)
        .collect()
}

/// 화면 줄들과 파일 줄들의 가장 긴 공통 줄 맞춤(LCS). (화면 목록 번호, 파일 줄) 쌍을 돌려준다.
fn align(page: &[(usize, &str)], file: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (page.len(), file.len());
    let patterns: Vec<Option<Vec<&str>>> = file.iter().map(|l| pattern(l)).collect();
    // ponytail: n×m 표. 화면 스크립트·JSP는 수백~수천 줄이라 충분하다. 아주 크면 줄 해시로 줄인다.
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if same(page[i].1, &patterns[j]) {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j, mut pairs) = (0, 0, Vec::new());
    while i < n && j < m {
        if same(page[i].1, &patterns[j]) {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// JSP 줄의 글자 조각. 바뀌는 부분(EL·스크립틀릿·태그)의 자리에서 나눈다. 바뀌는 부분이 없으면 조각
/// 하나, 바뀌는 부분만 있으면(태그만 있는 줄) None(화면 줄과 맞추지 않는다).
fn pattern(line: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut rest = line;
    let mut dynamic = false;
    while let Some((start, end)) = next_dynamic(rest) {
        parts.push(&rest[..start]);
        rest = &rest[end..];
        dynamic = true;
    }
    parts.push(rest);
    if dynamic && parts.iter().all(|p| p.trim().len() < 2) {
        return None;
    }
    Some(parts)
}

/// 바뀌는 부분의 (시작, 끝) 바이트: `${…}`·`#{…}`, `<%…%>`, `<접두사:태그 …>`·`</접두사:태그>`,
/// Thymeleaf 인라인 `[[…]]`·`[(…)]`.
fn next_dynamic(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        let close = if rest.starts_with("${") || rest.starts_with("#{") {
            Some("}")
        } else if rest.starts_with("<%") {
            Some("%>")
        } else if rest.starts_with("[[") {
            Some("]]")
        } else if rest.starts_with("[(") {
            Some(")]")
        } else if bytes[i] == b'<' && is_tag(&rest[1..]) {
            Some(">")
        } else {
            None
        };
        if let Some(close) = close {
            // EL 안의 `}`·태그 속성의 `${…}`를 넘으려고 여는 것 뒤에서 찾는다. 닫히지 않으면 줄 끝까지.
            let from = i + 2;
            let end = text[from.min(text.len())..]
                .find(close)
                .map_or(text.len(), |k| from + k + close.len());
            return Some((i, end));
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// `<` 뒤가 `접두사:이름`(또는 `/접두사:이름`) 꼴의 태그인지.
fn is_tag(after: &str) -> bool {
    let after = after.strip_prefix('/').unwrap_or(after);
    let Some((prefix, name)) = after.split_once(':') else {
        return false;
    };
    !prefix.is_empty()
        && prefix
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
}

/// 화면 줄이 JSP 줄의 조각들로 맞는지: 첫 조각으로 시작하고 끝 조각으로 끝나며 가운데 조각이 차례로 든다.
fn same(page: &str, pattern: &Option<Vec<&str>>) -> bool {
    let Some(parts) = pattern else {
        return false;
    };
    if let [only] = parts.as_slice() {
        return page == *only;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !page.starts_with(first) || page.len() < first.len() + last.len() || !page.ends_with(last) {
        return false;
    }
    let mut middle = &page[first.len()..page.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match middle.find(part) {
            Some(k) => middle = &middle[k + part.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// egov 목록 화면 JSP(지시자·taglib·`<c:url>`·EL·`<c:if>`가 섞인 스크립트).
    const JSP: &str = r#"<%@ page contentType="text/html; charset=utf-8" %>
<%@ taglib prefix="c" uri="http://java.sun.com/jsp/jstl/core" %>
<%@ taglib prefix="ui" uri="http://egovframework.gov/ctl/ui" %>
<!DOCTYPE html>
<html>
<head>
<script src="<c:url value='/js/jquery.min.js'/>"></script>
<script type="text/javascript">
var ctx = "${pageContext.request.contextPath}";
<c:if test="${not empty message}">
alert("${message}");
</c:if>
function fn_egov_link_page(pageNo) {
    document.listForm.pageIndex.value = pageNo;
    document.listForm.action = "<c:url value='/board/selectBoardList.do'/>";
    document.listForm.submit();
}
$(function() {
    fn_init();
});
</script>
</head>
"#;

    /// 그 JSP가 만든 화면의 인라인 스크립트(js-debug가 주는 꼴: 화면 줄에 맞춰 앞이 빈 줄). 지시자 줄은
    /// 빈 줄로 남고, `<c:if>`는 거짓이라 그 안의 줄이 빠지고 태그 줄은 빈 줄로 남는다. 앞에 다른 조각이
    /// 들어와 화면에서는 세 줄 아래에 있다.
    fn page() -> String {
        let mut lines = vec![""; 11];
        lines.extend([
            "var ctx = \"/app\";",
            "",
            "",
            "function fn_egov_link_page(pageNo) {",
            "    document.listForm.pageIndex.value = pageNo;",
            "    document.listForm.action = \"/app/board/selectBoardList.do\";",
            "    document.listForm.submit();",
            "}",
            "$(function() {",
            "    fn_init();",
            "});",
            "",
        ]);
        lines.join("\n")
    }

    fn files() -> Vec<(PathBuf, String)> {
        vec![
            (
                PathBuf::from("webapp/WEB-INF/jsp/main.jsp"),
                "<script>\nfunction other() {\n  return 1;\n}\n</script>\n".to_owned(),
            ),
            (
                PathBuf::from("webapp/WEB-INF/jsp/board/BoardList.jsp"),
                JSP.to_owned(),
            ),
        ]
    }

    #[test]
    fn 화면_스크립트를_만든_jsp와_줄을_찾는다() {
        let lines = find(
            &page(),
            &files(),
            "localhost:8080/app/board/selectBoardList.do",
        )
        .unwrap();
        assert_eq!(
            lines.file,
            PathBuf::from("webapp/WEB-INF/jsp/board/BoardList.jsp")
        );
        // EL로 바뀐 줄(화면 11 → JSP 8), 그대로 나온 줄, `<c:url>`로 바뀐 줄을 맞춘다.
        assert_eq!(lines.to_file(11), 8);
        assert_eq!(lines.to_file(14), 12);
        assert_eq!(lines.to_file(17), 15);
        assert_eq!(lines.to_file(20), 18);
        // 중단점: JSP 줄 → 화면 줄. 화면에 나오지 않은 줄(`<c:if>` 안)은 걸 곳이 없다.
        assert_eq!(lines.to_page(13), Some(15));
        assert_eq!(lines.to_page(18), Some(20));
        assert_eq!(lines.to_page(10), None);
        assert_eq!(lines.to_page(9), None);
        // 맞추지 못한 화면 줄(빈 줄)은 앞의 맞춘 줄부터 세되 다음 맞춘 줄을 넘지 않는다.
        assert_eq!(lines.to_file(12), 9);
        assert_eq!(lines.to_file(13), 10);
    }

    #[test]
    fn 맞는_파일이_없거나_조금만_맞으면_찾지_않는다() {
        assert_eq!(find("\n\nvar unrelated = compute();\n", &files(), ""), None);
        assert_eq!(
            find("\n});\n}\n", &files(), ""),
            None,
            "의미 있는 줄이 없다"
        );
        // 반도 맞지 않으면 다른 화면이다.
        let mixed = "fn_init();\nvar a1 = 1;\nvar a2 = 2;\nvar a3 = 3;\n";
        assert_eq!(find(mixed, &files(), ""), None);
    }

    #[test]
    fn 같은_스크립트가_여러_jsp에_있으면_주소와_닮은_파일을_고른다() {
        let script = "\nfunction fn_init() {\n    load();\n}\n";
        let jsp = "<script>\nfunction fn_init() {\n    load();\n}\n</script>\n";
        let files = vec![
            (PathBuf::from("jsp/notice/NoticeList.jsp"), jsp.to_owned()),
            (PathBuf::from("jsp/board/BoardList.jsp"), jsp.to_owned()),
        ];
        let found = find(script, &files, "localhost/app/board/list.do").unwrap();
        assert_eq!(found.file, PathBuf::from("jsp/board/BoardList.jsp"));
        let found = find(script, &files, "localhost/app/notice/list.do").unwrap();
        assert_eq!(found.file, PathBuf::from("jsp/notice/NoticeList.jsp"));
    }

    #[test]
    fn 바뀌는_부분을_빼고_앞뒤_글자로_맞춘다() {
        let check = |page: &str, jsp: &str| same(page, &pattern(jsp));
        assert!(check(
            "fn_egov_link_page(3);",
            "fn_egov_link_page(${paginationInfo.currentPageNo});"
        ));
        assert!(check(
            "var u = \"/a/b.do?id=\" + id;",
            "var u = \"<c:url value='/b.do'/>?id=\" + id;"
        ));
        assert!(check("var n = 10;", "var n = <%= list.size() %>;"));
        // Thymeleaf 자바스크립트 인라인은 따옴표까지 넣는다.
        assert!(check("var t = \"홍길동\";", "var t = [[${user.name}]];"));
        assert!(!check("var n = 10;", "var m = ${n};"));
        // 태그·EL만 있는 줄은 화면 줄과 맞추지 않는다.
        assert_eq!(pattern("<c:if test=\"${a}\">"), None);
        assert_eq!(pattern("</c:forEach>"), None);
        assert_eq!(pattern("${x}"), None);
        // `a < b:c` 같은 비교식은 태그가 아니다.
        assert!(check("if (a <b:c) {", "if (a <b:c) {"));
        assert!(!is_tag("1:2"));
    }

    #[test]
    fn 여러_jsp로_합친_화면은_스크립트_블록마다_jsp를_찾는다() {
        // Tiles 꼴: 레이아웃의 머리 스크립트, 머리글 조각과 본문의 스크립트가 한 화면에 든다.
        let layout = "<%@ page contentType=\"text/html; charset=UTF-8\" %>\n<!doctype html>\n<html><head>\n<script>\nvar layoutReady = false;\nfunction markLayout() {\n  layoutReady = true;\n}\n</script>\n</head><body>\n<tiles:insertAttribute name=\"header\"/>\n<tiles:insertAttribute name=\"body\"/>\n</body></html>\n";
        let header = "<%@ page contentType=\"text/html; charset=UTF-8\" %>\n<div id=\"header\">머리글</div>\n<script>\nfunction headerClock() {\n  var now = new Date();\n  return now.getTime();\n}\n</script>\n";
        let body = "<%@ page contentType=\"text/html; charset=UTF-8\" %>\n<h1>${title}</h1>\n<script>\nvar ticks = 0;\nfunction tick() {\n  ticks = ticks + 1;\n  headerClock();\n}\nsetInterval(tick, <c:out value=\"${interval}\"/>);\n</script>\n";
        let files = vec![
            (PathBuf::from("jsp/layout/default.jsp"), layout.to_owned()),
            (PathBuf::from("jsp/layout/header.jsp"), header.to_owned()),
            (PathBuf::from("jsp/board/BoardList.jsp"), body.to_owned()),
        ];
        // js-debug가 이름으로 주는 시작 자리와 화면에서 읽은 글(실제 js-debug·Chrome으로 본 것).
        let blocks = [
            (
                3,
                9,
                "\nvar layoutReady = false;\nfunction markLayout() {\n  layoutReady = true;\n}\n",
            ),
            (
                11,
                9,
                "\nfunction headerClock() {\n  var now = new Date();\n  return now.getTime();\n}\n",
            ),
            (
                18,
                9,
                "\nvar ticks = 0;\nfunction tick() {\n  ticks = ticks + 1;\n  headerClock();\n}\nsetInterval(tick, 200);\n",
            ),
        ];
        let map = PageMap::find(&blocks, &files, "127.0.0.1:8080/app/board/list.do").unwrap();
        let (layout, header, body) = (
            Path::new("jsp/layout/default.jsp"),
            Path::new("jsp/layout/header.jsp"),
            Path::new("jsp/board/BoardList.jsp"),
        );
        assert_eq!(map.files(), [layout, header, body]);
        // 화면 13줄(0부터 12, `var now`)은 머리글 5줄, 화면 21줄은 본문 6줄, 화면 6줄은 레이아웃 7줄.
        assert_eq!(map.to_file(12), Some((header, 4)));
        assert_eq!(map.to_file(20), Some((body, 5)));
        assert_eq!(map.to_file(5), Some((layout, 6)));
        assert_eq!(map.to_file(9), None, "스크립트 밖");
        // 중단점: JSP 줄 → 화면 줄. 바뀌는 부분(`<c:out>`)이 든 줄도 앞뒤 글자로 맞춘다.
        assert_eq!(map.to_pages(header, 4), [12]);
        assert_eq!(map.to_pages(body, 8), [23]);
        assert!(map.to_pages(body, 0).is_empty(), "지시자");
        assert!(map.to_pages(Path::new("jsp/other.jsp"), 3).is_empty());

        // 이름에서 시작 자리를 떼고, 블록을 화면 줄에 놓는다.
        assert_eq!(
            inline_script("127.0.0.1\u{a789}8080/app/tiles.do\u{a789}11:9"),
            Some(("127.0.0.1\u{a789}8080/app/tiles.do", 11, 9))
        );
        assert_eq!(inline_script("127.0.0.1\u{a789}8080/app/js/app.js"), None);
        assert_eq!(inline_script("a\u{a789}0:1"), None);
        let page = assemble(&blocks);
        let lines: Vec<&str> = page.lines().collect();
        assert_eq!(lines[3], "var layoutReady = false;");
        assert_eq!(lines[12], "  var now = new Date();");
        assert_eq!(lines[23], "setInterval(tick, 200);");
        assert_eq!(lines[8], "", "스크립트 밖은 빈 줄");
        assert_eq!(assemble(&[(2, 3, "x;\r\ny;")]), "\n  x;\ny;");
    }

    #[test]
    fn 원래_html에서_블록_자리의_스크립트를_잘라_낸다() {
        // 조각을 끼우는 화면: 스크립트 안의 `<script>` 글자, 대문자 닫는 태그, 한글 뒤의 자리, CRLF.
        let html = "<!doctype html>\r\n<html><head>\r\n<script>\r\nfunction load() {\r\n  $(\"#d\").html(\"<script>x()<\\/script>\");\r\n}\r\n</SCRIPT>\r\n</head><body><p>목록</p><script>var n = 1;</script>\r\n</body></html>\r\n";
        let scripts = scripts_at(html, &[(3, 9), (8, 31)]).unwrap();
        assert_eq!(
            scripts,
            [
                (
                    3,
                    9,
                    "\r\nfunction load() {\r\n  $(\"#d\").html(\"<script>x()<\\/script>\");\r\n}\r\n"
                ),
                (8, 31, "var n = 1;"),
            ]
        );
        assert_eq!(scripts_at(html, &[(99, 1)]), None, "HTML 밖");
        assert_eq!(scripts_at(html, &[(3, 99)]), None);
        assert_eq!(scripts_at(html, &[(0, 1)]), None);
    }

    #[test]
    fn 반복으로_여러_번_나온_블록은_jsp_줄을_화면_줄_여럿으로_바꾼다() {
        // `<c:forEach>` 안의 스크립트: 같은 JSP 블록이 항목마다 화면에 나온다.
        let jsp = "<c:forEach var=\"item\" items=\"${items}\">\n<script>\nregister(\"${item.id}\");\nlistReady = true;\n</script>\n</c:forEach>\n";
        let files = vec![(PathBuf::from("jsp/list.jsp"), jsp.to_owned())];
        let blocks = [
            (2, 9, "\nregister(\"a\");\nlistReady = true;\n"),
            (6, 9, "\nregister(\"b\");\nlistReady = true;\n"),
        ];
        let map = PageMap::find(&blocks, &files, "").unwrap();
        let list = Path::new("jsp/list.jsp");
        assert_eq!(map.to_pages(list, 2), [2, 6]);
        assert_eq!(map.to_pages(list, 3), [3, 7]);
        assert_eq!(map.to_file(6), Some((list, 2)));
        assert_eq!(map.files(), [list]);
    }

    #[test]
    fn jsp_파일을_찾는다() {
        let root = std::env::temp_dir().join(format!("ferrite-page-lines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for file in [
            "src/main/webapp/WEB-INF/jsp/board/BoardList.jsp",
            "src/main/webapp/WEB-INF/jsp/inc/header.jspf",
            "src/main/webapp/js/app.js",
            "target/board/WEB-INF/jsp/board/BoardList.jsp",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        assert_eq!(
            candidates(&root),
            [
                root.join("src/main/webapp/WEB-INF/jsp/board/BoardList.jsp"),
                root.join("src/main/webapp/WEB-INF/jsp/inc/header.jspf"),
            ]
        );
        assert!(is_page_file(Path::new("A.JSP")) && !is_page_file(Path::new("a.html")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
