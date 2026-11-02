//! 프로젝트의 의존성 파일 찾기와 읽기. Maven·Gradle은 전이 의존까지 빌드 도구로 풀고(`mvn
//! dependency:tree`, `gradle dependencies`), npm·PyPI·Cargo는 잠금 파일과 버전을 고정한 목록을 읽는다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Dependency, Ecosystem};
use crate::files;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Maven,
    Gradle,
    Npm,
    Pip,
    Cargo,
}

/// 의존성을 적은 파일 하나.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub kind: Kind,
    pub path: PathBuf,
}

/// 프로젝트의 의존성 파일. Maven은 맨 위 pom.xml만(아래 모듈은 그것이 함께 푼다).
pub fn find(root: &Path) -> Vec<Manifest> {
    let mut found = Vec::new();
    files::walk(root, 6, &mut |path, is_dir| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if is_dir {
            // 빌드 결과에는 pom.xml 사본 등이 있다.
            return !matches!(name.as_ref(), "target" | "build" | "dist" | "out");
        }
        let kind = match name.as_ref() {
            "pom.xml" => Kind::Maven,
            "build.gradle" | "build.gradle.kts" => Kind::Gradle,
            "package-lock.json" => Kind::Npm,
            "Cargo.lock" => Kind::Cargo,
            n if n.starts_with("requirements") && n.ends_with(".txt") => Kind::Pip,
            _ => return true,
        };
        found.push(Manifest {
            kind,
            path: path.to_path_buf(),
        });
        true
    });
    let poms: Vec<PathBuf> = found
        .iter()
        .filter(|m| m.kind == Kind::Maven)
        .filter_map(|m| m.path.parent().map(Path::to_path_buf))
        .collect();
    found.retain(|m| {
        m.kind != Kind::Maven
            || !poms
                .iter()
                .any(|dir| m.path.parent() != Some(dir) && m.path.starts_with(dir))
    });
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

impl Manifest {
    /// 빌드 도구로 의존성을 풀 명령 줄(Maven·Gradle만). 파일이 있는 폴더에서 돌린다. Maven은 결과를
    /// 비어 있는 `output` 파일에 쓰고(모듈마다 덧붙인다), Gradle은 표준 출력으로 낸다.
    pub fn command(&self, root: &Path, output: &Path) -> Option<String> {
        let dir = self.path.parent()?;
        match self.kind {
            Kind::Maven => Some(format!(
                "{} -B -q dependency:tree -DoutputFile=\"{}\" -DappendOutput=true",
                tool(root, dir, "mvnw", "mvn"),
                output.display()
            )),
            Kind::Gradle => Some(format!(
                "{} -q --console=plain dependencies",
                tool(root, dir, "gradlew", "gradle")
            )),
            _ => None,
        }
    }

    /// 의존성을 읽는다. `resolved`는 `command`의 결과(Maven은 결과 파일, Gradle은 표준 출력).
    pub fn read(&self, resolved: Option<&str>) -> Result<Vec<Dependency>, String> {
        let text = || {
            std::fs::read_to_string(&self.path)
                .map_err(|e| format!("{}을(를) 읽지 못했습니다: {e}", self.path.display()))
        };
        Ok(match self.kind {
            Kind::Maven => maven(&self.path, resolved.unwrap_or_default()),
            Kind::Gradle => gradle(&self.path, &text()?, resolved.unwrap_or_default()),
            Kind::Npm => npm(&self.path, &text()?)?,
            Kind::Pip => pip(&self.path, &text()?),
            Kind::Cargo => cargo(&self.path, &text()?),
        })
    }
}

