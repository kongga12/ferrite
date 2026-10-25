//! 프로젝트에 맞춘 기본값(실행환경 버전, DB, JUnit, 인코딩, 폴더, 패키지)과 만든 파일 쓰기.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ferrite_core::text_file::{self, Encoding, LineEnding, TextContent};

use crate::config::{ConfigLayout, Placeholders};
use crate::crud::{Db, GeneratedFile, JUnit, Layout, Rte};

/// 훑을 때 건너뛰는 폴더(빌드 결과물·도구 폴더).
const SKIP_DIRS: [&str; 10] = [
    ".git",
    ".gradle",
    ".idea",
    ".settings",
    ".vscode",
    "bin",
    "build",
    "node_modules",
    "out",
    "target",
];
/// 인코딩·패키지를 보려고 읽는 Java 파일 수의 상한(큰 프로젝트에서도 빨리 끝나게).
const MAX_JAVA_SAMPLES: usize = 400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defaults {
    pub rte: Rte,
    pub db: Db,
    pub junit: JUnit,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
    pub layout: Layout,
    /// 새 기능 패키지의 앞부분(`egovframework.example`). 뒤에 기능 이름을 붙인다.
    pub base_package: String,
    /// 알려 줄 것(예: `mapperLocations`가 파일을 하나씩 적는 방식이라 새 매퍼를 목록에 더해야 함).
    pub notes: Vec<String>,
    /// 설정 파일을 둘 곳(Spring XML 폴더 등).
    pub config_layout: ConfigLayout,
    /// 프로젝트의 데이터소스 프로퍼티 키(설정 생성의 기본값을 `${키}`로).
    pub placeholders: Placeholders,
}

