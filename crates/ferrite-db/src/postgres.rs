//! PostgreSQL(postgres, TLS 없음). 조회는 커서(`DECLARE`·`FETCH`)로 한 페이지씩 읽고, 값은 글자 결과로
//! 받는다(어떤 타입이든 서버가 보이는 그대로 보인다). 멈춤은 취소 요청(cancel token)으로 한다.

use postgres::{Client, NoTls, SimpleQueryMessage};

use crate::{
    Canceller, Cell, Column, ConnectionConfig, Driver, Event, Io, PAGE_ROWS, Row, is_query,
};

pub(crate) struct Postgres {
    client: Client,
    /// 자동 커밋을 껐다(지금 트랜잭션 안이다).
    manual: bool,
}

/// 커서 이름. 한 연결에서 한 번에 하나만 연다.
const CURSOR: &str = "ferrite_cursor";

pub(crate) fn connect(
    config: &ConnectionConfig,
    password: Option<&str>,
) -> Result<(Postgres, Canceller), String> {
    let mut pg = postgres::Config::new();
    pg.host(&config.host)
        .port(config.port)
        .user(&config.user)
        .application_name("Ferrite");
    if !config.database.is_empty() {
        pg.dbname(&config.database);
    }
    if let Some(password) = password {
        pg.password(password);
    }
    let client = pg.connect(NoTls).map_err(|e| error(&e))?;
    let token = client.cancel_token();
    Ok((
        Postgres {
            client,
            manual: false,
        },
        Box::new(move || {
            let _ = token.cancel_query(NoTls);
        }),
    ))
}

/// 서버 오류는 메시지와 자세한 내용을 보인다.
fn error(e: &postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => match db.detail() {
            Some(detail) => format!("{}: {} ({detail})", db.severity(), db.message()),
            None => format!("{}: {}", db.severity(), db.message()),
        },
        None => e.to_string(),
    }
}

/// 글자 결과를 열과 행으로 나눈다.
fn split(messages: Vec<SimpleQueryMessage>) -> (Option<Vec<Column>>, Vec<Row>, Option<u64>) {
    let (mut columns, mut rows, mut count) = (None, Vec::new(), None);
    for message in messages {
        match message {
            SimpleQueryMessage::RowDescription(described) => {
                columns = Some(
                    described
                        .iter()
                        .map(|c| Column {
                            name: c.name().to_owned(),
                        })
                        .collect(),
                );
            }
            SimpleQueryMessage::Row(row) => rows.push(
                (0..row.len())
                    .map(|i| row.get(i).map_or(Cell::Null, |t| Cell::Text(t.to_owned())))
                    .collect(),
            ),
            SimpleQueryMessage::CommandComplete(n) => count = Some(n),
            _ => {}
        }
    }
    (columns, rows, count)
}

impl Postgres {
    fn simple(&mut self, sql: &str) -> Result<Vec<SimpleQueryMessage>, String> {
        self.client.simple_query(sql).map_err(|e| error(&e))
    }

    /// 커서를 열어 한 페이지씩 보낸다. 트랜잭션 안에서만 커서를 쓸 수 있다.
    fn cursor(&mut self, sql: &str, io: &mut Io) -> Result<(), String> {
        self.simple(&format!("DECLARE {CURSOR} NO SCROLL CURSOR FOR {sql}"))?;
        let mut sent_columns = false;
        loop {
            let (columns, rows, _) =
                split(self.simple(&format!("FETCH {PAGE_ROWS} FROM {CURSOR}"))?);
            if !sent_columns {
                io.send(Event::Columns(columns.unwrap_or_default()));
                sent_columns = true;
            }
            let more = rows.len() == PAGE_ROWS;
            if !io.page(rows, more) {
                break;
            }
        }
        self.simple(&format!("CLOSE {CURSOR}"))?;
        Ok(())
    }
}

impl Postgres {
    /// 커서 없이 실행한다. 결과가 있는 문장(EXPLAIN, SHOW, RETURNING)은 한 페이지로 보낸다.
    fn plain(&mut self, sql: &str, io: &mut Io) -> Result<Option<u64>, String> {
        let (columns, rows, count) = split(self.simple(sql)?);
        Ok(match columns {
            Some(columns) => {
                io.send(Event::Columns(columns));
                io.page(rows, false);
                None
            }
            None => count,
        })
    }
}

impl Driver for Postgres {
    fn version(&mut self) -> String {
        let version = self
            .simple("SHOW server_version")
            .ok()
            .and_then(|m| split(m).1.into_iter().next())
            .and_then(|row| row.into_iter().next())
            .map_or_else(String::new, |cell| cell.to_string());
        format!("PostgreSQL {version}")
    }

    fn execute(&mut self, sql: &str, io: &mut Io) -> Result<Option<u64>, String> {
        let sql = sql.trim().trim_end_matches(';').trim_end();
        if !is_query(sql) {
            return self.plain(sql, io);
        }
        // 커서는 트랜잭션 안에서만 연다. 자동 커밋이면 잠깐 트랜잭션을 열고, 이미 트랜잭션 안이면
        // 세이브포인트를 둔다(조회가 실패·취소돼도 사용자의 트랜잭션이 "중단" 상태가 되지 않게).
        let (open, done, undo) = if self.manual {
            (
                "SAVEPOINT ferrite_cursor",
                "RELEASE SAVEPOINT ferrite_cursor",
                "ROLLBACK TO SAVEPOINT ferrite_cursor",
            )
        } else {
            ("BEGIN", "COMMIT", "ROLLBACK")
        };
        self.simple(open)?;
        match self.cursor(sql, io) {
            Ok(()) => self.simple(done).map(|_| None),
            Err(e) => {
                let _ = self.simple(undo);
                // 커서로 열 수 없는 조회(데이터를 바꾸는 WITH 등)는 그대로 실행한다.
                if e.contains("DECLARE CURSOR") {
                    self.plain(sql, io)
                } else {
                    Err(e)
                }
            }
        }
    }

    fn commit(&mut self) -> Result<(), String> {
        self.simple("COMMIT")?;
        if self.manual {
            self.simple("BEGIN")?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), String> {
        self.simple("ROLLBACK")?;
        if self.manual {
            self.simple("BEGIN")?;
        }
        Ok(())
    }

    fn set_autocommit(&mut self, on: bool) -> Result<(), String> {
        // 켜면 열린 트랜잭션을 커밋한다(JDBC와 같다).
        match (on, self.manual) {
            (true, true) => {
                self.simple("COMMIT")?;
            }
            (false, false) => {
                self.simple("BEGIN")?;
            }
            _ => {}
        }
        self.manual = !on;
        Ok(())
    }
}
