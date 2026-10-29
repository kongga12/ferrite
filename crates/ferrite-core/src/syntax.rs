//! tree-sitter 기반 구문 강조. 편집마다 트리를 증분 갱신하고, 화면에 보이는 줄 범위에만 쿼리를 실행한다.

use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use ropey::Rope;
use tree_sitter::{
    InputEdit, Language, Node, Parser, Point, Query, QueryCursor, StreamingIterator, TextProvider,
    Tree,
};

use crate::buffer::{Buffer, SyntaxEdit};
use crate::jsp::{self, Kind as JspKind, Region};

/// 이보다 큰 파일은 구문 강조를 끈다(초기 파싱 시간·메모리 때문).
// ponytail: UI 스레드에서 동기 파싱하는 상한. 백그라운드 파싱을 넣으면 상향.
pub const MAX_SYNTAX_BYTES: usize = 5 * 1024 * 1024;

/// 한 줄에서 강조를 계산하는 최대 바이트 수(압축된 한 줄짜리 파일 대비).
const MAX_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    Go,
    C,
    Cpp,
    Php,
    Java,
    Sql,
    Toml,
    Json,
    Yaml,
    Xml,
    Html,
    JavaScript,
    TypeScript,
    /// JSX가 든 TypeScript(`.tsx`).
    Tsx,
    Css,
    Properties,
    /// HTML에 스크립틀릿(Java)·EL이 섞인 JSP. HTML 층과 Java 층을 따로 칠하고 JSP 구간을 덧칠한다.
    Jsp,
}

impl Lang {
    pub const ALL: [Lang; 19] = [
        Lang::Rust,
        Lang::Python,
        Lang::Go,
        Lang::C,
        Lang::Cpp,
        Lang::Php,
        Lang::Java,
        Lang::Sql,
        Lang::Toml,
        Lang::Json,
        Lang::Yaml,
        Lang::Xml,
        Lang::Html,
        Lang::JavaScript,
        Lang::TypeScript,
        Lang::Tsx,
        Lang::Css,
        Lang::Properties,
        Lang::Jsp,
    ];