/// 프로젝트를 훑어 기본값을 정한다. 단서가 없으면 egov 4.x·Oracle·UTF-8이다.
pub fn defaults(root: &Path) -> Defaults {
    let mut scan = Scan::default();
    scan.walk(root, root);
    let build = scan.build.to_ascii_lowercase();
    let rte = if build.contains("org.egovframe.rte") || scan.imports_v4 > scan.imports_v3 {
        Rte::V4
    } else if build.contains("egovframework.rte") || scan.imports_v3 > 0 {
        Rte::V3
    } else {
        Rte::V4
    };
    let junit = if build.contains("junit-jupiter")
        || (rte == Rte::V4 && !build.contains("<artifactid>junit</artifactid>"))
    {
        JUnit::V5
    } else {
        JUnit::V4
    };
    let encoding = source_encoding(&scan.build).unwrap_or(if scan.cp949 > scan.utf8 {
        Encoding::Cp949
    } else {
        Encoding::Utf8
    });
    let line_ending = if scan.crlf > scan.lf {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    let mut layout = Layout::default();
    if root.join("src/main/java").is_dir() {
        layout.java = "src/main/java".into();
    } else if root.join("src").is_dir() {
        // 예전 Eclipse 동적 웹 프로젝트.
        layout.java = "src".into();
    }
    let mut notes = Vec::new();
    // MyBatis가 읽는 곳(`mapperLocations`)을 먼저 따른다. 없으면 매퍼가 가장 많은 폴더.
    let resources = if root.join("src/main/resources").is_dir() {
        "src/main/resources"
    } else {
        "src"
    };
    let configured = scan
        .mapper_locations
        .iter()
        .find_map(|value| mapper_location(value, resources));
    let existing = scan.mapper_layout();
    match (configured, existing) {
        (Some((dir, file)), existing) => {
            layout.mapper = dir;
            // `*.xml`처럼 이름을 가리지 않으면 기존 매퍼의 이름 모양을 따른다.
            layout.mapper_file = if file == "{class}.xml" {
                existing.map_or(file, |(_, pattern)| pattern)
            } else {
                file
            };
        }
        (None, Some((dir, file))) => {
            layout.mapper = dir;
            layout.mapper_file = file;
            if !scan.mapper_locations.is_empty() {
                notes.push(
                    "mapperLocations가 매퍼 파일을 하나씩 적는 방식입니다. 만든 매퍼 XML을 목록에 더해야 MyBatis가 읽습니다."
                        .into(),
                );
            }
        }
        (None, None) => {}
    }
    if let Some(jsp) = scan.jsp_dir() {
        layout.jsp = jsp;
    }
    let config_layout = ConfigLayout {
        spring: scan
            .spring_dir()
            .unwrap_or_else(|| ConfigLayout::default().spring),
        resources: PathBuf::from(resources),
        java: layout.java.clone(),
    };
    let placeholders = Placeholders::from_keys(scan.property_keys.iter().map(String::as_str));
    Defaults {
        config_layout,
        placeholders,
        rte,
        db: scan.db().unwrap_or(Db::Oracle),
        junit,
        encoding,
        line_ending,
        layout,
        notes,
        base_package: scan
            .base_package()
            .unwrap_or_else(|| "egovframework.example".into()),
    }
}

/// pom.xml의 `project.build.sourceEncoding`·컴파일러 `encoding`.
fn source_encoding(build: &str) -> Option<Encoding> {
    let value = ["<project.build.sourceEncoding>", "<encoding>"]
        .iter()
        .find_map(|tag| {
            let start = build.find(tag)? + tag.len();
            let end = build[start..].find('<')? + start;
            Some(build[start..end].trim().to_ascii_uppercase())
        })?;
    match value.as_str() {
        "UTF-8" | "UTF8" => Some(Encoding::Utf8),
        "MS949" | "CP949" | "EUC-KR" | "X-WINDOWS-949" | "KSC5601" => Some(Encoding::Cp949),
        _ => None,
    }
}

#[derive(Default)]
struct Scan {
    build: String,
    imports_v3: usize,
    imports_v4: usize,
    utf8: usize,
    cp949: usize,
    lf: usize,
    crlf: usize,
    java_samples: usize,
    packages: HashMap<String, usize>,
    /// 매퍼 XML(폴더, 파일 이름).
    mappers: Vec<(PathBuf, String)>,
    web_roots: Vec<PathBuf>,
    views: Vec<(String, String)>,
    /// MyBatis `mapperLocations`(Spring XML)·`mybatis.mapper-locations`(Boot) 값.
    mapper_locations: Vec<String>,
    /// Spring XML 설정(`context-*.xml`)이 있는 폴더(프로젝트 기준).
    spring_configs: Vec<PathBuf>,
    /// 데이터소스에 쓰는 프로퍼티 키(`…DriverClassName`·`…Url` 등).
    property_keys: Vec<String>,
    db_hints: Vec<Db>,
}

impl Scan {
    fn walk(&mut self, root: &Path, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if name == "WEB-INF" {
                    self.web_roots
                        .push(dir.strip_prefix(root).unwrap_or(dir).to_path_buf());
                }
                // `WEB-INF/classes`·`WEB-INF/lib`는 빌드 결과물(매퍼 사본이 있다).
                let built = dir.ends_with("WEB-INF") && matches!(name.as_str(), "classes" | "lib");
                if !SKIP_DIRS.contains(&name.as_str()) && !built {
                    self.walk(root, &path);
                }
                continue;
            }
            let lower = name.to_ascii_lowercase();
            if lower == "pom.xml" || lower.starts_with("build.gradle") {
                self.build
                    .push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            } else if lower.ends_with(".java") && self.java_samples < MAX_JAVA_SAMPLES {
                self.java(&path);
            } else if lower.ends_with(".xml") {
                self.xml(root, &path, &name);
            } else if (lower.ends_with(".properties")
                || lower.ends_with(".yml")
                || lower.ends_with(".yaml"))
                && let Some(text) = read(&path)
            {
                self.db_hints.extend(db_hints(&text));
                const DATASOURCE_KEYS: [&str; 5] =
                    ["DriverClassName", "Url", "URL", "UserName", "Password"];
                self.property_keys.extend(
                    ferrite_egov::spring::property_keys(&text)
                        .into_iter()
                        .map(|(key, _)| key)
                        .filter(|key| DATASOURCE_KEYS.iter().any(|s| key.ends_with(s))),
                );
                // Spring Boot `mybatis.mapper-locations=classpath:mapper/**/*.xml`
                for line in text.lines() {
                    let line = line.trim();
                    if let Some(at) = line.find("mapper-locations") {
                        let value = line[at + "mapper-locations".len()..]
                            .trim_start_matches([' ', '=', ':']);
                        self.mapper_locations
                            .extend(value.split(',').map(|v| v.trim().to_owned()));
                    }
                }
            }
        }
    }

    fn java(&mut self, path: &Path) {
        let Ok(bytes) = text_file::read(path) else {
            return;
        };
        let Ok(content) = text_file::decode(&bytes) else {
            return;
        };
        self.java_samples += 1;
        // 한글이 있는 파일만 인코딩 단서가 된다(ASCII뿐이면 UTF-8로도 읽힌다).
        if !content.text.is_ascii() {
            match content.encoding {
                Encoding::Cp949 => self.cp949 += 1,
                Encoding::Utf8 | Encoding::Utf8Bom => self.utf8 += 1,
            }
        }
        match content.line_ending {
            LineEnding::CrLf => self.crlf += 1,
            LineEnding::Lf => self.lf += 1,
        }
        let text = &content.text;
        self.imports_v3 += text.matches("import egovframework.rte.").count();
        self.imports_v4 += text.matches("import org.egovframe.rte.").count();
        if let Some(package) = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("package "))
            .and_then(|p| p.split(';').next())
        {
            *self.packages.entry(package.trim().to_owned()).or_default() += 1;
        }
    }

    fn xml(&mut self, root: &Path, path: &Path, name: &str) {
        let Some(text) = read(path) else {
            return;
        };
        // 앞 4KB만 본다. 한글 글자 중간에서 자르지 않게 글자 경계로 물린다.
        let mut end = text.len().min(4096);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let head = &text[..end];
        if head.contains("<mapper") && head.contains("namespace") {
            if let Some(dir) = path.parent().and_then(|d| d.strip_prefix(root).ok()) {
                self.mappers.push((dir.to_path_buf(), name.to_owned()));
            }
        } else if text.contains("ViewResolver") {
            self.views.extend(ferrite_egov::web::view_resolvers(&text));
        }
        if name.starts_with("context-")
            && head.contains("<beans")
            && let Some(dir) = path.parent().and_then(|d| d.strip_prefix(root).ok())
        {
            self.spring_configs.push(dir.to_path_buf());
        }
        if text.contains("mapperLocations") {
            self.mapper_locations.extend(mapper_location_values(&text));
        }
        self.db_hints.extend(db_hints(&text));
    }

    /// 매퍼가 가장 많은 폴더와, 그 폴더 파일 이름에서 본뜬 이름 모양.
    fn mapper_layout(&self) -> Option<(PathBuf, String)> {
        let mut counts: HashMap<&Path, usize> = HashMap::new();
        for (dir, _) in &self.mappers {
            *counts.entry(dir.as_path()).or_default() += 1;
        }
        let dir = counts.into_iter().max_by_key(|(_, n)| *n)?.0;
        let mut patterns: HashMap<String, usize> = HashMap::new();
        for (_, name) in self.mappers.iter().filter(|(d, _)| d == dir) {
            *patterns.entry(file_pattern(name)).or_default() += 1;
        }
        let pattern = patterns.into_iter().max_by_key(|(_, n)| *n)?.0;
        Some((dir.to_path_buf(), pattern))
    }

    /// `context-*.xml`이 가장 많은 폴더.
    fn spring_dir(&self) -> Option<PathBuf> {
        let mut counts: HashMap<&Path, usize> = HashMap::new();
        for dir in &self.spring_configs {
            *counts.entry(dir.as_path()).or_default() += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(dir, n)| (*n, std::cmp::Reverse(dir.components().count())))
            .map(|(dir, _)| dir.to_path_buf())
    }

    /// 웹 루트 + 뷰 리졸버 접두사(프로젝트 루트 기준). 웹 루트가 여럿이면(배포용 사본 등)
    /// `webapp`·`WebContent`·`WebRoot` 이름을 먼저, 그다음 얕은 것.
    fn jsp_dir(&self) -> Option<PathBuf> {
        let web = self.web_roots.iter().min_by_key(|root| {
            let named = root
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| matches!(n, "webapp" | "WebContent" | "WebRoot"));
            (!named, root.components().count())
        })?;
        let (prefix, _) = self.views.iter().find(|(_, suffix)| suffix == ".jsp")?;
        let mut path = web.clone();
        for segment in prefix.split('/').filter(|s| !s.is_empty()) {
            path.push(segment);
        }
        Some(path)
    }

    fn db(&self) -> Option<Db> {
        let mut counts: HashMap<Db, usize> = HashMap::new();
        for db in &self.db_hints {
            *counts.entry(*db).or_default() += 1;
        }
        // 같은 수면 Db::ALL에서 앞선 것.
        counts
            .into_iter()
            .max_by_key(|(db, n)| (*n, std::cmp::Reverse(Db::ALL.iter().position(|d| d == db))))
            .map(|(db, _)| db)
    }

    /// `….service`로 끝나는 패키지 중 가장 많은 것의 기능 패키지 부모(`egovframework.example.sample.service`
    /// → `egovframework.example`).
    fn base_package(&self) -> Option<String> {
        let (package, _) = self
            .packages
            .iter()
            .filter(|(p, _)| p.ends_with(".service"))
            .max_by_key(|(p, n)| (**n, std::cmp::Reverse(p.len())))?;
        let module = package.strip_suffix(".service")?;
        Some(
            module
                .rsplit_once('.')
                .map_or(module, |(parent, _)| parent)
                .to_owned(),
        )
    }
}