/// 빌드 도구: `dir`에서 프로젝트 루트까지 올라가며 래퍼(`mvnw`·`gradlew`)를 찾고, 없으면 PATH의 도구.
fn tool(root: &Path, dir: &Path, script: &str, fallback: &str) -> String {
    for d in dir.ancestors() {
        let names = if cfg!(windows) {
            vec![format!("{script}.cmd"), format!("{script}.bat")]
        } else {
            vec![script.to_owned()]
        };
        if let Some(file) = names.iter().map(|n| d.join(n)).find(|f| f.is_file()) {
            return format!("\"{}\"", file.display());
        }
        if d == root {
            break;
        }
    }
    fallback.to_owned()
}

/// 나무 그림의 앞(`+- `, `|  \- `, `+--- `)을 뗀 (깊이를 셀 앞 길이, 나머지).
fn split_tree(line: &str) -> (usize, &str) {
    let rest = line.trim_start_matches(['+', '-', '|', '\\', ' ']);
    (line.len() - rest.len(), rest)
}

/// `mvn dependency:tree` 결과. 모듈마다 `그룹:아티팩트:종류:버전` 줄로 시작하고, 그 아래가 의존
/// (`그룹:아티팩트:종류[:분류]:버전:범위`, 깊이마다 세 칸).
fn maven(manifest: &Path, tree: &str) -> Vec<Dependency> {
    // 모듈의 artifactId → pom.xml.
    let mut modules: HashMap<String, PathBuf> = HashMap::new();
    if let Some(dir) = manifest.parent() {
        files::walk(dir, 6, &mut |path, is_dir| {
            if is_dir {
                return path.file_name().is_none_or(|n| n != "target");
            }
            if path.file_name().is_some_and(|n| n == "pom.xml")
                && let Some(id) = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|t| project_artifact(&t))
            {
                modules.entry(id).or_insert_with(|| path.to_path_buf());
            }
            true
        });
    }
    let mut out = Vec::new();
    let mut pom = (manifest.to_path_buf(), String::new());
    let mut direct = (String::new(), 0);
    for line in tree.lines() {
        let (prefix, coords) = split_tree(line);
        let parts: Vec<&str> = coords.split(':').collect();
        if parts.len() < 4 {
            continue;
        }
        if prefix == 0 {
            let file = modules
                .get(parts[1])
                .cloned()
                .unwrap_or(manifest.to_path_buf());
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            pom = (file, text);
            continue;
        }
        if parts.len() < 5 {
            continue;
        }
        let (group, artifact) = (parts[0], parts[1]);
        let name = format!("{group}:{artifact}");
        let depth = prefix / 3;
        if depth <= 1 {
            direct = (name.clone(), pom_line(&pom.1, group, artifact));
        }
        out.push(Dependency {
            ecosystem: Ecosystem::Maven,
            version: parts[parts.len() - 2].to_owned(),
            file: pom.0.clone(),
            line: direct.1,
            via: (depth > 1).then(|| direct.0.clone()),
            name,
        });
    }
    out
}

/// pom.xml의 프로젝트 artifactId.
fn project_artifact(text: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(text).ok()?;
    doc.root_element()
        .children()
        .find(|n| n.tag_name().name() == "artifactId")
        .and_then(|n| n.text())
        .map(|t| t.trim().to_owned())
}

/// pom.xml에서 그 의존을 적은 줄(0부터). `<dependencies>` 아래를 먼저, 없으면 어디든(관리·프로필),
/// 그래도 없으면(부모 pom에서 온 것) 0.
fn pom_line(text: &str, group: &str, artifact: &str) -> usize {
    let Ok(doc) = roxmltree::Document::parse(text) else {
        return 0;
    };
    let child = |n: roxmltree::Node, tag: &str| {
        n.children()
            .find(|c| c.tag_name().name() == tag)
            .and_then(|c| c.text())
            .map(str::trim)
            .map(str::to_owned)
    };
    let matches = |n: &roxmltree::Node| {
        n.tag_name().name() == "dependency"
            && child(*n, "artifactId").as_deref() == Some(artifact)
            && child(*n, "groupId").is_none_or(|g| g == group || g.starts_with("${"))
    };
    let direct = |n: &roxmltree::Node| {
        n.parent().is_some_and(|p| {
            p.tag_name().name() == "dependencies"
                && p.parent().is_some_and(|pp| pp == doc.root_element())
        })
    };
    let all: Vec<_> = doc.descendants().filter(matches).collect();
    all.iter()
        .find(|n| direct(n))
        .or(all.first())
        .map_or(0, |n| doc.text_pos_at(n.range().start).row as usize - 1)
}

