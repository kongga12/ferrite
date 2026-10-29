//! SARIF 2.1.0(정적 분석 결과 교환 형식) 읽기. SpotBugs·PMD·Semgrep·clang 정적 분석기 등이 낸 결과를
//! 파일·줄·수준·메시지로 바꾼다. 도구를 돌리는 일은 실행 구성에 맡긴다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Note,
}

/// 결과 하나.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub tool: String,
    pub rule: String,
    pub level: Level,
    pub message: String,
    /// 규칙 설명 주소(`helpUri`).
    pub help: Option<String>,
    /// SARIF에 적힌 파일.
    pub uri: String,
    /// 프로젝트에서 찾은 파일. 못 찾으면 None.
    pub path: Option<PathBuf>,
    /// 1부터. 없으면 0(파일 전체).
    pub line: u32,
    /// 1부터. 없으면 0(줄 전체).
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

/// SARIF 글을 읽어 결과로 바꾼다. 파일은 `root`(프로젝트)와 `dir`(SARIF 파일의 폴더)에서 찾는다.
pub fn parse(text: &str, root: &Path, dir: &Path) -> Result<Vec<Finding>, String> {
    let sarif: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("SARIF를 읽지 못했습니다: {e}"))?;
    let runs = sarif
        .get("runs")
        .and_then(Value::as_array)
        .ok_or("SARIF에 runs가 없습니다")?;
    let mut findings = Vec::new();
    // 같은 파일의 결과가 많으므로 찾은 경로를 기억한다.
    let mut found: HashMap<(String, String), Option<PathBuf>> = HashMap::new();
    for run in runs {
        let driver = &run["tool"]["driver"];
        let tool = driver["name"].as_str().unwrap_or("SARIF").to_owned();
        let rules = driver["rules"].as_array().cloned().unwrap_or_default();
        let bases = run.get("originalUriBaseIds");
        for result in run["results"].as_array().into_iter().flatten() {
            // 통과·해당 없음은 문제가 아니다.
            if matches!(result["kind"].as_str(), Some("pass" | "notApplicable")) {
                continue;
            }
            let rule_id = result["ruleId"]
                .as_str()
                .or_else(|| result["rule"]["id"].as_str())
                .unwrap_or_default()
                .to_owned();
            let rule = result["ruleIndex"]
                .as_u64()
                .and_then(|i| rules.get(i as usize))
                .or_else(|| rules.iter().find(|r| r["id"] == rule_id.as_str()))
                .cloned()
                .unwrap_or(Value::Null);
            let level = result["level"]
                .as_str()
                .or_else(|| rule["defaultConfiguration"]["level"].as_str())
                .map_or(Level::Warning, |l| match l {
                    "error" => Level::Error,
                    "note" | "none" => Level::Note,
                    _ => Level::Warning,
                });
            let location = &result["locations"][0]["physicalLocation"];
            let artifact = &location["artifactLocation"];
            let uri = artifact["uri"].as_str().unwrap_or_default().to_owned();
            let base = artifact["uriBaseId"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let path = found
                .entry((base.clone(), uri.clone()))
                .or_insert_with(|| locate(&uri, &base, bases, root, dir))
                .clone();
            let region = &location["region"];
            let number = |key: &str| region[key].as_u64().unwrap_or(0) as u32;
            let line = number("startLine");
            findings.push(Finding {
                tool: tool.clone(),
                message: message(&result["message"], &rule),
                help: rule["helpUri"].as_str().map(str::to_owned),
                rule: rule_id,
                level,
                uri,
                path,
                line,
                column: number("startColumn"),
                end_line: number("endLine").max(line),
                end_column: number("endColumn"),
            });
        }
    }
    Ok(findings)
}