/// Spring XML의 `mapperLocations` 값들(`value="…"` 또는 `<value>…</value>` 목록).
fn mapper_location_values(xml: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find("mapperLocations") {
        let after = &rest[at..];
        let end = after
            .find("</property>")
            .or_else(|| after.find("/>"))
            .unwrap_or(after.len());
        let block = &after[..end];
        if let Some(value) = ferrite_egov::jsp::attribute(block, "value") {
            found.push(value.trim().to_owned());
        }
        let mut values = block;
        while let Some(open) = values.find("<value>") {
            let start = open + "<value>".len();
            let Some(close) = values[start..].find("</value>") else {
                break;
            };
            found.push(values[start..start + close].trim().to_owned());
            values = &values[start + close..];
        }
        rest = &after["mapperLocations".len()..];
    }
    found
}

/// `mapperLocations` 패턴 하나를 (폴더, 파일 이름 모양)으로. 파일 하나를 콕 집은 값은 None.
/// - `classpath:/egovframework/mapper/**/*_SQL_${Globals.DbType}.xml`
///   → `src/main/resources/egovframework/mapper/{module}`, `{class}_SQL_{db}.xml`
/// - `classpath:sqlmap/${DBTYPE}/*.xml` → `src/main/resources/sqlmap/{db}`, `{class}.xml`
fn mapper_location(value: &str, resources: &str) -> Option<(PathBuf, String)> {
    let path = value
        .trim()
        .trim_start_matches("classpath*:")
        .trim_start_matches("classpath:")
        .trim_start_matches('/');
    let (dir, file) = path.rsplit_once('/').unwrap_or(("", path));
    if !file.contains('*') || !file.ends_with(".xml") {
        return None;
    }
    let mut out = PathBuf::from(resources);
    let mut module = false;
    for segment in dir.split('/').filter(|s| !s.is_empty()) {
        if segment.contains("${") {
            out.push("{db}");
        } else if segment.contains('*') {
            // `**`·`*` 자리에는 새 기능 폴더를 하나 둔다(이어진 `**`는 0단계로 본다).
            if !module {
                out.push("{module}");
                module = true;
            }
        } else {
            out.push(segment);
        }
    }
    let mut pattern = String::new();
    let mut rest = file;
    while let Some(at) = rest.find("${") {
        pattern.push_str(&rest[..at]);
        pattern.push_str("{db}");
        rest = rest[at..].find('}').map_or("", |end| &rest[at + end + 1..]);
    }
    pattern.push_str(rest);
    let pattern = pattern.replacen('*', "{class}", 1).replace('*', "");
    Some((out, pattern))
}

