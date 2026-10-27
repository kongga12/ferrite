//! MariaDB·MySQL(mysql, 순수 Rust, TLS 없음). 결과는 글자 프로토콜로 한 행씩 흘려 읽는다. 멈춤은 다른
//! 연결을 잠깐 열어 `KILL QUERY`로 한다.

use mysql::consts::CapabilityFlags;
use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder, Value};

use crate::edit::{Editable, Target};
use crate::{
    Canceller, Cell, Column, ConnectionConfig, Driver, Event, Io, Kind, PAGE_ROWS, Row, Table,
    group_tables,
};

pub(crate) struct MariaDb {
    conn: Conn,
    opts: Opts,
    /// 기본 키를 찾을 때 쓰는 둘째 연결. 결과를 흘려 읽는 동안 첫 연결은 다른 문장을 못 쓴다.
    meta: Option<Conn>,
}

const PRIMARY_KEY: &str = "SELECT COLUMN_NAME FROM information_schema.KEY_COLUMN_USAGE \
     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND CONSTRAINT_NAME = 'PRIMARY' \
     ORDER BY ORDINAL_POSITION";

/// 스키마 브라우저: 현재 DB(없으면 시스템 DB 밖의 모든 DB)의 표·뷰와 열.
const SCHEMA_COLUMNS: &str = "SELECT c.TABLE_SCHEMA, c.TABLE_NAME, t.TABLE_TYPE = 'VIEW', \
     c.COLUMN_NAME, c.COLUMN_TYPE \
     FROM information_schema.COLUMNS c JOIN information_schema.TABLES t \
     ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME \
     WHERE c.TABLE_SCHEMA = DATABASE() OR (DATABASE() IS NULL AND c.TABLE_SCHEMA NOT IN \
     ('mysql', 'information_schema', 'performance_schema', 'sys')) \
     ORDER BY c.TABLE_SCHEMA, c.TABLE_NAME, c.ORDINAL_POSITION";

pub(crate) fn connect(
    config: &ConnectionConfig,
    password: Option<&str>,
) -> Result<(MariaDb, Canceller), String> {
    let opts: Opts = OptsBuilder::new()
        .ip_or_hostname(Some(config.host.clone()))
        .tcp_port(config.port)
        .user(Some(config.user.clone()))
        .pass(password.map(str::to_owned))
        .db_name((!config.database.is_empty()).then(|| config.database.clone()))
        // UPDATE의 바뀐 행 수를 "찾은 행"으로 센다(JDBC 기본과 같다). 같은 값으로 고쳐도 1행이다.
        .additional_capabilities(CapabilityFlags::CLIENT_FOUND_ROWS)
        .into();
    let conn = Conn::new(opts.clone()).map_err(|e| e.to_string())?;
    let id = conn.connection_id();
    Ok((
        MariaDb {
            conn,
            opts: opts.clone(),
            meta: None,
        },
        Box::new(move || {
            if let Ok(mut killer) = Conn::new(opts.clone()) {
                let _ = killer.query_drop(format!("KILL QUERY {id}"));
            }
        }),
    ))
}

fn cell(value: Value) -> Cell {
    match value {
        Value::NULL => Cell::Null,
        // 글자 프로토콜은 거의 모든 값을 글자로 준다.
        Value::Bytes(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Cell::Text(text),
            Err(e) => Cell::Bytes(e.into_bytes()),
        },
        Value::Int(n) => Cell::Int(n),
        Value::UInt(n) => i64::try_from(n).map_or_else(|_| Cell::Text(n.to_string()), Cell::Int),
        Value::Float(n) => Cell::Float(f64::from(n)),
        Value::Double(n) => Cell::Float(n),
        other => Cell::Text(other.as_sql(true)),
    }
}

fn param(cell: &Cell) -> Value {
    match cell {
        Cell::Null => Value::NULL,
        Cell::Int(n) => Value::Int(*n),
        Cell::Float(n) => Value::Double(*n),
        Cell::Text(text) => Value::Bytes(text.clone().into_bytes()),
        Cell::Bytes(bytes) => Value::Bytes(bytes.clone()),
    }
}

