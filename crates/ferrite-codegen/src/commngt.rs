//! 공통컴포넌트 추가. 공식 개발환경(Eclipse)의 공통컴포넌트 플러그인(`egovframework.dev.imp.commngt`)에
//! 든 목록(`components.xml`)과 컴포넌트별 zip을 그대로 쓴다. Ferrite는 이 파일들을 싣지 않는다.
//! 사용자가 가진 개발환경·플러그인에서 캐시로 가져오므로, 그 개발환경의 버전(3.x·4.x)을 따른다.
//!
//! 공식 마법사와 같이 고른 컴포넌트와 그것이 기대는 컴포넌트의 파일을 프로젝트에 푼다. 다른 점:
//! 내용이 다른 기존 파일은 기본으로 건너뛰고(덮어쓰기를 고르면 `.bak`으로 백업), `pom.xml`은
//! 덮어쓰지 않고 빠진 의존성만 알려 준다. DB 테이블은 만들지 않고 돌릴 스크립트를 알려 준다.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use ferrite_core::text_file;

use crate::crud::Db;

/// 플러그인 폴더·jar 이름의 앞부분. 모바일용(`egovframework.mdev.imp.commngt_`)은 다르다.
const PLUGIN: &str = "egovframework.dev.imp.commngt_";
const CATALOG: &str = "components.xml";
const LABELS: &str = "components_ko.properties";
const LABELS_IN_PLUGIN: &str = "egovframework/dev/imp/commngt/common/components_ko.properties";
/// 모든 컴포넌트가 기대는 기본 묶음. 스크립트도 이것부터 돌린다.
const BASE: &str = "egovframework.com.cmm";
const POM: &str = "pom.xml";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    /// 컴포넌트를 가리키는 패키지(`egovframework.com.uss.olh.faq`). zip 이름이기도 하다.
    pub package: String,
    /// 함께 있어야 하는 패키지.
    pub deps: Vec<String>,
    pub description: String,
    pub tables: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    pub name: String,
    pub description: String,
    pub components: Vec<Component>,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    /// 컴포넌트 zip의 버전(`4.2.0`).
    pub version: String,
    pub dir: PathBuf,
    pub categories: Vec<Category>,
    /// zip이 있는 패키지.
    zips: HashSet<String>,
}

impl Catalog {
    /// 가져온 목록 폴더(`commngt-<버전>`)를 읽는다.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let read = |name: &str| {
            let path = dir.join(name);
            fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
        };
        let xml = String::from_utf8(read(CATALOG)?)
            .map_err(|_| format!("{CATALOG}가 UTF-8이 아닙니다"))?;
        let labels = text_file::decode(&read(LABELS)?)
            .map(|content| parse_properties(&content.text))
            .map_err(|e| format!("{LABELS}: {e}"))?;
        let version = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("commngt-"))
            .ok_or_else(|| format!("공통컴포넌트 폴더 이름이 아닙니다: {}", dir.display()))?
            .to_owned();
        let suffix = format!(".Wizard.{version}.zip");
        let zips = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_suffix(&suffix)
                    .map(str::to_owned)
            })
            .collect();
        Ok(Self {
            version,
            dir: dir.to_path_buf(),
            categories: parse_catalog(&xml, &labels),
            zips,
        })
    }

    /// 버전의 주 번호(3, 4). 프로젝트 실행환경과 맞는지 볼 때 쓴다.
    pub fn major(&self) -> Option<u32> {
        self.version.split('.').next()?.parse().ok()
    }

    pub fn component(&self, package: &str) -> Option<&Component> {
        self.categories
            .iter()
            .flat_map(|c| &c.components)
            .find(|c| c.package == package)
    }

    fn zip(&self, package: &str) -> PathBuf {
        self.dir
            .join(format!("{package}.Wizard.{}.zip", self.version))
    }

    /// 고른 패키지와 그것들이 기대는 패키지 모두. 기본 묶음을 먼저, 나머지는 찾은 순서다.
    /// 같은 패키지가 두 분류에 나오면(게시판·블로그) 의존 목록을 합친다. 의존에 zip이 없는 상위
    /// 패키지가 적혀 있으면(4.2의 `egovframework.com.sym.ccm`) 그 아래 패키지 모두로 본다.
    pub fn closure(&self, selected: &[String]) -> Vec<String> {
        let mut deps: HashMap<&str, Vec<&str>> = HashMap::new();
        for component in self.categories.iter().flat_map(|c| &c.components) {
            deps.entry(&component.package)
                .or_default()
                .extend(component.deps.iter().map(String::as_str));
        }
        let zips = &self.zips;
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        let mut queue: Vec<String> = selected.to_vec();
        queue.reverse();
        while let Some(package) = queue.pop() {
            if !zips.contains(&package) {
                let prefix = format!("{package}.");
                let mut children: Vec<&String> =
                    zips.iter().filter(|z| z.starts_with(&prefix)).collect();
                if !children.is_empty() {
                    children.sort();
                    queue.extend(children.into_iter().rev().cloned());
                    continue;
                }
            }
            if !seen.insert(package.clone()) {
                continue;
            }
            if let Some(next) = deps.get(package.as_str()) {
                queue.extend(next.iter().rev().map(|p| (*p).to_owned()));
            }
            order.push(package);
        }
        if let Some(i) = order.iter().position(|p| p == BASE) {
            let base = order.remove(i);
            order.insert(0, base);
        }
        order
    }
}

