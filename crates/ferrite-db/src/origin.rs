//! JDBC 결과가 어느 표·열에서 왔는지 SQL에서 찾는다. Oracle 드라이버는 결과 열의 표를 알려 주지
//! 않고, CUBRID는 별칭을 열 이름으로 알려 준다(`v AS n`이 n 열로 보인다). 그래서 한 표만 읽는 단순한
//! SELECT만 받아들이고, 열 이름을 그대로 쓴 항목만 그 열로 본다. 모르는 모양은 고칠 수 없다고 본다
//! (엉뚱한 열·행을 고치는 것보다 읽기 전용이 낫다).

use crate::lex::{Token, lex};

/// 식별자. 따옴표로 감쌌으면 대소문자를 그대로 둔다.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ident {
    pub(crate) text: String,
    pub(crate) quoted: bool,
}

#[derive(Debug, PartialEq)]
enum Item {
    /// `*` 또는 `별칭.*`
    Star(Option<Ident>),
    /// `열` 또는 `별칭.열`
    Column(Option<Ident>, Ident),
    /// 식, 별칭을 붙인 열 등(고칠 수 없다).
    Other,
}

/// 한 표만 읽는 SELECT.
#[derive(Debug, PartialEq)]
pub(crate) struct Select {
    pub(crate) schema: Option<Ident>,
    pub(crate) table: Ident,
    alias: Option<Ident>,
    items: Vec<Item>,
}

/// 따옴표 없는(또는 있는) 식별자를 DB가 두는 대소문자로 바꾸는 법.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fold {
    Upper,
    Lower,
    None,
}

impl Fold {
    fn apply(self, text: &str) -> String {
        match self {
            Fold::Upper => text.to_uppercase(),
            Fold::Lower => text.to_lowercase(),
            Fold::None => text.to_owned(),
        }
    }
}

/// 표 이름 뒤에 와도 되는 낱말.
const TAIL: [&str; 6] = ["WHERE", "ORDER", "FOR", "FETCH", "OFFSET", "LIMIT"];

/// 한 표 조회가 아니게 만드는 낱말(괄호 밖에서), 그리고 별칭이 될 수 없는 낱말.
const KEYWORDS: [&str; 33] = [
    "WHERE",
    "ORDER",
    "GROUP",
    "HAVING",
    "UNION",
    "INTERSECT",
    "MINUS",
    "EXCEPT",
    "CONNECT",
    "START",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "FULL",
    "CROSS",
    "NATURAL",
    "OUTER",
    "FOR",
    "FETCH",
    "OFFSET",
    "LIMIT",
    "PIVOT",
    "UNPIVOT",
    "MODEL",
    "SAMPLE",
    "PARTITION",
    "OF",
    "WINDOW",
    "USING",
    "ON",
    "AS",
    "FROM",
];

fn is_keyword(word: &str) -> bool {
    KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(word))
}

/// 따옴표·주석이 다 닫힌 SQL의 낱말들(덜 친 SQL은 받지 않는다).
fn tokenize(sql: &str) -> Option<Vec<Token>> {
    let lexed = lex(sql);
    lexed
        .complete
        .then(|| lexed.tokens.into_iter().map(|(token, _)| token).collect())
}

fn ident(token: &Token) -> Option<Ident> {
    match token {
        Token::Word(word) if !is_keyword(word) => Some(Ident {
            text: word.clone(),
            quoted: false,
        }),
        Token::Quoted(text) => Some(Ident {
            text: text.clone(),
            quoted: true,
        }),
        _ => None,
    }
}

