//! OSV(Open Source Vulnerabilities) 레코드 읽기와 대조, 온라인 조회(api.osv.dev), 오프라인 DB 묶음
//! (생태계별 `all.zip`, 레코드마다 JSON 하나).

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use super::version::compare;
use super::{Dependency, Ecosystem, Finding, Severity};

/// 온라인 조회 주소.
pub const API: &str = "https://api.osv.dev/v1";

/// 생태계 DB 묶음 주소.
pub fn database_url(ecosystem: Ecosystem) -> String {
    format!(
        "https://osv-vulnerabilities.storage.googleapis.com/{}/all.zip",
        ecosystem.osv()
    )
}

/// 레코드의 영향 범위 하나(OSV `affected[].ranges[].events`).
#[derive(Clone, Debug, PartialEq)]
enum Event {
    Introduced(String),
    Fixed(String),
    LastAffected(String),
}

/// 한 패키지에 대한 영향(OSV `affected[]`).
#[derive(Clone, Debug, PartialEq)]
struct Affected {
    ecosystem: Ecosystem,
    name: String,
    versions: Vec<String>,
    ranges: Vec<Vec<Event>>,
}

/// 취약점 레코드.
#[derive(Clone, Debug, PartialEq)]
pub struct Advisory {
    pub id: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub severity: Severity,
    affected: Vec<Affected>,
}

impl Advisory {
    /// OSV JSON 레코드를 읽는다. 철회한 것(`withdrawn`)은 None.
    pub fn parse(record: &Value) -> Option<Advisory> {
        if record.get("withdrawn").is_some_and(|w| !w.is_null()) {
            return None;
        }
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        let affected = record["affected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                let package = &a["package"];
                let ecosystem = Ecosystem::from_osv(package["ecosystem"].as_str()?)?;
                Some(Affected {
                    ecosystem,
                    name: ecosystem.normalize(package["name"].as_str()?),
                    versions: a["versions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                    ranges: a["ranges"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        // GIT 범위는 커밋이라 버전과 견줄 수 없다.
                        .filter(|r| r["type"] != "GIT")
                        .map(|r| {
                            r["events"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|e| {
                                    let v = |k: &str| e[k].as_str().map(str::to_owned);
                                    v("introduced")
                                        .map(Event::Introduced)
                                        .or_else(|| v("fixed").map(Event::Fixed))
                                        .or_else(|| v("last_affected").map(Event::LastAffected))
                                })
                                .collect()
                        })
                        .collect(),
                })
            })
            .collect();
        let summary = record["summary"]
            .as_str()
            .or_else(|| record["details"].as_str().and_then(|d| d.lines().next()))
            .unwrap_or_default()
            .trim()
            .to_owned();
        Some(Advisory {
            id: text(&record["id"]),
            aliases: record["aliases"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a.as_str().map(str::to_owned))
                .collect(),
            summary,
            severity: severity(record),
            affected,
        })
    }

    /// 이 레코드가 그 의존성 버전에 해당하면 고친 버전(모르면 None)을 돌려준다.
    pub fn affects(&self, dependency: &Dependency) -> Option<Option<String>> {
        let name = dependency.ecosystem.normalize(&dependency.name);
        let ecosystem = dependency.ecosystem;
        let version = dependency.version.as_str();
        let mut hit = false;
        let mut fixed = None;
        for affected in self
            .affected
            .iter()
            .filter(|a| a.ecosystem == ecosystem && a.name == name)
        {
            if affected.versions.iter().any(|v| v == version) {
                hit = true;
            }
            for events in &affected.ranges {
                if let Some(found) = in_range(ecosystem, version, events) {
                    hit = true;
                    fixed = fixed.or(found);
                }
            }
        }
        hit.then_some(fixed)
    }
}

