//! 기능 요청의 매개변수 만들기와 응답 해석: 자동완성, 호버, 정의, 참조.
//! 응답은 에디터가 바로 쓰는 문자 범위·평문으로 바꿔 돌려준다.

use std::ops::Range;
use std::path::{Path, PathBuf};

use lsp_types::{
    CompletionItemKind, CompletionResponse, CompletionTextEdit, GotoDefinitionResponse, Hover,
    HoverContents, Location, MarkedString, Range as LspRange, TextEdit,
};
use ropey::Rope;
use serde_json::{Value, json};

use crate::position::{PositionEncoding, from_lsp, to_lsp};
use crate::uri::{path_to_uri, uri_to_path};

/// `textDocument` + `position` 형태의 공통 매개변수.
pub fn position_params(
    path: &Path,
    rope: &Rope,
    char_idx: usize,
    encoding: PositionEncoding,
) -> Option<Value> {
    let uri = path_to_uri(path).ok()?;
    Some(json!({ "textDocument": { "uri": uri }, "position": to_lsp(rope, char_idx, encoding) }))
}

/// 자동완성 요청. 트리거 문자(`.` 등)로 열렸으면 그 문자를 알려 준다.
pub fn completion_params(
    path: &Path,
    rope: &Rope,
    char_idx: usize,
    encoding: PositionEncoding,
    trigger: Option<char>,
) -> Option<Value> {
    let mut params = position_params(path, rope, char_idx, encoding)?;
    params["context"] = match trigger {
        Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
        None => json!({ "triggerKind": 1 }),
    };
    Some(params)
}

pub fn references_params(
    path: &Path,
    rope: &Rope,
    char_idx: usize,
    encoding: PositionEncoding,
) -> Option<Value> {
    let mut params = position_params(path, rope, char_idx, encoding)?;
    params["context"] = json!({ "includeDeclaration": true });
    Some(params)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    /// 오른쪽에 흐리게 보이는 설명(타입 등).
    pub detail: String,
    /// 거르기·정렬 기준.
    pub filter: String,
    pub sort: String,
    pub kind: &'static str,
    /// 주 편집. 범위가 None이면 커서 앞 단어를 `text`로 바꾼다.
    pub range: Option<Range<usize>>,
    pub text: String,
    /// 함께 적용할 편집(자동 import 등).
    pub additional: Vec<(Range<usize>, String)>,
}

#[derive(Debug, Default)]
pub struct CompletionList {
    pub items: Vec<Completion>,
    /// 서버가 목록을 다 보내지 않았다. 입력이 바뀌면 다시 요청해야 한다.
    pub incomplete: bool,
}

fn edit(rope: &Rope, e: &TextEdit, encoding: PositionEncoding) -> (Range<usize>, String) {
    (char_range(rope, e.range, encoding), e.new_text.clone())
}

pub fn char_range(rope: &Rope, range: LspRange, encoding: PositionEncoding) -> Range<usize> {
    let start = from_lsp(rope, range.start, encoding);
    start..from_lsp(rope, range.end, encoding).max(start)
}

fn kind_label(kind: Option<CompletionItemKind>) -> &'static str {
    match kind {
        Some(CompletionItemKind::METHOD | CompletionItemKind::FUNCTION) => "함수",
        Some(CompletionItemKind::CONSTRUCTOR) => "생성자",
        Some(CompletionItemKind::FIELD | CompletionItemKind::PROPERTY) => "필드",
        Some(CompletionItemKind::VARIABLE) => "변수",
        Some(CompletionItemKind::CLASS | CompletionItemKind::STRUCT) => "타입",
        Some(CompletionItemKind::INTERFACE) => "인터페이스",
        Some(CompletionItemKind::ENUM) => "열거형",
        Some(CompletionItemKind::ENUM_MEMBER) => "값",
        Some(CompletionItemKind::MODULE) => "모듈",
        Some(CompletionItemKind::KEYWORD) => "키워드",
        Some(CompletionItemKind::CONSTANT) => "상수",
        Some(CompletionItemKind::SNIPPET) => "조각",
        _ => "",
    }
}

pub fn parse_completion(result: &Value, rope: &Rope, encoding: PositionEncoding) -> CompletionList {
    let (items, incomplete) =
        match serde_json::from_value::<Option<CompletionResponse>>(result.clone()) {
            Ok(Some(CompletionResponse::Array(items))) => (items, false),
            Ok(Some(CompletionResponse::List(list))) => (list.items, list.is_incomplete),
            _ => (Vec::new(), false),
        };
    let items = items
        .into_iter()
        .map(|item| {
            // 서버가 편집을 주면 그 범위를 쓰고(삽입 모드), 아니면 insertText 또는 label을 넣는다.
            let (range, text) = match &item.text_edit {
                Some(CompletionTextEdit::Edit(e)) => {
                    let (r, t) = edit(rope, e, encoding);
                    (Some(r), t)
                }
                Some(CompletionTextEdit::InsertAndReplace(e)) => (
                    Some(char_range(rope, e.insert, encoding)),
                    e.new_text.clone(),
                ),
                None => (
                    None,
                    item.insert_text
                        .clone()
                        .unwrap_or_else(|| item.label.clone()),
                ),
            };
            let detail = item
                .label_details
                .as_ref()
                .and_then(|d| d.detail.clone().or_else(|| d.description.clone()))
                .or_else(|| item.detail.clone())
                .unwrap_or_default();
            Completion {
                filter: item
                    .filter_text
                    .clone()
                    .unwrap_or_else(|| item.label.clone()),
                sort: item.sort_text.clone().unwrap_or_else(|| item.label.clone()),
                kind: kind_label(item.kind),
                additional: item
                    .additional_text_edits
                    .iter()
                    .flatten()
                    .map(|e| edit(rope, e, encoding))
                    .collect(),
                label: item.label,
                detail,
                range,
                text,
            }
        })
        .collect();
    CompletionList { items, incomplete }
}

