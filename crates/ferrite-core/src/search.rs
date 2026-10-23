//! 찾기·바꾸기. 위치는 버퍼와 같은 문자(char) 인덱스다.

use std::ops::Range;
use std::time::Instant;

use regex::{Regex, RegexBuilder};
use ropey::Rope;

use crate::buffer::Buffer;

/// 한 번에 찾는 최대 개수.
// ponytail: 전부 기억해 화면에 칠하는 방식의 상한. 넘으면 보이는 범위만 다시 찾는 방식으로.
pub const MAX_MATCHES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchQuery {
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
}

impl SearchQuery {
    /// 정규식으로 컴파일한다. 일반 검색어는 특수문자를 이스케이프한다.
    pub fn compile(&self) -> Result<Regex, String> {
        let pattern = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| e.to_string())
    }
}

/// 모든 일치 위치(빈 일치는 제외). 최대 `MAX_MATCHES`개.
pub fn find_all(rope: &Rope, re: &Regex) -> Vec<Range<usize>> {
    let text = rope.to_string();
    re.find_iter(&text)
        .filter(|m| !m.is_empty())
        .take(MAX_MATCHES)
        .map(|m| rope.byte_to_char(m.start())..rope.byte_to_char(m.end()))
        .collect()
}

/// `from` 뒤(같으면 포함)의 첫 일치. 없으면 처음으로 돌아간다.
pub fn next_match(matches: &[Range<usize>], from: usize) -> Option<Range<usize>> {
    let i = matches.partition_point(|m| m.start < from);
    matches.get(i).or_else(|| matches.first()).cloned()
}

/// `before` 앞의 마지막 일치. 없으면 끝으로 돌아간다.
pub fn prev_match(matches: &[Range<usize>], before: usize) -> Option<Range<usize>> {
    let i = matches.partition_point(|m| m.start < before);
    i.checked_sub(1)
        .and_then(|i| matches.get(i))
        .or_else(|| matches.last())
        .cloned()
}

/// 바꿀 문자열. 정규식 검색이면 `$1`, `${name}` 같은 그룹 참조를 펼친다.
fn expand(
    re: &Regex,
    text: &str,
    byte_start: usize,
    query: &SearchQuery,
    replacement: &str,
) -> Option<String> {
    if !query.regex {
        return Some(replacement.to_owned());
    }
    let caps = re.captures_at(text, byte_start)?;
    let mut out = String::new();
    caps.expand(replacement, &mut out);
    Some(out)
}

/// 선택 영역이 일치 항목과 정확히 같으면 바꾼다. 바꿨으면 true.
pub fn replace_selection(
    buffer: &mut Buffer,
    query: &SearchQuery,
    replacement: &str,
    now: Instant,
) -> Result<bool, String> {
    let re = query.compile()?;
    let range = buffer.selection().range();
    if range.is_empty() {
        return Ok(false);
    }
    let text = buffer.text();
    let byte_start = buffer.rope().char_to_byte(range.start);
    let byte_end = buffer.rope().char_to_byte(range.end);
    let exact = re
        .find_at(&text, byte_start)
        .is_some_and(|m| m.start() == byte_start && m.end() == byte_end);
    if !exact {
        return Ok(false);
    }
    let Some(new_text) = expand(&re, &text, byte_start, query, replacement) else {
        return Ok(false);
    };
    buffer.insert(&new_text, now);
    Ok(true)
}

/// 모든 일치 항목을 하나의 실행 취소 단위로 바꾼다. 바꾼 개수를 돌려준다.
pub fn replace_all(
    buffer: &mut Buffer,
    query: &SearchQuery,
    replacement: &str,
    now: Instant,
) -> Result<usize, String> {
    let re = query.compile()?;
    let text = buffer.text();
    let rope = buffer.rope();
    let edits: Vec<(Range<usize>, String)> = re
        .captures_iter(&text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            if m.is_empty() {
                return None;
            }
            let new_text = if query.regex {
                let mut out = String::new();
                caps.expand(replacement, &mut out);
                out
            } else {
                replacement.to_owned()
            };
            Some((
                rope.byte_to_char(m.start())..rope.byte_to_char(m.end()),
                new_text,
            ))
        })
        .collect();
    let count = edits.len();
    buffer.replace_ranges(edits, now);
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Selection;

    fn query(pattern: &str, regex: bool, case_sensitive: bool) -> SearchQuery {
        SearchQuery {
            pattern: pattern.into(),
            regex,
            case_sensitive,
        }
    }

    #[test]
    fn 일반_검색은_특수문자를_그대로_찾고_대소문자를_무시한다() {
        let rope = Rope::from_str("a.b A.B axb 한글.b");
        let re = query("a.b", false, false).compile().unwrap();
        assert_eq!(find_all(&rope, &re), vec![0..3, 4..7]);
        let re = query("글.b", false, true).compile().unwrap();
        assert_eq!(find_all(&rope, &re), vec![13..16], "문자 인덱스로 돌려준다");
    }

    #[test]
    fn 다음_이전_일치는_끝에서_되돌아간다() {
        let matches = vec![2..3, 5..6, 9..10];
        assert_eq!(next_match(&matches, 4), Some(5..6));
        assert_eq!(next_match(&matches, 10), Some(2..3));
        assert_eq!(prev_match(&matches, 5), Some(2..3));
        assert_eq!(prev_match(&matches, 2), Some(9..10));
        assert_eq!(next_match(&[], 0), None);
    }

    #[test]
    fn 정규식_그룹_참조로_모두_바꾸고_한번에_실행취소() {
        let mut buffer = Buffer::new("get_name get_id");
        let n = replace_all(
            &mut buffer,
            &query(r"get_(\w+)", true, true),
            "${1}Get",
            Instant::now(),
        )
        .unwrap();
        assert_eq!(n, 2);
        assert_eq!(buffer.text(), "nameGet idGet");
        buffer.undo();
        assert_eq!(buffer.text(), "get_name get_id");
    }

    #[test]
    fn 선택이_일치_항목일_때만_바꾼다() {
        let now = Instant::now();
        let q = query("foo", false, true);
        let mut buffer = Buffer::new("foo bar foo");
        buffer.set_selection(Selection { anchor: 4, head: 7 });
        assert!(!replace_selection(&mut buffer, &q, "X", now).unwrap());
        buffer.set_selection(Selection {
            anchor: 8,
            head: 11,
        });
        assert!(replace_selection(&mut buffer, &q, "$X", now).unwrap());
        assert_eq!(buffer.text(), "foo bar $X", "일반 검색은 $를 그대로 쓴다");
    }

    #[test]
    fn 잘못된_정규식은_오류() {
        assert!(query("(", true, true).compile().is_err());
        assert!(query("(", false, true).compile().is_ok());
    }
}
