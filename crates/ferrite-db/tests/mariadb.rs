//! MariaDB·MySQL에 공통 시나리오를 돌린다. `FERRITE_TEST_MARIADB`(호스트:포트:사용자:비밀번호:DB)가 있을
//! 때만 돈다. CI는 Linux 작업에서 MariaDB 서비스 컨테이너를 띄워 알려 준다.

mod common;

use ferrite_db::{ConnectionConfig, Kind};

#[test]
fn mariadb로_시나리오를_돌린다() {
    let Ok(value) = std::env::var("FERRITE_TEST_MARIADB") else {
        eprintln!("FERRITE_TEST_MARIADB가 없어 건너뜁니다");
        return;
    };
    let parts: Vec<&str> = value.splitn(5, ':').collect();
    assert_eq!(
        parts.len(),
        5,
        "FERRITE_TEST_MARIADB=호스트:포트:사용자:비밀번호:DB"
    );
    let mut config = ConnectionConfig::new(Kind::MariaDb);
    config.name = "시험".into();
    config.host = parts[0].into();
    config.port = parts[1].parse().unwrap();
    config.user = parts[2].into();
    config.database = parts[4].into();
    let password = Some(parts[3].to_owned()).filter(|p| !p.is_empty());
    // KILL QUERY로 멈춘 SLEEP은 오류 없이 1을 돌려준다.
    common::scenario(
        &config,
        password.clone(),
        "MariaDB·MySQL ",
        "SELECT SLEEP(30)",
    );
    common::edit_scenario(&config, password.clone());
    common::schema_scenario(&config, password);
}
