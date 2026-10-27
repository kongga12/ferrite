//! JDBC 사이드카(Oracle·Tibero·CUBRID). 드라이버 jar는 로컬 Maven 저장소(`~/.m2/repository`)에서 [받기]와
//! 같은 판을 찾는다(CI는 `mvn dependency:get`으로 받아 둔다). Tibero jar는 공개 저장소가 없어
//! `FERRITE_TEST_TIBERO_JAR`로 알려 준다. JDK(javac)나 jar가 없으면 건너뛴다.
//!
//! 서버 시나리오는 `FERRITE_TEST_ORACLE`·`FERRITE_TEST_CUBRID`(호스트:포트:사용자:비밀번호:DB)가 있을
//! 때만 돈다. CI는 Linux 작업에서 서비스 컨테이너(Oracle XE 11g, CUBRID 11.2)를 띄워 알려 준다.

mod common;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Dialect, Session};
use ferrite_db::jdbc::{self, DOWNLOADS};
use ferrite_db::{Cell, ConnectionConfig, Kind};

fn setup() -> bool {
    let javac = format!("javac{}", std::env::consts::EXE_SUFFIX);
    let jdk = std::env::var_os("JAVA_HOME")
        .is_some_and(|home| Path::new(&home).join("bin").join(&javac).is_file())
        || Command::new("javac")
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success());
    if !jdk {
        eprintln!("JDK(javac)가 없어 건너뜁니다");
        return false;
    }
    jdbc::set_home(std::env::temp_dir().join("ferrite-jdbc-test"));
    true
}

