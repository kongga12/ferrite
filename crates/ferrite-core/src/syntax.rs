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
    Css,
}

impl Lang {
    pub const ALL: [Lang; 15] = [
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
        Lang::Css,
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
            "json" => Lang::Json,
            "yml" | "yaml" => Lang::Yaml,
            "xml" | "xsd" | "xsl" | "tld" => Lang::Xml,
            // ponytail: JSP는 HTML로만 강조한다. 스크립틀릿·EL은 Phase 2b(ferrite-egov-ls).
            "html" | "htm" | "jsp" => Lang::Html,
            "js" | "mjs" | "cjs" => Lang::JavaScript,
            "css" => Lang::Css,
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
            Lang::Css => "CSS",
        }
    }

    /// 줄 주석 기호. 블록 주석만 있는 언어는 None.
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Lang::Rust | Lang::Go | Lang::C | Lang::Cpp | Lang::Php | Lang::Java => Some("//"),
            Lang::JavaScript => Some("//"),
            Lang::Python | Lang::Toml | Lang::Yaml => Some("#"),
            Lang::Sql => Some("--"),
            Lang::Json | Lang::Xml | Lang::Html | Lang::Css => None,
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
            Lang::Html => tree_sitter_html::LANGUAGE.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::Css => tree_sitter_css::LANGUAGE.into(),
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
            Lang::Php => tree_sitter_php::HIGHLIGHTS_QUERY,
            Lang::Java => tree_sitter_java::HIGHLIGHTS_QUERY,
            Lang::Sql => tree_sitter_sequel::HIGHLIGHTS_QUERY,
            Lang::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            Lang::Json => tree_sitter_json::HIGHLIGHTS_QUERY,
            Lang::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY,
            Lang::Xml => tree_sitter_xml::XML_HIGHLIGHT_QUERY,
            Lang::Html => tree_sitter_html::HIGHLIGHTS_QUERY,
            Lang::JavaScript => tree_sitter_javascript::HIGHLIGHT_QUERY,
            Lang::Css => tree_sitter_css::HIGHLIGHTS_QUERY,
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

pub struct Syntax {
    lang: Lang,
    parser: Parser,
    tree: Option<Tree>,
    query: Query,
    /// 캡처 번호별 분류(쿼리를 만들 때 한 번 계산).
    kinds: Vec<Option<Highlight>>,
}

impl Syntax {
    pub fn new(lang: Lang) -> Result<Self, String> {
        let language = lang.language();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| format!("{} 문법을 불러오지 못했습니다: {e}", lang.name()))?;
        let query = Query::new(&language, &lang.highlights_query())
            .map_err(|e| format!("{} 강조 규칙 오류: {e}", lang.name()))?;
        let kinds = query
            .capture_names()
            .iter()
            .map(|n| Highlight::from_capture(n))
            .collect();
        Ok(Self {
            lang,
            parser,
            tree: None,
            query,
            kinds,
        })
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// 버퍼에 쌓인 변경을 트리에 반영하고 다시 파싱한다(바뀐 부분만 다시 분석된다).
    /// 버퍼는 `set_track_syntax_edits(true)` 상태여야 한다.
    pub fn update(&mut self, buffer: &mut Buffer) {
        let edits = buffer.take_syntax_edits();
        if self.tree.is_some() && edits.is_empty() {
            return;
        }
        if let Some(tree) = &mut self.tree {
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
        self.tree = self
            .parser
            .parse_with_options(&mut read, self.tree.as_ref(), None);
    }

    /// `lines` 범위 각 줄의 강조 구간. 여러 캡처가 겹치면 가장 안쪽(짧은) 노드가 이기고,
    /// 같은 노드면 쿼리에서 먼저 나온 패턴이 이긴다.
    pub fn highlight_lines(&self, rope: &Rope, lines: Range<usize>) -> LineHighlights {
        let Some(tree) = &self.tree else {
            return vec![Vec::new(); lines.len()];
        };
        if lines.is_empty() {
            return Vec::new();
        }
        let line_starts: Vec<usize> = lines.clone().map(|l| rope.line_to_byte(l)).collect();
        let start = line_starts[0];
        let end = rope.line_to_byte(lines.end);
        let mut marks: Vec<Vec<Option<(Highlight, usize)>>> = lines
            .clone()
            .map(|l| vec![None; rope.line(l).len_bytes().min(MAX_LINE_BYTES)])
            .collect();

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start..end.min(start + MAX_LINE_BYTES * marks.len()));
        let mut captures = cursor.captures(&self.query, tree.root_node(), RopeText(rope));
        while let Some((m, i)) = captures.next() {
            let capture = m.captures()[*i];
            let Some(kind) = self.kinds[capture.index as usize] else {
                continue;
            };
            let range = capture.node.byte_range();
            let (from, to) = (range.start.max(start), range.end.min(end));
            if from >= to {
                continue;
            }
            let first = rope.byte_to_line(from);
            let last = rope.byte_to_line(to - 1);
            for line in first..=last {
                let row = &mut marks[line - lines.start];
                let line_start = line_starts[line - lines.start];
                let a = from.saturating_sub(line_start).min(row.len());
                let b = (to - line_start).min(row.len());
                for slot in &mut row[a..b] {
                    if slot.is_none_or(|(_, len)| range.len() < len) {
                        *slot = Some((kind, range.len()));
                    }
                }
            }
        }
        marks.into_iter().map(|row| compress(&row)).collect()
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
}