/// 기존 매퍼 파일 이름을 이름 모양으로(`Board_SQL_oracle.xml` → `{class}_SQL_oracle.xml`의 DB 자리는 `{db}`).
fn file_pattern(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for db in [
        "oracle",
        "tibero",
        "postgres",
        "postgresql",
        "mysql",
        "maria",
        "mariadb",
        "cubrid",
    ] {
        if lower.ends_with(&format!("_sql_{db}.xml")) {
            return "{class}_SQL_{db}.xml".into();
        }
        if lower.ends_with(&format!("_{db}.xml")) {
            return "{class}_{db}.xml".into();
        }
    }
    if lower.ends_with("_sql.xml") {
        "{class}_SQL.xml".into()
    } else if lower.ends_with("mapper.xml") {
        "{class}Mapper.xml".into()
    } else {
        "{class}_SQL.xml".into()
    }
}

/// 설정 텍스트의 DB 단서(egov `Globals.DbType`, JDBC URL, 드라이버).
fn db_hints(text: &str) -> Vec<Db> {
    let lower = text.to_ascii_lowercase();
    let mut found = Vec::new();
    for line in lower.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with("<!--") {
            continue;
        }
        let db = if line.contains("jdbc:oracle")
            || line.contains("oracle.jdbc")
            || (line.contains("dbtype") && line.contains("oracle"))
        {
            Some(Db::Oracle)
        } else if line.contains("jdbc:tibero")
            || (line.contains("tibero") && line.contains("dbtype"))
        {
            Some(Db::Tibero)
        } else if line.contains("jdbc:postgresql")
            || (line.contains("dbtype") && line.contains("postgres"))
        {
            Some(Db::PostgreSql)
        } else if line.contains("jdbc:mariadb")
            || line.contains("jdbc:mysql")
            || (line.contains("dbtype") && (line.contains("mysql") || line.contains("maria")))
        {
            Some(Db::MariaDb)
        } else if line.contains("jdbc:cubrid")
            || (line.contains("dbtype") && line.contains("cubrid"))
        {
            Some(Db::Cubrid)
        } else {
            None
        };
        found.extend(db);
    }
    found
}

