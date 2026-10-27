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
    Oracle,
    Tibero,
    Cubrid,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Postgres,
        Kind::MariaDb,
        Kind::Oracle,
        Kind::Tibero,
        Kind::Cubrid,
        Kind::Sqlite,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Sqlite => "SQLite",
            Kind::Postgres => "PostgreSQL",
            Kind::MariaDb => "MariaDB·MySQL",
            Kind::Oracle => "Oracle",
            Kind::Tibero => "Tibero",
            Kind::Cubrid => "CUBRID",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Kind::Sqlite => 0,
            Kind::Postgres => 5432,
            Kind::MariaDb => 3306,
            Kind::Oracle => 1521,
            Kind::Tibero => 8629,
            Kind::Cubrid => 33000,
        }
    }

    /// 서버에 접속하는 종류(SQLite는 파일).
    pub fn is_server(self) -> bool {
        self != Kind::Sqlite
    }

    /// JDBC 사이드카로 여는 종류(드라이버 jar가 필요하다).
    pub fn is_jdbc(self) -> bool {
        matches!(self, Kind::Oracle | Kind::Tibero | Kind::Cubrid)
    }

    /// 연결 창의 "데이터베이스" 칸 이름.
    pub fn database_label(self) -> &'static str {
        match self {
            Kind::Oracle => "서비스 이름",
            _ => "데이터베이스",
        }
    }

    /// 따옴표 안의 역슬래시가 이스케이프인지(MariaDB·MySQL 기본 설정).
    pub fn backslash_escapes(self) -> bool {
        self == Kind::MariaDb
    }

    /// 실행 계획을 보는 문장들(차례로 실행한다). 아직 못 보는 종류는 `None`.
    pub fn explain(self, sql: &str) -> Option<Vec<String>> {
        Some(match self {
            Kind::Sqlite => vec![format!("EXPLAIN QUERY PLAN {sql}")],
            Kind::Postgres | Kind::MariaDb => vec![format!("EXPLAIN {sql}")],
            Kind::Oracle => vec![
                format!("EXPLAIN PLAN FOR {sql}"),
                "SELECT plan_table_output FROM TABLE(DBMS_XPLAN.DISPLAY())".into(),
            ],
            // Tibero는 시험할 서버가 없고, CUBRID는 계획을 문장이 아닌 드라이버 API로만 준다.
            Kind::Tibero | Kind::Cubrid => return None,
        })
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
    /// JDBC URL. 적으면 호스트·포트·데이터베이스 대신 쓴다(Oracle SID 등).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// JDBC 드라이버 jar(Oracle은 orai18n.jar도 함께 둘 수 있다).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drivers: Vec<PathBuf>,
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
            url: String::new(),
            drivers: Vec::new(),
        }
    }

    /// 목록에 보일 한 줄 요약.
    pub fn summary(&self) -> String {
        match self.kind {
            Kind::Sqlite => self
                .path
                .as_ref()
                .map_or_else(String::new, |p| p.display().to_string()),
            _ if !self.url.trim().is_empty() => format!("{} · {}", self.user, self.url.trim()),
            _ => format!(
                "{}@{}:{}/{}",
                self.user, self.host, self.port, self.database
            ),
        }
    }

    /// JDBC 종류의 URL. 적은 URL이 없으면 호스트·포트·데이터베이스로 만든다.
    pub fn jdbc_url(&self) -> String {
        let url = self.url.trim();
        if !url.is_empty() {
            return url.to_owned();
        }
        let (host, port, db) = (self.host.trim(), self.port, self.database.trim());
        match self.kind {
            // 서비스 이름. SID로만 여는 DB는 URL(`jdbc:oracle:thin:@호스트:포트:SID`)을 적는다.
            Kind::Oracle => format!("jdbc:oracle:thin:@//{host}:{port}/{db}"),
            Kind::Tibero => format!("jdbc:tibero:thin:@{host}:{port}:{db}"),
            // 사용자·비밀번호 자리는 비우고 따로 넘긴다.
            Kind::Cubrid => format!("jdbc:cubrid:{host}:{port}:{db}:::"),
            Kind::Sqlite | Kind::Postgres | Kind::MariaDb => String::new(),
        }
    }

    /// 저장하기 전에 고칠 것.
    pub fn problems(&self) -> Vec<&'static str> {
        let mut problems = Vec::new();
        if self.name.trim().is_empty() {
            problems.push("이름을 적으세요");
        }
        let url = self.url.trim();
        if self.kind.is_jdbc() && !url.is_empty() {
            if !url.starts_with("jdbc:") {
                problems.push("JDBC URL은 jdbc:로 시작해야 합니다");
            }
        } else if self.kind.is_server() {
            if self.host.trim().is_empty() {
                problems.push("호스트를 적으세요");
            }
            if self.port == 0 {
                problems.push("포트를 적으세요");
            }
            if self.kind.is_jdbc() && self.database.trim().is_empty() {
                problems.push(match self.kind {
                    Kind::Oracle => "서비스 이름(또는 JDBC URL)을 적으세요",
                    _ => "데이터베이스를 적으세요",
                });
            }
        } else if self.path.is_none() {
            problems.push("SQLite 파일을 고르세요");
        }
        if self.kind.is_server() && self.user.trim().is_empty() {
            problems.push("사용자를 적으세요");
        }
        if self.kind.is_jdbc() && self.drivers.is_empty() {
            problems.push("드라이버 jar를 고르세요");
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

    #[test]
    fn jdbc_url을_만들고_빠진_드라이버를_알린다() {
        let mut oracle = ConnectionConfig::new(Kind::Oracle);
        oracle.name = "운영".into();
        oracle.user = "egov".into();
        oracle.port = 1521;
        assert_eq!(
            oracle.problems(),
            [
                "서비스 이름(또는 JDBC URL)을 적으세요",
                "드라이버 jar를 고르세요"
            ]
        );
        oracle.database = "XE".into();
        oracle.drivers = vec![PathBuf::from("ojdbc6.jar")];
        assert!(oracle.problems().is_empty());
        assert_eq!(oracle.jdbc_url(), "jdbc:oracle:thin:@//localhost:1521/XE");
        // URL을 적으면 호스트·포트·이름 대신 쓴다(SID).
        oracle.url = " jdbc:oracle:thin:@db:1521:ORCL ".into();
        oracle.host.clear();
        oracle.database.clear();
        assert!(oracle.problems().is_empty());
        assert_eq!(oracle.jdbc_url(), "jdbc:oracle:thin:@db:1521:ORCL");
        assert_eq!(oracle.summary(), "egov · jdbc:oracle:thin:@db:1521:ORCL");
        oracle.url = "oracle:thin:@db".into();
        assert_eq!(oracle.problems(), ["JDBC URL은 jdbc:로 시작해야 합니다"]);

        let mut cubrid = ConnectionConfig::new(Kind::Cubrid);
        cubrid.database = "demodb".into();
        assert_eq!(cubrid.jdbc_url(), "jdbc:cubrid:localhost:33000:demodb:::");
        let mut tibero = ConnectionConfig::new(Kind::Tibero);
        tibero.database = "tibero".into();
        assert_eq!(tibero.jdbc_url(), "jdbc:tibero:thin:@localhost:8629:tibero");

        // 드라이버 목록은 저장하고, 비어 있으면 파일에 쓰지 않는다.
        let dir = std::env::temp_dir().join(format!("ferrite-db-jdbc-{}", std::process::id()));
        let path = dir.join("connections.toml");
        save(&path, &[oracle.clone(), cubrid.clone()]).unwrap();
        assert_eq!(load(&path).unwrap(), [oracle, cubrid]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("drivers").count(), 1, "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