    pub fn from_path(path: &Path) -> Option<Lang> {
        if path.file_name().is_some_and(|n| n == "Cargo.lock") {
            return Some(Lang::Toml);
        }
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Lang::Rust,
            "py" | "pyw" => Lang::Python,
            "go" => Lang::Go,
            "c" | "h" => Lang::C,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Lang::Cpp,
            "php" => Lang::Php,
            "java" => Lang::Java,
            "sql" => Lang::Sql,
            "toml" => Lang::Toml,
            // Jupyter 노트북도 JSON이다(앱은 그 위에 노트북 보기를 얹는다).
            "json" | "ipynb" => Lang::Json,
            "yml" | "yaml" => Lang::Yaml,
            "xml" | "xsd" | "xsl" | "tld" => Lang::Xml,
            "html" | "htm" => Lang::Html,
            "jsp" | "jspf" | "tag" => Lang::Jsp,
            "js" | "mjs" | "cjs" => Lang::JavaScript,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "css" => Lang::Css,
            "properties" => Lang::Properties,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::Python => "Python",
            Lang::Go => "Go",
            Lang::C => "C",
            Lang::Cpp => "C++",
            Lang::Php => "PHP",
            Lang::Java => "Java",
            Lang::Sql => "SQL",
            Lang::Toml => "TOML",
            Lang::Json => "JSON",
            Lang::Yaml => "YAML",
            Lang::Xml => "XML",
            Lang::Html => "HTML",
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Tsx => "TSX",
            Lang::Css => "CSS",
            Lang::Properties => "Properties",
            Lang::Jsp => "JSP",
        }
    }

    /// LSP `languageId`(textDocument/didOpen에 쓰는 표준 식별자).
    pub fn lsp_id(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::Go => "go",
            Lang::C => "c",
            Lang::Cpp => "cpp",
            Lang::Php => "php",
            Lang::Java => "java",
            Lang::Sql => "sql",
            Lang::Toml => "toml",
            Lang::Json => "json",
            Lang::Yaml => "yaml",
            Lang::Xml => "xml",
            Lang::Html => "html",
            Lang::JavaScript => "javascript",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "typescriptreact",
            Lang::Css => "css",
            Lang::Properties => "properties",
            Lang::Jsp => "jsp",
        }
    }

    /// 줄 주석 기호. 블록 주석만 있는 언어는 None.
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Lang::Rust | Lang::Go | Lang::C | Lang::Cpp | Lang::Php | Lang::Java => Some("//"),
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => Some("//"),
            Lang::Python | Lang::Toml | Lang::Yaml | Lang::Properties => Some("#"),
            Lang::Sql => Some("--"),
            Lang::Json | Lang::Xml | Lang::Html | Lang::Css | Lang::Jsp => None,
        }
    }

    fn language(self) -> Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
            Lang::C => tree_sitter_c::LANGUAGE.into(),
            Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Lang::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
            Lang::Sql => tree_sitter_sequel::LANGUAGE.into(),
            Lang::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Lang::Json => tree_sitter_json::LANGUAGE.into(),
            Lang::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Lang::Xml => tree_sitter_xml::LANGUAGE_XML.into(),
            Lang::Html | Lang::Jsp => tree_sitter_html::LANGUAGE.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Css => tree_sitter_css::LANGUAGE.into(),
            Lang::Properties => tree_sitter_properties::LANGUAGE.into(),
        }
    }

    fn highlights_query(self) -> Cow<'static, str> {
        Cow::Borrowed(match self {
            Lang::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
            Lang::Python => tree_sitter_python::HIGHLIGHTS_QUERY,
            Lang::Go => tree_sitter_go::HIGHLIGHTS_QUERY,
            Lang::C => tree_sitter_c::HIGHLIGHT_QUERY,
            // C++ 쿼리는 C++ 고유 규칙만 담고 있어 C 규칙을 뒤에 이어 붙인다(앞 패턴이 우선).
            Lang::Cpp => {
                return Cow::Owned(format!(
                    "{}\n{}",
                    tree_sitter_cpp::HIGHLIGHT_QUERY,
                    tree_sitter_c::HIGHLIGHT_QUERY
                ));
            }
            // TypeScript 쿼리는 TS 고유 규칙만 담고 있어 JS 규칙(TSX는 JSX 규칙도)을 뒤에 붙인다.
            Lang::TypeScript => {
                return Cow::Owned(format!(
                    "{}\n{}",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY
                ));
            }
            Lang::Tsx => {
                return Cow::Owned(format!(
                    "{}\n{}\n{}",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY
                ));
            }
            Lang::Php => tree_sitter_php::HIGHLIGHTS_QUERY,
            Lang::Java => tree_sitter_java::HIGHLIGHTS_QUERY,
            Lang::Sql => tree_sitter_sequel::HIGHLIGHTS_QUERY,
            Lang::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            Lang::Json => tree_sitter_json::HIGHLIGHTS_QUERY,
            Lang::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY,
            Lang::Xml => tree_sitter_xml::XML_HIGHLIGHT_QUERY,
            Lang::Html | Lang::Jsp => tree_sitter_html::HIGHLIGHTS_QUERY,
            Lang::JavaScript => tree_sitter_javascript::HIGHLIGHT_QUERY,
            Lang::Css => tree_sitter_css::HIGHLIGHTS_QUERY,
            Lang::Properties => tree_sitter_properties::HIGHLIGHTS_QUERY,
        })
    }
}

/// 색을 정하는 강조 분류. 실제 색은 UI 테마가 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Highlight {
    Keyword,
    String,
    Comment,
    Function,
    Type,
    Constant,
    Property,
    Tag,
}

impl Highlight {
    /// 쿼리 캡처 이름(`keyword.function` 등)을 분류로 바꾼다. 나머지는 기본 글자색.
    fn from_capture(name: &str) -> Option<Highlight> {
        if name == "variable.builtin" {
            return Some(Highlight::Keyword); // self, this 등
        }
        let head = name.split('.').next().unwrap_or(name);
        Some(match head {
            "keyword" | "conditional" | "repeat" | "include" | "exception" | "storageclass" => {
                Highlight::Keyword
            }
            "string" | "character" => Highlight::String,
            "comment" => Highlight::Comment,
            "function" | "method" => Highlight::Function,
            "type" | "constructor" | "namespace" | "module" => Highlight::Type,
            "constant" | "number" | "boolean" | "float" | "escape" => Highlight::Constant,
            "property" | "attribute" | "field" | "label" => Highlight::Property,
            "tag" => Highlight::Tag,
            _ => return None,
        })
    }
}

