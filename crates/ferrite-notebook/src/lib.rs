//! Jupyter 노트북(.ipynb, nbformat 4). 문서를 JSON 값 그대로 들고 있다가 고친 곳만 바꾼다.
//!
//! 쓸 때는 Jupyter(nbformat)와 같은 모양으로 쓴다: 키 정렬, 들여쓰기 한 칸, 한글은 그대로, 끝에 줄바꿈.
//! 그래서 Jupyter가 쓴 파일은 고치지 않은 곳이 바이트까지 그대로 남는다(다른 도구가 다른 모양으로 쓴
//! 파일은 처음 저장할 때 한 번 이 모양으로 바뀐다).

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use serde::Serialize;
use serde_json::{Map, Value};

pub mod kernel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    Code,
    Markdown,
    Raw,
}

impl CellKind {
    pub const ALL: [CellKind; 3] = [CellKind::Code, CellKind::Markdown, CellKind::Raw];

    fn name(self) -> &'static str {
        match self {
            CellKind::Code => "code",
            CellKind::Markdown => "markdown",
            CellKind::Raw => "raw",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CellKind::Code => "코드",
            CellKind::Markdown => "마크다운",
            CellKind::Raw => "원본",
        }
    }
}

/// 셀 출력 하나.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    /// 표준 출력·표준 오류(`name`이 `stdout`·`stderr`).
    Stream { name: String, text: String },
    /// `execute_result`·`display_data`. (MIME 형식, 값) 목록. 글 값은 줄 목록을 이어 붙인 것이다.
    Data {
        data: Vec<(String, String)>,
        execution_count: Option<u64>,
    },
    /// 예외. `traceback` 줄에는 터미널 색 코드가 들어 있다.
    Error {
        ename: String,
        evalue: String,
        traceback: Vec<String>,
    },
    /// 모르는 출력 형식(이름).
    Other(String),
}

