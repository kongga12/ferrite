//! PostgreSQL에 공통 시나리오를 돌린다. `FERRITE_TEST_POSTGRES`(호스트:포트:사용자:비밀번호:DB)가 있으면
//! 그 서버를, 없으면 PATH(또는 `PGBIN`, 흔한 설치 위치)의 initdb·pg_ctl로 임시 클러스터를 띄워 쓴다.
//! 어느 쪽도 없으면 로컬에서는 건너뛰고 CI에서는 실패한다.

mod common;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ferrite_db::{ConnectionConfig, Kind};

/// 끝나면 서버를 멈추고 폴더를 지운다.
struct Cluster {
    pg_ctl: PathBuf,
    dir: PathBuf,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        let _ = Command::new(&self.pg_ctl)
            .args(["-D"])
            .arg(self.dir.join("data"))
            .args(["-m", "immediate", "-w", "stop"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// initdb가 있는 폴더.
fn pg_bin() -> Option<PathBuf> {
    let exe = std::env::consts::EXE_SUFFIX;
    let has = |dir: &Path| dir.join(format!("initdb{exe}")).is_file();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut candidates: Vec<PathBuf> = std::env::split_paths(&path).collect();
    candidates.extend(std::env::var_os("PGBIN").map(PathBuf::from));
    for pattern in ["/usr/lib/postgresql", "/opt/homebrew/opt", "/usr/local/opt"] {
        if let Ok(entries) = std::fs::read_dir(pattern) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if pattern.ends_with("postgresql") || name.starts_with("postgresql") {
                    candidates.push(entry.path().join("bin"));
                }
            }
        }
    }
    candidates.into_iter().find(|dir| has(dir))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start_cluster(bin: &Path) -> (Cluster, ConnectionConfig) {
    let dir = std::env::temp_dir().join(format!("ferrite-db-pg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let data = dir.join("data");
    let exe = std::env::consts::EXE_SUFFIX;
    let initdb = Command::new(bin.join(format!("initdb{exe}")))
        .arg("-D")
        .arg(&data)
        .args(["-U", "ferrite", "--auth=trust", "-E", "UTF8", "--no-locale"])
        .output()
        .unwrap();
    assert!(
        initdb.status.success(),
        "initdb: {}",
        String::from_utf8_lossy(&initdb.stderr)
    );
    let port = free_port();
    // Unix 소켓은 기본 폴더(/var/run/postgresql)에 쓸 권한이 없을 수 있어(Ubuntu 러너) 임시 폴더에 둔다.
    // Windows는 소켓을 쓰지 않는다.
    let mut options = format!("-p {port} -c listen_addresses=127.0.0.1");
    if !cfg!(windows) {
        options.push_str(&format!(" -k {}", dir.display()));
    }
    let pg_ctl = bin.join(format!("pg_ctl{exe}"));
    let started = Command::new(&pg_ctl)
        .arg("-D")
        .arg(&data)
        .arg("-o")
        .arg(options)
        .arg("-l")
        .arg(dir.join("server.log"))
        .args(["-w", "start"])
        // 띄운 서버가 출력 파이프를 물려받아 닫지 않으므로(Windows) 출력을 받지 않는다. 로그는 파일에 남는다.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    let cluster = Cluster { pg_ctl, dir };
    assert!(
        started.success(),
        "pg_ctl 실패: {}",
        std::fs::read_to_string(cluster.dir.join("server.log")).unwrap_or_default()
    );
    let mut config = ConnectionConfig::new(Kind::Postgres);
    config.name = "시험".into();
    config.host = "127.0.0.1".into();
    config.port = port;
    config.user = "ferrite".into();
    config.database = "postgres".into();
    (cluster, config)
}

fn from_env(value: &str) -> (ConnectionConfig, Option<String>) {
    let parts: Vec<&str> = value.splitn(5, ':').collect();
    assert_eq!(
        parts.len(),
        5,
        "FERRITE_TEST_POSTGRES=호스트:포트:사용자:비밀번호:DB"
    );
    let mut config = ConnectionConfig::new(Kind::Postgres);
    config.name = "시험".into();
    config.host = parts[0].into();
    config.port = parts[1].parse().unwrap();
    config.user = parts[2].into();
    config.database = parts[4].into();
    (config, Some(parts[3].to_owned()).filter(|p| !p.is_empty()))
}

#[test]
fn postgresql로_시나리오를_돌린다() {
    let long = "SELECT pg_sleep(30)";
    if let Ok(value) = std::env::var("FERRITE_TEST_POSTGRES") {
        let (config, password) = from_env(&value);
        common::scenario(&config, password.clone(), "PostgreSQL ", long);
        common::postgres_extras(&config, password.clone());
        common::edit_scenario(&config, password.clone());
        common::schema_scenario(&config, password);
        return;
    }
    let Some(bin) = pg_bin() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 PostgreSQL(initdb)이 있어야 합니다"
        );
        eprintln!("PostgreSQL(initdb)이 없어 건너뜁니다");
        return;
    };
    let (_cluster, config) = start_cluster(&bin);
    common::scenario(&config, None, "PostgreSQL ", long);
    common::postgres_extras(&config, None);
    common::edit_scenario(&config, None);
    common::schema_scenario(&config, None);
}