/// 범위(이벤트 목록)에 버전이 드는지. 들면 그 구간을 고친 버전(없으면 None)을 돌려준다.
fn in_range(ecosystem: Ecosystem, version: &str, events: &[Event]) -> Option<Option<String>> {
    let le = |a: &str, b: &str| compare(ecosystem, a, b).is_le();
    let mut start: Option<&str> = None;
    for event in events {
        match event {
            Event::Introduced(v) => start = Some(v),
            Event::Fixed(v) => {
                if let Some(s) = start.take()
                    && (s == "0" || le(s, version))
                    && !le(v, version)
                {
                    return Some(Some(v.clone()));
                }
            }
            Event::LastAffected(v) => {
                if let Some(s) = start.take()
                    && (s == "0" || le(s, version))
                    && le(version, v)
                {
                    return Some(None);
                }
            }
        }
    }
    // 끝을 적지 않은 구간은 그 뒤 모든 버전.
    start.filter(|s| *s == "0" || le(s, version)).map(|_| None)
}

/// 심각도: GitHub 등의 `database_specific.severity`, 없으면 CVSS v3 점수.
fn severity(record: &Value) -> Severity {
    let label = record["database_specific"]["severity"]
        .as_str()
        .or_else(|| record["affected"][0]["database_specific"]["severity"].as_str())
        .or_else(|| record["affected"][0]["ecosystem_specific"]["severity"].as_str());
    if let Some(label) = label {
        match label.to_ascii_uppercase().as_str() {
            "CRITICAL" => return Severity::Critical,
            "HIGH" => return Severity::High,
            "MODERATE" | "MEDIUM" => return Severity::Moderate,
            "LOW" => return Severity::Low,
            _ => {}
        }
    }
    let score = record["severity"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["type"] == "CVSS_V3")
        .filter_map(|s| s["score"].as_str().and_then(cvss3))
        .fold(None, |best: Option<f64>, s| {
            Some(best.map_or(s, |b| b.max(s)))
        });
    match score {
        Some(s) if s >= 9.0 => Severity::Critical,
        Some(s) if s >= 7.0 => Severity::High,
        Some(s) if s >= 4.0 => Severity::Moderate,
        Some(s) if s > 0.0 => Severity::Low,
        _ => Severity::Unknown,
    }
}

/// CVSS v3 벡터(`CVSS:3.1/AV:N/AC:L/...`)의 기본 점수.
pub fn cvss3(vector: &str) -> Option<f64> {
    let metrics: HashMap<&str, &str> = vector
        .split('/')
        .skip(1)
        .filter_map(|m| m.split_once(':'))
        .collect();
    let get = |k: &str| metrics.get(k).copied();
    let scope_changed = get("S")? == "C";
    let av = match get("AV")? {
        "N" => 0.85,
        "A" => 0.62,
        "L" => 0.55,
        _ => 0.2,
    };
    let ac = if get("AC")? == "L" { 0.77 } else { 0.44 };
    let pr = match (get("PR")?, scope_changed) {
        ("N", _) => 0.85,
        ("L", false) => 0.62,
        ("L", true) => 0.68,
        (_, false) => 0.27,
        (_, true) => 0.5,
    };
    let ui = if get("UI")? == "N" { 0.85 } else { 0.62 };
    let cia = |k: &str| -> Option<f64> {
        Some(match get(k)? {
            "H" => 0.56,
            "L" => 0.22,
            _ => 0.0,
        })
    };
    let iss = 1.0 - (1.0 - cia("C")?) * (1.0 - cia("I")?) * (1.0 - cia("A")?);
    let impact = if scope_changed {
        7.52 * (iss - 0.029) - 3.25 * (iss - 0.02).powi(15)
    } else {
        6.42 * iss
    };
    if impact <= 0.0 {
        return Some(0.0);
    }
    let exploitability = 8.22 * av * ac * pr * ui;
    let base = if scope_changed {
        (1.08 * (impact + exploitability)).min(10.0)
    } else {
        (impact + exploitability).min(10.0)
    };
    // CVSS 3.1의 올림(소수 첫째 자리). 부동소수 오차를 피하려고 정수로 견준다.
    let scaled = (base * 100_000.0).round() as u64;
    Some(if scaled.is_multiple_of(10_000) {
        scaled as f64 / 100_000.0
    } else {
        (scaled / 10_000 + 1) as f64 / 10.0
    })
}

