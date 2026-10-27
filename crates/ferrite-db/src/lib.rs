//! Ferrite DB 도구: 연결 설정·비밀번호 보관과, 연결마다 작업 스레드 하나를 두고 명령·결과를 채널로
//! 주고받는 세션. 언어 서버·디버그 클라이언트처럼 비동기 런타임 없이 스레드와 채널만 쓴다.
//!
//! 결과 집합은 한 번에 다 읽지 않는다. 첫 페이지를 보낸 뒤 작업 스레드는 커서를 쥔 채 다음 명령을
//! 기다리고, UI가 `fetch`하면 다음 페이지를 보낸다(100만 행도 메모리는 받은 페이지만큼만 쓴다).

pub mod config;
pub mod edit;
pub mod export;
mod mysql;
mod postgres;
pub mod result;
pub mod secret;
pub mod split;
mod sqlite;

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

pub use config::{ConnectionConfig, Kind};
pub use edit::Editable;

/// 한 번에 보내는 행 수.
pub const PAGE_ROWS: usize = 500;

/// 칸 하나. 드라이버가 글자로만 주는 값(PostgreSQL 커서, MariaDB 글자 프로토콜)은 `Text`다.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

impl std::fmt::Display for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cell::Null => f.write_str("NULL"),
            Cell::Int(n) => write!(f, "{n}"),
            Cell::Float(n) => write!(f, "{n}"),
            Cell::Text(text) => f.write_str(text),
            Cell::Bytes(bytes) => {
                f.write_str("0x")?;
                bytes.iter().try_for_each(|b| write!(f, "{b:02X}"))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
}

pub type Row = Vec<Cell>;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Connected {
        version: String,
    },
    /// 연결하지 못했거나 끊겼다. 이후 명령은 버려진다.
    Disconnected(String),
    /// 결과 집합의 열. 이어서 `Rows`가 온다.
    Columns(Vec<Column>),
    /// 방금 온 열의 결과를 고칠 수 있다(`Columns`와 첫 `Rows` 사이에 온다).
    Editable(Editable),
    /// 결과 페이지. `more`면 `fetch`로 다음 페이지를 받는다.
    Rows {
        rows: Vec<Row>,
        more: bool,
    },
    /// 문장 하나가 끝났다. 결과 집합이 아니면 바뀐 행 수.
    Done {
        affected: Option<u64>,
        elapsed: Duration,
    },
    Error(String),
    /// 커밋·롤백·자동 커밋 전환이 끝났다. `autocommit`이 꺼져 있으면 지금 트랜잭션 안이다.
    Transaction {
        autocommit: bool,
    },
}

enum Command {
    Execute(String),
    /// 파라미터를 묶어 실행한다(결과 집합이 아닌 문장).
    ExecuteParams(String, Vec<Cell>),
    Fetch,
    CloseResult,
    Commit,
    Rollback,
    Autocommit(bool),
}

/// 작업 스레드 쪽에서 결과를 보내고, 결과 집합을 연 동안 다음 명령을 기다린다.
pub(crate) struct Io {
    events: Sender<Event>,
    commands: Receiver<Command>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// 결과를 읽는 중에 온 다른 명령. 결과를 닫은 뒤 처리한다.
    deferred: Option<Command>,
}

impl Io {
    pub(crate) fn send(&self, event: Event) {
        let _ = self.events.send(event);
        (self.wake)();
    }

    /// 결과 페이지를 보낸다. 더 있으면 다음 명령을 기다려, 다음 페이지를 원하면 true를 돌려준다.
    /// 결과를 닫거나 다른 명령이 오면(그 명령은 미뤄 둔다) false.
    pub(crate) fn page(&mut self, rows: Vec<Row>, more: bool) -> bool {
        self.send(Event::Rows { rows, more });
        if !more {
            return false;
        }
        match self.commands.recv() {
            Ok(Command::Fetch) => true,
            Ok(Command::CloseResult) | Err(_) => false,
            Ok(other) => {
                self.deferred = Some(other);
                false
            }
        }
    }
}

/// 드라이버. 작업 스레드 안에서만 쓴다.
pub(crate) trait Driver {
    fn version(&mut self) -> String;
    /// 문장 하나를 실행한다. 결과 집합이면 `Columns`를 보내고 `io.page`로 페이지를 보낸다.
    /// 결과 집합이 아니면 바뀐 행 수를 돌려준다.
    fn execute(&mut self, sql: &str, io: &mut Io) -> Result<Option<u64>, String>;
    /// 파라미터를 묶어 결과 집합이 아닌 문장을 실행하고 바뀐 행 수를 돌려준다.
    fn execute_params(&mut self, sql: &str, params: &[Cell]) -> Result<u64, String>;
    fn commit(&mut self) -> Result<(), String>;
    fn rollback(&mut self) -> Result<(), String>;
    fn set_autocommit(&mut self, on: bool) -> Result<(), String>;
}

/// 다른 스레드에서 실행 중인 문장을 멈춘다.
pub(crate) type Canceller = Box<dyn Fn() + Send>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// DB 연결 하나. 버리면 작업 스레드가 연결을 닫는다.
pub struct Connection {
    commands: Sender<Command>,
    events: Receiver<Event>,
    cancel: Arc<Mutex<Option<Canceller>>>,
}

