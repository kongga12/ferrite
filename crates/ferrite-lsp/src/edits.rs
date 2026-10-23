//! 편집을 돌려주는 기능: 포맷, 이름 바꾸기, 코드 액션.
//! 서버 응답(WorkspaceEdit 등)을 파일별 편집 목록으로 바꾼다. 위치는 그 파일을 연 뒤
//! `features::char_range`로 문자 범위로 바꾼다(다른 파일일 수 있어서).

use std::ops::Range;
use std::path::{Path, PathBuf};

use lsp_types::{
    Diagnostic, DocumentChangeOperation, DocumentChanges, OneOf, Range as LspRange, TextEdit, Uri,
    WorkspaceEdit,
};
use ropey::Rope;
use serde_json::{Value, json};

use crate::position::{PositionEncoding, to_lsp};
use crate::uri::{path_to_uri, uri_to_path};

/// 한 파일에 적용할 편집들. 범위는 서로 겹치지 않는다(LSP 규칙).
#[derive(Debug, Clone, PartialEq)]
pub struct FileEdit {
    pub path: PathBuf,
    pub edits: Vec<(LspRange, String)>,
    /// 서버가 이 편집을 계산한 문서 버전(알려 준 경우). 지금 버전과 다르면 적용하면 안 된다.
    pub version: Option<i32>,
}

fn text_edits(edits: Vec<TextEdit>) -> Vec<(LspRange, String)> {
    edits.into_iter().map(|e| (e.range, e.new_text)).collect()
}

/// `WorkspaceEdit`을 파일별 편집으로. 같은 파일 편집은 모은다.
/// 파일 만들기·이름 바꾸기·지우기 같은 리소스 작업은 지원하지 않는다고 알렸으므로 무시한다.
pub fn parse_workspace_edit(value: &Value) -> Vec<FileEdit> {
    let Ok(edit) = serde_json::from_value::<WorkspaceEdit>(value.clone()) else {
        return Vec::new();
    };
    type Collected = (Uri, Vec<(LspRange, String)>, Option<i32>);
    let mut by_uri: Vec<Collected> = Vec::new();
    let mut push = |uri: Uri, edits: Vec<(LspRange, String)>, version: Option<i32>| match by_uri
        .iter_mut()
        .find(|(u, _, _)| *u == uri)
    {
        Some((_, existing, _)) => existing.extend(edits),
        None => by_uri.push((uri, edits, version)),
    };
    match edit.document_changes {
        Some(DocumentChanges::Edits(docs)) => {
            for doc in docs {
                let version = doc.text_document.version;
                push(doc.text_document.uri, one_of_edits(doc.edits), version);
            }
        }
        Some(DocumentChanges::Operations(ops)) => {
            for op in ops {
                if let DocumentChangeOperation::Edit(doc) = op {
                    let version = doc.text_document.version;
                    push(doc.text_document.uri, one_of_edits(doc.edits), version);
                }
            }
        }
        None => {
            for (uri, edits) in edit.changes.into_iter().flatten() {
                push(uri, text_edits(edits), None);
            }
        }
    }
    by_uri
        .into_iter()
        .filter_map(|(uri, edits, version)| {
            Some(FileEdit {
                path: uri_to_path(&uri)?,
                edits,
                version,
            })
        })
        .collect()
}

fn one_of_edits(
    edits: Vec<OneOf<TextEdit, lsp_types::AnnotatedTextEdit>>,
) -> Vec<(LspRange, String)> {
    edits
        .into_iter()
        .map(|e| match e {
            OneOf::Left(e) => (e.range, e.new_text),
            OneOf::Right(annotated) => (annotated.text_edit.range, annotated.text_edit.new_text),
        })
        .collect()
}

/// 포맷 응답(`TextEdit[]` 또는 null).
pub fn parse_text_edits(value: &Value) -> Vec<(LspRange, String)> {
    serde_json::from_value::<Option<Vec<TextEdit>>>(value.clone())
        .ok()
        .flatten()
        .map(text_edits)
        .unwrap_or_default()
}

pub fn formatting_params(path: &Path, tab_size: usize, insert_spaces: bool) -> Option<Value> {
    let uri = path_to_uri(path).ok()?;
    Some(json!({
        "textDocument": { "uri": uri },
        "options": { "tabSize": tab_size, "insertSpaces": insert_spaces },
    }))
}

pub fn rename_params(
    path: &Path,
    rope: &Rope,
    char_idx: usize,
    encoding: PositionEncoding,
    new_name: &str,
) -> Option<Value> {
    let mut params = crate::features::position_params(path, rope, char_idx, encoding)?;
    params["newName"] = json!(new_name);
    Some(params)
}

/// 선택 범위(없으면 커서 위치)에 걸친 진단을 함께 보내야 서버가 빠른 수정을 제안한다.
pub fn code_action_params(
    path: &Path,
    rope: &Rope,
    range: Range<usize>,
    encoding: PositionEncoding,
    diagnostics: &[Diagnostic],
) -> Option<Value> {
    let uri = path_to_uri(path).ok()?;
    Some(json!({
        "textDocument": { "uri": uri },
        "range": { "start": to_lsp(rope, range.start, encoding), "end": to_lsp(rope, range.end, encoding) },
        "context": { "diagnostics": diagnostics, "triggerKind": 1 },
    }))
}

