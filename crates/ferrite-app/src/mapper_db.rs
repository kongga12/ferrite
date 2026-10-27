//! 매퍼 XML SQL의 DB 표·열 자동 완성과 검증. 매퍼 기능은 앱이 띄우는 별도 LSP 서버(`--egov-ls`)라
//! 앱의 DB 연결을 모르므로 앱에서 한다. 스키마는 스키마 브라우저가 읽은 것을 쓴다(읽지 않았으면 아무것도
//! 하지 않는다). 문장 본문은 `ferrite_egov::mapper::statement_sql`, SQL은 `ferrite_db::refs`가 읽는다.

use std::ops::Range;

use ferrite_db::Table;
use ferrite_db::refs::{self, Context, References, TableRef};
use ferrite_egov::mapper::{StatementSql, statement_sql};
use ferrite_lsp::features::Completion;

/// 문서(바이트)의 경고 하나.
#[derive(Debug, PartialEq)]
pub struct Problem {
    pub range: Range<usize>,
    pub message: String,
}

/// 스키마에서 표 참조가 가리키는 표. 스키마를 적었으면 그 스키마의 표만 본다.
fn find<'t>(tables: &'t [Table], table: &TableRef) -> Option<&'t Table> {
    tables.iter().find(|t| {
        t.name.eq_ignore_ascii_case(&table.name)
            && table.schema.as_ref().is_none_or(|s| {
                t.schema
                    .as_ref()
                    .is_some_and(|ts| ts.eq_ignore_ascii_case(s))
            })
    })
}

/// 확인할 수 있는 표 참조인지. WITH 이름과 Oracle의 DUAL은 스키마 밖이고, 읽지 않은 스키마를 적은
/// 표는 알 수 없다.
fn checkable(tables: &[Table], refs: &References, table: &TableRef) -> bool {
    let cte = refs
        .ctes
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&table.name));
    let dual = table.schema.is_none() && table.name.eq_ignore_ascii_case("dual");
    let known_schema = table.schema.as_ref().is_none_or(|s| {
        tables.iter().any(|t| {
            t.schema
                .as_ref()
                .is_some_and(|ts| ts.eq_ignore_ascii_case(s))
        })
    });
    !cte && !dual && known_schema
}

fn item(label: String, detail: String, kind: &'static str) -> Completion {
    Completion {
        filter: label.clone(),
        sort: label.clone(),
        text: label.clone(),
        label,
        detail,
        kind,
        range: None,
        additional: Vec::new(),
    }
}

/// `cursor`(바이트)가 매퍼 문장 SQL 안이면 표나 열 항목과, 바꿀 범위(커서 앞에 친 낱말, 바이트).
pub fn completions(
    text: &str,
    cursor: usize,
    tables: &[Table],
) -> Option<(Range<usize>, Vec<Completion>)> {
    if tables.is_empty() {
        return None;
    }
    let statement = statement_sql(text)
        .into_iter()
        .find(|s| s.element.contains(&cursor) && s.sql_offset(cursor).is_some())?;
    let at = statement.sql_offset(cursor)?;
    let (context, typed) = refs::context(&statement.sql, at)?;
    let range = statement.document_offset(typed.start)?..cursor;
    let refs = refs::references(&statement.sql);
    let columns = |table: &Table, detail: &dyn Fn(&str) -> String| -> Vec<Completion> {
        table
            .columns
            .iter()
            .map(|c| item(c.name.clone(), detail(&c.ty), "열"))
            .collect()
    };
    let items: Vec<Completion> = match context {
        Context::Table => tables
            .iter()
            .map(|t| {
                let (kind, what) = match (&t.synonym, t.view) {
                    (Some(target), _) => ("동의어", format!("동의어 → {target}")),
                    (None, true) => ("뷰", "뷰".to_owned()),
                    (None, false) => ("표", "표".to_owned()),
                };
                item(
                    t.label(),
                    format!("{what} · 열 {}개", t.columns.len()),
                    kind,
                )
            })
            .collect(),
        Context::Column {
            qualifier: Some(qualifier),
        } => {
            let table = find(tables, refs.resolve(&qualifier)?)?;
            columns(table, &|ty| ty.to_owned())
        }
        Context::Column { qualifier: None } => refs
            .tables
            .iter()
            .filter_map(|r| find(tables, r))
            .flat_map(|t| {
                let name = t.label();
                columns(t, &|ty| format!("{name} · {ty}"))
            })
            .collect(),
    };
    (!items.is_empty()).then_some((range, items))
}

/// 매퍼의 문장마다 스키마에 없는 표, 표에 없는 열. `connection`은 경고에 보일 연결 이름.
pub fn problems(text: &str, tables: &[Table], connection: &str) -> Vec<Problem> {
    if tables.is_empty() {
        return Vec::new();
    }
    let mut problems = Vec::new();
    for statement in statement_sql(text) {
        let refs = refs::references(&statement.sql);
        let mut push = |range: &Range<usize>, message: String| {
            if let Some(range) = document_range(&statement, range) {
                problems.push(Problem { range, message });
            }
        };
        for table in &refs.tables {
            if checkable(tables, &refs, table) && find(tables, table).is_none() {
                push(
                    &table.range,
                    format!(
                        "표 {}이(가) 연결 '{connection}'의 스키마에 없습니다",
                        table.name
                    ),
                );
            }
        }
        for column in &refs.columns {
            let Some(table_ref) = refs.resolve(&column.qualifier) else {
                continue;
            };
            let Some(table) =
                find(tables, table_ref).filter(|_| checkable(tables, &refs, table_ref))
            else {
                continue;
            };
            if !table
                .columns
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(&column.name))
            {
                push(
                    &column.range,
                    format!("열 {}이(가) 표 {}에 없습니다", column.name, table.label()),
                );
            }
        }
    }
    problems
}