/// 프로젝트에 이미 있는가(공식 마법사처럼 패키지 폴더로 본다).
pub fn installed(project: &Path, package: &str) -> bool {
    project
        .join("src/main/java")
        .join(package.replace('.', "/"))
        .is_dir()
}

/// 가져와 둔 목록들(`<root>/commngt-<버전>/`), 버전이 높은 것부터.
pub fn imported(root: &Path) -> Vec<Catalog> {
    let mut catalogs: Vec<Catalog> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("commngt-"))
        .filter_map(|e| Catalog::load(&e.path()).ok())
        .collect();
    let key = |c: &Catalog| -> Vec<u32> {
        c.version
            .split('.')
            .map(|n| n.parse().unwrap_or(0))
            .collect()
    };
    catalogs.sort_by_key(|c| std::cmp::Reverse(key(c)));
    catalogs
}

/// 공식 개발환경에서 공통컴포넌트 플러그인을 찾아 `<root>/commngt-<버전>/`에 가져온다. `source`는
/// 개발환경 폴더(`eclipse/plugins`를 담은 곳), 플러그인 폴더·jar, 플러그인 업데이트 사이트 zip,
/// 개발환경 zip 중 하나다. 같은 버전을 다시 가져오면 바꾼다.
pub fn import(source: &Path, root: &Path) -> Result<Catalog, String> {
    let staging = root.join(".commngt-import");
    let io_err = |e: io::Error| format!("{}: {e}", staging.display());
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(io_err)?;
    let result = copy_plugin(source, &staging).and_then(|()| {
        let version = version_in(&staging)?;
        let dest = root.join(format!("commngt-{version}"));
        let _ = fs::remove_dir_all(&dest);
        fs::rename(&staging, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        Catalog::load(&dest)
    });
    let _ = fs::remove_dir_all(&staging);
    result
}

fn not_found(source: &Path) -> String {
    format!(
        "{}에서 공통컴포넌트 플러그인({PLUGIN}…)을 찾지 못했습니다. 개발환경 폴더나 플러그인 jar를 고르세요",
        source.display()
    )
}

fn copy_plugin(source: &Path, dest: &Path) -> Result<(), String> {
    if source.is_dir() {
        let plugin = find_plugin(source).ok_or_else(|| not_found(source))?;
        if plugin.is_dir() {
            return copy_plugin_dir(&plugin, dest);
        }
        return copy_plugin(&plugin, dest);
    }
    let file = File::open(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", source.display()))?;
    copy_from_archive(&mut zip, dest, source)
}

/// 폴더 안의 플러그인(폴더 또는 jar). 개발환경 폴더·eclipse 폴더·plugins 폴더 어느 것을 골라도 찾는다.
fn find_plugin(dir: &Path) -> Option<PathBuf> {
    if dir.join("examples").join(CATALOG).is_file() {
        return Some(dir.to_path_buf());
    }
    let mut candidates = vec![
        dir.to_path_buf(),
        dir.join("plugins"),
        dir.join("eclipse/plugins"),
    ];
    // 개발환경 zip을 푼 폴더(`<압축 푼 곳>/eGovFrameDev-…/eclipse/plugins`)
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        candidates.push(entry.path().join("eclipse/plugins"));
    }
    candidates.iter().find_map(|plugins| {
        fs::read_dir(plugins)
            .ok()?
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(PLUGIN))
            .map(|e| e.path())
            .max()
    })
}