/// `gradle dependencies`의 `runtimeClasspath` 나무(깊이마다 다섯 칸). `그룹:이름:버전`, 고른 버전은
/// `-> 버전`, 끝에 `(*)`·`(c)`·`(n)`.
fn gradle(manifest: &Path, build: &str, output: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    let mut direct = (String::new(), 0);
    let section = output
        .lines()
        .skip_while(|l| !l.starts_with("runtimeClasspath"))
        .skip(1)
        .take_while(|l| !l.trim().is_empty());
    for line in section {
        let (prefix, rest) = split_tree(line);
        if prefix == 0 {
            continue;
        }
        let coords = rest.split(" (").next().unwrap_or(rest).trim();
        let depth = prefix / 5;
        let (left, chosen) = match coords.split_once(" -> ") {
            Some((l, r)) => (l, Some(r.trim())),
            None => (coords, None),
        };
        // 다른 하위 프로젝트(`project :lib`)는 그 아래의 끌어온 것만 본다.
        if let Some(project) = left.strip_prefix("project ") {
            if depth <= 1 {
                direct = (left.to_owned(), text_line(build, &[project]));
            }
            continue;
        }
        let parts: Vec<&str> = left.split(':').collect();
        let Some(version) = chosen.or(parts.get(2).copied()) else {
            continue;
        };
        if parts.len() < 2 {
            continue;
        }
        let name = format!("{}:{}", parts[0], parts[1]);
        if depth <= 1 {
            direct = (name.clone(), text_line(build, &[&name, parts[1]]));
        }
        out.push(Dependency {
            ecosystem: Ecosystem::Maven,
            version: version.to_owned(),
            file: manifest.to_path_buf(),
            line: direct.1,
            via: (depth > 1).then(|| direct.0.clone()),
            name,
        });
    }
    out
}

/// 글에서 `needles`를 차례로 찾아 처음 나오는 줄(0부터). 없으면 0.
fn text_line(text: &str, needles: &[&str]) -> usize {
    needles
        .iter()
        .find_map(|n| text.lines().position(|l| l.contains(n)))
        .unwrap_or(0)
}

/// 꾸민 JSON에서 `"키": {`가 처음 나오는 줄(0부터).
fn key_lines(text: &str) -> HashMap<&str, usize> {
    let mut lines = HashMap::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix('"')
            && let Some((key, after)) = rest.split_once('"')
            && after.trim_start().starts_with(':')
            && after.trim_end().ends_with('{')
        {
            lines.entry(key).or_insert(i);
        }
    }
    lines
}

