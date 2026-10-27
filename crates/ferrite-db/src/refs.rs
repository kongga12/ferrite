//! 매퍼 SQL에서 표·열 이름을 읽는다(자동 완성과 검증). 실행하는 SQL이 아니라 사람이 쓰는 중인 SQL이라
//! 덜 끝났어도 읽을 수 있는 만큼 읽는다. 확실하지 않은 것은 빼서(표나 열로 보지 않아) 잘못된 경고를
//! 줄인다.

use std::ops::Range;

use crate::lex::{Token, lex};

/// 표 자리(FROM·JOIN·UPDATE·INTO·USING 뒤)의 표 하나.
#[derive(Clone, Debug, PartialEq)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
    pub alias: Option<String>,
    /// 표 이름의 바이트 범위.
    pub range: Range<usize>,
}

/// 표를 가리켜 쓴 열: `별칭.열`, INSERT의 열 목록, UPDATE SET의 열.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnRef {
    /// 별칭이나 표 이름.
    pub qualifier: String,
    pub name: String,
    pub range: Range<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct References {
    pub tables: Vec<TableRef>,
    pub columns: Vec<ColumnRef>,
    /// WITH로 만든 이름(DB의 표가 아니다).
    pub ctes: Vec<String>,
}

impl References {
    /// 별칭이나 (별칭 없는) 표 이름이 가리키는 표(대소문자 무시).
    pub fn resolve(&self, qualifier: &str) -> Option<&TableRef> {
        let same = |name: &str| name.eq_ignore_ascii_case(qualifier);
        self.tables
            .iter()
            .find(|t| t.alias.as_deref().is_some_and(same))
            .or_else(|| self.tables.iter().find(|t| same(&t.name)))
    }
}

/// 자동 완성할 것.
#[derive(Clone, Debug, PartialEq)]
pub enum Context {
    /// 표 이름.
    Table,
    /// 열 이름. `qualifier`가 있으면 그 별칭·표의 열, 없으면 문장의 모든 표의 열.
    Column { qualifier: Option<String> },
}

/// 표 자리를 여는 낱말.
const TABLE_KEYWORDS: [&str; 5] = ["FROM", "JOIN", "UPDATE", "INTO", "USING"];

/// 이름(표·별칭)이 될 수 없는 낱말.
const RESERVED: [&str; 52] = [
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS",
    "NULL",
    "LIKE",
    "BETWEEN",
    "EXISTS",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "AS",
    "ON",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "FULL",
    "CROSS",
    "NATURAL",
    "OUTER",
    "SET",
    "VALUES",
    "INTO",
    "UPDATE",
    "DELETE",
    "INSERT",
    "GROUP",
    "ORDER",
    "BY",
    "HAVING",
    "UNION",
    "INTERSECT",
    "MINUS",
    "EXCEPT",
    "CONNECT",
    "START",
    "USING",
    "FOR",
    "FETCH",
    "OFFSET",
    "LIMIT",
    "WITH",
    "RETURNING",
    "PARTITION",
    "DISTINCT",
];

fn is_reserved(word: &str) -> bool {
    RESERVED.iter().any(|k| k.eq_ignore_ascii_case(word))
}

fn word(token: &Token, word: &str) -> bool {
    matches!(token, Token::Word(w) if w.eq_ignore_ascii_case(word))
}

fn name(token: &Token) -> Option<String> {
    match token {
        Token::Word(w) if !is_reserved(w) => Some(w.clone()),
        Token::Quoted(q) => Some(q.clone()),
        _ => None,
    }
}

/// 괄호 하나: 부분 조회(`(SELECT`)인지, 함수 인자·목록인지.
#[derive(Clone, Copy, PartialEq)]
enum Paren {
    Query,
    Other,
}

/// 낱말마다 그 낱말이 들어 있는 괄호(가장 안쪽). 괄호 밖이면 None.
fn parens(tokens: &[(Token, Range<usize>)]) -> Vec<Option<Paren>> {
    let mut stack: Vec<Paren> = Vec::new();
    let mut inside = Vec::with_capacity(tokens.len());
    for (i, (token, _)) in tokens.iter().enumerate() {
        inside.push(stack.last().copied());
        match token {
            Token::Symbol('(') => {
                let query = tokens
                    .get(i + 1)
                    .is_some_and(|(t, _)| word(t, "SELECT") || word(t, "WITH"));
                stack.push(if query { Paren::Query } else { Paren::Other });
            }
            Token::Symbol(')') => {
                stack.pop();
            }
            _ => {}
        }
    }
    inside
}

