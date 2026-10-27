//! 연결 설정. 사용자 설정 폴더의 `connections.toml`에 둔다. 비밀번호는 여기에 쓰지 않고 OS 키체인에
//! 둔다(`crate::secret`).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Sqlite,
    Postgres,
    #[serde(rename = "mariadb")]
    MariaDb,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Postgres, Kind::MariaDb, Kind::Sqlite];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Sqlite => "SQLite",
            Kind::Postgres => "PostgreSQL",
            Kind::MariaDb => "MariaDB·MySQL",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Kind::Sqlite => 0,
            Kind::Postgres => 5432,
            Kind::MariaDb => 3306,
        }
    }

    /// 서버에 접속하는 종류(SQLite는 파일).
    pub fn is_server(self) -> bool {
        self != Kind::Sqlite
    }

    /// 따옴표 안의 역슬래시가 이스케이프인지(MariaDB·MySQL 기본 설정).
    pub fn backslash_escapes(self) -> bool {
        self == Kind::MariaDb
    }

    /// 실행 계획을 보는 문장.
    pub fn explain(self, sql: &str) -> String {
        match self {
            Kind::Sqlite => format!("EXPLAIN QUERY PLAN {sql}"),
            Kind::Postgres | Kind::MariaDb => format!("EXPLAIN {sql}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    /// 바뀌지 않는 고유 값. 키체인 항목 이름으로 쓴다.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub user: String,
    /// SQLite 파일.
    #[serde(default)]
    pub path: Option<PathBuf>,
}

impl ConnectionConfig {
    pub fn new(kind: Kind) -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            kind,
            host: if kind.is_server() {
                "localhost".into()
            } else {
                String::new()
            },
            port: kind.default_port(),
            database: String::new(),
            user: String::new(),
            path: None,
        }
    }

    /// 목록에 보일 한 줄 요약.
    pub fn summary(&self) -> String {
        match self.kind {
            Kind::Sqlite => self
                .path
                .as_ref()
                .map_or_else(String::new, |p| p.display().to_string()),
            _ => format!(
                "{}@{}:{}/{}",
                self.user, self.host, self.port, self.database
            ),
        }
    }

    /// 저장하기 전에 고칠 것.
    pub fn problems(&self) -> Vec<&'static str> {
        let mut problems = Vec::new();
        if self.name.trim().is_empty() {
            problems.push("이름을 적으세요");
        }
        if self.kind.is_server() {
            if self.host.trim().is_empty() {
                problems.push("호스트를 적으세요");
            }
            if self.port == 0 {
                problems.push("포트를 적으세요");
            }
            if self.user.trim().is_empty() {
                problems.push("사용자를 적으세요");
            }
        } else if self.path.is_none() {
            problems.push("SQLite 파일을 고르세요");
        }
        problems
    }
}

/// 시각과 프로세스 안 순번으로 만든 고유 값.
pub fn new_id() -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{nanos:x}-{:x}-{:x}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    connection: Vec<ConnectionConfig>,
}

/// 연결 목록을 읽는다. 파일이 없으면 빈 목록.
pub fn load(path: &Path) -> Result<Vec<ConnectionConfig>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    toml::from_str::<File>(&text)
        .map(|f| f.connection)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// 연결 목록을 쓴다. 쓰다 끊겨도 앞 내용이 남도록 임시 파일에 쓴 뒤 바꾼다.
pub fn save(path: &Path, connections: &[ConnectionConfig]) -> io::Result<()> {
    let text = toml::to_string(&File {
        connection: connections.to_vec(),
    })
    .map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("toml.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 연결_목록을_쓰고_읽는다_비밀번호는_없다() {
        let dir = std::env::temp_dir().join(format!("ferrite-db-config-{}", std::process::id()));
        let path = dir.join("connections.toml");
        assert!(load(&path).unwrap().is_empty(), "파일이 없으면 빈 목록");
        let mut pg = ConnectionConfig::new(Kind::Postgres);
        pg.name = "운영 DB".into();
        pg.user = "egov".into();
        pg.database = "sample".into();
        let mut lite = ConnectionConfig::new(Kind::Sqlite);
        lite.name = "로컬".into();
        lite.path = Some(dir.join("a.db"));
        save(&path, &[pg.clone(), lite.clone()]).unwrap();
        assert_eq!(load(&path).unwrap(), [pg.clone(), lite]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("password"), "{text}");
        assert!(text.contains("kind = \"postgres\""), "{text}");
        assert_eq!(pg.summary(), "egov@localhost:5432/sample");
        assert_ne!(pg.id, ConnectionConfig::new(Kind::Postgres).id);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 빠진_값을_알린다() {
        let mut config = ConnectionConfig::new(Kind::MariaDb);
        assert_eq!(config.problems(), ["이름을 적으세요", "사용자를 적으세요"]);
        config.name = "a".into();
        config.user = "root".into();
        assert!(config.problems().is_empty());
        let lite = ConnectionConfig::new(Kind::Sqlite);
        assert!(lite.problems().contains(&"SQLite 파일을 고르세요"));
        assert!(load(Path::new("Cargo.toml")).is_err(), "다른 TOML은 오류");
    }
}
