//! SQLite(rusqlite, SQLite 내장 빌드). 멈춤은 인터럽트로 한다.

use rusqlite::types::ValueRef;

use crate::{Canceller, Cell, Column, ConnectionConfig, Driver, Event, Io, PAGE_ROWS, Row};

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

impl Sqlite {
    fn batch(&self, sql: &str) -> Result<(), String> {
        self.conn.execute_batch(sql).map_err(|e| e.to_string())
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
        io.send(Event::Columns(columns));
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
}
