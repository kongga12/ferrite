//! `CREATE TABLE` 문 읽기. Oracle·Tibero·PostgreSQL·MariaDB(MySQL)·CUBRID의 흔한 모양을 다룬다.
//! - 열: 이름(따옴표·백틱·대괄호 가능), 타입과 길이·소수 자릿수, `NOT NULL`, `PRIMARY KEY`, `COMMENT '…'`
//! - 표 제약: `PRIMARY KEY (a, b)`, `CONSTRAINT pk PRIMARY KEY (…)`
//! - 주석: MySQL `COMMENT '…'`, Oracle·PostgreSQL·Tibero·CUBRID `COMMENT ON COLUMN t.c IS '…'`

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// 스키마를 뺀 표 이름(따옴표 없이, 쓴 그대로의 대소문자).
    pub name: String,
    pub comment: Option<String>,
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// 기본 타입 이름(대문자, `VARCHAR2`, `NUMBER`, `TIMESTAMP`).
    pub data_type: String,
    /// `VARCHAR(100)`의 100, `NUMBER(10,2)`의 10.
    pub length: Option<u32>,
    /// `NUMBER(10,2)`의 2.
    pub scale: Option<u32>,
    pub primary_key: bool,
    pub not_null: bool,
    pub comment: Option<String>,
}

pub fn parse(ddl: &str) -> Result<Table, String> {
    let text = strip_comments(ddl);
    let upper = text.to_ascii_uppercase();
    let create = upper
        .find("CREATE")
        .and_then(|at| upper[at..].find("TABLE").map(|t| at + t + "TABLE".len()))
        .ok_or("CREATE TABLE 문이 없습니다")?;
    let open = text[create..]
        .find('(')
        .map(|o| create + o)
        .ok_or("열 목록 `(`이 없습니다")?;
    let mut name = text[create..open].trim();
    for prefix in ["IF NOT EXISTS", "if not exists"] {
        name = name.strip_prefix(prefix).unwrap_or(name).trim();
    }
    let name = unquote(name.rsplit('.').next().unwrap_or(name));
    if name.is_empty() {
        return Err("표 이름이 없습니다".into());
    }
    let close = matching_paren(&text, open).ok_or("열 목록의 `)`이 닫히지 않았습니다")?;
    let mut columns = Vec::new();
    let mut primary: Vec<String> = Vec::new();
    for item in split_top(&text[open + 1..close], ',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let head = item.to_ascii_uppercase();
        let constraint = [
            "CONSTRAINT",
            "PRIMARY",
            "UNIQUE",
            "KEY",
            "INDEX",
            "FOREIGN",
            "CHECK",
        ]
        .iter()
        .any(|k| first_word(&head) == *k);
        if constraint {
            if let Some(at) = head.find("PRIMARY KEY") {
                let rest = &item[at..];
                if let Some(o) = rest.find('(')
                    && let Some(c) = matching_paren(rest, o)
                {
                    primary.extend(split_top(&rest[o + 1..c], ',').map(|c| unquote(c.trim())));
                }
            }
            continue;
        }
        columns.push(column(item)?);
    }
    if columns.is_empty() {
        return Err("열이 없습니다".into());
    }
    for column in &mut columns {
        if primary.iter().any(|p| p.eq_ignore_ascii_case(&column.name)) {
            column.primary_key = true;
            column.not_null = true;
        }
    }
    let after = &text[close + 1..];
    let mut table = Table {
        name,
        comment: table_comment(after),
        columns,
    };
    comment_on(&text, &mut table);
    Ok(table)
}