/// 결과 열이 모두 한 표에서 왔고 그 표의 기본 키 열이 다 있으면 고칠 수 있다.
fn editable(meta: &mut Option<Conn>, opts: &Opts, columns: &[mysql::Column]) -> Option<Editable> {
    let origins: Vec<Option<(String, String, String)>> = columns
        .iter()
        .map(|c| {
            let (schema, table, name) = (c.schema_str(), c.org_table_str(), c.org_name_str());
            (!schema.is_empty() && !table.is_empty() && !name.is_empty())
                .then(|| (schema.into_owned(), table.into_owned(), name.into_owned()))
        })
        .collect();
    let (schema, table, _) = origins.iter().flatten().next()?.clone();
    if origins
        .iter()
        .flatten()
        .any(|(s, t, _)| (s, t) != (&schema, &table))
    {
        return None;
    }
    let key_names = primary_key(meta, opts, &schema, &table)?;
    let columns = origins
        .into_iter()
        .map(|o| o.map(|(_, _, name)| Target { name, cast: None }))
        .collect();
    Editable::new(Kind::MariaDb, Some(&schema), &table, columns, &key_names)
}

fn primary_key(
    meta: &mut Option<Conn>,
    opts: &Opts,
    schema: &str,
    table: &str,
) -> Option<Vec<String>> {
    // 오래 쉬어 서버가 끊은 연결이면 한 번 다시 연다.
    for _ in 0..2 {
        if meta.is_none() {
            *meta = Conn::new(opts.clone()).ok();
        }
        match meta
            .as_mut()?
            .exec::<String, _, _>(PRIMARY_KEY, (schema, table))
        {
            Ok(keys) => return Some(keys),
            Err(_) => *meta = None,
        }
    }
    None
}

impl MariaDb {
    fn drop_query(&mut self, sql: &str) -> Result<(), String> {
        self.conn.query_drop(sql).map_err(|e| e.to_string())
    }
}

impl Driver for MariaDb {
    fn version(&mut self) -> String {
        self.conn
            .query_first::<String, _>("SELECT VERSION()")
            .ok()
            .flatten()
            .map_or_else(|| "MariaDB·MySQL".into(), |v| format!("MariaDB·MySQL {v}"))
    }

    fn execute(&mut self, sql: &str, io: &mut Io) -> Result<Option<u64>, String> {
        let mut result = self.conn.query_iter(sql).map_err(|e| e.to_string())?;
        let columns: Vec<Column> = result
            .columns()
            .as_ref()
            .iter()
            .map(|c| Column {
                name: c.name_str().into_owned(),
            })
            .collect();
        if columns.is_empty() {
            return Ok(Some(result.affected_rows()));
        }
        let editable = editable(&mut self.meta, &self.opts, result.columns().as_ref());
        io.send(Event::Columns(columns));
        if let Some(editable) = editable {
            io.send(Event::Editable(editable));
        }
        let mut page: Vec<Row> = Vec::with_capacity(PAGE_ROWS);
        // 결과를 일찍 닫으면 남은 행은 버리며 읽는다(연결을 다음 문장에 쓰려면 다 받아야 한다).
        for row in result.by_ref() {
            let row = row.map_err(|e| e.to_string())?;
            page.push(row.unwrap().into_iter().map(cell).collect());
            if page.len() == PAGE_ROWS && !io.page(std::mem::take(&mut page), true) {
                return Ok(None);
            }
        }
        io.page(page, false);
        Ok(None)
    }

    fn execute_params(&mut self, sql: &str, params: &[Cell]) -> Result<u64, String> {
        let values: Vec<Value> = params.iter().map(param).collect();
        self.conn
            .exec_drop(sql, values)
            .map_err(|e| e.to_string())?;
        Ok(self.conn.affected_rows())
    }

    fn commit(&mut self) -> Result<(), String> {
        self.drop_query("COMMIT")
    }

    fn rollback(&mut self) -> Result<(), String> {
        self.drop_query("ROLLBACK")
    }

    fn set_autocommit(&mut self, on: bool) -> Result<(), String> {
        // 켜면 열린 트랜잭션을 커밋한다(서버도 autocommit=1로 바꿀 때 커밋한다).
        self.drop_query(if on {
            "SET autocommit=1"
        } else {
            "SET autocommit=0"
        })
    }

    fn schema(&mut self) -> Result<Vec<Table>, String> {
        // (스키마, 표, 뷰인지, 열, 형식). 현재 DB가 없으면 시스템 DB 밖의 모든 DB.
        type Line = (String, String, i64, String, String);
        let rows: Vec<Line> = self.conn.query(SCHEMA_COLUMNS).map_err(|e| e.to_string())?;
        let current: Option<String> = self
            .conn
            .query_first("SELECT DATABASE()")
            .map_err(|e| e.to_string())?
            .flatten();
        Ok(group_tables(rows.into_iter().map(
            |(schema, table, view, column, ty)| {
                let schema = (current.as_ref() != Some(&schema)).then_some(schema);
                (schema, table, view != 0, column, ty)
            },
        )))
    }
}
