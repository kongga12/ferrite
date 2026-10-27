//! JDBC 사이드카(Java)로 여는 DB: Oracle·Tibero·CUBRID. 연결마다 `java` 프로세스 하나를 띄워 표준
//! 입출력으로 한 줄에 JSON 하나씩 주고받는다(`sidecar/FerriteJdbc.java`). 사이드카는 처음 쓸 때 사용자
//! JDK의 javac로 컴파일해 둔다. 드라이버 jar는 Ferrite가 싣지 않고 사용자가 고르거나 받는다.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::edit::{Editable, Target};
use crate::origin::{self, Fold};
use crate::{
    Canceller, Cell, Column, ConnectionConfig, Driver, Event, Io, Kind, PAGE_ROWS, Row, lock,
};

const SOURCE: &str = include_str!("../sidecar/FerriteJdbc.java");
const MAIN: &str = "FerriteJdbc";
/// 사이드카가 끝났을 때 오류에 붙이는 표준 오류 줄 수.
const LOG_LINES: usize = 20;

static HOME: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 컴파일한 사이드카를 둘 폴더. 앱은 사용자 데이터 폴더를 준다. 정하지 않으면 프로세스마다 임시 폴더.
pub fn set_home(dir: PathBuf) {
    *lock(&HOME) = Some(dir);
}

fn home() -> PathBuf {
    lock(&HOME).clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("ferrite-jdbc-{}", std::process::id()))
    })
}

impl Kind {
    /// 드라이버 클래스(jar 안의 `java.sql.Driver` 구현).
    fn driver_class(self) -> &'static str {
        match self {
            Kind::Oracle => "oracle.jdbc.OracleDriver",
            Kind::Tibero => "com.tmax.tibero.jdbc.TbDriver",
            Kind::Cubrid => "cubrid.jdbc.driver.CUBRIDDriver",
            Kind::Sqlite | Kind::Postgres | Kind::MariaDb => "",
        }
    }

    /// 드라이버 연결 속성.
    fn driver_properties(self) -> Value {
        match self {
            // Oracle 드라이버는 문장을 멈출 때 TCP 긴급 데이터(OOB)를 보내는데, NAT·방화벽·Docker를 거치면
            // 닿지 않아 멈추지 않는다. 연결 안의 멈춤 신호로 보내게 한다.
            Kind::Oracle => json!({ "oracle.net.disableOob": "true" }),
            _ => json!({}),
        }
    }
}