/// `package-lock.json`. v2·v3는 `packages`(`node_modules/이름` 경로마다), v1은 `dependencies` 나무.
fn npm(path: &Path, text: &str) -> Result<Vec<Dependency>, String> {
    let lock: Value = serde_json::from_str(text)
        .map_err(|e| format!("{}을(를) 읽지 못했습니다: {e}", path.display()))?;
    let lines = key_lines(text);
    let mut out = Vec::new();
    let package = |key: &str| key.rsplit("node_modules/").next().unwrap_or(key).to_owned();
    if let Some(packages) = lock["packages"].as_object() {
        for (key, entry) in packages {
            if !key.contains("node_modules/") || entry["link"].as_bool() == Some(true) {
                continue;
            }
            let Some(version) = entry["version"].as_str() else {
                continue;
            };
            // `node_modules/a/node_modules/b`는 a가 끌어온 b.
            let via = key
                .rsplit_once("/node_modules/")
                .map(|(parent, _)| package(parent));
            out.push(Dependency {
                ecosystem: Ecosystem::Npm,
                name: entry["name"]
                    .as_str()
                    .map_or_else(|| package(key), str::to_owned),
                version: version.to_owned(),
                file: path.to_path_buf(),
                line: lines.get(key.as_str()).copied().unwrap_or(0),
                via,
            });
        }
    } else {
        fn walk_v1(
            deps: &Value,
            via: Option<&str>,
            path: &Path,
            lines: &HashMap<&str, usize>,
            out: &mut Vec<Dependency>,
        ) {
            for (name, entry) in deps.as_object().into_iter().flatten() {
                let Some(version) = entry["version"].as_str() else {
                    continue;
                };
                // 깃·파일 의존(`github:…`, `file:…`)은 레지스트리 버전이 아니다.
                if !version.contains(':') {
                    out.push(Dependency {
                        ecosystem: Ecosystem::Npm,
                        name: name.clone(),
                        version: version.to_owned(),
                        file: path.to_path_buf(),
                        line: lines.get(name.as_str()).copied().unwrap_or(0),
                        via: via.map(str::to_owned),
                    });
                }
                walk_v1(&entry["dependencies"], Some(name), path, lines, out);
            }
        }
        walk_v1(&lock["dependencies"], None, path, &lines, &mut out);
    }
    Ok(out)
}

/// `requirements*.txt`의 `이름==버전`(고정한 것만). 주석·환경 표시(`;`)·옵션 줄·해시는 건너뛴다.
fn pip(path: &Path, text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let spec = line.split('#').next().unwrap_or_default();
        let spec = spec.split(';').next().unwrap_or_default().trim();
        if spec.starts_with('-') {
            continue;
        }
        let Some((name, version)) = spec.split_once("==") else {
            continue;
        };
        let name = name.split('[').next().unwrap_or_default().trim();
        let version = version
            .trim_start_matches('=')
            .split([' ', '\t', '\\', ','])
            .next()
            .unwrap_or_default();
        if name.is_empty() || version.is_empty() || version.contains('*') {
            continue;
        }
        out.push(Dependency {
            ecosystem: Ecosystem::PyPI,
            name: Ecosystem::PyPI.normalize(name),
            version: version.to_owned(),
            file: path.to_path_buf(),
            line: i,
            via: None,
        });
    }
    out
}

/// `Cargo.lock`의 `[[package]]` 중 crates.io에서 온 것(경로·깃 의존은 뺀다).
fn cargo(path: &Path, text: &str) -> Vec<Dependency> {
    let mut out = Vec::new();
    let value = |line: &str, key: &str| {
        line.strip_prefix(key)?
            .trim_start()
            .strip_prefix('=')?
            .trim()
            .strip_prefix('"')?
            .strip_suffix('"')
            .map(str::to_owned)
    };
    // (이름, 버전, 줄, crates.io인지)
    let mut current: Option<(String, String, usize, bool)> = None;
    let mut flush = |current: &mut Option<(String, String, usize, bool)>| {
        if let Some((name, version, line, true)) = current.take() {
            out.push(Dependency {
                ecosystem: Ecosystem::Crates,
                name,
                version,
                file: path.to_path_buf(),
                line,
                via: None,
            });
        }
    };
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.starts_with('[') {
            flush(&mut current);
            if line == "[[package]]" {
                current = Some((String::new(), String::new(), i, false));
            }
        } else if let Some(package) = &mut current {
            if let Some(name) = value(line, "name") {
                package.0 = name;
                package.2 = i;
            } else if let Some(version) = value(line, "version") {
                package.1 = version;
            } else if let Some(source) = value(line, "source") {
                package.3 = source.contains("crates.io");
            }
        }
    }
    flush(&mut current);
    out
}