fn item(tokens: &[Token]) -> Item {
    let dot = |t: &Token| *t == Token::Symbol('.');
    match tokens {
        [Token::Symbol('*')] => Item::Star(None),
        [q, d, Token::Symbol('*')] if dot(d) => {
            ident(q).map_or(Item::Other, |q| Item::Star(Some(q)))
        }
        [c] => ident(c).map_or(Item::Other, |c| Item::Column(None, c)),
        [q, d, c] if dot(d) => match (ident(q), ident(c)) {
            (Some(q), Some(c)) => Item::Column(Some(q), c),
            _ => Item::Other,
        },
        // 스키마.표.열
        [s, d1, q, d2, c] if dot(d1) && dot(d2) && ident(s).is_some() => {
            match (ident(q), ident(c)) {
                (Some(q), Some(c)) => Item::Column(Some(q), c),
                _ => Item::Other,
            }
        }
        _ => Item::Other,
    }
}

fn word_is(token: Option<&Token>, word: &str) -> bool {
    matches!(token, Some(Token::Word(w)) if w.eq_ignore_ascii_case(word))
}

/// 한 표만 읽는 SELECT면 그 표와 항목. 아니면(조인, 집계, 합치기, 부분 조회 위의 조회 등) `None`.
pub(crate) fn parse(sql: &str) -> Option<Select> {
    let tokens = tokenize(sql)?;
    let mut rest = tokens.iter().peekable();
    if !word_is(rest.next(), "SELECT") {
        return None;
    }
    if word_is(rest.peek().copied(), "ALL") {
        rest.next();
    }
    let mut items = Vec::new();
    let (mut current, mut depth) = (Vec::new(), 0usize);
    loop {
        let token = rest.next()?;
        match token {
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth = depth.checked_sub(1)?,
            Token::Symbol(',') if depth == 0 => {
                items.push(item(&current));
                current.clear();
                continue;
            }
            Token::Word(w) if depth == 0 && w.eq_ignore_ascii_case("FROM") => {
                items.push(item(&current));
                break;
            }
            // DISTINCT·UNIQUE는 여러 행을 하나로 합친다.
            Token::Word(w)
                if current.is_empty()
                    && items.is_empty()
                    && (w.eq_ignore_ascii_case("DISTINCT") || w.eq_ignore_ascii_case("UNIQUE")) =>
            {
                return None;
            }
            _ => {}
        }
        current.push(token.clone());
    }
    let first = ident(rest.next()?)?;
    let (schema, table) = if rest.next_if(|t| **t == Token::Symbol('.')).is_some() {
        (Some(first), ident(rest.next()?)?)
    } else {
        (None, first)
    };
    let alias = if word_is(rest.peek().copied(), "AS") {
        rest.next();
        Some(ident(rest.next()?)?)
    } else {
        rest.next_if(|t| ident(t).is_some()).and_then(ident)
    };
    let tail: Vec<&Token> = rest.collect();
    match tail.first() {
        None | Some(Token::Symbol(';')) => {}
        Some(Token::Word(w)) if TAIL.iter().any(|k| k.eq_ignore_ascii_case(w)) => {}
        _ => return None,
    }
    // 괄호 밖에 표를 더하거나 행을 합치는 낱말이 있으면 한 표 조회가 아니다(WHERE 안 부분 조회는 괜찮다).
    // 표를 쉼표로 더하는 것은 위에서 막았다(여기의 쉼표는 ORDER BY 목록이다).
    let mut depth = 0usize;
    for (index, token) in tail.iter().enumerate() {
        match token {
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth = depth.checked_sub(1)?,
            Token::Word(w)
                if depth == 0
                    && index > 0
                    && is_keyword(w)
                    && !["ORDER", "FOR", "FETCH", "OFFSET", "LIMIT"]
                        .iter()
                        .any(|k| k.eq_ignore_ascii_case(w)) =>
            {
                return None;
            }
            _ => {}
        }
    }
    Some(Select {
        schema,
        table,
        alias,
        items,
    })
}

