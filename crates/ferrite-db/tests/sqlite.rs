//! SQLite 파일에 공통 시나리오를 돌린다.

mod common;

use ferrite_db::{ConnectionConfig, Kind};

#[test]
fn sqlite_파일로_시나리오를_돌린다() {
    let dir = std::env::temp_dir().join(format!("ferrite-db-sqlite-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut config = ConnectionConfig::new(Kind::Sqlite);
    config.name = "시험".into();
    config.path = Some(dir.join("시험.db"));
    common::scenario(
        &config,
        None,
        "SQLite ",
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sqlite_결과를_고친다() {
    let (dir, config) = temp_config("edit");
    common::edit_scenario(&config, None);
    let _ = std::fs::remove_dir_all(&dir);
}

fn temp_config(tag: &str) -> (std::path::PathBuf, ConnectionConfig) {
    let dir = std::env::temp_dir().join(format!("ferrite-db-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut config = ConnectionConfig::new(Kind::Sqlite);
    config.name = "시험".into();
    config.path = Some(dir.join("시험.db"));
    (dir, config)
}

/// 100만 행을 끝까지 내려받아도 그리드 버퍼는 상한(10만 행)을 넘지 않는다.
#[test]
fn 백만_행을_내려받아도_버퍼는_상한을_넘지_않는다() {
    use ferrite_db::result::{ROW_LIMIT, ResultBuffer};
    use ferrite_db::{Cell, Event};
    use std::time::Duration;

    const TOTAL: usize = 1_000_000;
    let (dir, config) = temp_config("million");
    let conn = ferrite_db::Connection::open(&config, None, || {});
    conn.execute(&format!(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c LIMIT {TOTAL}) SELECT x, 'row ' || x FROM c"
    ));
    let mut buffer = None;
    loop {
        match conn.next_event(Duration::from_secs(60)).expect("시간 초과") {
            Event::Columns(columns) => buffer = Some(ResultBuffer::new(columns, ROW_LIMIT)),
            Event::Rows { rows, more } => {
                let buffer = buffer.as_mut().unwrap();
                buffer.push(rows, more);
                assert!(buffer.len() <= ROW_LIMIT, "{}행을 들고 있다", buffer.len());
                // 화면이 늘 맨 아래를 보고 있다고 치고 다음 페이지를 받는다.
                if buffer.wants_more(buffer.len()) {
                    conn.fetch();
                }
            }
            Event::Done { .. } => break,
            Event::Connected { .. } => {}
            other => panic!("{other:?}"),
        }
    }
    let buffer = buffer.unwrap();
    assert_eq!((buffer.received(), buffer.len()), (TOTAL, ROW_LIMIT));
    assert_eq!(buffer.dropped(), TOTAL - ROW_LIMIT);
    assert_eq!(
        buffer.row(0).unwrap()[0],
        Cell::Int((TOTAL - ROW_LIMIT + 1) as i64)
    );
    assert_eq!(
        buffer.row(ROW_LIMIT - 1).unwrap()[0],
        Cell::Int(TOTAL as i64)
    );
    drop(conn);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 조회를_csv로_내보낸다() {
    use ferrite_db::export::export_csv;
    use std::sync::atomic::AtomicBool;

    let (dir, config) = temp_config("csv");
    fill_table(&config);
    let path = dir.join("결과.csv");
    let stop = AtomicBool::new(false);
    let progress = std::cell::Cell::new(0);
    let written = export_csv(
        &config,
        None,
        "SELECT id, name, note FROM t ORDER BY id",
        &path,
        &stop,
        &|n| progress.set(n),
    )
    .unwrap();
    assert_eq!((written, progress.get()), (1203, 1203));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    assert_eq!(lines.next(), Some("\u{feff}id,name,note"));
    assert_eq!(lines.next(), Some("1,\"김, 철수\",\"say \"\"hi\"\"\""));
    assert_eq!(lines.next(), Some("2,이름2,"), "NULL은 빈칸");
    assert!(!dir.join("결과.csv.part").exists());

    // 실패하면 쓰던 파일을 남기지 않는다.
    let bad = dir.join("실패.csv");
    assert!(export_csv(&config, None, "SELECT * FROM 없는_표", &bad, &stop, &|_| {}).is_err());
    assert!(!bad.exists() && !dir.join("실패.csv.part").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 표 t(id, name, note)에 1203행을 넣는다.
fn fill_table(config: &ConnectionConfig) {
    use ferrite_db::Event;
    use std::time::Duration;

    let conn = ferrite_db::Connection::open(config, None, || {});
    let mut values = vec![
        "(1, '김, 철수', 'say \"hi\"')".to_owned(),
        "(2, '이름2', NULL)".to_owned(),
    ];
    values.extend((3..=1203).map(|i| format!("({i}, '이름{i}', 'n')")));
    for sql in [
        "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, note TEXT)".to_owned(),
        format!("INSERT INTO t VALUES {}", values.join(",")),
    ] {
        conn.execute(&sql);
        loop {
            match conn.next_event(Duration::from_secs(30)).expect("시간 초과") {
                Event::Done { .. } => break,
                Event::Error(e) | Event::Disconnected(e) => panic!("{sql}: {e}"),
                _ => {}
            }
        }
    }
}