fn copy_plugin_dir(plugin: &Path, dest: &Path) -> Result<(), String> {
    let copy = |from: PathBuf, name: &str| {
        fs::copy(&from, dest.join(name))
            .map(drop)
            .map_err(|e| format!("{}: {e}", from.display()))
    };
    let examples = plugin.join("examples");
    copy(examples.join(CATALOG), CATALOG)?;
    copy(plugin.join(LABELS_IN_PLUGIN), LABELS)?;
    for entry in fs::read_dir(&examples)
        .map_err(|e| format!("{}: {e}", examples.display()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".zip") {
            copy(entry.path(), &name)?;
        }
    }
    Ok(())
}

/// jar(플러그인 자체)나 zip(업데이트 사이트, 개발환경)에서 플러그인 파일을 꺼낸다. 업데이트 사이트
/// zip은 플러그인 jar를 담고 있어 그 jar를 메모리에서 다시 연다.
fn copy_from_archive<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    dest: &Path,
    source: &Path,
) -> Result<(), String> {
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|name| name.ok().map(|n| n.into_owned()))
        .collect();
    let suffix = format!("examples/{CATALOG}");
    let prefix = names.iter().find_map(|name| {
        let prefix = name.strip_suffix(&suffix)?;
        let folder = prefix
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("");
        (prefix.is_empty() || folder.starts_with(PLUGIN)).then(|| prefix.to_owned())
    });
    if let Some(prefix) = prefix {
        let mut wanted = vec![
            (format!("{prefix}examples/{CATALOG}"), CATALOG.to_owned()),
            (format!("{prefix}{LABELS_IN_PLUGIN}"), LABELS.to_owned()),
        ];
        let examples = format!("{prefix}examples/");
        wanted.extend(names.iter().filter_map(|name| {
            let file = name.strip_prefix(&examples)?;
            (file.ends_with(".zip") && !file.contains('/')).then(|| (name.clone(), file.to_owned()))
        }));
        for (entry, file) in wanted {
            let mut reader = zip
                .by_name(&entry)
                .map_err(|e| format!("{}: {entry}: {e}", source.display()))?;
            let path = dest.join(&file);
            let mut out = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            io::copy(&mut reader, &mut out).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        return Ok(());
    }
    let jar = names
        .iter()
        .find(|name| {
            let file = name.rsplit('/').next().unwrap_or("");
            file.starts_with(PLUGIN) && file.ends_with(".jar")
        })
        .ok_or_else(|| not_found(source))?
        .clone();
    let mut bytes = Vec::new();
    zip.by_name(&jar)
        .and_then(|mut entry| entry.read_to_end(&mut bytes).map_err(Into::into))
        .map_err(|e| format!("{}: {jar}: {e}", source.display()))?;
    let mut inner = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("{jar}: {e}"))?;
    copy_from_archive(&mut inner, dest, source)
}

/// 기본 묶음 zip 이름(`egovframework.com.cmm.Wizard.<버전>.zip`)에서 버전을 읽는다.
fn version_in(dir: &Path) -> Result<String, String> {
    let prefix = format!("{BASE}.Wizard.");
    fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .find_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let version = name.strip_prefix(&prefix)?.strip_suffix(".zip")?.to_owned();
            (!version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.'))
                .then_some(version)
        })
        .ok_or_else(|| format!("기본 컴포넌트({BASE}) zip이 없습니다"))
}

/// 프로젝트의 파일과 견준 상태.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    New,
    /// 내용이 같다(줄바꿈만 다른 것도 같다고 본다).
    Same,
    Differs,
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// 프로젝트 기준 상대 경로(`/` 구분).
    pub path: String,
    pub state: State,
    /// 이 파일을 담은 패키지.
    pub package: String,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    /// 추가할 패키지(의존 포함), 기본 묶음부터.
    pub packages: Vec<String>,
    pub entries: Vec<Entry>,
    /// zip이 없어 넣을 수 없는 패키지.
    pub missing: Vec<String>,
}

impl Plan {
    pub fn count(&self, state: State) -> usize {
        self.entries.iter().filter(|e| e.state == state).count()
    }
}