fn column(item: &str) -> Result<Column, String> {
    let (name, rest) = split_name(item);
    let rest = rest.trim_start();
    let type_end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let data_type = rest[..type_end].to_ascii_uppercase();
    if data_type.is_empty() {
        return Err(format!("열 `{name}`의 타입이 없습니다"));
    }
    let mut after = rest[type_end..].trim_start();
    // `DOUBLE PRECISION`, `CHARACTER VARYING`, `LONG RAW` 같은 두 낱말 타입.
    let second = first_word(&after.to_ascii_uppercase()).to_owned();
    let data_type = match (data_type.as_str(), second.as_str()) {
        ("DOUBLE", "PRECISION") | ("CHARACTER", "VARYING") | ("LONG", "RAW") => {
            after = after[second.len()..].trim_start();
            format!("{data_type} {second}")
        }
        _ => data_type,
    };
    let (mut length, mut scale) = (None, None);
    if after.starts_with('(')
        && let Some(close) = matching_paren(after, 0)
    {
        let mut parts = after[1..close].split(',').map(|p| {
            p.split_whitespace()
                .next()
                .and_then(|n| n.parse::<u32>().ok())
        });
        length = parts.next().flatten();
        scale = parts.next().flatten();
        after = &after[close + 1..];
    }
    let upper = after.to_ascii_uppercase();
    let comment = upper
        .find("COMMENT")
        .and_then(|at| quoted(&after[at + "COMMENT".len()..]));
    Ok(Column {
        name,
        data_type,
        length,
        scale,
        primary_key: upper.contains("PRIMARY KEY"),
        not_null: upper.contains("NOT NULL") || upper.contains("PRIMARY KEY"),
        comment,
    })
}

/// 열 정의 앞의 이름과 나머지. 이름은 따옴표·백틱·대괄호로 감쌀 수 있다.
fn split_name(item: &str) -> (String, &str) {
    let item = item.trim_start();
    let close = match item.chars().next() {
        Some('"') => Some('"'),
        Some('`') => Some('`'),
        Some('[') => Some(']'),
        _ => None,
    };
    match close.and_then(|c| item[1..].find(c).map(|end| end + 1)) {
        Some(end) => (item[1..end].to_owned(), &item[end + 1..]),
        None => {
            let end = item.find(char::is_whitespace).unwrap_or(item.len());
            (item[..end].to_owned(), &item[end..])
        }
    }
}

fn unquote(name: &str) -> String {
    name.trim()
        .trim_matches(|c| matches!(c, '"' | '`' | '[' | ']'))
        .to_owned()
}

fn first_word(text: &str) -> &str {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("")
}

/// `'…'` 하나(작은따옴표 두 개는 따옴표 하나).
fn quoted(text: &str) -> Option<String> {
    let start = text.find('\'')? + 1;
    let mut value = String::new();
    let mut chars = text[start..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                value.push('\'');
                continue;
            }
            return Some(value);
        }
        value.push(c);
    }
    None
}

/// `) COMMENT='…'`·`) COMMENT '…'`(MySQL·MariaDB 표 주석).
fn table_comment(after: &str) -> Option<String> {
    let statement = after.split(';').next().unwrap_or(after);
    let upper = statement.to_ascii_uppercase();
    let at = upper.find("COMMENT")?;
    quoted(&statement[at + "COMMENT".len()..])
}

/// `COMMENT ON TABLE t IS '…'`, `COMMENT ON COLUMN t.c IS '…'`.
fn comment_on(text: &str, table: &mut Table) {
    for statement in text.split(';') {
        let trimmed = statement.trim();
        let upper = trimmed.to_ascii_uppercase();
        let Some(rest) = upper.strip_prefix("COMMENT ON ") else {
            continue;
        };
        let Some(is) = upper.find(" IS ") else {
            continue;
        };
        let Some(comment) = quoted(&trimmed[is + 4..]) else {
            continue;
        };
        let target = trimmed["COMMENT ON ".len()..is].trim();
        if rest.starts_with("TABLE") {
            table.comment = Some(comment);
        } else if rest.starts_with("COLUMN") {
            let path = target["COLUMN".len()..].trim();
            let column = unquote(path.rsplit('.').next().unwrap_or(path));
            if let Some(c) = table
                .columns
                .iter_mut()
                .find(|c| c.name.eq_ignore_ascii_case(&column))
            {
                c.comment = Some(comment);
            }
        }
    }
}

