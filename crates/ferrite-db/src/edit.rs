//! 결과 그리드에서 값 고치기. 결과가 한 표에서 왔고 그 표의 기본 키 열이 모두 결과에 있으면, 칸 하나를
//! 기본 키로 찾은 행의 UPDATE로 고친다. 값은 SQL 글에 붙이지 않고 파라미터로 묶는다.

use crate::{Cell, Kind, Row};

/// 고칠 수 있는 결과. 드라이버가 열을 보낸 뒤 `Event::Editable`로 보낸다.
#[derive(Clone, Debug, PartialEq)]
pub struct Editable {
    kind: Kind,
    /// 보이는 표 이름.
    pub table: String,
    /// SQL에 쓰는 표 이름(따옴표 처리).
    table_sql: String,
    /// 결과 열마다 표의 열. `None`은 고칠 수 없는 열(식, 다른 표).
    columns: Vec<Option<Target>>,
    /// 기본 키 열(결과 열 번호).
    key: Vec<usize>,
}

/// 결과 열이 온 표의 열.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    pub(crate) name: String,
    /// 열 형식. PostgreSQL은 파라미터를 글자로 보내 이 형식으로 바꾸고, Oracle은 날짜 형식만 쓴다.
    pub(crate) cast: Option<String>,
}

impl Editable {
    /// 기본 키 열이 하나라도 결과에 없으면(또는 기본 키가 없으면) 고칠 수 없다.
    pub(crate) fn new(
        kind: Kind,
        schema: Option<&str>,
        table: &str,
        columns: Vec<Option<Target>>,
        key_names: &[String],
    ) -> Option<Self> {
        if key_names.is_empty() {
            return None;
        }
        let key = key_names
            .iter()
            .map(|name| {
                columns
                    .iter()
                    .position(|c| c.as_ref().is_some_and(|c| &c.name == name))
            })
            .collect::<Option<Vec<usize>>>()?;
        let table_sql = match schema {
            Some(schema) => format!("{}.{}", quote(kind, schema), quote(kind, table)),
            None => quote(kind, table),
        };
        let table = match schema {
            Some(schema) if !matches!(schema, "main" | "public") => format!("{schema}.{table}"),
            _ => table.to_owned(),
        };
        Some(Self {
            kind,
            table,
            table_sql,
            columns,
            key,
        })
    }

    pub fn can_edit(&self, column: usize) -> bool {
        self.columns.get(column).is_some_and(Option::is_some)
    }

    /// 기본 키 열(결과 열 번호).
    pub fn key(&self) -> &[usize] {
        &self.key
    }

    /// `row`(결과의 행)의 `column` 칸을 `value`로 바꾸는 UPDATE와 파라미터. 고칠 수 없는 열이면 `None`.
    pub fn update(&self, column: usize, value: Cell, row: &Row) -> Option<(String, Vec<Cell>)> {
        let target = self.columns.get(column)?.as_ref()?;
        let mut params = vec![value];
        let set = format!(
            "{} = {}",
            quote(self.kind, &target.name),
            self.placeholder(1, target)
        );
        let mut conditions = Vec::new();
        for &index in &self.key {
            let key = self.columns[index].as_ref()?;
            params.push(row.get(index)?.clone());
            conditions.push(format!(
                "{} = {}",
                quote(self.kind, &key.name),
                self.placeholder(params.len(), key)
            ));
        }
        let sql = format!(
            "UPDATE {} SET {set} WHERE {}",
            self.table_sql,
            conditions.join(" AND ")
        );
        Some((sql, params))
    }

    fn placeholder(&self, n: usize, target: &Target) -> String {
        match (self.kind, target.cast.as_deref()) {
            (Kind::Postgres, Some(cast)) => format!("CAST(${n} AS {cast})"),
            (Kind::Postgres, None) => format!("${n}"),
            // Oracle 드라이버는 날짜를 `2024-01-02 00:00:00.0`처럼 보인다. 세션의 날짜 형식(NLS)과
            // 상관없이 그 글자(또는 날짜만, 소수 초 없이)를 받게 형식을 적는다.
            (Kind::Oracle | Kind::Tibero, Some("DATE")) => {
                "CAST(TO_TIMESTAMP(?, 'YYYY-MM-DD HH24:MI:SS.FF') AS DATE)".into()
            }
            (Kind::Oracle | Kind::Tibero, Some(ty))
                if ty.starts_with("TIMESTAMP") && !ty.contains("ZONE") =>
            {
                "TO_TIMESTAMP(?, 'YYYY-MM-DD HH24:MI:SS.FF')".into()
            }
            _ => "?".into(),
        }
    }
}

/// SQL에 쓸 식별자. 따옴표 없이 써도 같은 이름이면 그대로, 아니면 따옴표로 감싼다(Oracle·Tibero는
/// 대문자, PostgreSQL은 소문자 이름만 따옴표 없이 같다).
pub fn identifier(kind: Kind, name: &str) -> String {
    let mut chars = name.chars();
    let plain = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && match kind {
            Kind::Oracle | Kind::Tibero => !name.chars().any(|c| c.is_ascii_lowercase()),
            Kind::Postgres => !name.chars().any(|c| c.is_ascii_uppercase()),
            Kind::Sqlite | Kind::MariaDb | Kind::Cubrid => true,
        };
    if plain {
        name.to_owned()
    } else {
        quote(kind, name)
    }
}