/// 노트북 글 값: 문자열이거나 줄 목록(nbformat은 줄 목록으로 쓴다).
fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(lines) => lines.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// nbformat처럼 줄마다 나눈 목록(줄바꿈을 줄 끝에 둔다. 빈 글은 빈 목록).
fn lines(text: &str) -> Value {
    Value::Array(
        text.split_inclusive('\n')
            .map(|line| Value::String(line.to_owned()))
            .collect(),
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notebook {
    doc: Value,
}

impl Notebook {
    /// nbformat 4 노트북을 읽는다.
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc: Value =
            serde_json::from_str(text).map_err(|e| format!("JSON을 읽지 못했습니다: {e}"))?;
        match doc.get("nbformat").and_then(Value::as_u64) {
            Some(4) => {}
            Some(other) => return Err(format!("nbformat 4만 엽니다(이 파일은 {other})")),
            None => return Err("노트북이 아닙니다(nbformat이 없습니다)".into()),
        }
        if !doc.get("cells").is_some_and(Value::is_array) {
            return Err("노트북이 아닙니다(cells가 없습니다)".into());
        }
        Ok(Self { doc })
    }

    /// Jupyter와 같은 모양의 글(키 정렬, 들여쓰기 한 칸, 끝 줄바꿈).
    pub fn to_json(&self) -> String {
        let mut out = Vec::new();
        let mut serializer = serde_json::Serializer::with_formatter(
            &mut out,
            serde_json::ser::PrettyFormatter::with_indent(b" "),
        );
        // 메모리에 쓰므로 실패하지 않는다.
        let _ = self.doc.serialize(&mut serializer);
        let mut text = String::from_utf8(out).unwrap_or_default();
        text.push('\n');
        text
    }

    fn cells(&self) -> &[Value] {
        self.doc["cells"].as_array().map_or(&[], Vec::as_slice)
    }

    fn cells_mut(&mut self) -> &mut Vec<Value> {
        // `parse`가 배열임을 확인했으므로 바뀌는 일은 없다.
        if !self.doc["cells"].is_array() {
            self.doc["cells"] = Value::Array(Vec::new());
        }
        self.doc["cells"].as_array_mut().expect("cells는 배열이다")
    }

    fn cell_mut(&mut self, index: usize) -> Option<&mut Map<String, Value>> {
        self.cells_mut().get_mut(index)?.as_object_mut()
    }

    pub fn len(&self) -> usize {
        self.cells().len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells().is_empty()
    }

    pub fn kind(&self, index: usize) -> CellKind {
        match self.cells()[index]["cell_type"].as_str() {
            Some("code") => CellKind::Code,
            Some("markdown") => CellKind::Markdown,
            _ => CellKind::Raw,
        }
    }

    pub fn source(&self, index: usize) -> String {
        text(&self.cells()[index]["source"])
    }

    /// 셀 글을 바꾼다. 같은 글이면 그대로 둔다(줄 목록·문자열 모양도 남는다).
    pub fn set_source(&mut self, index: usize, source: &str) {
        if self.source(index) == source {
            return;
        }
        if let Some(cell) = self.cell_mut(index) {
            cell.insert("source".into(), lines(source));
        }
    }

    pub fn execution_count(&self, index: usize) -> Option<u64> {
        self.cells()[index]["execution_count"].as_u64()
    }

    pub fn outputs(&self, index: usize) -> Vec<Output> {
        let Some(outputs) = self.cells()[index]["outputs"].as_array() else {
            return Vec::new();
        };
        outputs.iter().map(output).collect()
    }

    /// 노트북의 언어(커널 정보에서). 모르면 None.
    pub fn language(&self) -> Option<&str> {
        let metadata = &self.doc["metadata"];
        metadata["language_info"]["name"]
            .as_str()
            .or_else(|| metadata["kernelspec"]["language"].as_str())
    }

    /// 노트북을 만든 커널의 이름(kernelspec). 모르면 None.
    pub fn kernel_name(&self) -> Option<&str> {
        self.doc["metadata"]["kernelspec"]["name"].as_str()
    }

    /// 코드 셀의 출력을 비운다.
    pub fn clear_outputs(&mut self, index: usize) {
        if self.kind(index) == CellKind::Code
            && let Some(cell) = self.cell_mut(index)
        {
            cell.insert("outputs".into(), Value::Array(Vec::new()));
        }
    }

    pub fn set_execution_count(&mut self, index: usize, count: Option<u64>) {
        if self.kind(index) == CellKind::Code
            && let Some(cell) = self.cell_mut(index)
        {
            cell.insert(
                "execution_count".into(),
                count.map_or(Value::Null, Value::from),
            );
        }
    }

    /// 커널 출력(nbformat 출력 JSON)을 코드 셀에 더한다. nbformat처럼 글은 줄 목록으로 쓰고, 바로 앞
    /// 출력과 같은 스트림이면 이어 붙인다.
    pub fn push_output(&mut self, index: usize, mut output: Value) {
        if self.kind(index) != CellKind::Code {
            return;
        }
        let Some(cell) = self.cell_mut(index) else {
            return;
        };
        let outputs = cell
            .entry("outputs")
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(outputs) = outputs.as_array_mut() else {
            return;
        };
        if output["output_type"] == "stream" {
            if let Some(last) = outputs.last_mut()
                && last["output_type"] == "stream"
                && last["name"] == output["name"]
            {
                let joined = text(&last["text"]) + &text(&output["text"]);
                last["text"] = lines(&joined);
                return;
            }
            output["text"] = lines(&text(&output["text"]));
        } else if let Some(data) = output.get_mut("data").and_then(Value::as_object_mut) {
            for (mime, value) in data.iter_mut() {
                // nbformat은 글 형식만 줄로 나눈다(그림 base64는 한 줄 그대로).
                let split = mime.starts_with("text/")
                    || mime == "image/svg+xml"
                    || mime == "application/javascript";
                if split && value.is_string() {
                    *value = lines(&text(value));
                }
            }
        }
        outputs.push(output);
    }

    /// `at` 자리에 빈 셀을 넣는다. nbformat 4.5부터는 셀마다 고유 id가 있어야 한다.
    pub fn insert(&mut self, at: usize, kind: CellKind) {
        let with_id = self.doc["nbformat_minor"].as_u64().unwrap_or(0) >= 5;
        let id = with_id.then(|| self.new_id());
        let mut cell = Map::new();
        cell.insert("cell_type".into(), Value::String(kind.name().into()));
        if let Some(id) = id {
            cell.insert("id".into(), Value::String(id));
        }
        cell.insert("metadata".into(), Value::Object(Map::new()));
        cell.insert("source".into(), Value::Array(Vec::new()));
        if kind == CellKind::Code {
            cell.insert("execution_count".into(), Value::Null);
            cell.insert("outputs".into(), Value::Array(Vec::new()));
        }
        let cells = self.cells_mut();
        let at = at.min(cells.len());
        cells.insert(at, Value::Object(cell));
    }

    pub fn remove(&mut self, index: usize) {
        let cells = self.cells_mut();
        if index < cells.len() {
            cells.remove(index);
        }
    }

    /// 셀을 위(-1)나 아래(+1)로 옮긴다. 옮긴 뒤 자리를 돌려준다.
    pub fn move_cell(&mut self, index: usize, up: bool) -> usize {
        let len = self.len();
        let to = if up {
            index.checked_sub(1)
        } else {
            (index + 1 < len).then_some(index + 1)
        };
        match to {
            Some(to) => {
                self.cells_mut().swap(index, to);
                to
            }
            None => index,
        }
    }

    /// 셀 종류를 바꾼다. 코드 셀이 되면 실행 번호·출력 칸을 더하고, 코드 셀이 아니게 되면 뺀다.
    pub fn set_kind(&mut self, index: usize, kind: CellKind) {
        if self.kind(index) == kind {
            return;
        }
        let Some(cell) = self.cell_mut(index) else {
            return;
        };
        cell.insert("cell_type".into(), Value::String(kind.name().into()));
        if kind == CellKind::Code {
            cell.insert("execution_count".into(), Value::Null);
            cell.insert("outputs".into(), Value::Array(Vec::new()));
        } else {
            cell.remove("execution_count");
            cell.remove("outputs");
        }
    }

    /// 다른 셀과 겹치지 않는 8자리 id(nbformat이 만드는 모양과 같다).
    fn new_id(&self) -> String {
        loop {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_usize(self.len());
            let id = format!("{:08x}", hasher.finish() as u32);
            if !self.cells().iter().any(|c| c["id"].as_str() == Some(&id)) {
                return id;
            }
        }
    }
}

fn output(value: &Value) -> Output {
    let field = |name: &str| value[name].as_str().unwrap_or_default().to_owned();
    match value["output_type"].as_str().unwrap_or_default() {
        "stream" => Output::Stream {
            name: field("name"),
            text: text(&value["text"]),
        },
        "execute_result" | "display_data" => Output::Data {
            data: value["data"]
                .as_object()
                .map(|data| {
                    data.iter()
                        .map(|(mime, value)| {
                            let shown = match value {
                                Value::String(_) | Value::Array(_) => text(value),
                                // JSON 출력(application/json 등)은 글로 보인다.
                                other => other.to_string(),
                            };
                            (mime.clone(), shown)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            execution_count: value["execution_count"].as_u64(),
        },
        "error" => Output::Error {
            ename: field("ename"),
            evalue: field("evalue"),
            traceback: value["traceback"]
                .as_array()
                .map(|lines| {
                    lines
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        },
        other => Output::Other(other.to_owned()),
    }
}

/// 터미널 색 코드(`ESC [ … 글자`)를 뺀다(Python 오류 추적에 들어 있다).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            while chars.next().is_some_and(|c| !c.is_ascii_alphabetic()) {}
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SMALL: &str = r#"{
 "cells": [
  {
   "cell_type": "code",
   "execution_count": 1,
   "id": "a1",
   "metadata": {},
   "outputs": [],
   "source": [
    "print(1)\n",
    "x = 2"
   ]
  }
 ],
 "metadata": {
  "language_info": {
   "name": "python"
  }
 },
 "nbformat": 4,
 "nbformat_minor": 5
}
"#;

    #[test]
    fn 읽고_그대로_쓰면_같다() {
        let notebook = Notebook::parse(SMALL).unwrap();
        assert_eq!(notebook.to_json(), SMALL);
        assert_eq!(notebook.len(), 1);
        assert_eq!(notebook.kind(0), CellKind::Code);
        assert_eq!(notebook.source(0), "print(1)\nx = 2");
        assert_eq!(notebook.execution_count(0), Some(1));
        assert_eq!(notebook.language(), Some("python"));
    }

    #[test]
    fn 셀을_고치고_넣고_옮기고_지운다() {
        let mut notebook = Notebook::parse(SMALL).unwrap();
        notebook.set_source(0, "print(1)\nx = 2");
        assert_eq!(notebook.to_json(), SMALL, "같은 글이면 그대로");
        notebook.set_source(0, "y = 3\n");
        assert!(
            notebook
                .to_json()
                .contains("\"source\": [\n    \"y = 3\\n\"\n   ]")
        );
        notebook.insert(1, CellKind::Markdown);
        assert_eq!(notebook.kind(1), CellKind::Markdown);
        let json: Value = serde_json::from_str(&notebook.to_json()).unwrap();
        let cell = &json["cells"][1];
        assert_eq!(
            cell["id"].as_str().unwrap().len(),
            8,
            "nbformat 4.5는 셀 id가 있다"
        );
        assert!(cell.get("outputs").is_none());
        assert_eq!(notebook.move_cell(1, true), 0);
        assert_eq!(notebook.kind(0), CellKind::Markdown);
        assert_eq!(notebook.move_cell(0, true), 0, "맨 위는 그대로");
        notebook.set_kind(0, CellKind::Code);
        assert_eq!(notebook.execution_count(0), None);
        assert!(notebook.outputs(0).is_empty());
        notebook.set_kind(1, CellKind::Raw);
        let json: Value = serde_json::from_str(&notebook.to_json()).unwrap();
        assert!(json["cells"][1].get("execution_count").is_none());
        notebook.remove(0);
        assert_eq!(notebook.len(), 1);
    }

    #[test]
    fn 노트북이_아니면_알린다() {
        assert!(Notebook::parse("{").unwrap_err().contains("JSON"));
        assert!(
            Notebook::parse(r#"{"nbformat": 3, "cells": []}"#)
                .unwrap_err()
                .contains("nbformat 4만")
        );
        assert!(
            Notebook::parse(r#"{"a": 1}"#)
                .unwrap_err()
                .contains("nbformat이 없습니다")
        );
    }

    #[test]
    fn 터미널_색_코드를_뺀다() {
        assert_eq!(
            strip_ansi("\u{1b}[0;31mZeroDivisionError\u{1b}[0m: division by zero"),
            "ZeroDivisionError: division by zero"
        );
    }

    #[test]
    fn 커널_출력을_nbformat_모양으로_더한다() {
        let mut notebook = Notebook::parse(SMALL).unwrap();
        notebook.clear_outputs(0);
        notebook.set_execution_count(0, None);
        let stream = |text: &str| json!({"output_type": "stream", "name": "stdout", "text": text});
        notebook.push_output(
            0,
            stream(
                "안녕
둘",
            ),
        );
        notebook.push_output(
            0,
            stream(
                "째
",
            ),
        );
        notebook.push_output(
            0,
            json!({"output_type": "execute_result", "execution_count": 2, "metadata": {},
                "data": {"text/plain": "a
b", "image/png": "iVBOR
"}}),
        );
        notebook.set_execution_count(0, Some(2));
        let cell = &notebook.doc["cells"][0];
        assert_eq!(
            cell["outputs"][0]["text"],
            json!([
                "안녕
", "둘째
"
            ]),
            "스트림은 이어 붙인다"
        );
        assert_eq!(
            cell["outputs"][1]["data"]["text/plain"],
            json!([
                "a
", "b"
            ])
        );
        assert_eq!(
            cell["outputs"][1]["data"]["image/png"],
            "iVBOR
",
            "그림은 나누지 않는다"
        );
        assert_eq!(notebook.execution_count(0), Some(2));
        assert_eq!(notebook.outputs(0).len(), 2);
    }
}