/// 줄마다 (줄 안 바이트 범위, 분류) 목록. 범위는 겹치지 않고 시작 순으로 정렬되어 있다.
pub type LineHighlights = Vec<Vec<(Range<usize>, Highlight)>>;

/// 줄마다 바이트별 (분류, 그 분류를 준 노드 길이). 짧은 노드(더 안쪽)가 이긴다.
type Marks = Vec<Vec<Option<(Highlight, usize)>>>;

/// 문법 하나의 파서·트리·쿼리.
struct Layer {
    parser: Parser,
    tree: Option<Tree>,
    query: Query,
    /// 캡처 번호별 분류(쿼리를 만들 때 한 번 계산).
    kinds: Vec<Option<Highlight>>,
}

impl Layer {
    fn new(language: Language, query: &str, name: &str) -> Result<Self, String> {
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| format!("{name} 문법을 불러오지 못했습니다: {e}"))?;
        let query =
            Query::new(&language, query).map_err(|e| format!("{name} 강조 규칙 오류: {e}"))?;
        let kinds = query
            .capture_names()
            .iter()
            .map(|n| Highlight::from_capture(n))
            .collect();
        Ok(Self {
            parser,
            tree: None,
            query,
            kinds,
        })
    }

    /// 보이는 줄 범위의 캡처를 `marks`에 칠한다.
    fn mark(&self, rope: &Rope, view: &View, marks: &mut Marks) {
        let Some(tree) = &self.tree else {
            return;
        };
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(view.start..view.end.min(view.start + MAX_LINE_BYTES * marks.len()));
        let mut captures = cursor.captures(&self.query, tree.root_node(), RopeText(rope));
        while let Some((m, i)) = captures.next() {
            let capture = m.captures()[*i];
            let Some(kind) = self.kinds[capture.index as usize] else {
                continue;
            };
            let range = capture.node.byte_range();
            let len = range.len();
            view.paint(rope, marks, range, |slot| {
                if slot.is_none_or(|(_, old)| len < old) {
                    *slot = Some((kind, len));
                }
            });
        }
    }
}

/// 칠할 줄 범위.
struct View {
    lines: Range<usize>,
    line_starts: Vec<usize>,
    start: usize,
    end: usize,
}

impl View {
    /// `range`(문서 바이트)가 걸친 줄의 칸마다 `f`를 부른다.
    fn paint(
        &self,
        rope: &Rope,
        marks: &mut Marks,
        range: Range<usize>,
        mut f: impl FnMut(&mut Option<(Highlight, usize)>),
    ) {
        let (from, to) = (range.start.max(self.start), range.end.min(self.end));
        if from >= to {
            return;
        }
        for line in rope.byte_to_line(from)..=rope.byte_to_line(to - 1) {
            let row = &mut marks[line - self.lines.start];
            let line_start = self.line_starts[line - self.lines.start];
            let a = from.saturating_sub(line_start).min(row.len());
            let b = (to - line_start).min(row.len());
            row[a..b].iter_mut().for_each(&mut f);
        }
    }

    /// 무조건 이 분류로 덮는다(JSP 구간 덧칠).
    fn force(&self, rope: &Rope, marks: &mut Marks, range: Range<usize>, kind: Highlight) {
        self.paint(rope, marks, range, |slot| *slot = Some((kind, 0)));
    }
}

/// JSP의 Java 층과 구간.
struct JspLayers {
    java: Layer,
    regions: Vec<Region>,
}

pub struct Syntax {
    lang: Lang,
    main: Layer,
    jsp: Option<JspLayers>,
}