/// 호버 내용을 평문으로. 마크다운 코드 울타리(```)는 뺀다. 내용이 없으면 None.
pub fn parse_hover(result: &Value) -> Option<String> {
    let hover: Hover = serde_json::from_value::<Option<Hover>>(result.clone()).ok()??;
    let marked = |m: &MarkedString| match m {
        MarkedString::String(s) => s.clone(),
        MarkedString::LanguageString(ls) => ls.value.clone(),
    };
    let raw = match &hover.contents {
        HoverContents::Scalar(m) => marked(m),
        HoverContents::Array(list) => list.iter().map(marked).collect::<Vec<_>>().join("\n\n"),
        HoverContents::Markup(markup) => markup.value.clone(),
    };
    let text = raw
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// 다른 파일일 수 있는 위치. 범위는 그 파일을 연 뒤 `char_range`로 바꾼다.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub path: PathBuf,
    pub range: LspRange,
}

/// 정의(`Location`, `Location[]`, `LocationLink[]`)와 참조(`Location[]`) 응답을 위치 목록으로.
pub fn parse_locations(result: &Value) -> Vec<Target> {
    let from_location = |l: Location| {
        uri_to_path(&l.uri).map(|path| Target {
            path,
            range: l.range,
        })
    };
    match serde_json::from_value::<Option<GotoDefinitionResponse>>(result.clone()) {
        Ok(Some(GotoDefinitionResponse::Scalar(l))) => from_location(l).into_iter().collect(),
        Ok(Some(GotoDefinitionResponse::Array(list))) => {
            list.into_iter().filter_map(from_location).collect()
        }
        Ok(Some(GotoDefinitionResponse::Link(links))) => links
            .into_iter()
            .filter_map(|link| {
                uri_to_path(&link.target_uri).map(|path| Target {
                    path,
                    // 이름 위치(선택 범위)로 이동한다. 전체 범위는 함수 본문까지 포함한다.
                    range: link.target_selection_range,
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use lsp_types::Position;

    use super::*;

    #[test]
    fn 자동완성_편집_범위와_자동_import를_문자_범위로_바꾼다() {
        let rope = Rope::from_str("fn main() {\n    let s = St\n}\n");
        let result = json!({
            "isIncomplete": true,
            "items": [{
                "label": "String",
                "kind": 22,
                "detail": "alloc::string",
                "textEdit": { "range": { "start": { "line": 1, "character": 12 }, "end": { "line": 1, "character": 14 } }, "newText": "String" },
                "additionalTextEdits": [{ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }, "newText": "use std::string::String;\n" }]
            }, {
                "label": "str"
            }]
        });
        let list = parse_completion(&result, &rope, PositionEncoding::Utf16);
        assert!(list.incomplete);
        let first = &list.items[0];
        assert_eq!(first.range, Some(24..26));
        assert_eq!(first.text, "String");
        assert_eq!(first.kind, "타입");
        assert_eq!(
            first.additional,
            vec![(0..0, "use std::string::String;\n".to_owned())]
        );
        assert_eq!(list.items[1].range, None);
        assert_eq!(list.items[1].text, "str");
    }

    #[test]
    fn 호버는_코드_울타리를_뺀_평문() {
        let result = json!({ "contents": { "kind": "markdown", "value": "```rust\nfn len(&self) -> usize\n```\n\n길이를 돌려준다." } });
        assert_eq!(
            parse_hover(&result).unwrap(),
            "fn len(&self) -> usize\n\n길이를 돌려준다."
        );
        assert_eq!(parse_hover(&Value::Null), None);
    }

    #[test]
    fn 정의_응답의_세_가지_형태를_모두_읽는다() {
        let range =
            json!({ "start": { "line": 3, "character": 4 }, "end": { "line": 3, "character": 8 } });
        let full =
            json!({ "start": { "line": 3, "character": 0 }, "end": { "line": 9, "character": 1 } });
        let scalar = json!({ "uri": "file:///tmp/a.rs", "range": range });
        let link = json!([{ "targetUri": "file:///tmp/b.rs", "targetRange": full, "targetSelectionRange": range }]);
        assert_eq!(parse_locations(&scalar)[0].path, PathBuf::from("/tmp/a.rs"));
        assert_eq!(parse_locations(&json!([scalar])).len(), 1);
        let linked = parse_locations(&link);
        assert_eq!(
            linked[0].range.start,
            Position::new(3, 4),
            "선택 범위(이름 위치)로 이동"
        );
        assert!(parse_locations(&Value::Null).is_empty());
    }
}
