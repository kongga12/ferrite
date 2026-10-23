//! 서버 진단(`Diagnostic`)을 에디터가 쓰는 문자 범위로 바꾼다.

use std::ops::Range;

use lsp_types::{Diagnostic, DiagnosticSeverity};
use ropey::Rope;

use crate::position::{PositionEncoding, from_lsp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "오류",
            Severity::Warning => "경고",
            Severity::Info => "정보",
            Severity::Hint => "힌트",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// 문서의 문자 범위. 빈 범위면 그 위치 한 곳을 가리킨다.
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    /// 진단을 낸 도구(rustc, clippy 등). 없으면 빈 문자열.
    pub source: String,
    /// 서버가 보낸 진단 원본. 코드 액션 요청에 그대로 돌려줘야 서버가 빠른 수정을 찾는다.
    pub raw: Diagnostic,
}

/// 위치 순으로 정렬해 돌려준다(에디터가 보이는 줄만 빠르게 찾을 수 있게).
pub fn to_problems(
    rope: &Rope,
    diagnostics: &[Diagnostic],
    encoding: PositionEncoding,
) -> Vec<Problem> {
    let mut problems: Vec<Problem> = diagnostics
        .iter()
        .map(|d| {
            let start = from_lsp(rope, d.range.start, encoding);
            let end = from_lsp(rope, d.range.end, encoding).max(start);
            Problem {
                range: start..end,
                // 심각도가 없으면 오류로 본다(LSP 명세: 클라이언트가 정함).
                severity: match d.severity {
                    Some(DiagnosticSeverity::WARNING) => Severity::Warning,
                    Some(DiagnosticSeverity::INFORMATION) => Severity::Info,
                    Some(DiagnosticSeverity::HINT) => Severity::Hint,
                    _ => Severity::Error,
                },
                message: d.message.clone(),
                source: d.source.clone().unwrap_or_default(),
                raw: d.clone(),
            }
        })
        .collect();
    problems.sort_by_key(|p| (p.range.start, p.severity));
    problems
}

#[cfg(test)]
mod tests {
    use lsp_types::{Position, Range as LspRange};

    use super::*;

    #[test]
    fn 서버_진단을_문자_범위로_바꾸고_위치순_정렬() {
        let rope = Rope::from_str("let 가 = ;\nfn x() {}\n");
        let diag = |line, start, end, severity| Diagnostic {
            range: LspRange::new(Position::new(line, start), Position::new(line, end)),
            severity,
            message: "m".into(),
            ..Default::default()
        };
        let problems = to_problems(
            &rope,
            &[
                diag(1, 3, 4, Some(DiagnosticSeverity::WARNING)),
                // UTF-8 기준 열 8 = '=' 뒤 (가는 3바이트)
                diag(0, 10, 10, None),
            ],
            PositionEncoding::Utf8,
        );
        assert_eq!(problems[0].range, 8..8);
        assert_eq!(problems[0].severity, Severity::Error);
        assert_eq!(problems[1].range, 13..14, "둘째 줄 fn x의 x");
        assert_eq!(problems[1].severity, Severity::Warning);
    }
}