fn read(path: &Path) -> Option<String> {
    let bytes = text_file::read(path).ok()?;
    text_file::decode(&bytes).ok().map(|c| c.text)
}

/// 이미 있는 파일(덮어쓰게 될 것).
pub fn existing(root: &Path, files: &[GeneratedFile]) -> Vec<PathBuf> {
    files
        .iter()
        .map(|f| root.join(&f.path))
        .filter(|p| p.exists())
        .collect()
}

/// 만든 파일을 쓴다. 이미 있는 파일은 `overwrite`일 때만 덮어쓴다(하나라도 있으면 아무것도 쓰지 않는다).
pub fn write(
    root: &Path,
    files: &[GeneratedFile],
    encoding: Encoding,
    line_ending: LineEnding,
    overwrite: bool,
) -> Result<Vec<PathBuf>, String> {
    let existing = existing(root, files);
    if !overwrite && !existing.is_empty() {
        let names: Vec<String> = existing.iter().map(|p| p.display().to_string()).collect();
        return Err(format!("이미 있는 파일: {}", names.join(", ")));
    }
    // 인코딩 변환을 먼저 모두 해 본다(쓰다 말고 멈추지 않게).
    let encoded: Vec<(PathBuf, Vec<u8>)> = files
        .iter()
        .map(|file| {
            let content = TextContent {
                text: file.content.replace("\r\n", "\n"),
                // BOM은 javac·JSP에서 문제가 되므로 만들지 않는다.
                encoding: if encoding == Encoding::Utf8Bom {
                    Encoding::Utf8
                } else {
                    encoding
                },
                line_ending,
            };
            text_file::encode(&content)
                .map(|bytes| (root.join(&file.path), bytes))
                .map_err(|e| format!("{}: {e}", file.path.display()))
        })
        .collect::<Result<_, _>>()?;
    let mut written = Vec::new();
    for (path, bytes) in encoded {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        text_file::write_atomic(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ferrite-codegen-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(root: &Path, path: &str, bytes: &[u8]) {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }

    #[test]
    fn 프로젝트에서_기본값을_찾는다() {
        let root = project("defaults");
        write_file(&root, "pom.xml", b"<project><properties><project.build.sourceEncoding>MS949</project.build.sourceEncoding></properties><dependency><groupId>egovframework.rte</groupId><artifactId>junit</artifactId></dependency></project>");
        let korean = text_file::encode(&TextContent {
            text: "package egovframework.example.sample.service;\n// 한글\nimport egovframework.rte.fdl.cmmn.EgovAbstractServiceImpl;\n".into(),
            encoding: Encoding::Cp949,
            line_ending: LineEnding::CrLf,
        })
        .unwrap();
        write_file(
            &root,
            "src/main/java/egovframework/example/sample/service/SampleService.java",
            &korean,
        );
        write_file(
            &root,
            "src/main/resources/egovframework/sqlmap/example/mappers/EgovSample_Sample_SQL.xml",
            b"<mapper namespace=\"x\"></mapper>",
        );
        write_file(
            &root,
            "src/main/resources/egovframework/egovProps/globals.properties",
            b"Globals.DbType = tibero\nGlobals.DriverClassName = com.tmax.tibero.jdbc.TbDriver\nGlobals.Url = jdbc:tibero:thin:@db:8629:egov\nGlobals.UserName = egov\nGlobals.Password = x\n",
        );
        write_file(
            &root,
            "src/main/resources/egovframework/spring/com/context-common.xml",
            b"<beans></beans>",
        );
        write_file(&root, "src/main/webapp/WEB-INF/config/egovframework/springmvc/dispatcher-servlet.xml", b"<beans><bean class=\"org.springframework.web.servlet.view.UrlBasedViewResolver\" p:prefix=\"/WEB-INF/jsp/egovframework/example/\" p:suffix=\".jsp\"/></beans>");
        let found = defaults(&root);
        assert_eq!(found.rte, Rte::V3);
        assert_eq!(found.junit, JUnit::V4);
        assert_eq!(found.encoding, Encoding::Cp949);
        assert_eq!(found.line_ending, LineEnding::CrLf);
        assert_eq!(found.db, Db::Tibero);
        assert_eq!(
            found.config_layout.spring,
            PathBuf::from("src/main/resources/egovframework/spring/com")
        );
        assert_eq!(found.placeholders.url.as_deref(), Some("Globals.Url"));
        assert_eq!(found.base_package, "egovframework.example");
        assert_eq!(
            found.layout.mapper,
            PathBuf::from("src/main/resources/egovframework/sqlmap/example/mappers")
        );
        assert_eq!(
            found.layout.mapper_file, "{class}_SQL.xml",
            "기존 매퍼 이름 모양"
        );
        assert_eq!(
            found.layout.jsp,
            PathBuf::from("src/main/webapp/WEB-INF/jsp/egovframework/example"),
            "웹 루트 + 뷰 리졸버 접두사(프로젝트 기준)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn 이미_있는_파일은_허락할_때만_덮어쓴다() {
        let root = project("write");
        let files = vec![GeneratedFile {
            path: "a/b/한글.java".into(),
            content: "// 한글 주석\nclass A {}\n".into(),
        }];
        let written = write(&root, &files, Encoding::Cp949, LineEnding::CrLf, false).unwrap();
        let bytes = std::fs::read(&written[0]).unwrap();
        let decoded = text_file::decode(&bytes).unwrap();
        assert_eq!(decoded.encoding, Encoding::Cp949);
        assert_eq!(decoded.line_ending, LineEnding::CrLf);
        assert!(write(&root, &files, Encoding::Utf8, LineEnding::Lf, false).is_err());
        assert!(write(&root, &files, Encoding::Utf8, LineEnding::Lf, true).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mapper_locations를_폴더와_이름_모양으로_바꾼다() {
        let at = |value: &str| mapper_location(value, "src/main/resources");
        assert_eq!(
            at("classpath:/egovframework/mapper/**/*_SQL_${Globals.DbType}.xml"),
            Some((
                "src/main/resources/egovframework/mapper/{module}".into(),
                "{class}_SQL_{db}.xml".into()
            ))
        );
        assert_eq!(
            at("classpath:sqlmap/${DBTYPE}/*.xml"),
            Some((
                "src/main/resources/sqlmap/{db}".into(),
                "{class}.xml".into()
            ))
        );
        assert_eq!(
            at("classpath:/egovframework/sqlmap/**/mappers/**/*.xml"),
            Some((
                "src/main/resources/egovframework/sqlmap/{module}/mappers".into(),
                "{class}.xml".into()
            ))
        );
        assert_eq!(
            at("classpath:/egovframework/sqlmap/mappers/Sample*.xml"),
            Some((
                "src/main/resources/egovframework/sqlmap/mappers".into(),
                "Sample{class}.xml".into()
            ))
        );
        assert_eq!(
            at("classpath:/egovframework/sqlmap/mappers/Car_SQL.xml"),
            None,
            "파일 하나"
        );
        let xml = r#"<bean id="sqlSession" class="org.mybatis.spring.SqlSessionFactoryBean">
  <property name="mapperLocations">
    <list><value>classpath:/a/*_SQL.xml</value><value>classpath:/b/Car_SQL.xml</value></list>
  </property></bean>
<bean><property name="mapperLocations" value="classpath*:c/**/*.xml"/></bean>"#;
        assert_eq!(
            mapper_location_values(xml),
            [
                "classpath:/a/*_SQL.xml",
                "classpath:/b/Car_SQL.xml",
                "classpath*:c/**/*.xml"
            ]
        );
    }

    #[test]
    fn 매퍼_이름_모양을_본뜬다() {
        assert_eq!(
            file_pattern("EgovBbs_SQL_oracle.xml"),
            "{class}_SQL_{db}.xml"
        );
        assert_eq!(file_pattern("Board_mysql.xml"), "{class}_{db}.xml");
        assert_eq!(file_pattern("Sample_SQL.xml"), "{class}_SQL.xml");
        assert_eq!(file_pattern("BoardMapper.xml"), "{class}Mapper.xml");
    }
}