impl Connection {
    /// 별도 스레드에서 연결한다. 결과는 `Connected`나 `Disconnected`로 온다.
    /// `wake`는 이벤트가 생길 때마다 작업 스레드에서 불린다(UI를 다시 그리게).
    pub fn open(
        config: &ConnectionConfig,
        password: Option<String>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (commands, command_rx) = channel();
        let (event_tx, events) = channel();
        let cancel: Arc<Mutex<Option<Canceller>>> = Arc::new(Mutex::new(None));
        let (config, slot) = (config.clone(), cancel.clone());
        thread::spawn(move || {
            let mut io = Io {
                events: event_tx,
                commands: command_rx,
                wake: Arc::new(wake),
                deferred: None,
            };
            let connected = match config.kind {
                Kind::Sqlite => {
                    sqlite::connect(&config).map(|(d, c)| (Box::new(d) as Box<dyn Driver>, c))
                }
                Kind::Postgres => postgres::connect(&config, password.as_deref())
                    .map(|(d, c)| (Box::new(d) as Box<dyn Driver>, c)),
                Kind::MariaDb => mysql::connect(&config, password.as_deref())
                    .map(|(d, c)| (Box::new(d) as Box<dyn Driver>, c)),
            };
            let mut driver = match connected {
                Ok((driver, canceller)) => {
                    *lock(&slot) = Some(canceller);
                    driver
                }
                Err(e) => {
                    io.send(Event::Disconnected(e));
                    return;
                }
            };
            io.send(Event::Connected {
                version: driver.version(),
            });
            serve(driver.as_mut(), &mut io);
        });
        Self {
            commands,
            events,
            cancel,
        }
    }

    /// 문장 하나를 실행한다. 읽던 결과가 있으면 닫는다.
    pub fn execute(&self, sql: &str) {
        let _ = self.commands.send(Command::Execute(sql.to_owned()));
    }

    /// 파라미터를 묶어 결과 집합이 아닌 문장(UPDATE 등)을 실행한다. 값은 SQL 글에 붙지 않는다.
    /// 읽던 결과가 있으면 닫는다.
    pub fn execute_params(&self, sql: &str, params: Vec<Cell>) {
        let _ = self
            .commands
            .send(Command::ExecuteParams(sql.to_owned(), params));
    }

    /// 다음 결과 페이지를 받는다.
    pub fn fetch(&self) {
        let _ = self.commands.send(Command::Fetch);
    }

    /// 읽던 결과를 닫는다(남은 행을 버린다).
    pub fn close_result(&self) {
        let _ = self.commands.send(Command::CloseResult);
    }

    pub fn commit(&self) {
        let _ = self.commands.send(Command::Commit);
    }

    pub fn rollback(&self) {
        let _ = self.commands.send(Command::Rollback);
    }

    /// 자동 커밋을 켜고 끈다. 끄면 다음 문장부터 트랜잭션이고 `commit`·`rollback`으로 끝낸다.
    pub fn set_autocommit(&self, on: bool) {
        let _ = self.commands.send(Command::Autocommit(on));
    }

    /// 실행 중인 문장을 멈춘다(끝난 뒤면 아무 일도 없다). 멈춘 문장은 `Error`로 끝난다.
    pub fn cancel(&self) {
        if let Some(cancel) = lock(&self.cancel).as_ref() {
            cancel();
        }
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// 다음 이벤트를 `timeout`까지 기다린다(UI가 아닌 곳에서 쓴다). 작업 스레드가 끝났으면 `Disconnected`.
    pub fn next_event(&self, timeout: Duration) -> Option<Event> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                Some(Event::Disconnected("연결이 끝났습니다".into()))
            }
        }
    }
}

fn serve(driver: &mut dyn Driver, io: &mut Io) {
    let mut autocommit = true;
    loop {
        let command = match io.deferred.take() {
            Some(command) => command,
            None => match io.commands.recv() {
                Ok(command) => command,
                Err(_) => return,
            },
        };
        match command {
            Command::Execute(sql) => {
                let started = Instant::now();
                let result = driver.execute(&sql, io);
                finish(io, started, result);
            }
            Command::ExecuteParams(sql, params) => {
                let started = Instant::now();
                let result = driver.execute_params(&sql, &params).map(Some);
                finish(io, started, result);
            }
            // 열린 결과가 없을 때 온 것은 버린다.
            Command::Fetch | Command::CloseResult => {}
            Command::Commit | Command::Rollback | Command::Autocommit(_) => {
                let result = match command {
                    Command::Commit => driver.commit(),
                    Command::Rollback => driver.rollback(),
                    Command::Autocommit(on) => driver.set_autocommit(on).map(|()| {
                        autocommit = on;
                    }),
                    _ => Ok(()),
                };
                match result {
                    Ok(()) => io.send(Event::Transaction { autocommit }),
                    Err(e) => io.send(Event::Error(e)),
                }
            }
        }
    }
}

fn finish(io: &Io, started: Instant, result: Result<Option<u64>, String>) {
    match result {
        Ok(affected) => io.send(Event::Done {
            affected,
            elapsed: started.elapsed(),
        }),
        Err(e) => io.send(Event::Error(e)),
    }
}

/// 첫 낱말이 결과 집합을 돌려주는 조회인지(주석과 괄호는 건너뛴다).
pub(crate) fn is_query(sql: &str) -> bool {
    let mut rest = sql.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else if let Some(after) = rest.strip_prefix('(') {
            rest = after.trim_start();
        } else {
            break;
        }
    }
    let word: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(word.as_str(), "select" | "with" | "values" | "table")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 조회문을_알아본다() {
        assert!(is_query("SELECT 1"));
        assert!(is_query("  -- 주석\n/* 여러 줄 */ (select 1)"));
        assert!(is_query("with t as (select 1) select * from t"));
        assert!(!is_query("insert into t values (1)"));
        assert!(!is_query("explain select 1"));
        assert!(!is_query(""));
    }

    #[test]
    fn 칸을_글자로_보인다() {
        assert_eq!(Cell::Null.to_string(), "NULL");
        assert_eq!(Cell::Bytes(vec![0x0a, 0xff]).to_string(), "0x0AFF");
        assert_eq!(Cell::Text("한글".into()).to_string(), "한글");
    }
}