impl Select {
    /// 결과 열마다 표의 열 이름(DB가 둔 대소문자). 열 이름을 그대로 쓴 항목만 그 열이다.
    /// `labels`는 드라이버가 준 결과 열 이름, `table`은 표의 열 이름들. `fold`는 따옴표 없는
    /// 식별자, `quoted`는 따옴표 있는 식별자의 대소문자 규칙.
    pub(crate) fn columns(
        &self,
        labels: &[String],
        table: &[String],
        fold: Fold,
        quoted: Fold,
    ) -> Vec<Option<String>> {
        let none = vec![None; labels.len()];
        let name = |i: &Ident| {
            if i.quoted {
                quoted.apply(&i.text)
            } else {
                fold.apply(&i.text)
            }
        };
        let (table_name, alias) = (name(&self.table), self.alias.as_ref().map(name));
        let qualifies = |q: &Option<Ident>| {
            q.as_ref().is_none_or(|q| {
                let q = name(q);
                alias.as_ref().map_or(q == table_name, |a| q == *a)
            })
        };
        let stars = self
            .items
            .iter()
            .filter(|i| matches!(i, Item::Star(_)))
            .count();
        let star_len = match stars {
            0 if self.items.len() == labels.len() => 0,
            1 if labels.len() + 1 >= self.items.len() => labels.len() + 1 - self.items.len(),
            _ => return none,
        };
        let known = |column: &str| table.iter().any(|c| c == column).then(|| column.to_owned());
        let mut columns = Vec::with_capacity(labels.len());
        let mut labels = labels.iter();
        for item in &self.items {
            match item {
                Item::Star(q) => {
                    for label in labels.by_ref().take(star_len) {
                        columns.push(qualifies(q).then(|| known(label)).flatten());
                    }
                }
                Item::Column(q, c) => {
                    let column = name(c);
                    // 드라이버가 준 이름과 다르면 SQL을 잘못 읽은 것이다.
                    let same = labels
                        .next()
                        .is_some_and(|l| l.eq_ignore_ascii_case(&column));
                    columns.push((same && qualifies(q)).then(|| known(&column)).flatten());
                }
                Item::Other => {
                    labels.next();
                    columns.push(None);
                }
            }
        }
        if columns.len() == none.len() {
            columns
        } else {
            none
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(sql: &str, labels: &[&str], table: &[&str]) -> Option<Vec<Option<String>>> {
        let labels: Vec<String> = labels.iter().map(|s| s.to_string()).collect();
        let table: Vec<String> = table.iter().map(|s| s.to_string()).collect();
        Some(parse(sql)?.columns(&labels, &table, Fold::Upper, Fold::None))
    }

    fn some(names: &[&str]) -> Vec<Option<String>> {
        names
            .iter()
            .map(|n| (!n.is_empty()).then(|| n.to_string()))
            .collect()
    }

    #[test]
    fn 한_표를_읽는_조회의_표와_열을_찾는다() {
        let table = ["ID", "NAME", "DEPT"];
        let select = parse("/* 주석 */ select e.id, name, dept + 1, e.\"NAME\" from hr.emp e where e.id in (select 1 from dual) order by 1").unwrap();
        assert_eq!(select.schema.as_ref().unwrap().text, "hr");
        assert_eq!(select.table.text, "emp");
        assert_eq!(
            names(
                "select e.id, name, dept + 1, e.\"NAME\" from hr.emp e where e.id in (select 1 from dual)",
                &["ID", "NAME", "DEPT+1", "NAME"],
                &table
            ),
            Some(some(&["ID", "NAME", "", "NAME"]))
        );
        // 별·별칭.별은 드라이버가 준 열 이름으로 맞춘다.
        assert_eq!(
            names(
                "SELECT e.*, ROWNUM FROM emp e",
                &["ID", "NAME", "DEPT", "ROWNUM"],
                &table
            ),
            Some(some(&["ID", "NAME", "DEPT", ""]))
        );
        assert_eq!(
            names(
                "SELECT * FROM \"Emp\" FOR UPDATE",
                &["ID", "NAME", "DEPT"],
                &table
            ),
            Some(some(&["ID", "NAME", "DEPT"]))
        );
        // 따옴표 안 글자·주석의 FROM, 줄 주석은 건너뛴다.
        assert_eq!(
            names(
                "SELECT id -- , from x\n, 'a,from' FROM emp",
                &["ID", "'A,FROM'"],
                &table
            ),
            Some(some(&["ID", ""]))
        );
    }

    #[test]
    fn 별칭을_붙인_열은_그_열로_보지_않는다() {
        let table = ["ID", "NAME", "DEPT"];
        // CUBRID는 `name AS id`를 id 열로 알려 주지만, 여기서는 고칠 수 없는 열이다.
        assert_eq!(
            names(
                "SELECT id, name AS id, dept d FROM emp",
                &["ID", "ID", "D"],
                &table
            ),
            Some(some(&["ID", "", ""]))
        );
        // 다른 표의 별칭으로 가리킨 열, 드라이버가 준 이름과 다른 열(잘못 읽음)은 아니다.
        assert_eq!(
            names("SELECT x.id FROM emp e", &["ID"], &table),
            Some(some(&[""]))
        );
        assert_eq!(
            names("SELECT id FROM emp", &["NAME"], &table),
            Some(some(&[""]))
        );
        // 결과 열 수가 항목과 맞지 않으면 모두 아니다.
        assert_eq!(
            names("SELECT id, name FROM emp", &["ID"], &table),
            Some(some(&[""]))
        );
        assert_eq!(
            names("SELECT *, * FROM emp", &["ID", "ID"], &table),
            Some(some(&["", ""]))
        );
    }

    #[test]
    fn 한_표_조회가_아니면_받지_않는다() {
        for sql in [
            "SELECT e.id FROM emp e, dept d",
            "SELECT e.id FROM emp e JOIN dept d ON d.id = e.dept",
            "SELECT id FROM emp LEFT OUTER JOIN dept USING (id)",
            "SELECT id FROM emp WHERE id = 1 UNION SELECT id FROM emp",
            "SELECT dept FROM emp GROUP BY dept",
            "SELECT DISTINCT id FROM emp",
            "SELECT id FROM (SELECT id FROM emp)",
            "SELECT id FROM emp@remote",
            "SELECT id FROM emp AS OF TIMESTAMP SYSDATE",
            "SELECT id FROM emp START WITH id = 1 CONNECT BY PRIOR id = dept",
            "WITH t AS (SELECT 1 FROM dual) SELECT * FROM t",
            "UPDATE emp SET id = 1",
            "SELECT 'unterminated FROM emp",
            "SELECT id FROM `emp`",
        ] {
            assert_eq!(parse(sql), None, "{sql}");
        }
        assert!(parse("SELECT ALL id FROM emp WHERE name = 'a, b' ORDER BY id;").is_some());
        assert!(parse("SELECT id FROM emp e ORDER BY e.dept, id DESC").is_some());
        assert!(parse("SELECT id FROM emp OFFSET 10 ROWS FETCH NEXT 5 ROWS ONLY").is_some());
    }

    #[test]
    fn 따옴표_식별자와_대소문자() {
        let select = parse("SELECT \"Id\", name FROM \"Emp\"\"s\"").unwrap();
        assert_eq!(
            select.table,
            Ident {
                text: "Emp\"s".into(),
                quoted: true
            }
        );
        let labels = ["Id".to_string(), "name".to_string()];
        let table = ["Id".to_string(), "name".to_string()];
        // CUBRID: 따옴표가 없거나 있어도 소문자로 둔다.
        assert_eq!(
            select.columns(&labels, &table, Fold::Lower, Fold::Lower),
            [None, Some("name".to_string())]
        );
        assert_eq!(
            select.columns(&labels, &table, Fold::Lower, Fold::None),
            [Some("Id".to_string()), Some("name".to_string())]
        );
    }
}
