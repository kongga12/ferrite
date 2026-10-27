//! SQL을 낱말로 나눈다(`origin`·`refs`가 쓴다). 주석은 건너뛰고, 글자 값과 수는 값 하나로 본다.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Token {
    Word(String),
    /// 큰따옴표로 감싼 식별자(안의 `""`는 `"` 하나).
    Quoted(String),
    Symbol(char),
    /// 글자·수 등 값.
    Value,
}

pub(crate) struct Lexed {
    /// 낱말과 그 바이트 범위.
    pub(crate) tokens: Vec<(Token, Range<usize>)>,
    /// 따옴표·주석이 모두 닫혔다(입력 중인 SQL은 아닐 수 있다).
    pub(crate) complete: bool,
}

pub(crate) fn lex(sql: &str) -> Lexed {
    let mut tokens = Vec::new();
    let mut complete = true;
    let mut chars = sql.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let rest = &sql[at..];
        if c.is_whitespace() {
            continue;
        }
        let token = if rest.starts_with("--") {
            while chars.next_if(|&(_, c)| c != '\n').is_some() {}
            continue;
        } else if rest.starts_with("/*") {
            let end = rest.find("*/").map(|end| at + end + 2);
            complete &= end.is_some();
            let end = end.unwrap_or(sql.len());
            while chars.next_if(|&(i, _)| i < end).is_some() {}
            continue;
        } else if c == '\'' || c == '"' {
            // 같은 따옴표 두 개는 그 따옴표 하나다.
            let mut text = String::new();
            let mut closed = false;
            while let Some((_, next)) = chars.next() {
                if next == c && chars.next_if(|&(_, n)| n == c).is_none() {
                    closed = true;
                    break;
                }
                text.push(next);
            }
            complete &= closed;
            if c == '"' {
                Token::Quoted(text)
            } else {
                Token::Value
            }
        } else if c.is_alphabetic() || c == '_' {
            let mut word = String::from(c);
            while let Some((_, n)) =
                chars.next_if(|&(_, n)| n.is_alphanumeric() || matches!(n, '_' | '$' | '#'))
            {
                word.push(n);
            }
            Token::Word(word)
        } else if c.is_ascii_digit() {
            while chars
                .next_if(|&(_, n)| n.is_alphanumeric() || n == '.')
                .is_some()
            {}
            Token::Value
        } else {
            Token::Symbol(c)
        };
        let end = chars.peek().map_or(sql.len(), |&(i, _)| i);
        tokens.push((token, at..end));
    }
    Lexed { tokens, complete }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 낱말과_범위를_나누고_닫히지_않은_것을_알린다() {
        let lexed = lex("SELECT a.\"B\"\"c\", 'x''y' -- 주석\nFROM t /* 끝");
        let tokens: Vec<(Token, &str)> = lexed
            .tokens
            .iter()
            .map(|(t, r)| {
                (
                    t.clone(),
                    &"SELECT a.\"B\"\"c\", 'x''y' -- 주석\nFROM t /* 끝"[r.clone()],
                )
            })
            .collect();
        assert_eq!(
            tokens,
            [
                (Token::Word("SELECT".into()), "SELECT"),
                (Token::Word("a".into()), "a"),
                (Token::Symbol('.'), "."),
                (Token::Quoted("B\"c".into()), "\"B\"\"c\""),
                (Token::Symbol(','), ","),
                (Token::Value, "'x''y'"),
                (Token::Word("FROM".into()), "FROM"),
                (Token::Word("t".into()), "t"),
            ]
        );
        assert!(!lexed.complete, "닫히지 않은 주석");
        assert!(!lex("WHERE a = 'abc").complete);
        assert!(lex("SELECT 1").complete);
    }
}
