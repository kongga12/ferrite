//! MariaDB·MySQL(mysql, 순수 Rust, TLS 없음). 결과는 글자 프로토콜로 한 행씩 흘려 읽는다. 멈춤은 다른
//! 연결을 잠깐 열어 `KILL QUERY`로 한다.

use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder, Value};

use crate::{Canceller, Cell, Column, ConnectionConfig, Driver, Event, Io, PAGE_ROWS, Row};

pub(crate) struct MariaDb {
    conn: Conn,
}

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
        .into();
    let conn = Conn::new(opts.clone()).map_err(|e| e.to_string())?;
    let id = conn.connection_id();
    Ok((
        MariaDb { conn },
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
        io.send(Event::Columns(columns));
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
}