/// JAVA_HOME의 bin, 없으면 PATH에서 찾는다.
fn find_tool(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let java_home = std::env::var_os("JAVA_HOME").map(|home| PathBuf::from(home).join("bin"));
    let path = std::env::var_os("PATH").unwrap_or_default();
    java_home
        .into_iter()
        .chain(std::env::split_paths(&path))
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// 소스 내용으로 정한 폴더 이름(소스가 바뀌면 다시 컴파일한다).
fn source_hash() -> String {
    let hash = SOURCE.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// 컴파일한 사이드카 폴더. 없으면 javac로 컴파일한다.
fn classes() -> Result<PathBuf, String> {
    static COMPILING: Mutex<()> = Mutex::new(());
    let _compiling = lock(&COMPILING);
    let home = home();
    let dir = home.join(source_hash());
    if dir.join(format!("{MAIN}.class")).is_file() {
        return Ok(dir);
    }
    let javac = find_tool("javac").ok_or(
        "JDBC 연결에는 JDK가 필요합니다. javac를 찾지 못했습니다(JAVA_HOME, PATH)".to_owned(),
    )?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let work = home.join(format!("build-{}-{nanos}", std::process::id()));
    let result = compile(&javac, &work, &dir);
    let _ = std::fs::remove_dir_all(&work);
    result.map(|()| dir)
}

fn compile(javac: &Path, work: &Path, dir: &Path) -> Result<(), String> {
    let io = |e: std::io::Error| format!("{}: {e}", work.display());
    std::fs::create_dir_all(work).map_err(io)?;
    let source = work.join(format!("{MAIN}.java"));
    std::fs::write(&source, SOURCE).map_err(io)?;
    let out = work.join("classes");
    let run = |release: &[&str]| -> Result<(), String> {
        let mut command = Command::new(javac);
        command
            .args(["-encoding", "UTF-8", "-nowarn"])
            .args(release)
            .arg("-d")
            .arg(&out)
            .arg(&source)
            .stdin(Stdio::null());
        hide_window(&mut command);
        let output = command
            .output()
            .map_err(|e| format!("{}: {e}", javac.display()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "JDBC 사이드카를 컴파일하지 못했습니다: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    };
    // Java 8로 돌려도 되게 컴파일한다. javac 8은 --release를 모르므로 그때는 그대로 컴파일한다.
    run(&["--release", "8", "-Xlint:-options"]).or_else(|_| run(&[]))?;
    match std::fs::rename(&out, dir) {
        Ok(()) => Ok(()),
        // 다른 프로세스가 먼저 둔 경우.
        Err(_) if dir.join(format!("{MAIN}.class")).is_file() => Ok(()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

pub(crate) struct Jdbc {
    child: Child,
    /// 닫으면(None) 사이드카가 연결을 닫고 끝난다. 멈춤도 여기에 쓴다.
    input: Arc<Mutex<Option<ChildStdin>>>,
    output: BufReader<ChildStdout>,
    /// 표준 오류의 마지막 줄들(사이드카가 갑자기 끝났을 때 보인다).
    log: Arc<Mutex<VecDeque<String>>>,
    version: String,
    kind: Kind,
}

pub(crate) fn connect(
    config: &ConnectionConfig,
    password: Option<&str>,
) -> Result<(Jdbc, Canceller), String> {
    if let Some(missing) = config.drivers.iter().find(|jar| !jar.is_file()) {
        return Err(format!("드라이버 jar가 없습니다: {}", missing.display()));
    }
    let java = find_tool("java")
        .ok_or("Java를 찾지 못했습니다(JAVA_HOME, PATH). JDBC 연결에는 JDK가 필요합니다")?;
    let classpath = std::env::join_paths(std::iter::once(classes()?).chain(config.drivers.clone()))
        .map_err(|e| format!("드라이버 jar 경로: {e}"))?;
    let mut command = Command::new(&java);
    command
        .arg("-cp")
        .arg(classpath)
        .arg(MAIN)
        .arg(PAGE_ROWS.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("{}: {e}", java.display()))?;
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        return Err("JDBC 사이드카의 입출력을 열지 못했습니다".into());
    };
    let log = Arc::new(Mutex::new(VecDeque::new()));
    let tail = log.clone();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            let mut tail = lock(&tail);
            if tail.len() == LOG_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    });
    let mut jdbc = Jdbc {
        child,
        input: Arc::new(Mutex::new(Some(stdin))),
        output: BufReader::new(stdout),
        log,
        version: String::new(),
        kind: config.kind,
    };
    let opened = jdbc.call(
        "open",
        json!({
            "driver": config.kind.driver_class(),
            "properties": config.kind.driver_properties(),
            "url": config.jdbc_url(),
            "user": config.user,
            "password": password,
        }),
    )?;
    jdbc.version = opened["version"].as_str().unwrap_or_default().to_owned();
    let input = jdbc.input.clone();
    let cancel: Canceller = Box::new(move || {
        if let Some(stdin) = lock(&input).as_mut() {
            let _ = writeln!(stdin, r#"{{"method":"cancel"}}"#).and_then(|()| stdin.flush());
        }
    });
    Ok((jdbc, cancel))
}

/// 칸 하나: null, 글자, 또는 이진 값 `{"hex": ...}`.
fn cell(value: &Value) -> Cell {
    match value {
        Value::Null => Cell::Null,
        Value::String(text) => Cell::Text(text.clone()),
        Value::Object(map) => {
            let hex = map.get("hex").and_then(Value::as_str).unwrap_or_default();
            Cell::Bytes(
                (0..hex.len() / 2)
                    .filter_map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok())
                    .collect(),
            )
        }
        other => Cell::Text(other.to_string()),
    }
}

/// 파라미터 값. 사이드카는 글자를 `setString`, null을 `setNull`, 이진 값을 `setBytes`로 묶는다.
fn param(cell: &Cell) -> Value {
    match cell {
        Cell::Null => Value::Null,
        Cell::Bytes(bytes) => {
            json!({ "hex": bytes.iter().map(|b| format!("{b:02x}")).collect::<String>() })
        }
        other => Value::String(other.to_string()),
    }
}

/// 페이지 응답의 행과 남은 행이 있는지.
fn page(value: &Value) -> (Vec<Row>, bool) {
    let rows = value["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    row.as_array()
                        .map_or_else(Vec::new, |r| r.iter().map(cell).collect())
                })
                .collect()
        })
        .unwrap_or_default();
    (rows, value["more"].as_bool().unwrap_or(false))
}

impl Jdbc {
    /// 요청 하나를 보내고 응답을 기다린다.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let request = json!({ "method": method, "params": params }).to_string();
        let sent = match lock(&self.input).as_mut() {
            Some(stdin) => writeln!(stdin, "{request}")
                .and_then(|()| stdin.flush())
                .is_ok(),
            None => false,
        };
        let mut line = String::new();
        if !sent || !matches!(self.output.read_line(&mut line), Ok(n) if n > 0) {
            return Err(self.ended());
        }
        let response: Value = serde_json::from_str(&line)
            .map_err(|e| format!("JDBC 사이드카의 응답을 읽지 못했습니다: {e}"))?;
        match response.get("error") {
            Some(error) => Err(error.as_str().unwrap_or("알 수 없는 오류").to_owned()),
            None => Ok(response.get("result").cloned().unwrap_or(Value::Null)),
        }
    }

    /// 조회가 한 표에서 왔고 그 표의 기본 키 열이 모두 결과에 있으면 고칠 수 있다. 표는 SQL에서 찾고
    /// (`origin`), 열·기본 키는 사이드카가 드라이버 메타데이터로 알려 준다. Tibero는 시험할 서버가 없어
    /// 아직 고치지 않는다.
    fn editable(&mut self, sql: &str, labels: &[String]) -> Option<Editable> {
        if !matches!(self.kind, Kind::Oracle | Kind::Cubrid) {
            return None;
        }
        let select = origin::parse(sql)?;
        let ident = |i: &origin::Ident| json!({ "text": i.text, "quoted": i.quoted });
        let info = self
            .call(
                "table",
                json!({ "schema": select.schema.as_ref().map(ident), "table": ident(&select.table) }),
            )
            .ok()
            .filter(|info| !info.is_null())?;
        let fold = |name: &str| match info[name].as_str() {
            Some("upper") => Fold::Upper,
            Some("lower") => Fold::Lower,
            _ => Fold::None,
        };
        // (열 이름, 형식)
        let columns: Vec<(String, Option<&str>)> = info["columns"]
            .as_array()?
            .iter()
            .filter_map(|c| Some((c["name"].as_str()?.to_owned(), c["type"].as_str())))
            .collect();
        let names: Vec<String> = columns.iter().map(|(name, _)| name.clone()).collect();
        let key: Vec<String> = info["key"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let targets = select
            .columns(labels, &names, fold("fold"), fold("quoted"))
            .into_iter()
            .map(|name| {
                let name = name?;
                // Oracle은 날짜를 보이는 글자에서 되돌리게 형식을 알린다(`edit::placeholder`).
                let cast = columns
                    .iter()
                    .find(|(n, _)| *n == name)
                    .and_then(|(_, ty)| *ty)
                    .filter(|_| self.kind == Kind::Oracle)
                    .map(str::to_owned);
                Some(Target { name, cast })
            })
            .collect();
        Editable::new(
            self.kind,
            info["schema"].as_str(),
            info["table"].as_str()?,
            targets,
            &key,
        )
    }

    /// 사이드카가 끝났을 때의 오류. 표준 오류의 마지막 줄을 붙인다.
    fn ended(&mut self) -> String {
        // 표준 오류를 다 읽을 틈을 잠깐 준다.
        let _ = self.child.try_wait();
        thread::sleep(Duration::from_millis(100));
        let log = lock(&self.log);
        let tail: Vec<&str> = log.iter().map(String::as_str).collect();
        if tail.is_empty() {
            "JDBC 사이드카가 끝났습니다".into()
        } else {
            format!("JDBC 사이드카가 끝났습니다: {}", tail.join("\n"))
        }
    }
}

impl Drop for Jdbc {
    fn drop(&mut self) {
        // 입력을 닫으면 사이드카가 (트랜잭션 중이면 롤백하고) 연결을 닫고 끝난다. 늦으면 끝낸다.
        lock(&self.input).take();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Driver for Jdbc {
    fn version(&mut self) -> String {
        self.version.clone()
    }

    fn execute(&mut self, sql: &str, io: &mut Io) -> Result<Option<u64>, String> {
        // JDBC는 끝의 세미콜론을 문장의 일부로 읽는다(Oracle은 ORA-00911).
        let sql = sql.trim().trim_end_matches(';').trim_end();
        let mut response = self.call("execute", json!({ "sql": sql }))?;
        let Some(columns) = response["columns"].as_array() else {
            return Ok(response["affected"].as_u64());
        };
        let labels: Vec<String> = columns
            .iter()
            .map(|name| name.as_str().unwrap_or_default().to_owned())
            .collect();
        io.send(Event::Columns(
            labels
                .iter()
                .map(|name| Column { name: name.clone() })
                .collect(),
        ));
        if let Some(editable) = self.editable(sql, &labels) {
            io.send(Event::Editable(editable));
        }
        loop {
            let (rows, more) = page(&response);
            if !io.page(rows, more) {
                if more {
                    self.call("close", json!({}))?;
                }
                return Ok(None);
            }
            response = self.call("fetch", json!({}))?;
        }
    }

    fn execute_params(&mut self, sql: &str, params: &[Cell]) -> Result<u64, String> {
        let params: Vec<Value> = params.iter().map(param).collect();
        let response = self.call("update", json!({ "sql": sql, "params": params }))?;
        Ok(response["affected"].as_u64().unwrap_or(0))
    }

    fn commit(&mut self) -> Result<(), String> {
        self.call("commit", json!({})).map(drop)
    }

    fn rollback(&mut self) -> Result<(), String> {
        self.call("rollback", json!({})).map(drop)
    }

    fn set_autocommit(&mut self, on: bool) -> Result<(), String> {
        self.call("autocommit", json!({ "on": on })).map(drop)
    }
}

/// 이 종류의 드라이버 jar 후보: 로컬 Maven 저장소(`m2`, 보통 `~/.m2/repository`)와 프로젝트의 lib
/// 폴더(`WEB-INF/lib` 등)에서 이름으로 찾는다. 소스·문서 jar는 뺀다.
pub fn candidates(kind: Kind, m2: Option<&Path>, project: Option<&Path>) -> Vec<PathBuf> {
    let groups: &[&str] = match kind {
        Kind::Oracle => &["com/oracle"],
        Kind::Tibero => &["com/tmax", "tibero"],
        Kind::Cubrid => &["cubrid"],
        Kind::Sqlite | Kind::Postgres | Kind::MariaDb => return Vec::new(),
    };
    let mut found = Vec::new();
    for group in groups {
        if let Some(m2) = m2 {
            walk(&m2.join(group), 5, kind, &mut found);
        }
    }
    if let Some(project) = project {
        for lib in [
            "src/main/webapp/WEB-INF/lib",
            "WebContent/WEB-INF/lib",
            "WEB-INF/lib",
            "lib",
            "libs",
        ] {
            walk(&project.join(lib), 0, kind, &mut found);
        }
    }
    found.sort_by(|a, b| a.file_name().cmp(&b.file_name()).then(a.cmp(b)));
    found.dedup();
    found
}

fn walk(dir: &Path, depth: usize, kind: Kind, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                walk(&path, depth - 1, kind, found);
            }
        } else if is_driver(kind, &entry.file_name().to_string_lossy()) {
            found.push(path);
        }
    }
}

/// 드라이버 jar 이름인지(대소문자 무시).
fn is_driver(kind: Kind, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let Some(stem) = name.strip_suffix(".jar") else {
        return false;
    };
    if stem.ends_with("-sources") || stem.ends_with("-javadoc") {
        return false;
    }
    match kind {
        Kind::Oracle => stem.starts_with("ojdbc") || stem.starts_with("orai18n"),
        Kind::Tibero => stem.starts_with("tibero") && stem.contains("jdbc"),
        Kind::Cubrid => stem.contains("cubrid") && stem.contains("jdbc"),
        Kind::Sqlite | Kind::Postgres | Kind::MariaDb => false,
    }
}

/// [받기]로 내려받는 드라이버. 크기와 SHA-256을 고정한다(저장소의 SHA-1과 맞는 jar에서 구했다).
pub struct Download {
    pub kind: Kind,
    pub file: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// Oracle 11g용 드라이버(Maven Central, Oracle Free Use Terms)와 CUBRID 11.2용 드라이버(CUBRID 저장소).
/// Tibero는 공개 저장소가 없어 jar를 직접 고른다.
pub const DOWNLOADS: [Download; 3] = [
    Download {
        kind: Kind::Oracle,
        file: "ojdbc6-11.2.0.4.jar",
        url: "https://repo1.maven.org/maven2/com/oracle/database/jdbc/ojdbc6/11.2.0.4/ojdbc6-11.2.0.4.jar",
        size: 2_739_670,
        sha256: "e70213917b5f0d7448072836da07c709930b89dd4b0cc14a1eef814836747900",
    },
    // 한글 문자셋(KO16MSWIN949 등) DB의 글자를 바꾸는 데 쓴다.
    Download {
        kind: Kind::Oracle,
        file: "orai18n-11.2.0.4.jar",
        url: "https://repo1.maven.org/maven2/com/oracle/database/nls/orai18n/11.2.0.4/orai18n-11.2.0.4.jar",
        size: 1_655_734,
        sha256: "0e3c0a0ceb08770e93466197c2da1a8ff9453616a0ed53f004050da541e4c785",
    },
    Download {
        kind: Kind::Cubrid,
        file: "cubrid-jdbc-11.2.0.0035.jar",
        url: "https://maven.cubrid.org/cubrid/cubrid-jdbc/11.2.0.0035/cubrid-jdbc-11.2.0.0035.jar",
        size: 282_027,
        sha256: "81248f3f73e9df5c61b392b4ddc5d9c0e52fce7522ec32a83635c38f7dac20eb",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 칸과_파라미터를_json으로_바꾼다() {
        let row = json!([null, "한글", { "hex": "0aff" }, 3]);
        let cells: Vec<Cell> = row.as_array().unwrap().iter().map(cell).collect();
        assert_eq!(
            cells,
            [
                Cell::Null,
                Cell::Text("한글".into()),
                Cell::Bytes(vec![0x0a, 0xff]),
                Cell::Text("3".into())
            ]
        );
        let params: Vec<Value> = cells.iter().map(param).collect();
        assert_eq!(
            params,
            [
                json!(null),
                json!("한글"),
                json!({ "hex": "0aff" }),
                json!("3")
            ]
        );
        assert_eq!(param(&Cell::Int(7)), json!("7"));
        let (rows, more) = page(&json!({ "rows": [["a", null]], "more": true }));
        assert_eq!(
            (rows, more),
            (vec![vec![Cell::Text("a".into()), Cell::Null]], true)
        );
    }

    #[test]
    fn 드라이버_jar를_이름으로_찾는다() {
        let root = std::env::temp_dir().join(format!("ferrite-jdbc-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (m2, project) = (root.join("m2"), root.join("project"));
        let files = [
            "m2/com/oracle/database/jdbc/ojdbc6/11.2.0.4/ojdbc6-11.2.0.4.jar",
            "m2/com/oracle/database/jdbc/ojdbc6/11.2.0.4/ojdbc6-11.2.0.4-sources.jar",
            "m2/com/oracle/database/nls/orai18n/11.2.0.4/orai18n-11.2.0.4.jar",
            "m2/com/oracle/database/jdbc/ucp/11.2.0.4/ucp-11.2.0.4.jar",
            "m2/cubrid/cubrid-jdbc/11.2.0.0035/cubrid-jdbc-11.2.0.0035.jar",
            "project/src/main/webapp/WEB-INF/lib/tibero6-jdbc.jar",
            "project/src/main/webapp/WEB-INF/lib/JDBC-9.3.9.0002-cubrid.jar",
            "project/src/main/webapp/WEB-INF/lib/OJDBC14.JAR",
        ];
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"").unwrap();
        }
        let names = |kind| -> Vec<String> {
            candidates(kind, Some(&m2), Some(&project))
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(
            names(Kind::Oracle),
            ["OJDBC14.JAR", "ojdbc6-11.2.0.4.jar", "orai18n-11.2.0.4.jar"]
        );
        assert_eq!(
            names(Kind::Cubrid),
            ["JDBC-9.3.9.0002-cubrid.jar", "cubrid-jdbc-11.2.0.0035.jar"]
        );
        assert_eq!(names(Kind::Tibero), ["tibero6-jdbc.jar"]);
        assert!(names(Kind::Postgres).is_empty());
        assert!(candidates(Kind::Oracle, None, None).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn 받을_드라이버는_크기와_해시를_고정한다() {
        for download in &DOWNLOADS {
            assert!(download.kind.is_jdbc());
            assert_eq!(download.sha256.len(), 64, "{}", download.file);
            assert!(download.url.starts_with("https://"));
            assert!(download.url.ends_with(download.file));
            assert!(is_driver(download.kind, download.file), "{}", download.file);
        }
    }
}