/// `--` 줄 주석과 `/* */` 주석을 지운다(문자열 안은 그대로).
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut quote = false;
    while let Some(c) = chars.next() {
        if quote {
            out.push(c);
            if c == '\'' {
                quote = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('\'', _) => {
                quote = true;
                out.push(c);
            }
            ('-', Some('-')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// `open` 위치의 `(`와 짝인 `)`(따옴표 안은 건너뛴다).
fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut quote = None;
    for (i, c) in text[open..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// 괄호·따옴표 밖의 구분자로 나눈다.
fn split_top(text: &str, separator: char) -> impl Iterator<Item = &str> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None, 0);
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, c) if c == separator && depth == 0 => {
                parts.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_ddl과_comment_on을_읽는다() {
        let ddl = r#"
-- 게시판
CREATE TABLE EGOV.TB_BOARD (
    BOARD_ID    NUMBER(10)      NOT NULL,
    TITLE       VARCHAR2(200)   NOT NULL,  /* 제목 */
    CONTENT     CLOB,
    AMOUNT      NUMBER(12, 2),
    REG_DT      DATE DEFAULT SYSDATE,
    CONSTRAINT PK_TB_BOARD PRIMARY KEY (BOARD_ID)
);
COMMENT ON TABLE TB_BOARD IS '게시판';
COMMENT ON COLUMN TB_BOARD.BOARD_ID IS '게시글 번호';
COMMENT ON COLUMN EGOV.TB_BOARD.TITLE IS '제목(''필수'')';
"#;
        let table = parse(ddl).unwrap();
        assert_eq!(table.name, "TB_BOARD");
        assert_eq!(table.comment.as_deref(), Some("게시판"));
        type Row<'a> = (&'a str, &'a str, Option<u32>, Option<u32>, bool, bool);
        let summary: Vec<Row> = table
            .columns
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.data_type.as_str(),
                    c.length,
                    c.scale,
                    c.primary_key,
                    c.not_null,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("BOARD_ID", "NUMBER", Some(10), None, true, true),
                ("TITLE", "VARCHAR2", Some(200), None, false, true),
                ("CONTENT", "CLOB", None, None, false, false),
                ("AMOUNT", "NUMBER", Some(12), Some(2), false, false),
                ("REG_DT", "DATE", None, None, false, false),
            ]
        );
        assert_eq!(table.columns[0].comment.as_deref(), Some("게시글 번호"));
        assert_eq!(table.columns[1].comment.as_deref(), Some("제목('필수')"));
    }

    #[test]
    fn mariadb_postgresql_cubrid_모양도_읽는다() {
        let maria = "CREATE TABLE IF NOT EXISTS `board` (\n `id` VARCHAR(36) PRIMARY KEY COMMENT '아이디',\n `view_count` INT DEFAULT 0 COMMENT '조회수, 정수',\n `price` DECIMAL(10,2) NOT NULL\n) ENGINE=InnoDB COMMENT='게시판';";
        let table = parse(maria).unwrap();
        assert_eq!(table.name, "board");
        assert_eq!(table.comment.as_deref(), Some("게시판"));
        assert!(table.columns[0].primary_key);
        assert_eq!(
            table.columns[1].comment.as_deref(),
            Some("조회수, 정수"),
            "주석 안 쉼표"
        );
        assert_eq!(
            (table.columns[2].length, table.columns[2].scale),
            (Some(10), Some(2))
        );

        let pg = "create table public.\"member\" (\n  member_id bigserial,\n  user_name character varying(50) not null,\n  score double precision,\n  primary key (member_id)\n);\ncomment on column member.user_name is '사용자 이름';";
        let table = parse(pg).unwrap();
        assert_eq!(table.name, "member");
        assert_eq!(table.columns[0].data_type, "BIGSERIAL");
        assert!(table.columns[0].primary_key);
        assert_eq!(table.columns[1].data_type, "CHARACTER VARYING");
        assert_eq!(table.columns[1].length, Some(50));
        assert_eq!(table.columns[2].data_type, "DOUBLE PRECISION");
        assert_eq!(table.columns[1].comment.as_deref(), Some("사용자 이름"));

        assert!(parse("SELECT 1").is_err());
        assert!(parse("CREATE TABLE t (").is_err());
    }
}