/// [받기]와 같은 판의 jar. 하나라도 없으면 `None`.
fn drivers(kind: Kind) -> Option<Vec<PathBuf>> {
    if kind == Kind::Tibero {
        return std::env::var_os("FERRITE_TEST_TIBERO_JAR").map(|jar| vec![PathBuf::from(jar)]);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    let m2 = PathBuf::from(home).join(".m2").join("repository");
    let found = jdbc::candidates(kind, Some(&m2), None);
    DOWNLOADS
        .iter()
        .filter(|d| d.kind == kind)
        .map(|d| {
            found
                .iter()
                .find(|jar| jar.file_name().is_some_and(|name| name == d.file))
                .cloned()
        })
        .collect()
}

/// `var`(호스트:포트:사용자:비밀번호:DB)가 가리키는 서버. 없으면 건너뛴다.
fn server(kind: Kind, var: &str) -> Option<(ConnectionConfig, Option<String>)> {
    let Ok(value) = std::env::var(var) else {
        eprintln!("{var}가 없어 건너뜁니다");
        return None;
    };
    if !setup() {
        return None;
    }
    let parts: Vec<&str> = value.splitn(5, ':').collect();
    assert_eq!(parts.len(), 5, "{var}=호스트:포트:사용자:비밀번호:DB");
    let mut config = ConnectionConfig::new(kind);
    config.name = "시험".into();
    config.host = parts[0].into();
    config.port = parts[1].parse().unwrap();
    config.user = parts[2].into();
    config.database = parts[4].into();
    config.drivers = drivers(kind).unwrap_or_else(|| panic!("{kind:?} 드라이버 jar가 없습니다"));
    Some((config, Some(parts[3].to_owned()).filter(|p| !p.is_empty())))
}

/// 막 띄운 컨테이너는 한동안 연결을 받지 않으므로 될 때까지 다시 연결한다.
fn open(config: &ConnectionConfig, password: Option<String>) -> (Session, String) {
    let started = Instant::now();
    loop {
        match Session::try_open(config, password.clone()) {
            Ok(opened) => return opened,
            Err(e) if started.elapsed() < Duration::from_secs(180) => {
                eprintln!("다시 연결합니다: {e}");
                std::thread::sleep(Duration::from_secs(3));
            }
            Err(e) => panic!("연결 실패: {e}"),
        }
    }
}

/// 파라미터 바인딩, 이진 값, 읽기 전용, 닫을 때 커밋하지 않은 변경은 버린다.
fn jdbc_scenario(config: &ConnectionConfig, password: Option<String>, dialect: Dialect) {
    let (mut s, _) = open(config, password.clone());
    s.drop_table("ferrite_j");
    s.exec("CREATE TABLE ferrite_j (id INT PRIMARY KEY, name VARCHAR(50))");
    // 따옴표·세미콜론이 든 값도 파라미터라 그대로 들어간다.
    let insert = "INSERT INTO ferrite_j (id, name) VALUES (?, ?)";
    let values = vec![Cell::Text("1".into()), Cell::Text("it's; 한글".into())];
    assert_eq!(s.exec_params(insert, values), Ok(1));
    assert_eq!(s.exec_params(insert, vec![Cell::Int(2), Cell::Null]), Ok(1));
    let (editable, rows) = s.query_editable("SELECT id, name FROM ferrite_j ORDER BY id");
    assert!(
        editable.is_none(),
        "JDBC 결과는 아직 그리드에서 고치지 않는다"
    );
    assert_eq!(
        rows,
        [
            [Cell::Text("1".into()), Cell::Text("it's; 한글".into())],
            [Cell::Text("2".into()), Cell::Null]
        ]
    );
    let bytes = match dialect {
        Dialect::Oracle => "SELECT HEXTORAW('0AFF') FROM dual",
        Dialect::Standard => "SELECT X'0AFF'",
    };
    let (_, rows, _) = s.query(bytes);
    assert_eq!(rows[0][0], Cell::Bytes(vec![0x0a, 0xff]));

    // 트랜잭션 중에 연결을 닫으면 롤백한다(Oracle 드라이버는 그냥 닫으면 커밋한다).
    s.conn().set_autocommit(false);
    s.transaction();
    assert_eq!(s.exec("DELETE FROM ferrite_j"), Some(2));
    drop(s);
    let (mut s, _) = open(config, password);
    // 앞 연결의 행 잠금이 풀릴 때까지 기다렸다가 바뀐다(커밋됐다면 0행).
    assert_eq!(s.exec("UPDATE ferrite_j SET id = id"), Some(2));
    s.exec("DROP TABLE ferrite_j");
}

#[test]
fn oracle로_시나리오를_돈다() {
    let Some((config, password)) = server(Kind::Oracle, "FERRITE_TEST_ORACLE") else {
        return;
    };
    let (s, version) = open(&config, password.clone());
    common::scenario_with(
        s,
        &version,
        "Oracle Database",
        "SELECT COUNT(*) FROM all_objects a, all_objects b, all_objects c",
        Dialect::Oracle,
    );
    jdbc_scenario(&config, password.clone(), Dialect::Oracle);

    // 실행 계획은 두 문장(계획 세우기, 보기)이다. CLOB은 글자로 온다.
    let (mut s, _) = open(&config, password);
    let plan = Kind::Oracle.explain("SELECT * FROM dual").unwrap();
    s.exec(&plan[0]);
    let (_, rows, _) = s.query(&plan[1]);
    assert!(
        rows.iter().any(|row| row[0].to_string().contains("DUAL")),
        "{rows:?}"
    );
    assert_eq!(s.scalar("SELECT TO_CLOB('긴 글') FROM dual"), "긴 글");
}

#[test]
fn cubrid로_시나리오를_돈다() {
    let Some((config, password)) = server(Kind::Cubrid, "FERRITE_TEST_CUBRID") else {
        return;
    };
    let (s, version) = open(&config, password.clone());
    common::scenario_with(
        s,
        &version,
        "CUBRID ",
        "SELECT COUNT(*) FROM db_attribute a, db_attribute b, db_attribute c",
        Dialect::Standard,
    );
    jdbc_scenario(&config, password, Dialect::Standard);
}

#[test]
fn 드라이버를_싣고_없는_서버에는_드라이버의_오류를_보인다() {
    // jar가 없으면 Java를 띄우기 전에 알린다.
    let mut missing = ConnectionConfig::new(Kind::Oracle);
    missing.drivers = vec![PathBuf::from("없는-ojdbc.jar")];
    let error = Session::try_open(&missing, None).err().unwrap();
    assert!(error.contains("드라이버 jar가 없습니다"), "{error}");
    if !setup() {
        return;
    }
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    for kind in [Kind::Oracle, Kind::Tibero, Kind::Cubrid] {
        let Some(drivers) = drivers(kind) else {
            eprintln!("{kind:?} jar가 없어 건너뜁니다");
            continue;
        };
        let mut config = ConnectionConfig::new(kind);
        config.host = "127.0.0.1".into();
        config.port = port;
        config.database = "ferrite".into();
        config.user = "ferrite".into();
        config.drivers = drivers;
        let started = Instant::now();
        let error = Session::try_open(&config, Some("x".into())).err().unwrap();
        assert!(
            !error.contains("사이드카") && !error.contains("찾지 못했습니다"),
            "{kind:?}: 드라이버가 실린 뒤의 연결 오류여야 한다: {error}"
        );
        assert!(started.elapsed() < Duration::from_secs(30), "{kind:?}");
    }
    // 다른 DB의 jar면 드라이버 클래스가 없다고 알린다.
    if let Some(oracle) = drivers(Kind::Oracle) {
        let mut tibero = ConnectionConfig::new(Kind::Tibero);
        tibero.drivers = oracle;
        let error = Session::try_open(&tibero, None).err().unwrap();
        assert!(
            error.contains("드라이버 클래스를 찾지 못했습니다(com.tmax.tibero.jdbc.TbDriver)"),
            "{error}"
        );
    }
}