/// 고른 컴포넌트를 넣으면 프로젝트의 무엇이 바뀌는지 본다(아직 쓰지 않는다).
pub fn plan(catalog: &Catalog, selected: &[String], project: &Path) -> Result<Plan, String> {
    let packages = catalog.closure(selected);
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen = HashSet::new();
    let mut missing = Vec::new();
    for package in &packages {
        let path = catalog.zip(package);
        let Ok(file) = File::open(&path) else {
            missing.push(package.clone());
            continue;
        };
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if entry.is_dir() {
                continue;
            }
            // 폴더 밖을 가리키는 항목 경로(`..`, 절대 경로)는 넣지 않는다.
            let Some(relative) = entry.enclosed_name() else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            // 컴포넌트들이 같이 싣는 파일(mapper-config.xml 등)은 내용이 같아 처음 것만 쓴다.
            if !seen.insert(relative.clone()) {
                continue;
            }
            let mut data = Vec::new();
            entry
                .read_to_end(&mut data)
                .map_err(|e| format!("{}: {relative}: {e}", path.display()))?;
            let state = match fs::read(project.join(&relative)) {
                Err(_) => State::New,
                Ok(existing) if same(&existing, &data) => State::Same,
                Ok(_) => State::Differs,
            };
            entries.push(Entry {
                path: relative,
                state,
                package: package.clone(),
                data,
            });
        }
    }
    Ok(Plan {
        packages,
        entries,
        missing,
    })
}

fn same(a: &[u8], b: &[u8]) -> bool {
    let strip =
        |bytes: &[u8]| -> Vec<u8> { bytes.iter().copied().filter(|&b| b != b'\r').collect() };
    a == b || strip(a) == strip(b)
}

/// 넣은 결과와 사용자가 손으로 할 일.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub created: usize,
    pub same: usize,
    /// (덮어쓴 파일, 백업)
    pub overwritten: Vec<(String, PathBuf)>,
    /// 내용이 달라 건너뛴 파일.
    pub skipped: Vec<String>,
    /// 프로젝트 pom에 없는 공통컴포넌트 pom의 의존성(`groupId:artifactId`).
    pub missing_dependencies: Vec<String>,
    /// 건너뛴 .properties에서 프로젝트 파일에 없는 키.
    pub missing_keys: Vec<(String, Vec<String>)>,
    /// 프로젝트 DB에 돌릴 스크립트(테이블 생성 다음 기초 자료), 기본 묶음부터.
    pub scripts: Vec<String>,
}

/// 계획대로 쓴다. `overwrite`면 내용이 다른 파일을 `.bak`으로 백업하고 바꾼다(`pom.xml`은 빼고).
pub fn apply(plan: &Plan, project: &Path, overwrite: bool, db: Db) -> Result<Outcome, String> {
    let mut outcome = Outcome::default();
    for entry in &plan.entries {
        let path = project.join(&entry.path);
        let write = |path: &Path| {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(path, &entry.data)
        };
        let written = outcome.created + outcome.overwritten.len();
        let failed = |e: io::Error| {
            format!(
                "{}: {e} (그 전까지 파일 {written}개를 썼습니다)",
                path.display()
            )
        };
        match entry.state {
            State::New => {
                write(&path).map_err(failed)?;
                outcome.created += 1;
            }
            State::Same => outcome.same += 1,
            State::Differs if overwrite && entry.path != POM => {
                let backup = backup_path(&path);
                fs::rename(&path, &backup)
                    .and_then(|()| write(&path))
                    .map_err(failed)?;
                outcome.overwritten.push((entry.path.clone(), backup));
            }
            State::Differs => {
                outcome.skipped.push(entry.path.clone());
                let existing = fs::read(&path).unwrap_or_default();
                if entry.path == POM {
                    outcome.missing_dependencies = missing_dependencies(&existing, &entry.data);
                } else if entry.path.ends_with(".properties") {
                    let keys = missing_keys(&existing, &entry.data);
                    if !keys.is_empty() {
                        outcome.missing_keys.push((entry.path.clone(), keys));
                    }
                }
            }
        }
    }
    outcome.scripts = scripts(plan, db);
    Ok(outcome)
}

/// 같은 이름의 백업이 있으면 번호를 붙인다(`.bak`, `.bak.1`, …).
fn backup_path(path: &Path) -> PathBuf {
    let base = path.as_os_str().to_owned();
    (0..)
        .map(|i| {
            let mut name = base.clone();
            name.push(if i == 0 {
                ".bak".to_owned()
            } else {
                format!(".bak.{i}")
            });
            PathBuf::from(name)
        })
        .find(|p| !p.exists())
        .expect("빈 백업 이름은 언젠가 나온다")
}

