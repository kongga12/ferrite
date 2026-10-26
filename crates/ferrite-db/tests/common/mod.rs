// 시험 파일마다 쓰는 함수가 다르다(SQLite 시험은 PostgreSQL 전용 함수를 쓰지 않는다).
#![allow(dead_code)]

//! 세 드라이버에 같은 시나리오를 돌린다: 넣기, 페이지로 읽기, 읽다 말고 다음 문장, 트랜잭션, 오류 뒤
//! 계속 쓰기, 실행 중 멈춤.

use std::time::{Duration, Instant};

use ferrite_db::{Cell, Column, Connection, ConnectionConfig, Event, PAGE_ROWS, Row};

pub struct Session {
    conn: Connection,
    seen: Vec<Event>,
    next: usize,
}

impl Session {
    pub fn open(config: &ConnectionConfig, password: Option<String>) -> (Self, String) {
        let mut session = Self {
            conn: Connection::open(config, password, || {}),
            seen: Vec::new(),
            next: 0,
        };
        let event = session.wait(|e| matches!(e, Event::Connected { .. } | Event::Disconnected(_)));
        match event {
            Event::Connected { version } => (session, version),
            other => panic!("연결 실패: {other:?}"),
        }
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 아직 보지 않은 이벤트 중 `done`이 참인 첫 이벤트(최대 60초).
    pub fn wait(&mut self, done: impl Fn(&Event) -> bool) -> Event {
        let started = Instant::now();
        loop {
            if let Some(offset) = self.seen[self.next..].iter().position(&done) {
                self.next += offset + 1;
                return self.seen[self.next - 1].clone();
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "시간 초과. 받은 이벤트: {:?}",
                &self.seen[self.next..]
            );
            self.seen.extend(self.conn.poll());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// 결과 집합이 아닌 문장을 실행하고 바뀐 행 수를 돌려준다.
    pub fn exec(&mut self, sql: &str) -> Option<u64> {
        self.conn.execute(sql);
        match self.wait(|e| matches!(e, Event::Done { .. } | Event::Error(_))) {
            Event::Done { affected, .. } => affected,
            other => panic!("{sql}: {other:?}"),
        }
    }

    /// 조회해 모든 페이지를 받는다. (열, 행, 페이지 수)
    pub fn query(&mut self, sql: &str) -> (Vec<Column>, Vec<Row>, usize) {
        self.conn.execute(sql);
        let columns = match self.wait(|e| matches!(e, Event::Columns(_) | Event::Error(_))) {
            Event::Columns(columns) => columns,
            other => panic!("{sql}: {other:?}"),
        };
        let (mut rows, mut pages) = (Vec::new(), 0);
        loop {
            match self.wait(|e| matches!(e, Event::Rows { .. } | Event::Error(_))) {
                Event::Rows { rows: page, more } => {
                    rows.extend(page);
                    pages += 1;
                    if !more {
                        break;
                    }
                    self.conn.fetch();
                }
                other => panic!("{sql}: {other:?}"),
            }
        }
        self.wait(|e| matches!(e, Event::Done { .. }));
        (columns, rows, pages)
    }

    /// 한 칸짜리 결과를 글자로.
    pub fn scalar(&mut self, sql: &str) -> String {
        let (_, rows, _) = self.query(sql);
        rows[0][0].to_string()
    }

    pub fn transaction(&mut self) -> bool {
        match self.wait(|e| matches!(e, Event::Transaction { .. } | Event::Error(_))) {
            Event::Transaction { autocommit } => autocommit,
            other => panic!("트랜잭션: {other:?}"),
        }
    }
}

/// `long_sql`은 몇 초 넘게 도는 문장(멈춤 확인용).
pub fn scenario(
    config: &ConnectionConfig,
    password: Option<String>,
    version_prefix: &str,
    long_sql: &str,
) {
    let (mut s, version) = Session::open(config, password);
    assert!(version.starts_with(version_prefix), "{version}");

    s.exec("DROP TABLE IF EXISTS ferrite_t");
    s.exec("CREATE TABLE ferrite_t (id INT PRIMARY KEY, name VARCHAR(50))");
    let values: Vec<String> = (1..=1234).map(|i| format!("({i}, '이름{i}')")).collect();
    assert_eq!(
        s.exec(&format!(
            "INSERT INTO ferrite_t (id, name) VALUES {}",
            values.join(",")
        )),
        Some(1234)
    );

    // 500·500·234행 세 페이지로 받는다.
    let (columns, rows, pages) = s.query("SELECT id, name FROM ferrite_t ORDER BY id");
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name"]);
    assert_eq!((rows.len(), pages), (1234, 1234usize.div_ceil(PAGE_ROWS)));
    assert_eq!(
        (rows[0][0].to_string(), rows[0][1].clone()),
        ("1".into(), Cell::Text("이름1".into()))
    );
    assert_eq!(rows[1233][0].to_string(), "1234");

    // 첫 페이지만 받고 다음 문장을 실행하면 읽던 결과를 닫고 실행한다.
    s.conn().execute("SELECT id FROM ferrite_t ORDER BY id");
    s.wait(|e| matches!(e, Event::Rows { more: true, .. }));
    assert_eq!(s.scalar("SELECT COUNT(*) FROM ferrite_t"), "1234");

    // 자동 커밋을 끄면 롤백·커밋이 된다.
    s.conn().set_autocommit(false);
    assert!(!s.transaction());
    assert_eq!(s.exec("DELETE FROM ferrite_t WHERE id > 1000"), Some(234));
    s.conn().rollback();
    s.transaction();
    assert_eq!(s.scalar("SELECT COUNT(*) FROM ferrite_t"), "1234", "롤백");
    assert_eq!(s.exec("DELETE FROM ferrite_t WHERE id > 1000"), Some(234));
    s.conn().commit();
    s.transaction();
    s.conn().set_autocommit(true);
    assert!(s.transaction());
    assert_eq!(s.scalar("SELECT COUNT(*) FROM ferrite_t"), "1000", "커밋");

    // 오류 뒤에도 연결을 쓴다.
    s.conn().execute("SELECT * FROM ferrite_없는_표");
    assert!(matches!(
        s.wait(|e| matches!(e, Event::Error(_) | Event::Done { .. })),
        Event::Error(_)
    ));
    assert_eq!(s.scalar("SELECT 1"), "1");

    // 실행 중인 문장을 멈춘다(오류로 끝나거나, SLEEP처럼 바로 끝난다).
    let started = Instant::now();
    s.conn().execute(long_sql);
    std::thread::sleep(Duration::from_millis(300));
    s.conn().cancel();
    s.wait(|e| matches!(e, Event::Error(_) | Event::Done { .. }));
    assert!(started.elapsed() < Duration::from_secs(10), "멈추지 않았다");
    assert_eq!(s.scalar("SELECT 2"), "2", "멈춘 뒤에도 쓴다");

    s.exec("DROP TABLE ferrite_t");
}

/// PostgreSQL만: 트랜잭션 안에서 조회가 실패해도 트랜잭션을 계속 쓰고, 데이터를 바꾸는 WITH도 실행한다.
pub fn postgres_extras(config: &ConnectionConfig, password: Option<String>) {
    let (mut s, _) = Session::open(config, password);
    s.exec("DROP TABLE IF EXISTS ferrite_x");
    s.exec("CREATE TABLE ferrite_x (id INT)");
    s.conn().set_autocommit(false);
    s.transaction();
    s.exec("INSERT INTO ferrite_x VALUES (1)");
    s.conn().execute("SELECT 1 / 0");
    assert!(matches!(
        s.wait(|e| matches!(e, Event::Error(_) | Event::Done { .. })),
        Event::Error(_)
    ));
    assert_eq!(
        s.scalar("SELECT COUNT(*) FROM ferrite_x"),
        "1",
        "트랜잭션이 중단되지 않았다"
    );
    s.conn().commit();
    s.transaction();
    s.conn().set_autocommit(true);
    s.transaction();
    let (_, rows, _) =
        s.query("WITH n AS (INSERT INTO ferrite_x VALUES (2) RETURNING id) SELECT id FROM n");
    assert_eq!(rows[0][0].to_string(), "2");
    assert_eq!(s.scalar("SELECT COUNT(*) FROM ferrite_x"), "2");
    s.exec("DROP TABLE ferrite_x");
}
