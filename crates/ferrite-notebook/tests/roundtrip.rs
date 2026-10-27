//! nbformat(Jupyter)이 쓴 모양의 노트북(`fixtures/`, Python `json.dumps(sort_keys=True, indent=1,
//! ensure_ascii=False)`로 만들었다)을 읽고 쓰면 바이트까지 같고, 셀 하나를 고치면 그 셀만 달라진다.

use ferrite_notebook::{CellKind, Notebook, Output, strip_ansi};

const SAMPLE: &str = include_str!("fixtures/sample.ipynb");
const EDITED: &str = include_str!("fixtures/sample_edited.ipynb");

#[test]
fn jupyter가_쓴_노트북을_바이트까지_그대로_쓴다() {
    let notebook = Notebook::parse(SAMPLE).unwrap();
    assert_eq!(notebook.to_json(), SAMPLE);
    assert_eq!(notebook.len(), 5);
    let kinds: Vec<CellKind> = (0..notebook.len()).map(|i| notebook.kind(i)).collect();
    assert_eq!(
        kinds,
        [
            CellKind::Markdown,
            CellKind::Code,
            CellKind::Code,
            CellKind::Code,
            CellKind::Raw
        ]
    );
    assert_eq!(notebook.source(0), "# 한글 제목\n설명 *강조*");
}

#[test]
fn 셀_하나를_고치면_그_셀만_달라진다() {
    let mut notebook = Notebook::parse(SAMPLE).unwrap();
    notebook.set_source(1, "print('바뀜')\n");
    assert_eq!(notebook.to_json(), EDITED);
}

#[test]
fn 저장된_출력을_읽는다() {
    let notebook = Notebook::parse(SAMPLE).unwrap();
    assert_eq!(
        notebook.outputs(1),
        [
            Output::Stream {
                name: "stdout".into(),
                text: "안녕\n둘째 줄\n".into()
            },
            Output::Data {
                data: vec![
                    ("text/html".into(), "<b>굵게</b>\n".into()),
                    ("text/plain".into(), "'결과'".into())
                ],
                execution_count: Some(1)
            }
        ]
    );
    let Output::Data { data, .. } = &notebook.outputs(2)[0] else {
        panic!("그림 출력");
    };
    assert!(data[0].0 == "image/png" && data[0].1.starts_with("iVBORw0KGgo"));
    let Output::Error {
        ename, traceback, ..
    } = &notebook.outputs(3)[0]
    else {
        panic!("오류 출력");
    };
    assert_eq!(ename, "ZeroDivisionError");
    assert_eq!(
        strip_ansi(&traceback[1]),
        "ZeroDivisionError: division by zero"
    );
    assert!(notebook.outputs(0).is_empty(), "마크다운 셀은 출력이 없다");
}