/// `i`부터 표 하나(`[스키마.]표 [[AS] 별칭]`). 다음 낱말 자리도 돌려준다. 표 함수(`TABLE(…)`)나
/// DB 링크(`@`)면 None. `columns`면 표 이름 뒤 괄호를 열 목록으로 본다(INSERT INTO).
fn table_at(
    tokens: &[(Token, Range<usize>)],
    i: usize,
    columns: bool,
) -> Option<(TableRef, usize)> {
    let at = |j: usize| tokens.get(j).map(|(t, _)| t);
    let first = name(at(i)?)?;
    let (schema, table, range, mut j) = if at(i + 1) == Some(&Token::Symbol('.')) {
        (
            Some(first),
            name(at(i + 2)?)?,
            tokens[i + 2].1.clone(),
            i + 3,
        )
    } else {
        (None, first, tokens[i].1.clone(), i + 1)
    };
    match at(j) {
        Some(Token::Symbol('@')) => return None,
        Some(Token::Symbol('(')) if !columns => return None,
        _ => {}
    }
    let mut alias = None;
    if at(j).is_some_and(|t| word(t, "AS")) {
        alias = at(j + 1).and_then(name);
        j += 2;
    } else if let Some(a) = at(j).and_then(name) {
        alias = Some(a);
        j += 1;
    }
    Some((
        TableRef {
            schema,
            name: table,
            alias,
            range,
        },
        j,
    ))
}

/// 문장의 표·열 참조.
pub fn references(sql: &str) -> References {
    let tokens = lex(sql).tokens;
    let inside = parens(&tokens);
    let mut refs = References::default();
    // 표 참조가 쓴 낱말(스키마.표)은 열 참조로 읽지 않는다.
    let mut used = vec![false; tokens.len()];
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i].0;
        // WITH 이름 AS (…), 이름 AS (…)
        if (word(token, "WITH") || token == &Token::Symbol(','))
            && let (Some(cte), Some(next)) = (
                tokens.get(i + 1).and_then(|(t, _)| name(t)),
                tokens.get(i + 2),
            )
            && word(&next.0, "AS")
            && tokens
                .get(i + 3)
                .is_some_and(|(t, _)| *t == Token::Symbol('('))
        {
            refs.ctes.push(cte);
        }
        let opens =
            TABLE_KEYWORDS.iter().any(|k| word(token, k)) && inside[i] != Some(Paren::Other);
        if !opens {
            i += 1;
            continue;
        }
        let into = word(token, "INTO");
        let from = word(token, "FROM");
        let mut j = i + 1;
        while let Some((table, next)) = table_at(&tokens, j, into) {
            used[j..next].iter_mut().for_each(|u| *u = true);
            let qualifier = table.alias.clone().unwrap_or_else(|| table.name.clone());
            refs.tables.push(table);
            // INSERT INTO 표 (열, 열, …)
            if into
                && tokens
                    .get(next)
                    .is_some_and(|(t, _)| *t == Token::Symbol('('))
            {
                let mut k = next + 1;
                while let Some((t, range)) = tokens.get(k) {
                    match t {
                        Token::Symbol(')') => break,
                        Token::Symbol(',') => {}
                        _ => match name(t) {
                            Some(column) => {
                                used[k] = true;
                                refs.columns.push(ColumnRef {
                                    qualifier: qualifier.clone(),
                                    name: column,
                                    range: range.clone(),
                                });
                            }
                            None => break,
                        },
                    }
                    k += 1;
                }
            }
            // UPDATE 표 SET 열 = …, 열 = …
            if word(token, "UPDATE") && tokens.get(next).is_some_and(|(t, _)| word(t, "SET")) {
                let depth = inside[next];
                let mut k = next + 1;
                while let Some((t, range)) = tokens.get(k) {
                    if inside[k] == depth {
                        if word(t, "WHERE") || word(t, "RETURNING") {
                            break;
                        }
                        let starts = matches!(&tokens[k - 1].0, Token::Symbol(','))
                            || word(&tokens[k - 1].0, "SET");
                        if starts
                            && let Some(column) = name(t)
                            && tokens
                                .get(k + 1)
                                .is_some_and(|(t, _)| *t == Token::Symbol('='))
                        {
                            used[k] = true;
                            refs.columns.push(ColumnRef {
                                qualifier: qualifier.clone(),
                                name: column,
                                range: range.clone(),
                            });
                        }
                    }
                    k += 1;
                }
            }
            // FROM 가, 나 b, …
            if from
                && tokens
                    .get(next)
                    .is_some_and(|(t, _)| *t == Token::Symbol(','))
            {
                j = next + 1;
                continue;
            }
            break;
        }
        i = j.max(i + 1);
    }
    // 별칭.열(스키마.표.열이면 뒤의 둘)
    for k in 0..tokens.len().saturating_sub(2) {
        if used[k] || tokens[k + 1].0 != Token::Symbol('.') {
            continue;
        }
        if tokens
            .get(k + 3)
            .is_some_and(|(t, _)| *t == Token::Symbol('.'))
        {
            continue;
        }
        let qualifier = match &tokens[k].0 {
            Token::Word(w) => w.clone(),
            Token::Quoted(q) => q.clone(),
            _ => continue,
        };
        let column = match &tokens[k + 2].0 {
            Token::Word(w) => w.clone(),
            Token::Quoted(q) => q.clone(),
            _ => continue,
        };
        // 함수·프로시저 부르기(`패키지.함수(`)는 열이 아니다.
        if tokens
            .get(k + 3)
            .is_some_and(|(t, _)| *t == Token::Symbol('('))
        {
            continue;
        }
        refs.columns.push(ColumnRef {
            qualifier,
            name: column,
            range: tokens[k + 2].1.clone(),
        });
    }
    refs
}