/// 결과의 메시지. 규칙의 메시지 틀(`messageStrings`)이 있으면 인자를 채운 것을 쓴다(SpotBugs는 짧은
/// `text`와 자세한 틀을 함께 준다). 없으면 `text`, 그것도 없으면 규칙의 짧은 설명.
fn message(message: &Value, rule: &Value) -> String {
    let arguments: Vec<String> = message["arguments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| a.as_str().map_or_else(|| a.to_string(), str::to_owned))
        .collect();
    let template = message["id"]
        .as_str()
        .and_then(|id| rule["messageStrings"][id]["text"].as_str());
    match (template, message["text"].as_str()) {
        (Some(template), text) if !arguments.is_empty() || text.is_none() => {
            fill(template, &arguments)
        }
        (_, Some(text)) => fill(text, &arguments),
        _ => rule["shortDescription"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    }
}

/// 메시지 틀의 `{0}`·`{1}`을 인자로 채운다. `{{`·`}}`는 중괄호 하나다.
fn fill(template: &str, arguments: &[String]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut number = String::new();
                while let Some(d) = chars.next_if(char::is_ascii_digit) {
                    number.push(d);
                }
                match (
                    chars.next_if_eq(&'}'),
                    number.parse::<usize>().ok().and_then(|n| arguments.get(n)),
                ) {
                    (Some(_), Some(argument)) => out.push_str(argument),
                    (closed, _) => {
                        out.push('{');
                        out.push_str(&number);
                        if closed.is_some() {
                            out.push('}');
                        }
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// SARIF의 파일 주소를 프로젝트의 파일로: 기준 주소(`uriBaseId` → `originalUriBaseIds`)와 잇거나,
/// `file:` 주소면 그 경로, 아니면 SARIF 폴더·프로젝트 기준 상대 경로, 그래도 없으면 경로 끝이 같은 파일
/// (SpotBugs는 원본 폴더 기준 `com/example/Board.java`만 적는다).
fn locate(
    uri: &str,
    base: &str,
    bases: Option<&Value>,
    root: &Path,
    dir: &Path,
) -> Option<PathBuf> {
    if uri.is_empty() {
        return None;
    }
    let base = bases
        .and_then(|b| b[base]["uri"].as_str())
        .filter(|_| !base.is_empty());
    let joined = match base {
        Some(base) if !uri.contains(':') => format!("{}/{uri}", base.trim_end_matches('/')),
        _ => uri.to_owned(),
    };
    if let Some(path) = file_path(&joined).filter(|p| p.is_file()) {
        return Some(path);
    }
    let relative = decode(&joined);
    if !relative.contains(':') {
        for dir in [dir, root] {
            let path = dir.join(&relative);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    let tail = relative
        .rsplit_once("://")
        .map_or(relative.as_str(), |(_, t)| t);
    crate::files::find_suffix(root, tail)
}

/// `file:` 주소의 경로(`file:///C:/a`, `file:/C:/a`, `file:///home/a`).
fn file_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file:")?;
    let rest = rest.trim_start_matches('/');
    let decoded = decode(rest);
    // Windows 드라이브(`C:/…`)가 아니면 절대 경로의 `/`를 되살린다.
    let windows = decoded.as_bytes().get(1) == Some(&b':');
    Some(PathBuf::from(if windows {
        decoded
    } else {
        format!("/{decoded}")
    }))
}

/// URI의 `%XX`를 푼다.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).and_then(|b| hex(*b)),
            bytes.get(i + 2).and_then(|b| hex(*b)),
        ) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 프로젝트의 SARIF 파일(`*.sarif`, `*.sarif.json`, SpotBugs의 `spotbugsSarif.json`). 최근에 고친 것이 앞이다.
pub fn candidates(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    crate::files::walk(root, 6, &mut |path, is_dir| {
        if !is_dir {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if name.ends_with(".sarif")
                || name.ends_with(".sarif.json")
                || name == "spotbugssarif.json"
            {
                let modified = std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                found.push((modified, path.to_path_buf()));
            }
        }
        true
    });
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests;
