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