/// SQL 안 범위 → 문서 범위(한 글 조각 안에 있을 때만).
fn document_range(statement: &StatementSql, range: &Range<usize>) -> Option<Range<usize>> {
    Some(statement.document_offset(range.start)?..statement.document_offset(range.end)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_db::TableColumn;

    fn table(name: &str, columns: &[&str]) -> Table {
        Table {
            schema: None,
            name: name.into(),
            view: false,
            synonym: None,
            columns: columns
                .iter()
                .map(|c| TableColumn {
                    name: c.to_string(),
                    ty: "VARCHAR2".into(),
                })
                .collect(),
        }
    }

    const MAPPER: &str = r#"<mapper namespace="egov.sample">
  <select id="list">
    SELECT e.EMP_NM, e.NO_SUCH FROM COMTNEMP e
    <where>AND e.DEPT_ID = #{deptId}</where>
  </select>
  <insert id="add">INSERT INTO COMTNMISSING (A) VALUES (#{a})</insert>
  <update id="edit">UPDATE COMTNEMP SET EMP_NM = #{name}, WRONG = 1 WHERE EMP_ID = #{id}</update>
  <select id="ok">WITH t AS (SELECT 1 FROM DUAL) SELECT * FROM t, other.REMOTE r, ${table}</select>
</mapper>"#;

    #[test]
    fn 스키마에_없는_표와_열을_알린다() {
        let tables = [table("COMTNEMP", &["EMP_ID", "EMP_NM", "DEPT_ID"])];
        let found: Vec<(String, String)> = problems(MAPPER, &tables, "운영")
            .into_iter()
            .map(|p| (MAPPER[p.range].to_owned(), p.message))
            .collect();
        assert_eq!(
            found,
            [
                (
                    "NO_SUCH".into(),
                    "열 NO_SUCH이(가) 표 COMTNEMP에 없습니다".into()
                ),
                (
                    "COMTNMISSING".into(),
                    "표 COMTNMISSING이(가) 연결 '운영'의 스키마에 없습니다".into()
                ),
                (
                    "WRONG".into(),
                    "열 WRONG이(가) 표 COMTNEMP에 없습니다".into()
                ),
            ]
        );
        // 스키마를 읽지 않았으면 아무것도 알리지 않는다.
        assert!(problems(MAPPER, &[], "운영").is_empty());
    }

    #[test]
    fn 동의어로_쓴_표를_안다() {
        // 다른 스키마의 공통코드 표를 동의어로 쓰는 매퍼(Oracle·Tibero egov 프로젝트에 흔하다).
        let text = r#"<mapper namespace="a"><select id="code">SELECT c.CODE_NM, c.NOPE FROM COMTCCMMNCODE c WHERE c.CODE_ID = #{id}</select></mapper>"#;
        let mut synonym = table("COMTCCMMNCODE", &["CODE_ID", "CODE_NM"]);
        synonym.synonym = Some("COM.COMTCCMMNCODE".into());
        let found: Vec<String> = problems(text, std::slice::from_ref(&synonym), "운영")
            .into_iter()
            .map(|p| p.message)
            .collect();
        assert_eq!(found, ["열 NOPE이(가) 표 COMTCCMMNCODE에 없습니다"]);
        let cursor = text.find("FROM COMT").unwrap() + "FROM COMT".len();
        let (_, items) = completions(text, cursor, std::slice::from_ref(&synonym)).unwrap();
        assert_eq!(
            (
                items[0].label.as_str(),
                items[0].detail.as_str(),
                items[0].kind
            ),
            (
                "COMTCCMMNCODE",
                "동의어 → COM.COMTCCMMNCODE · 열 2개",
                "동의어"
            )
        );
    }

    #[test]
    fn 표와_열을_자동_완성한다() {
        let tables = [
            table("COMTNEMP", &["EMP_ID", "EMP_NM"]),
            table("COMTNDEPT", &["DEPT_ID"]),
        ];
        let at = |marker: &str| {
            let text = MAPPER.replacen(marker, &format!("{marker}|"), 1);
            let cursor = text.find('|').unwrap();
            let text = text.replacen('|', "", 1);
            completions(&text, cursor, &tables).map(|(range, items)| {
                let labels: Vec<String> = items.iter().map(|i| i.label.clone()).collect();
                (text[range].to_owned(), labels)
            })
        };
        // 별칭 뒤에는 그 표의 열, 표 자리에는 표, 그 밖의 식 자리에는 문장 표들의 열.
        assert_eq!(
            at("SELECT e.EMP"),
            Some(("EMP".into(), vec!["EMP_ID".into(), "EMP_NM".into()]))
        );
        assert_eq!(
            at("FROM COMT"),
            Some(("COMT".into(), vec!["COMTNEMP".into(), "COMTNDEPT".into()]))
        );
        assert_eq!(
            at("WHERE EMP_ID = #{id}").map(|(typed, _)| typed),
            None,
            "#{{}} 뒤는 값 자리다"
        );
        assert_eq!(
            at("UPDATE COMTNEMP SET ").map(|(_, labels)| labels),
            Some(vec!["EMP_ID".into(), "EMP_NM".into()])
        );
        // 태그 안(속성)은 SQL이 아니다.
        assert_eq!(at("namespace=\"egov"), None);
    }
}