/// 공통컴포넌트 스크립트 폴더 이름(`src/script/ddl/<이름>/`).
pub fn script_dir(db: Db) -> &'static str {
    match db {
        Db::Oracle => "oracle",
        Db::Tibero => "tibero",
        Db::PostgreSql => "postgres",
        Db::MariaDb => "maria",
        Db::Cubrid => "cubrid",
    }
}

fn scripts(plan: &Plan, db: Db) -> Vec<String> {
    let dir = script_dir(db);
    let mut scripts = Vec::new();
    for kind in ["ddl", "dml"] {
        let prefix = format!("src/script/{kind}/{dir}/");
        for package in &plan.packages {
            scripts.extend(
                plan.entries
                    .iter()
                    .filter(|e| &e.package == package && e.path.starts_with(&prefix))
                    .map(|e| e.path.clone()),
            );
        }
    }
    scripts
}

fn missing_dependencies(project_pom: &[u8], component_pom: &[u8]) -> Vec<String> {
    let have: HashSet<String> = dependencies(&String::from_utf8_lossy(project_pom))
        .into_iter()
        .collect();
    dependencies(&String::from_utf8_lossy(component_pom))
        .into_iter()
        .filter(|d| !have.contains(d))
        .collect()
}

/// `<dependency>`마다 `groupId:artifactId`(버전은 보지 않는다).
fn dependencies(pom: &str) -> Vec<String> {
    let tag = |block: &str, name: &str| -> Option<String> {
        let start = block.find(&format!("<{name}>"))? + name.len() + 2;
        let end = block[start..].find(&format!("</{name}>"))? + start;
        Some(block[start..end].trim().to_owned())
    };
    let mut found = Vec::new();
    let mut rest = pom;
    while let Some(start) = rest.find("<dependency>") {
        let block = &rest[start..];
        let end = block.find("</dependency>").unwrap_or(block.len());
        if let (Some(group), Some(artifact)) = (
            tag(&block[..end], "groupId"),
            tag(&block[..end], "artifactId"),
        ) {
            let id = format!("{group}:{artifact}");
            if !found.contains(&id) {
                found.push(id);
            }
        }
        rest = &block[end..];
    }
    found
}

fn missing_keys(project: &[u8], component: &[u8]) -> Vec<String> {
    let parse = |bytes: &[u8]| {
        text_file::decode(bytes)
            .map(|c| parse_properties(&c.text))
            .unwrap_or_default()
    };
    let have = parse(project);
    let mut keys: Vec<String> = parse(component)
        .into_keys()
        .filter(|k| !have.contains_key(k))
        .collect();
    keys.sort();
    keys
}

/// Java `.properties`: 주석(`#`, `!`), `=`·`:`·공백 구분, 줄 끝 `\` 이어 쓰기, `\uXXXX` 등 이스케이프.
fn parse_properties(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let mut logical = line.trim_start().to_owned();
        if logical.is_empty() || logical.starts_with(['#', '!']) {
            continue;
        }
        while logical.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1 {
            logical.pop();
            match lines.next() {
                Some(next) => logical.push_str(next.trim_start()),
                None => break,
            }
        }
        let (key, value) = split_property(&logical);
        map.insert(unescape(key), unescape(value));
    }
    map
}

fn split_property(line: &str) -> (&str, &str) {
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '=' | ':' | ' ' | '\t' | '\u{c}' => {
                let rest = line[i..].trim_start_matches([' ', '\t', '\u{c}']);
                let rest = rest.strip_prefix(['=', ':']).unwrap_or(rest);
                return (&line[..i], rest.trim_start_matches([' ', '\t', '\u{c}']));
            }
            _ => {}
        }
    }
    (line, "")
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// 공식 `components.xml`: `<category>` 안에 `<component>`, 그 안에 `<packageName>` 등. 값은 대부분
/// `#키#`이고 키는 `components_ko.properties`에서 찾는다.
fn parse_catalog(xml: &str, labels: &HashMap<String, String>) -> Vec<Category> {
    let value = |raw: &str| -> String {
        let raw = raw.trim();
        let text = match raw.strip_prefix('#').and_then(|r| r.strip_suffix('#')) {
            Some(key) => labels.get(key).map(String::as_str).unwrap_or(""),
            None => raw,
        };
        xml_unescape(text.trim())
    };
    let mut categories: Vec<Category> = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            break;
        };
        let tag = &after[..end];
        let body = &after[end + 1..];
        let text = &body[..body.find('<').unwrap_or(body.len())];
        rest = body;
        if tag.starts_with(['/', '?', '!']) {
            continue;
        }
        match tag.split_whitespace().next().unwrap_or("") {
            "category" => categories.push(Category {
                name: value(&attribute(tag, "name")),
                description: value(&attribute(tag, "description")),
                components: Vec::new(),
            }),
            "component" => {
                if let Some(category) = categories.last_mut() {
                    category.components.push(Component {
                        name: value(&attribute(tag, "name")),
                        ..Default::default()
                    });
                }
            }
            field => {
                let Some(component) = categories.last_mut().and_then(|c| c.components.last_mut())
                else {
                    continue;
                };
                let text = value(text);
                match field {
                    "packageName" => component.package = text,
                    // 쉼표 대신 공백으로 이어 쓴 항목도 있다.
                    "dependencyPackage" => component.deps.extend(
                        text.split(|c: char| c == ',' || c.is_whitespace())
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned),
                    ),
                    "description" => component.description = text,
                    "useTable" => {
                        component.tables = text
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect();
                    }
                    _ => {}
                }
            }
        }
    }
    // 목록 틀의 빈 자리(값이 없는 항목)는 뺀다.
    for category in &mut categories {
        category
            .components
            .retain(|c| !c.package.is_empty() && !c.name.is_empty());
    }
    categories.retain(|c| !c.components.is_empty());
    categories
}

