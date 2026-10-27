//! SQL 글을 문장으로 나눈다. 따옴표('…', "…", `…`), PostgreSQL 달러 따옴표($$…$$, $tag$…$tag$),
//! 주석(-- 줄, /* 블록 */) 안의 세미콜론에서는 나누지 않는다.

use std::ops::Range;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Code,
    Quote(u8),
    LineComment,
    BlockComment,
}

/// 비어 있지 않은 문장의 바이트 범위(앞뒤 공백과 끝 세미콜론은 뺀다). 주석뿐인 조각은 건너뛴다.
/// `backslash_escapes`면 따옴표 안의 `\'`를 글자로 본다(MariaDB·MySQL).
pub fn statements(sql: &str, backslash_escapes: bool) -> Vec<Range<usize>> {
    let bytes = sql.as_bytes();
    let mut found = Vec::new();
    let (mut start, mut has_code, mut state) = (0, false, State::Code);
    let mut i = 0;
    let mut finish = |start: usize, end: usize, has_code: bool| {
        let piece = &sql[start..end];
        let trimmed_start = start + (piece.len() - piece.trim_start().len());
        let trimmed_end = start + piece.trim_end().len();
        if has_code && trimmed_start < trimmed_end {
            found.push(trimmed_start..trimmed_end);
        }
    };
    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();
        match state {
            State::Code => match b {
                b';' => {
                    finish(start, i, has_code);
                    (start, has_code) = (i + 1, false);
                }
                b'-' if next == Some(b'-') => {
                    state = State::LineComment;
                    i += 1;
                }
                b'/' if next == Some(b'*') => {
                    state = State::BlockComment;
                    i += 1;
                }
                b'\'' | b'"' | b'`' => {
                    state = State::Quote(b);
                    has_code = true;
                }
                // 달러 따옴표: $$ 또는 $이름$. 여는 표지와 같은 표지가 다시 나올 때까지 건너뛴다.
                b'$' => {
                    has_code = true;
                    let tag_end = sql[i + 1..]
                        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .map(|n| i + 1 + n);
                    if let Some(end) = tag_end.filter(|&e| bytes[e] == b'$') {
                        let tag = &sql[i..=end];
                        match sql[end + 1..].find(tag) {
                            Some(close) => i = end + close + tag.len(),
                            None => i = bytes.len() - 1,
                        }
                    }
                }
                _ if !b.is_ascii_whitespace() => has_code = true,
                _ => {}
            },
            State::Quote(quote) => {
                if backslash_escapes && b == b'\\' {
                    i += 1;
                } else if b == quote {
                    state = State::Code;
                }
            }
            State::LineComment => {
                if b == b'\n' {
                    state = State::Code;
                }
            }
            State::BlockComment => {
                if b == b'*' && next == Some(b'/') {
                    state = State::Code;
                    i += 1;
                }
            }
        }
        i += 1;
    }
    finish(start, bytes.len(), has_code);
    found
}

/// 커서(바이트 위치)가 있는 문장. 문장 사이에 있으면 바로 앞 문장, 그것도 없으면 첫 문장.
pub fn statement_at(sql: &str, cursor: usize, backslash_escapes: bool) -> Option<Range<usize>> {
    let found = statements(sql, backslash_escapes);
    found
        .iter()
        .find(|r| r.start <= cursor && cursor <= r.end)
        .or_else(|| found.iter().rev().find(|r| r.end <= cursor))
        .or_else(|| found.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str, backslash: bool) -> Vec<&str> {
        statements(sql, backslash)
            .into_iter()
            .map(|r| &sql[r])
            .collect()
    }

    #[test]
    fn 세미콜론으로_나누되_따옴표와_주석_안은_나누지_않는다() {
        let sql = "select 'a;b' from t;\n-- 주석; 끝\ninsert into t values (\"x;y\", `z;w`);\n/* 블록; */ update t set a = 1  ;  ";
        assert_eq!(
            texts(sql, false),
            [
                "select 'a;b' from t",
                "-- 주석; 끝\ninsert into t values (\"x;y\", `z;w`)",
                "/* 블록; */ update t set a = 1",
            ]
        );
        assert_eq!(
            texts("select 'it''s; ok'; select 2", false),
            ["select 'it''s; ok'", "select 2"]
        );
        assert!(
            texts("  -- 주석만\n ;  ", false).is_empty(),
            "주석뿐인 조각은 건너뛴다"
        );
    }

    #[test]
    fn 달러_따옴표와_역슬래시를_다룬다() {
        let function = "create function f() returns int as $body$ begin return 1; end; $body$ language plpgsql";
        assert_eq!(
            texts(&format!("{function}; select f()"), false),
            [function, "select f()"]
        );
        assert_eq!(
            texts("select $$a;b$$; select 1", false),
            ["select $$a;b$$", "select 1"]
        );
        assert_eq!(
            texts("select 'it\\'s;'; select 1", true),
            ["select 'it\\'s;'", "select 1"]
        );
        assert_eq!(
            texts("select $1 from t; select 2", false),
            ["select $1 from t", "select 2"]
        );
    }

    #[test]
    fn 커서가_있는_문장을_고른다() {
        let sql = "select 1;\n\nselect 2;\nselect 3";
        let at = |cursor| statement_at(sql, cursor, false).map(|r| &sql[r]);
        assert_eq!(at(3), Some("select 1"));
        assert_eq!(at(10), Some("select 1"), "문장 사이면 앞 문장");
        assert_eq!(at(sql.find("2").unwrap()), Some("select 2"));
        assert_eq!(at(sql.len()), Some("select 3"));
        assert_eq!(statement_at("  ", 0, false), None);
    }
}