/// 레코드들과 의존성들을 견줘 취약한 것을 모은다. 같은 의존성의 같은 취약점은 하나로 합친다: OSV에는
/// 한 취약점이 여러 DB의 레코드(GHSA·PYSEC·RUSTSEC)로 있고 ID·다른 이름(CVE)이 겹친다. 합칠 때
/// 심각도는 높은(아는) 것, 고친 버전은 있는 것을 쓴다.
// ponytail: 결과마다 앞의 결과를 훑는다(의존성×취약점이 수천 건이면 느려진다). 그러면 이름 색인으로.
pub fn findings(advisories: &[Advisory], dependencies: &[Dependency]) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    for dependency in dependencies {
        for advisory in advisories {
            let Some(fixed) = advisory.affects(dependency) else {
                continue;
            };
            let names: Vec<&String> = std::iter::once(&advisory.id)
                .chain(&advisory.aliases)
                .collect();
            let same = out.iter_mut().find(|f| {
                f.dependency == *dependency
                    && (names.contains(&&f.id) || f.aliases.iter().any(|a| names.contains(&a)))
            });
            let Some(same) = same else {
                out.push(Finding {
                    dependency: dependency.clone(),
                    id: advisory.id.clone(),
                    aliases: advisory.aliases.clone(),
                    summary: advisory.summary.clone(),
                    severity: advisory.severity,
                    fixed,
                });
                continue;
            };
            same.severity = same.severity.min(advisory.severity);
            if same.fixed.is_none() {
                same.fixed = fixed;
            }
            if same.summary.is_empty() {
                same.summary.clone_from(&advisory.summary);
            }
            for name in names {
                if *name != same.id && !same.aliases.contains(name) {
                    same.aliases.push(name.clone());
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.dependency.name.cmp(&b.dependency.name))
    });
    out
}

fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_connect(Some(Duration::from_secs(20)))
        .user_agent(concat!("ferrite/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// 온라인으로 묻는다: 의존성 이름·버전만 api.osv.dev에 보내 해당 레코드를 받는다. 같은
/// (생태계, 이름, 버전)은 한 번만 묻는다.
pub fn query_online(dependencies: &[Dependency]) -> Result<Vec<Advisory>, String> {
    let agent = agent();
    let mut keys: Vec<(Ecosystem, String, String)> = dependencies
        .iter()
        .map(|d| (d.ecosystem, d.name.clone(), d.version.clone()))
        .collect();
    keys.sort();
    keys.dedup();
    let mut ids: Vec<String> = Vec::new();
    // querybatch는 한 번에 1000개까지 받는다.
    for chunk in keys.chunks(1000) {
        let queries: Vec<Value> = chunk
            .iter()
            .map(
                |(e, n, v)| json!({ "package": { "name": n, "ecosystem": e.osv() }, "version": v }),
            )
            .collect();
        let body: Value = agent
            .post(format!("{API}/querybatch"))
            .header("content-type", "application/json")
            .send(json!({ "queries": queries }).to_string())
            .map_err(|e| format!("OSV에 묻지 못했습니다: {e}"))
            .and_then(|mut r| {
                serde_json::from_reader(r.body_mut().as_reader())
                    .map_err(|e| format!("OSV 응답을 읽지 못했습니다: {e}"))
            })?;
        for result in body["results"].as_array().into_iter().flatten() {
            for vuln in result["vulns"].as_array().into_iter().flatten() {
                if let Some(id) = vuln["id"].as_str()
                    && !ids.iter().any(|i| i == id)
                {
                    ids.push(id.to_owned());
                }
            }
        }
    }
    let mut advisories = Vec::new();
    for id in ids {
        let record: Value = agent
            .get(format!("{API}/vulns/{id}"))
            .call()
            .map_err(|e| format!("OSV에서 {id}를 받지 못했습니다: {e}"))
            .and_then(|mut r| {
                serde_json::from_reader(r.body_mut().as_reader())
                    .map_err(|e| format!("OSV의 {id}를 읽지 못했습니다: {e}"))
            })?;
        advisories.extend(Advisory::parse(&record));
    }
    Ok(advisories)
}

/// 레코드의 패키지 이름만(묶음을 훑을 때 나머지는 읽지 않고 건너뛴다).
#[derive(serde::Deserialize)]
struct Names {
    #[serde(default)]
    affected: Vec<NamedPackage>,
}

#[derive(serde::Deserialize)]
struct NamedPackage {
    package: Option<PackageName>,
}

#[derive(serde::Deserialize)]
struct PackageName {
    ecosystem: String,
    name: String,
}

/// 오프라인 DB 묶음(`all.zip`)에서 이 의존성들의 패키지에 해당하는 레코드만 읽는다.
// ponytail: 검사할 때마다 묶음을 다 푼다(Maven 10MB는 몇 초, npm 220MB는 수십 초). 느리면 들일 때
// 패키지별 색인을 만든다.
pub fn query_offline(zip: &Path, dependencies: &[Dependency]) -> Result<Vec<Advisory>, String> {
    let file = std::fs::File::open(zip)
        .map_err(|e| format!("{}을(를) 열지 못했습니다: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("{}은(는) zip이 아닙니다: {e}", zip.display()))?;
    let names: HashSet<(Ecosystem, String)> = dependencies
        .iter()
        .map(|d| (d.ecosystem, d.ecosystem.normalize(&d.name)))
        .collect();
    let wanted = |p: &PackageName| {
        Ecosystem::from_osv(&p.ecosystem)
            .is_some_and(|e| names.contains(&(e, e.normalize(&p.name))))
    };
    let mut advisories = Vec::new();
    let mut bytes = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if !entry.name().is_ok_and(|n| n.ends_with(".json")) {
            continue;
        }
        bytes.clear();
        if entry.read_to_end(&mut bytes).is_err() {
            continue;
        }
        // 이름만 먼저 보고, 맞는 레코드만 다 읽는다.
        let hit = serde_json::from_slice::<Names>(&bytes).is_ok_and(|n| {
            n.affected
                .iter()
                .filter_map(|a| a.package.as_ref())
                .any(wanted)
        });
        if hit && let Ok(record) = serde_json::from_slice::<Value>(&bytes) {
            advisories.extend(Advisory::parse(&record));
        }
    }
    Ok(advisories)
}

/// 고른 DB 묶음 파일을 `dest`로 들인다(zip인지 본다).
pub fn import_database(source: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(source)
        .map_err(|e| format!("{}을(를) 열지 못했습니다: {e}", source.display()))?;
    zip::ZipArchive::new(file)
        .map_err(|e| format!("{}은(는) zip이 아닙니다: {e}", source.display()))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::copy(source, dest)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 생태계 DB 묶음을 받아 `dest`에 둔다(받는 동안은 옆의 `.download`). 진행(받은, 전체).
pub fn download_database(
    ecosystem: Ecosystem,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let partial = dest.with_extension("zip.download");
    let result = (|| -> Result<(), String> {
        let mut response = agent()
            .get(database_url(ecosystem))
            .call()
            .map_err(|e| format!("받지 못했습니다: {e}"))?;
        let total = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = response.body_mut().as_reader();
        let mut file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
        let mut buf = vec![0; 64 * 1024];
        let mut received = 0;
        loop {
            let n = body
                .read(&mut buf)
                .map_err(|e| format!("받다가 끊겼습니다: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            received += n as u64;
            progress(received, total);
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        // 받은 것이 zip인지 보고 바꾼다.
        zip::ZipArchive::new(std::fs::File::open(&partial).map_err(|e| e.to_string())?)
            .map_err(|e| format!("받은 파일이 zip이 아닙니다: {e}"))?;
        std::fs::rename(&partial, dest).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&partial);
    result
}
