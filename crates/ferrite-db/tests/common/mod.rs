// 시험 파일마다 쓰는 함수가 다르다(SQLite 시험은 PostgreSQL 전용 함수를 쓰지 않는다).
#![allow(dead_code)]

//! 세 드라이버에 같은 시나리오를 돌린다: 넣기, 페이지로 읽기, 읽다 말고 다음 문장, 트랜잭션, 오류 뒤
//! 계속 쓰기, 실행 중 멈춤.

use std::time::{Duration, Instant};

use ferrite_db::{Cell, Column, Connection, ConnectionConfig, Editable, Event, PAGE_ROWS, Row};

pub struct Session {
    conn: Connection,
    seen: Vec<Event>,
    next: usize,
}

impl Session {
    pub fn open(config: &ConnectionConfig, password: Option<String>) -> (Self, String) {
        Self::try_open(config, password).unwrap_or_else(|e| panic!("연결 실패: {e}"))
    }

    pub fn try_open(
        config: &ConnectionConfig,
        password: Option<String>,
    ) -> Result<(Self, String), String> {
        let mut session = Self {
            conn: Connection::open(config, password, || {}),
            seen: Vec::new(),
            next: 0,
        };
        let event = session.wait(|e| matches!(e, Event::Connected { .. } | Event::Disconnected(_)));
        match event {
            Event::Connected { version } => Ok((session, version)),
            Event::Disconnected(e) => Err(e),
            other => Err(format!("{other:?}")),
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

    /// 조회해 고칠 수 있는지(열 다음, 첫 페이지 전에 온다)와 모든 행을 받는다.
    pub fn query_editable(&mut self, sql: &str) -> (Option<Editable>, Vec<Row>) {
        self.conn.execute(sql);
        if let Event::Error(e) = self.wait(|e| matches!(e, Event::Columns(_) | Event::Error(_))) {
            panic!("{sql}: {e}");
        }
        let editable = match self.wait(|e| matches!(e, Event::Editable(_) | Event::Rows { .. })) {
            Event::Editable(editable) => Some(editable),
            _ => {
                // 첫 페이지는 아래에서 다시 읽는다.
                self.next -= 1;
                None
            }
        };
        let mut rows = Vec::new();
        loop {
            match self.wait(|e| matches!(e, Event::Rows { .. } | Event::Error(_))) {
                Event::Rows { rows: page, more } => {
                    rows.extend(page);
                    if !more {
                        break;
                    }
                    self.conn.fetch();
                }
                other => panic!("{sql}: {other:?}"),
            }
        }
        self.wait(|e| matches!(e, Event::Done { .. }));
        (editable, rows)
    }

    /// 파라미터를 묶어 실행하고 바뀐 행 수를 돌려준다.
    pub fn exec_params(&mut self, sql: &str, params: Vec<Cell>) -> Result<u64, String> {
        self.conn.execute_params(sql, params);
        match self.wait(|e| matches!(e, Event::Done { .. } | Event::Error(_))) {
            Event::Done {
                affected: Some(n), ..
            } => Ok(n),
            Event::Error(e) => Err(e),
            other => panic!("{sql}: {other:?}"),
        }
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

/// DB마다 다른 SQL. Oracle 계열(Oracle 11g·Tibero)은 `FROM` 없는 SELECT와 여러 행 VALUES가 없다.
#[derive(Clone, Copy, PartialEq)]
pub enum Dialect {
    Standard,
    Oracle,
}

impl Dialect {
    /// `SELECT 1` 뒤에 붙일 것.
    pub fn from(self) -> &'static str {
        match self {
            Dialect::Standard => "",
            Dialect::Oracle => " FROM dual",
        }
    }

    /// `ferrite_t(id, name)`에 1..=n행을 넣는 문장 하나.
    fn fill(self, n: usize) -> String {
        match self {
            Dialect::Standard => {
                let values: Vec<String> = (1..=n).map(|i| format!("({i}, '이름{i}')")).collect();
                format!(
                    "INSERT INTO ferrite_t (id, name) VALUES {}",
                    values.join(",")
                )
            }
            Dialect::Oracle => format!(
                "INSERT INTO ferrite_t (id, name) SELECT level, '이름' || level FROM dual CONNECT BY level <= {n}"
            ),
        }
    }
}

impl Session {
    /// 표가 있으면 지운다(`DROP TABLE IF EXISTS`가 없는 DB도 있어 오류는 버린다).
    pub fn drop_table(&mut self, table: &str) {
        self.conn.execute(&format!("DROP TABLE {table}"));
        self.wait(|e| matches!(e, Event::Done { .. } | Event::Error(_)));
    }
}

/// `long_sql`은 몇 초 넘게 도는 문장(멈춤 확인용).
pub fn scenario(
    config: &ConnectionConfig,
    password: Option<String>,
    version_prefix: &str,
    long_sql: &str,
) {
    let (s, version) = Session::open(config, password);
    scenario_with(s, &version, version_prefix, long_sql, Dialect::Standard);
}

pub fn scenario_with(
    mut s: Session,
    version: &str,
    version_prefix: &str,
    long_sql: &str,
    dialect: Dialect,
) {
    assert!(version.starts_with(version_prefix), "{version}");
    let one = |n: u8| format!("SELECT {n}{}", dialect.from());

    s.drop_table("ferrite_t");
    s.exec("CREATE TABLE ferrite_t (id INT PRIMARY KEY, name VARCHAR(50))");
    assert_eq!(s.exec(&dialect.fill(1234)), Some(1234));

    // 500·500·234행 세 페이지로 받는다.
    let (columns, rows, pages) = s.query("SELECT id, name FROM ferrite_t ORDER BY id");
    // Oracle은 따옴표 없는 이름을 대문자로 둔다.
    let names: Vec<String> = columns.iter().map(|c| c.name.to_lowercase()).collect();
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
    assert_eq!(s.scalar(&one(1)), "1");

    // 실행 중인 문장을 멈춘다(오류로 끝나거나, SLEEP처럼 바로 끝난다).
    let started = Instant::now();
    s.conn().execute(long_sql);
    std::thread::sleep(Duration::from_millis(300));
    s.conn().cancel();
    s.wait(|e| matches!(e, Event::Error(_) | Event::Done { .. }));
    assert!(started.elapsed() < Duration::from_secs(10), "멈추지 않았다");
    assert_eq!(s.scalar(&one(2)), "2", "멈춘 뒤에도 쓴다");

    s.exec("DROP TABLE ferrite_t");
}

/// 스키마 브라우저: 연결의 기본 자리에 있는 표·뷰와 열을 읽는다.
pub fn schema_scenario(config: &ConnectionConfig, password: Option<String>) {
    let (mut s, _) = Session::open(config, password);
    s.conn().execute("DROP VIEW ferrite_sv");
    s.wait(|e| matches!(e, Event::Done { .. } | Event::Error(_)));
    s.drop_table("ferrite_s");
    s.exec("CREATE TABLE ferrite_s (id INT PRIMARY KEY, name VARCHAR(30))");
    s.exec("CREATE VIEW ferrite_sv AS SELECT id FROM ferrite_s");
    s.conn().load_schema();
    let tables = match s.wait(|e| matches!(e, Event::Schema(_) | Event::Error(_))) {
        Event::Schema(tables) => tables,
        other => panic!("스키마: {other:?}"),
    };
    let find = |name: &str| {
        tables
            .iter()
            .find(|t| t.schema.is_none() && t.name.eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("{name}가 없다: {tables:?}"))
    };
    let names = |table: &ferrite_db::Table| -> Vec<String> {
        table
            .columns
            .iter()
            .map(|c| c.name.to_lowercase())
            .collect()
    };
    let table = find("ferrite_s");
    assert!(!table.view);
    assert_eq!(names(table), ["id", "name"]);
    assert!(
        table.columns[1].ty.to_lowercase().contains("char"),
        "{:?}",
        table.columns
    );
    let view = find("ferrite_sv");
    assert!(view.view);
    assert_eq!(names(view), ["id"]);
    s.exec("DROP VIEW ferrite_sv");
    s.exec("DROP TABLE ferrite_s");
}

/// 결과 그리드 고치기: 기본 키로 찾는 UPDATE를 파라미터로 실행한다.
pub fn edit_scenario(config: &ConnectionConfig, password: Option<String>) {
    let (mut s, _) = Session::open(config, password);
    s.drop_table("ferrite_e");
    s.drop_table("ferrite_n");
    s.exec(
        "CREATE TABLE ferrite_e (k1 INT NOT NULL, k2 VARCHAR(10) NOT NULL, v VARCHAR(20), n INT, \
         PRIMARY KEY (k1, k2))",
    );
    // Oracle 11g는 여러 행 VALUES가 없다.
    for row in [
        "1, 'a', '처음', 10",
        "1, 'b', '둘째', 20",
        "2, 'a', '셋째', 30",
    ] {
        s.exec(&format!("INSERT INTO ferrite_e VALUES ({row})"));
    }
    s.exec("CREATE TABLE ferrite_n (v VARCHAR(20))");

    // 기본 키 열이 다 있으면 고칠 수 있고, 식 열은 못 고친다.
    let (editable, rows) =
        s.query_editable("SELECT k2, v, n, n + 1 AS m, k1 FROM ferrite_e ORDER BY k1, k2");
    let editable = editable.expect("기본 키 열이 다 있으면 고칠 수 있다");
    assert!(
        editable.table.to_lowercase().ends_with("ferrite_e"),
        "{}",
        editable.table
    );
    assert_eq!(editable.key().len(), 2);
    assert!(editable.can_edit(1) && editable.can_edit(2) && !editable.can_edit(3));
    let mut update = |column: usize, value: Cell, row: usize| {
        let (sql, params) = editable.update(column, value, &rows[row]).unwrap();
        s.exec_params(&sql, params)
    };

    // 따옴표·세미콜론이 든 값도 파라미터라 그대로 들어간다. 그 행(1, 'b')만 바뀐다.
    assert_eq!(update(1, Cell::Text("it's; 값".into()), 1), Ok(1));
    // 글자로 적은 수도 열 형식으로 들어가고, NULL로도 고친다.
    assert_eq!(update(2, Cell::Text("7".into()), 1), Ok(1));
    assert_eq!(update(2, Cell::Null, 0), Ok(1));
    // 같은 값으로 고쳐도 찾은 행 1개다.
    assert_eq!(update(1, Cell::Text("셋째".into()), 2), Ok(1));
    let (_, after, _) = s.query("SELECT v, n FROM ferrite_e ORDER BY k1, k2");
    let texts: Vec<Vec<String>> = after
        .iter()
        .map(|row| row.iter().map(ToString::to_string).collect())
        .collect();
    assert_eq!(texts, [["처음", "NULL"], ["it's; 값", "7"], ["셋째", "30"]]);

    // 다른 곳에서 지운 행은 0행이다.
    s.exec("DELETE FROM ferrite_e WHERE k1 = 2");
    let (sql, params) = editable
        .update(1, Cell::Text("x".into()), &rows[2])
        .unwrap();
    assert_eq!(s.exec_params(&sql, params), Ok(0));

    // 트랜잭션 안에서 고치고 롤백하면 돌아온다. 틀린 값으로 실패해도 트랜잭션은 계속 쓴다.
    s.conn().set_autocommit(false);
    s.transaction();
    let (sql, params) = editable
        .update(1, Cell::Text("롤백될 값".into()), &rows[0])
        .unwrap();
    assert_eq!(s.exec_params(&sql, params), Ok(1));
    let (sql, params) = editable
        .update(2, Cell::Text("수가 아님".into()), &rows[0])
        .unwrap();
    let _ = s.exec_params(&sql, params);
    assert_eq!(
        s.scalar("SELECT COUNT(*) FROM ferrite_e"),
        "2",
        "트랜잭션이 살아 있다"
    );
    s.conn().rollback();
    s.transaction();
    s.conn().set_autocommit(true);
    s.transaction();
    assert_eq!(
        s.scalar("SELECT v FROM ferrite_e WHERE k1 = 1 AND k2 = 'a'"),
        "처음"
    );

    // 기본 키가 없거나, 결과에 빠졌거나, 두 표에서 온 결과는 고칠 수 없다.
    assert!(s.query_editable("SELECT v FROM ferrite_n").0.is_none());
    assert!(s.query_editable("SELECT k1, v FROM ferrite_e").0.is_none());
    assert!(
        s.query_editable("SELECT e.k1, e.k2, x.v FROM ferrite_e e, ferrite_n x")
            .0
            .is_none()
    );
    s.exec("DROP TABLE ferrite_e");
    s.exec("DROP TABLE ferrite_n");
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