fn attribute(tag: &str, name: &str) -> String {
    let needle = format!("{name}=\"");
    tag.find(&needle)
        .and_then(|i| {
            let rest = &tag[i + needle.len()..];
            rest.find('"').map(|end| rest[..end].to_owned())
        })
        .unwrap_or_default()
}

fn xml_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let decoded = after.find(';').and_then(|end| {
            let entity = &after[..end];
            let c = match entity {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => entity
                    .strip_prefix("#x")
                    .map(|h| u32::from_str_radix(h, 16))
                    .or_else(|| entity.strip_prefix('#').map(str::parse))
                    .and_then(Result::ok)
                    .and_then(char::from_u32),
            }?;
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties는_이스케이프와_이어_쓰기를_읽는다() {
        let map = parse_properties(
            "# 주석\n! 주석\nkey1=값\nkey2 : a\\u0041\\\n    b\nkey\\=3 = x\\\\\nempty\n",
        );
        assert_eq!(map["key1"], "값");
        assert_eq!(map["key2"], "aAb");
        assert_eq!(map["key=3"], "x\\");
        assert_eq!(map["empty"], "");
    }

    #[test]
    fn 목록의_자리표시를_이름으로_바꾸고_빈_자리는_뺀다() {
        let labels = parse_properties(
            "c1nam=게시판\nc1des=설명\nc1p1nam=게시판&#40;기본&#41;\nc1p1pac=egovframework.com.cop.bbs\nc1p1de1=egovframework.com.cmm\nc1p1de2=a b,c\nc1p1use=T1, T2\n",
        );
        let xml = r##"<?xml version="1.0"?><components>
            <category name="#c1nam#" description="#c1des#">
              <component name="#c1p1nam#">
                <packageName      >#c1p1pac#</packageName      >
                <dependencyPackage>#c1p1de1#</dependencyPackage>
                <dependencyPackage>#c1p1de2#</dependencyPackage>
                <dependencyPackage>#c1p1de3#</dependencyPackage>
                <useTable>#c1p1use#</useTable>
              </component>
              <component name="#c1p2nam#"><packageName>#c1p2pac#</packageName></component>
            </category>
            <category name="#c2nam#" description=""></category>
        </components>"##;
        let categories = parse_catalog(xml, &labels);
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "게시판");
        let component = &categories[0].components[0];
        assert_eq!(categories[0].components.len(), 1);
        assert_eq!(component.name, "게시판(기본)");
        assert_eq!(component.package, "egovframework.com.cop.bbs");
        assert_eq!(component.deps, ["egovframework.com.cmm", "a", "b", "c"]);
        assert_eq!(component.tables, ["T1", "T2"]);
    }

    #[test]
    fn pom_의존성은_groupid와_artifactid로_견준다() {
        let project = b"<project><dependencies><dependency><groupId>a</groupId><artifactId>x</artifactId><version>1</version></dependency></dependencies></project>";
        let component = b"<dependency><groupId>a</groupId><artifactId>x</artifactId><version>2</version></dependency>\n<dependency>\n  <groupId> b </groupId>\n  <artifactId>y</artifactId>\n</dependency>";
        assert_eq!(missing_dependencies(project, component), ["b:y"]);
    }
}