#[derive(Debug, Clone)]
pub struct CodeActionItem {
    pub title: String,
    /// 빠른 수정(quickfix)이면 목록 앞에 둔다.
    pub preferred: bool,
    pub edit: Vec<FileEdit>,
    /// 편집이 아직 없어 `codeAction/resolve`로 받아야 하는 액션 원본.
    pub unresolved: Option<Value>,
    /// 편집 대신(또는 편집 뒤에) 실행할 서버 명령(`workspace/executeCommand`의 매개변수).
    pub command: Option<Value>,
}

/// `(Command | CodeAction)[]` 응답. 비활성(disabled) 액션은 뺀다.
pub fn parse_code_actions(value: &Value) -> Vec<CodeActionItem> {
    let Some(list) = value.as_array() else {
        return Vec::new();
    };
    let mut items: Vec<CodeActionItem> = list
        .iter()
        .filter(|v| v.get("disabled").is_none())
        .filter_map(|v| {
            let title = v.get("title")?.as_str()?.to_owned();
            // `command`가 문자열이면 액션이 아니라 명령 자체다.
            if v.get("command").is_some_and(Value::is_string) {
                return Some(CodeActionItem {
                    title,
                    preferred: false,
                    edit: Vec::new(),
                    unresolved: None,
                    command: Some(command_params(v)),
                });
            }
            let edit = v.get("edit").map(parse_workspace_edit).unwrap_or_default();
            Some(CodeActionItem {
                title,
                preferred: v
                    .get("isPreferred")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || v.get("kind").and_then(Value::as_str) == Some("quickfix"),
                unresolved: (v.get("edit").is_none() && v.get("data").is_some()).then(|| v.clone()),
                command: v.get("command").map(command_params),
                edit,
            })
        })
        .collect();
    // 빠른 수정을 먼저, 나머지는 서버 순서대로(stable sort).
    items.sort_by_key(|i| !i.preferred);
    items
}

/// `Command` 객체를 `workspace/executeCommand` 매개변수로.
fn command_params(command: &Value) -> Value {
    json!({
        "command": command.get("command").cloned().unwrap_or(Value::Null),
        "arguments": command.get("arguments").cloned().unwrap_or(json!([])),
    })
}

/// `codeAction/resolve` 응답에서 편집과 명령을 꺼낸다.
pub fn parse_resolved_action(value: &Value) -> (Vec<FileEdit>, Option<Value>) {
    (
        value
            .get("edit")
            .map(parse_workspace_edit)
            .unwrap_or_default(),
        value.get("command").map(command_params),
    )
}

#[cfg(test)]
mod tests {
    use lsp_types::Position;

    use super::*;

    fn range(l0: u32, c0: u32, l1: u32, c1: u32) -> Value {
        json!({ "start": { "line": l0, "character": c0 }, "end": { "line": l1, "character": c1 } })
    }

    #[test]
    fn 작업공간_편집의_두_형식을_파일별로_모은다() {
        let changes = json!({ "changes": {
            "file:///tmp/a.rs": [{ "range": range(0, 0, 0, 1), "newText": "x" }],
        } });
        let parsed = parse_workspace_edit(&changes);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, PathBuf::from("/tmp/a.rs"));

        let document_changes = json!({ "documentChanges": [
            { "textDocument": { "uri": "file:///tmp/a.rs", "version": 3 }, "edits": [{ "range": range(1, 0, 1, 3), "newText": "foo" }] },
            { "kind": "create", "uri": "file:///tmp/new.rs" },
            { "textDocument": { "uri": "file:///tmp/a.rs", "version": 3 }, "edits": [{ "range": range(5, 0, 5, 3), "newText": "bar", "annotationId": "x" }] },
        ] });
        let parsed = parse_workspace_edit(&document_changes);
        assert_eq!(
            parsed.len(),
            1,
            "리소스 작업은 무시하고 같은 파일 편집은 모은다"
        );
        assert_eq!(parsed[0].edits.len(), 2);
        assert_eq!(parsed[0].edits[1].0.start, Position::new(5, 0));
        assert_eq!(parsed[0].version, Some(3), "서버가 계산한 문서 버전을 보존");
    }

    #[test]
    fn 코드_액션은_빠른_수정이_먼저이고_비활성은_뺀다() {
        let actions = json!([
            { "title": "리팩터링", "kind": "refactor", "data": { "id": 1 } },
            { "title": "고치기", "kind": "quickfix", "edit": { "changes": { "file:///tmp/a.rs": [] } } },
            { "title": "꺼짐", "disabled": { "reason": "x" } },
            { "title": "명령", "command": "do.it", "arguments": [1] },
        ]);
        let items = parse_code_actions(&actions);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["고치기", "리팩터링", "명령"]);
        assert!(
            items[1].unresolved.is_some(),
            "편집 없이 data만 있으면 resolve가 필요"
        );
        assert_eq!(items[2].command.as_ref().unwrap()["command"], "do.it");
    }

    #[test]
    fn 포맷_응답이_null이면_빈_목록() {
        assert!(parse_text_edits(&Value::Null).is_empty());
        let edits = parse_text_edits(&json!([{ "range": range(0, 0, 0, 2), "newText": "  " }]));
        assert_eq!(edits.len(), 1);
    }
}