/// `cursor`(바이트) 자리에서 자동 완성할 것과, 커서 앞에 친 낱말의 범위.
pub fn context(sql: &str, cursor: usize) -> Option<(Context, Range<usize>)> {
    let before = sql.get(..cursor)?;
    let typed = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || matches!(c, '_' | '$' | '#'))
        .last()
        .map_or(cursor, |(i, _)| i);
    let lexed = lex(&before[..typed]);
    // 글자 값·주석 안이다.
    if !lexed.complete {
        return None;
    }
    let tokens = lexed.tokens;
    let inside = parens(&tokens);
    let last = tokens.len().checked_sub(1)?;
    let column = |qualifier| Some((Context::Column { qualifier }, typed..cursor));
    match &tokens[last].0 {
        Token::Symbol('.') => {
            let qualifier = tokens
                .get(last.checked_sub(1)?)
                .and_then(|(t, _)| match t {
                    Token::Word(w) => Some(w.clone()),
                    Token::Quoted(q) => Some(q.clone()),
                    _ => None,
                })?;
            column(Some(qualifier))
        }
        Token::Word(w)
            if TABLE_KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(w))
                && inside[last] != Some(Paren::Other) =>
        {
            Some((Context::Table, typed..cursor))
        }
        Token::Symbol(',') => {
            // 같은 괄호 안에서 마지막으로 연 절이 FROM이면 표 목록이다.
            let depth = inside[last];
            let clause = (0..last).rev().find_map(|k| match &tokens[k].0 {
                Token::Word(w)
                    if inside[k] == depth
                        && [
                            "FROM", "SELECT", "WHERE", "SET", "BY", "VALUES", "ON", "HAVING",
                        ]
                        .iter()
                        .any(|c| c.eq_ignore_ascii_case(w)) =>
                {
                    Some(w.to_ascii_uppercase())
                }
                _ => None,
            });
            if clause.as_deref() == Some("FROM") {
                Some((Context::Table, typed..cursor))
            } else {
                column(None)
            }
        }
        Token::Word(w)
            if [
                "SELECT", "WHERE", "AND", "OR", "ON", "SET", "BY", "HAVING", "WHEN", "THEN",
                "ELSE", "NOT", "DISTINCT",
            ]
            .iter()
            .any(|k| k.eq_ignore_ascii_case(w)) =>
        {
            column(None)
        }
        Token::Symbol('(' | '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '|') => column(None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(sql: &str) -> Vec<(Option<String>, String, Option<String>)> {
        references(sql)
            .tables
            .into_iter()
            .map(|t| (t.schema, t.name, t.alias))
            .collect()
    }

    fn t(
        schema: Option<&str>,
        name: &str,
        alias: Option<&str>,
    ) -> (Option<String>, String, Option<String>) {
        (
            schema.map(str::to_owned),
            name.to_owned(),
            alias.map(str::to_owned),
        )
    }

    #[test]
    fn 표_자리의_표와_별칭을_읽는다() {
        assert_eq!(
            tables(
                "SELECT a.id, b.name FROM hr.emp a, dept b \
                 LEFT OUTER JOIN loc AS l ON l.id = b.loc \
                 WHERE a.id IN (SELECT id FROM vip) AND EXTRACT(YEAR FROM a.hired) = 2024"
            ),
            [
                t(Some("hr"), "emp", Some("a")),
                t(None, "dept", Some("b")),
                t(None, "loc", Some("l")),
                t(None, "vip", None),
            ]
        );
        // 표 함수, DB 링크, 부분 조회, 치환(`${}`를 가린 `?`)은 표로 보지 않는다.
        assert_eq!(
            tables(
                "SELECT * FROM TABLE(DBMS_XPLAN.DISPLAY) x, remote@link r, (SELECT 1 FROM DUAL) d, ?"
            ),
            [t(None, "DUAL", None)]
        );
        // WITH 이름은 따로 모은다.
        let refs = references("WITH recent AS (SELECT * FROM orders) SELECT * FROM recent r");
        assert_eq!(refs.ctes, ["recent"]);
        assert_eq!(refs.tables.len(), 2);
    }

    #[test]
    fn 별칭_열과_insert_update의_열을_읽는다() {
        let refs = references(
            "SELECT e.name, \"E\".\"Id\", hr.emp.dept, pkg.func(1), seq.NEXTVAL FROM hr.emp e",
        );
        let columns: Vec<(&str, &str)> = refs
            .columns
            .iter()
            .map(|c| (c.qualifier.as_str(), c.name.as_str()))
            .collect();
        assert_eq!(
            columns,
            [
                ("e", "name"),
                ("E", "Id"),
                ("emp", "dept"),
                ("seq", "NEXTVAL")
            ]
        );
        assert_eq!(refs.resolve("E").unwrap().name, "emp");
        assert!(refs.resolve("seq").is_none());

        let refs = references("INSERT INTO emp (id, name) VALUES (?, ?)");
        let names: Vec<&str> = refs.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "name"]);
        let refs = references(
            "UPDATE emp SET name = ?, dept = NVL(?, 'x'), memo = ? WHERE id = ? AND e.x = 1",
        );
        let names: Vec<&str> = refs.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["name", "dept", "memo", "x"]);
        assert_eq!(refs.columns[0].qualifier, "emp");
    }

    #[test]
    fn 커서_자리의_문맥() {
        let at = |sql: &str| {
            let cursor = sql.find('|').unwrap();
            let sql = sql.replace('|', "");
            context(&sql, cursor).map(|(c, range)| (c, sql[range].to_owned()))
        };
        let column = |q: Option<&str>| Context::Column {
            qualifier: q.map(str::to_owned),
        };
        assert_eq!(at("SELECT * FROM em|"), Some((Context::Table, "em".into())));
        assert_eq!(
            at("SELECT * FROM emp e JOIN |"),
            Some((Context::Table, String::new()))
        );
        assert_eq!(
            at("SELECT * FROM emp e, d|"),
            Some((Context::Table, "d".into()))
        );
        assert_eq!(
            at("SELECT e.na| FROM emp e"),
            Some((column(Some("e")), "na".into()))
        );
        assert_eq!(
            at("SELECT id, na| FROM emp"),
            Some((column(None), "na".into()))
        );
        assert_eq!(
            at("SELECT * FROM emp WHERE |"),
            Some((column(None), String::new()))
        );
        assert_eq!(
            at("INSERT INTO emp (|"),
            Some((column(None), String::new()))
        );
        // 별칭을 치는 자리, 글자 값 안, 함수 인자의 FROM 뒤는 아니다.
        assert_eq!(at("SELECT * FROM emp e|"), None);
        assert_eq!(at("SELECT * FROM emp WHERE name = 'ab|"), None);
        assert_eq!(at("SELECT EXTRACT(YEAR FROM |"), None);
    }
}
