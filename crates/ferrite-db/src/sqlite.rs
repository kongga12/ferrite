//! SQLite(rusqlite, SQLite 내장 빌드). 멈춤은 인터럽트로 한다.

use rusqlite::types::{Value, ValueRef};

use crate::edit::{Editable, Target, quote};
use crate::{
    Canceller, Cell, Column, ConnectionConfig, Driver, Event, Io, Kind, PAGE_ROWS, Row, Table,
    group_tables,
};

pub(crate) struct Sqlite {
    conn: rusqlite::Connection,
    /// 자동 커밋을 껐다(커밋·롤백 뒤에 새 트랜잭션을 연다).
    manual: bool,
}

pub(crate) fn connect(config: &ConnectionConfig) -> Result<(Sqlite, Canceller), String> {
    let path = config.path.as_ref().ok_or("SQLite 파일을 고르세요")?;
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| format!("{}을(를) 열지 못했습니다: {e}", path.display()))?;
    let handle = conn.get_interrupt_handle();
    Ok((
        Sqlite {
            conn,
            manual: false,
        },
        Box::new(move || handle.interrupt()),
    ))
}

fn cell(value: ValueRef<'_>) -> Cell {
    match value {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(n) => Cell::Int(n),
        ValueRef::Real(n) => Cell::Float(n),
        ValueRef::Text(text) => Cell::Text(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(bytes) => Cell::Bytes(bytes.to_vec()),
    }
}

fn param(cell: &Cell) -> Value {
    match cell {
        Cell::Null => Value::Null,
        Cell::Int(n) => Value::Integer(*n),
        Cell::Float(n) => Value::Real(*n),
        Cell::Text(text) => Value::Text(text.clone()),
        Cell::Bytes(bytes) => Value::Blob(bytes.clone()),
    }
}

/// 결과 열이 모두 한 표에서 왔고 그 표의 기본 키 열이 다 있으면 고칠 수 있다.
fn editable(conn: &rusqlite::Connection, statement: &rusqlite::Statement<'_>) -> Option<Editable> {
    let origins: Vec<Option<(String, String, String)>> = statement
        .columns_with_metadata()
        .map(|c| {
            Some((
                c.database_name()?.to_owned(),
                c.table_name()?.to_owned(),
                c.origin_name()?.to_owned(),
            ))
        })
        .collect();
    let (database, table, _) = origins.iter().flatten().next()?.clone();
    if origins
        .iter()
        .flatten()
        .any(|(d, t, _)| (d, t) != (&database, &table))
    {
        return None;
    }
    let pragma = format!(
        "PRAGMA {}.table_info({})",
        quote(Kind::Sqlite, &database),
        quote(Kind::Sqlite, &table)
    );
    let mut pragma = conn.prepare(&pragma).ok()?;
    let mut keys: Vec<(i64, String)> = pragma
        .query_map([], |row| Ok((row.get("pk")?, row.get("name")?)))
        .ok()?
        .collect::<Result<_, _>>()
        .ok()?;
    // pk는 기본 키 안의 순서(1부터), 기본 키가 아니면 0.
    keys.retain(|(pk, _)| *pk > 0);
    keys.sort();
    let key_names: Vec<String> = keys.into_iter().map(|(_, name)| name).collect();
    let columns = origins
        .into_iter()
        .map(|o| o.map(|(_, _, name)| Target { name, cast: None }))
        .collect();
    Editable::new(Kind::Sqlite, Some(&database), &table, columns, &key_names)
}

impl Sqlite {
    fn batch(&self, sql: &str) -> Result<(), String> {
        self.conn.execute_batch(sql).map_err(|e| e.to_string())
    }

    fn tables(&self) -> rusqlite::Result<Vec<Table>> {
        let mut list = self.conn.prepare(
            "SELECT name, type = 'view' FROM sqlite_master \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name",
        )?;
        let names: Vec<(String, bool)> = list
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut rows = Vec::new();
        for (name, view) in names {
            let mut pragma = self.conn.prepare(&format!(
                "PRAGMA table_info({})",
                quote(Kind::Sqlite, &name)
            ))?;
            let columns: Vec<(String, String)> = pragma
                .query_map([], |row| Ok((row.get("name")?, row.get("type")?)))?
                .collect::<Result<_, _>>()?;
            rows.extend(
                columns
                    .into_iter()
                    .map(|(column, ty)| (None, name.clone(), view, column, ty)),
            );
        }
        Ok(group_tables(rows))
    }
}

impl Driver for Sqlite {
    fn version(&mut self) -> String {
        format!("SQLite {}", rusqlite::version())
    }

    fn execute(&mut self, sql: &str, io: &mut Io) -> Result<Option<u64>, String> {
        let mut statement = self.conn.prepare(sql).map_err(|e| e.to_string())?;
        let count = statement.column_count();
        if count == 0 {
            return statement
                .execute([])
                .map(|n| Some(n as u64))
                .map_err(|e| e.to_string());
        }
        let columns = statement
            .column_names()
            .map(|name| Column {
                name: name.to_owned(),
            })
            .collect();
        let editable = editable(&self.conn, &statement);
        io.send(Event::Columns(columns));
        if let Some(editable) = editable {
            io.send(Event::Editable(editable));
        }
        let mut rows = statement.query([]).map_err(|e| e.to_string())?;
        let mut page: Vec<Row> = Vec::with_capacity(PAGE_ROWS);
        loop {
            match rows.next().map_err(|e| e.to_string())? {
                Some(row) => {
                    page.push(
                        (0..count)
                            .map(|i| row.get_ref(i).map_or(Cell::Null, cell))
                            .collect(),
                    );
                    if page.len() == PAGE_ROWS && !io.page(std::mem::take(&mut page), true) {
                        return Ok(None);
                    }
                }
                None => {
                    io.page(page, false);
                    return Ok(None);
                }
            }
        }
    }

    fn execute_params(&mut self, sql: &str, params: &[Cell]) -> Result<u64, String> {
        self.conn
            .execute(sql, rusqlite::params_from_iter(params.iter().map(param)))
            .map(|n| n as u64)
            .map_err(|e| e.to_string())
    }

    fn commit(&mut self) -> Result<(), String> {
        if !self.conn.is_autocommit() {
            self.batch("COMMIT")?;
        }
        if self.manual {
            self.batch("BEGIN")?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), String> {
        if !self.conn.is_autocommit() {
            self.batch("ROLLBACK")?;
        }
        if self.manual {
            self.batch("BEGIN")?;
        }
        Ok(())
    }

    fn set_autocommit(&mut self, on: bool) -> Result<(), String> {
        // 켜면 열린 트랜잭션을 커밋한다(JDBC와 같다).
        match (on, self.conn.is_autocommit()) {
            (true, false) => self.batch("COMMIT")?,
            (false, true) => self.batch("BEGIN")?,
            _ => {}
        }
        self.manual = !on;
        Ok(())
    }

    fn schema(&mut self) -> Result<Vec<Table>, String> {
        self.tables().map_err(|e| e.to_string())
    }
}