impl Syntax {
    pub fn new(lang: Lang) -> Result<Self, String> {
        let main = Layer::new(lang.language(), &lang.highlights_query(), lang.name())?;
        let jsp = match lang {
            Lang::Jsp => Some(JspLayers {
                java: Layer::new(
                    Lang::Java.language(),
                    &Lang::Java.highlights_query(),
                    Lang::Java.name(),
                )?,
                regions: Vec::new(),
            }),
            _ => None,
        };
        Ok(Self { lang, main, jsp })
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// 버퍼에 쌓인 변경을 트리에 반영하고 다시 파싱한다(바뀐 부분만 다시 분석된다).
    /// 버퍼는 `set_track_syntax_edits(true)` 상태여야 한다.
    pub fn update(&mut self, buffer: &mut Buffer) {
        let edits = buffer.take_syntax_edits();
        if self.main.tree.is_some() && edits.is_empty() {
            return;
        }
        if let Some(jsp) = &mut self.jsp {
            // ponytail: JSP는 매번 처음부터 파싱한다. `<%`를 하나 치면 구간 경계가 편집 밖까지
            // 바뀌어 증분 파싱의 전제(편집 밖 내용은 그대로)가 깨진다. 큰 JSP에서 느리면 구간이
            // 바뀌지 않은 편집만 증분으로.
            let text = buffer.rope().to_string();
            jsp.regions = jsp::regions(&text);
            self.main.tree = self
                .main
                .parser
                .parse(jsp::html_text(&text, &jsp.regions), None);
            jsp.java.tree = jsp
                .java
                .parser
                .parse(jsp::java_text(&text, &jsp.regions), None);
            return;
        }
        if let Some(tree) = &mut self.main.tree {
            for edit in &edits {
                tree.edit(&input_edit(edit));
            }
        }
        let rope = buffer.rope();
        let len = rope.len_bytes();
        let mut read = |byte: usize, _: Point| -> &[u8] {
            if byte >= len {
                return &[];
            }
            let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
            &chunk.as_bytes()[byte - chunk_start..]
        };
        self.main.tree =
            self.main
                .parser
                .parse_with_options(&mut read, self.main.tree.as_ref(), None);
    }

    /// `lines` 범위 각 줄의 강조 구간. 여러 캡처가 겹치면 가장 안쪽(짧은) 노드가 이기고,
    /// 같은 노드면 쿼리에서 먼저 나온 패턴이 이긴다. JSP 구간은 그 위에 덧칠한다.
    pub fn highlight_lines(&self, rope: &Rope, lines: Range<usize>) -> LineHighlights {
        if self.main.tree.is_none() {
            return vec![Vec::new(); lines.len()];
        }
        if lines.is_empty() {
            return Vec::new();
        }
        let line_starts: Vec<usize> = lines.clone().map(|l| rope.line_to_byte(l)).collect();
        let view = View {
            start: line_starts[0],
            end: rope.line_to_byte(lines.end),
            lines: lines.clone(),
            line_starts,
        };
        let mut marks: Marks = lines
            .clone()
            .map(|l| vec![None; rope.line(l).len_bytes().min(MAX_LINE_BYTES)])
            .collect();
        self.main.mark(rope, &view, &mut marks);
        if let Some(jsp) = &self.jsp {
            jsp.java.mark(rope, &view, &mut marks);
            for region in jsp
                .regions
                .iter()
                .filter(|r| r.range.end > view.start && r.range.start < view.end)
            {
                paint_region(rope, &view, &mut marks, region);
            }
        }
        marks.into_iter().map(|row| compress(&row)).collect()
    }
}

/// JSP 구간 덧칠: 주석은 통째로, 구분자는 키워드, EL 안쪽은 속성, 지시자의 문자열은 문자열.
/// 스크립틀릿 안쪽은 Java 층이 칠한 그대로 둔다.
fn paint_region(rope: &Rope, view: &View, marks: &mut Marks, region: &Region) {
    let range = region.range.clone();
    let inner = region.inner();
    if region.kind == JspKind::Comment {
        view.force(rope, marks, range, Highlight::Comment);
        return;
    }
    view.force(rope, marks, range.start..inner.start, Highlight::Keyword);
    view.force(rope, marks, inner.end..range.end, Highlight::Keyword);
    match region.kind {
        JspKind::El => view.force(rope, marks, inner, Highlight::Property),
        JspKind::Directive => {
            // `<%@ page contentType="…" %>`: 지시자 이름은 키워드, 따옴표 값은 문자열.
            let text = rope.byte_slice(inner.clone()).to_string();
            let name_len = text
                .trim_start()
                .find(char::is_whitespace)
                .unwrap_or(text.trim_start().len());
            let name_start = inner.start + (text.len() - text.trim_start().len());
            view.force(
                rope,
                marks,
                name_start..name_start + name_len,
                Highlight::Keyword,
            );
            let mut quote: Option<(char, usize)> = None;
            for (i, c) in text.char_indices() {
                match quote {
                    Some((q, start)) if c == q => {
                        view.force(
                            rope,
                            marks,
                            inner.start + start..inner.start + i + 1,
                            Highlight::String,
                        );
                        quote = None;
                    }
                    None if c == '"' || c == '\'' => quote = Some((c, i)),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// 바이트별 분류를 같은 분류가 이어진 구간으로 묶는다.
fn compress(row: &[Option<(Highlight, usize)>]) -> Vec<(Range<usize>, Highlight)> {
    let mut out: Vec<(Range<usize>, Highlight)> = Vec::new();
    for (i, slot) in row.iter().enumerate() {
        let Some((kind, _)) = slot else {
            continue;
        };
        match out.last_mut() {
            Some((range, last)) if range.end == i && last == kind => range.end = i + 1,
            _ => out.push((i..i + 1, *kind)),
        }
    }
    out
}

fn input_edit(e: &SyntaxEdit) -> InputEdit {
    let point = |(row, column): (usize, usize)| Point { row, column };
    InputEdit {
        start_byte: e.start_byte,
        old_end_byte: e.old_end_byte,
        new_end_byte: e.new_end_byte,
        start_position: point(e.start_point),
        old_end_position: point(e.old_end_point),
        new_end_position: point(e.new_end_point),
    }
}

/// 쿼리의 텍스트 조건(`#eq?`, `#match?`)이 로프를 직접 읽게 한다.
struct RopeText<'a>(&'a Rope);

impl<'a> TextProvider<&'a [u8]> for RopeText<'a> {
    type I = std::iter::Map<ropey::iter::Chunks<'a>, fn(&'a str) -> &'a [u8]>;

    fn text(&mut self, node: Node) -> Self::I {
        self.0
            .byte_slice(node.byte_range())
            .chunks()
            .map(str::as_bytes as fn(&'a str) -> &'a [u8])
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::buffer::Selection;

    fn kinds_on_line(syntax: &Syntax, buffer: &Buffer, line: usize) -> Vec<(String, Highlight)> {
        let text = buffer.rope().line(line).to_string();
        syntax.highlight_lines(buffer.rope(), line..line + 1)[0]
            .iter()
            .map(|(r, k)| (text[r.clone()].to_owned(), *k))
            .collect()
    }

    #[test]
    fn 모든_언어의_강조_규칙이_컴파일된다() {
        for lang in Lang::ALL {
            if let Err(e) = Syntax::new(lang) {
                panic!("{e}");
            }
        }
    }

    #[test]
    fn 러스트_키워드_문자열_주석() {
        let mut buffer = Buffer::new("fn main() {\n    let s = \"한글\"; // 주석\n}\n");
        buffer.set_track_syntax_edits(true);
        let mut syntax = Syntax::new(Lang::Rust).unwrap();
        syntax.update(&mut buffer);
        let line1 = kinds_on_line(&syntax, &buffer, 1);
        assert!(
            line1.contains(&("let".into(), Highlight::Keyword)),
            "{line1:?}"
        );
        assert!(
            line1.contains(&("\"한글\"".into(), Highlight::String)),
            "{line1:?}"
        );
        assert!(
            line1.contains(&("// 주석".into(), Highlight::Comment)),
            "{line1:?}"
        );
    }

    #[test]
    fn 타입스크립트는_ts_규칙과_js_규칙으로_칠한다() {
        assert_eq!(Lang::from_path(Path::new("a.ts")), Some(Lang::TypeScript));
        assert_eq!(Lang::from_path(Path::new("a.tsx")), Some(Lang::Tsx));
        assert_eq!(Lang::Tsx.lsp_id(), "typescriptreact");
        let mut buffer = Buffer::new("interface A { n: number }\nconst s = \"x\"; // c\n");
        buffer.set_track_syntax_edits(true);
        let mut syntax = Syntax::new(Lang::TypeScript).unwrap();
        syntax.update(&mut buffer);
        let line0 = kinds_on_line(&syntax, &buffer, 0);
        for want in [
            ("interface", Highlight::Keyword),
            ("A", Highlight::Type),
            ("number", Highlight::Type),
        ] {
            assert!(line0.contains(&(want.0.into(), want.1)), "{line0:?}");
        }
        let line1 = kinds_on_line(&syntax, &buffer, 1);
        for want in [
            ("const", Highlight::Keyword),
            ("\"x\"", Highlight::String),
            ("// c", Highlight::Comment),
        ] {
            assert!(line1.contains(&(want.0.into(), want.1)), "{line1:?}");
        }
        let mut buffer = Buffer::new("const e = <div className=\"a\" />;\n");
        buffer.set_track_syntax_edits(true);
        let mut syntax = Syntax::new(Lang::Tsx).unwrap();
        syntax.update(&mut buffer);
        let line0 = kinds_on_line(&syntax, &buffer, 0);
        assert!(line0.contains(&("div".into(), Highlight::Tag)), "{line0:?}");
    }

    #[test]
    fn 편집하면_증분_갱신된다() {
        let mut buffer = Buffer::new("fn main() {}\n");
        buffer.set_track_syntax_edits(true);
        let mut syntax = Syntax::new(Lang::Rust).unwrap();
        syntax.update(&mut buffer);
        assert_eq!(
            kinds_on_line(&syntax, &buffer, 0)[0],
            ("fn".into(), Highlight::Keyword)
        );

        // "fn"을 "// fn"으로 바꾸면 줄 전체가 주석이 된다
        buffer.set_selection(Selection::caret(0));
        buffer.insert("// ", Instant::now());
        syntax.update(&mut buffer);
        assert_eq!(
            kinds_on_line(&syntax, &buffer, 0),
            vec![("// fn main() {}".into(), Highlight::Comment)]
        );
    }

    #[test]
    fn 확장자로_언어를_고른다() {
        assert_eq!(Lang::from_path(Path::new("a/B.JAVA")), Some(Lang::Java));
        assert_eq!(Lang::from_path(Path::new("mapper.xml")), Some(Lang::Xml));
        assert_eq!(Lang::from_path(Path::new("Cargo.lock")), Some(Lang::Toml));
        assert_eq!(Lang::from_path(Path::new("README")), None);
    }

    #[test]
    fn jsp는_html과_스크립틀릿_java와_el을_함께_칠한다() {
        let text = "<%@ page contentType=\"text/html\" %>\n<p class=\"t\">${vo.title}</p>\n<% String s = \"한글\"; %>\n<%-- 주석 --%>\n";
        let mut buffer = Buffer::new(text);
        buffer.set_track_syntax_edits(true);
        let mut syntax = Syntax::new(Lang::Jsp).unwrap();
        syntax.update(&mut buffer);
        let line = |n| kinds_on_line(&syntax, &buffer, n);
        let directive = line(0);
        assert!(
            directive.contains(&("page".into(), Highlight::Keyword)),
            "{directive:?}"
        );
        assert!(
            directive.contains(&("\"text/html\"".into(), Highlight::String)),
            "{directive:?}"
        );
        let html = line(1);
        assert!(
            html.contains(&("p".into(), Highlight::Tag)),
            "HTML 층: {html:?}"
        );
        assert!(
            html.contains(&("${".into(), Highlight::Keyword)),
            "{html:?}"
        );
        assert!(
            html.contains(&("vo.title".into(), Highlight::Property)),
            "EL: {html:?}"
        );
        let java = line(2);
        assert!(
            java.contains(&("\"한글\"".into(), Highlight::String)),
            "Java 층: {java:?}"
        );
        assert!(
            java.contains(&("<%".into(), Highlight::Keyword)),
            "{java:?}"
        );
        assert_eq!(line(3), [("<%-- 주석 --%>".into(), Highlight::Comment)]);
    }

    #[test]
    fn jsp는_편집하면_구간을_다시_찾는다() {
        let mut buffer = Buffer::new("<p>a</p>");
        buffer.set_track_syntax_edits(true);
        let mut syntax = Syntax::new(Lang::Jsp).unwrap();
        syntax.update(&mut buffer);
        buffer.set_selection(Selection::caret(0));
        buffer.insert("<%-- ", Instant::now());
        syntax.update(&mut buffer);
        assert_eq!(
            kinds_on_line(&syntax, &buffer, 0),
            vec![("<%-- <p>a</p>".into(), Highlight::Comment)],
            "닫히지 않은 JSP 주석은 끝까지"
        );
        assert_eq!(Lang::from_path(Path::new("list.jsp")), Some(Lang::Jsp));
    }
}