/// 식별자를 따옴표로 감싼다(MariaDB는 역따옴표, 나머지는 큰따옴표).
pub(crate) fn quote(kind: Kind, name: &str) -> String {
    match kind {
        Kind::MariaDb => format!("`{}`", name.replace('`', "``")),
        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, cast: Option<&str>) -> Option<Target> {
        Some(Target {
            name: name.into(),
            cast: cast.map(str::to_owned),
        })
    }

    #[test]
    fn 기본_키로_찾는_update를_만든다() {
        let columns = vec![
            target("k2", None),
            target("v", None),
            None,
            target("k1", None),
        ];
        let editable = Editable::new(
            Kind::Sqlite,
            Some("main"),
            "t",
            columns,
            &["k1".into(), "k2".into()],
        )
        .unwrap();
        assert_eq!(editable.table, "t");
        assert_eq!(editable.key(), [3, 0]);
        assert!(editable.can_edit(1) && !editable.can_edit(2) && !editable.can_edit(9));
        let row = vec![
            Cell::Text("b".into()),
            Cell::Int(1),
            Cell::Int(2),
            Cell::Int(7),
        ];
        let (sql, params) = editable
            .update(1, Cell::Text("새 값".into()), &row)
            .unwrap();
        assert_eq!(
            sql,
            r#"UPDATE "main"."t" SET "v" = ? WHERE "k1" = ? AND "k2" = ?"#
        );
        assert_eq!(
            params,
            [
                Cell::Text("새 값".into()),
                Cell::Int(7),
                Cell::Text("b".into())
            ]
        );
        assert!(editable.update(2, Cell::Null, &row).is_none(), "식 열");
    }

    #[test]
    fn 종류마다_따옴표와_파라미터가_다르다() {
        let pg = Editable::new(
            Kind::Postgres,
            Some("app"),
            "주문\"표",
            vec![
                target("id", Some("integer")),
                target("메모", Some("character varying(20)")),
            ],
            &["id".into()],
        )
        .unwrap();
        assert_eq!(pg.table, "app.주문\"표");
        let row = vec![Cell::Text("3".into()), Cell::Null];
        assert_eq!(
            pg.update(1, Cell::Null, &row).unwrap().0,
            r#"UPDATE "app"."주문""표" SET "메모" = CAST($1 AS character varying(20)) WHERE "id" = CAST($2 AS integer)"#
        );
        let maria = Editable::new(
            Kind::MariaDb,
            Some("egov"),
            "t`x",
            vec![target("id", None), target("v", None)],
            &["id".into()],
        )
        .unwrap();
        assert_eq!(
            maria.update(1, Cell::Null, &row).unwrap().0,
            "UPDATE `egov`.`t``x` SET `v` = ? WHERE `id` = ?"
        );
        let oracle = Editable::new(
            Kind::Oracle,
            Some("HR"),
            "EMP",
            vec![
                target("ID", Some("NUMBER")),
                target("HIRED", Some("DATE")),
                target("AT", Some("TIMESTAMP(6)")),
            ],
            &["ID".into()],
        )
        .unwrap();
        assert_eq!(oracle.table, "HR.EMP");
        let row = vec![Cell::Text("1".into()), Cell::Null, Cell::Null];
        assert_eq!(
            oracle
                .update(1, Cell::Text("2024-01-02 00:00:00.0".into()), &row)
                .unwrap()
                .0,
            r#"UPDATE "HR"."EMP" SET "HIRED" = CAST(TO_TIMESTAMP(?, 'YYYY-MM-DD HH24:MI:SS.FF') AS DATE) WHERE "ID" = ?"#
        );
        assert!(
            oracle
                .update(2, Cell::Null, &row)
                .unwrap()
                .0
                .contains(r#""AT" = TO_TIMESTAMP(?, 'YYYY-MM-DD HH24:MI:SS.FF')"#)
        );
    }

    #[test]
    fn 필요할_때만_식별자를_따옴표로_감싼다() {
        assert_eq!(identifier(Kind::Oracle, "EMP_01"), "EMP_01");
        assert_eq!(identifier(Kind::Oracle, "Emp"), "\"Emp\"");
        assert_eq!(identifier(Kind::Postgres, "emp"), "emp");
        assert_eq!(identifier(Kind::Postgres, "Emp"), "\"Emp\"");
        assert_eq!(identifier(Kind::MariaDb, "주문"), "`주문`");
        assert_eq!(identifier(Kind::Cubrid, "order item"), "\"order item\"");
        assert_eq!(identifier(Kind::Sqlite, "Emp"), "Emp");
    }

    #[test]
    fn 기본_키가_없거나_빠지면_고칠_수_없다() {
        let columns = || vec![target("v", None)];
        assert!(Editable::new(Kind::Sqlite, None, "t", columns(), &[]).is_none());
        assert!(Editable::new(Kind::Sqlite, None, "t", columns(), &["id".into()]).is_none());
    }
}
