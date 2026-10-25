//! 실제 egov 프로젝트 코퍼스로 문자열 치환(`${}`·`$x$`) 탐지에 빠진 것이 없는지 본다.
//! 고객사 소스는 저장소에 넣지 않으므로 로컬 경로를 환경 변수로만 받는다.
//! `FERRITE_EGOV_CORPUS=D:\works cargo test -p ferrite-egov --test corpus -- --ignored --nocapture`
//!
//! 파서(tree-sitter-xml)와 따로 짠 단순 검사기로 같은 파일을 세어 맞춰 본다. 단순 검사기는 주석을
//! 지우고 CDATA·태그 밖 텍스트만 남겨 센다. 둘이 다르면 파서가 놓쳤거나 잘못 잡은 것이다.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ferrite_core::buffer::Buffer;
use ferrite_core::jsp::{self, Kind};
use ferrite_core::syntax::{Lang, Syntax};
use ferrite_core::text_file;
use ferrite_egov::mapper::{self, Dialect, Severity};

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

/// 코퍼스에서 이 확장자(대소문자 무시)의 파일을 모두 찾는다.
fn files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if !SKIP_DIRS.iter().any(|s| entry.file_name() == *s) {
                        stack.push(path);
                    }
                }
                Ok(_)
                    if path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x))) =>
                {
                    found.push(path)
                }
                _ => {}
            }
        }
    }
    found
}

/// 주석을 빼고 CDATA 안과 태그 밖 텍스트만 돌려준다.
fn text_segments(text: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
        } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>").unwrap_or(after.len());
            segments.push(&after[..end]);
            rest = after.get(end + 3..).unwrap_or("");
        } else if rest.starts_with('<') {
            // DOCTYPE 등 태그 안은 SQL이 아니다. 속성값의 '>'는 매퍼에서 쓰지 않는다고 본다.
            rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            segments.push(&rest[..end]);
            rest = &rest[end..];
        }
    }
    segments
}

fn count(segment: &str, dialect: Dialect) -> usize {
    match dialect {
        Dialect::MyBatis => {
            let mut n = 0;
            let mut rest = segment;
            while let Some(start) = rest.find("${") {
                match rest[start..].find('}') {
                    Some(end) => {
                        n += 1;
                        rest = &rest[start + end + 1..];
                    }
                    None => break,
                }
            }
            n
        }
        Dialect::IBatis => {
            let chars: Vec<char> = segment.chars().collect();
            let mut n = 0;
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '$'
                    && chars
                        .get(i + 1)
                        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_')
                {
                    let len = chars[i + 1..]
                        .iter()
                        .take_while(|c| {
                            c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '[' | ']')
                        })
                        .count();
                    if chars.get(i + 1 + len) == Some(&'$') {
                        n += 1;
                        i += len + 2;
                        continue;
                    }
                }
                i += 1;
            }
            n
        }
    }
}

/// 문서의 첫 요소 이름(prolog·DOCTYPE·주석 다음).
fn root_name(text: &str) -> Option<&str> {
    let mut rest = text;
    loop {
        let start = rest.find('<')?;
        rest = &rest[start..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->")? + 3..];
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = &rest[rest.find('>')? + 1..];
        } else {
            let name = &rest[1..];
            let end = name.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
            return Some(&name[..end]);
        }
    }
}

#[test]
#[ignore = "로컬 egov 코퍼스가 필요하다(FERRITE_EGOV_CORPUS)"]
fn 코퍼스의_문자열_치환을_빠짐없이_찾는다() {
    let Some(root) = std::env::var_os("FERRITE_EGOV_CORPUS") else {
        eprintln!("FERRITE_EGOV_CORPUS가 없어 건너뜁니다");
        return;
    };
    let (mut mybatis, mut ibatis, mut found_total) = (0, 0, 0);
    let mut failures = Vec::new();
    let mut mismatches = Vec::new();
    for path in files(Path::new(&root), &["xml"]) {
        let Ok(bytes) = text_file::read(&path) else {
            continue;
        };
        let Ok(content) = text_file::decode(&bytes) else {
            continue;
        };
        let text = &content.text;
        let dialect = match root_name(text) {
            Some("mapper") => Dialect::MyBatis,
            Some("sqlMap") => Dialect::IBatis,
            _ => continue,
        };
        match dialect {
            Dialect::MyBatis => mybatis += 1,
            Dialect::IBatis => ibatis += 1,
        }
        let Some(parsed) = mapper::parse(text) else {
            failures.push(path);
            continue;
        };
        let expected: usize = text_segments(text).iter().map(|s| count(s, dialect)).sum();
        found_total += parsed.substitutions.len();
        if parsed.substitutions.len() != expected {
            mismatches.push((path, parsed.substitutions.len(), expected));
        }
    }
    eprintln!(
        "매퍼 {}개(MyBatis {mybatis}, iBatis {ibatis}), 찾은 치환 {found_total}곳, 분석 실패 {}, 불일치 {}",
        mybatis + ibatis,
        failures.len(),
        mismatches.len()
    );
    for (path, found, expected) in mismatches.iter().take(10) {
        eprintln!(
            "  불일치 {}: 파서 {found}, 단순 검사 {expected}",
            path.display()
        );
    }
    for path in failures.iter().take(10) {
        eprintln!("  분석 실패 {}", path.display());
    }
    assert!(failures.is_empty() && mismatches.is_empty());
}

/// 프로젝트(코퍼스 바로 아래 폴더)마다 Mapper 인터페이스와 매퍼 XML을 이어 본다. "XML 문장이 없는
/// 메서드" 경고가 실제 코드에서 얼마나 나오는지(오탐 위험) 보려는 통계다.
#[test]
#[ignore = "로컬 egov 코퍼스가 필요하다(FERRITE_EGOV_CORPUS)"]
fn 코퍼스의_mapper_인터페이스와_xml_연결_통계() {
    use ferrite_egov::java;

    let Some(root) = std::env::var_os("FERRITE_EGOV_CORPUS") else {
        eprintln!("FERRITE_EGOV_CORPUS가 없어 건너뜁니다");
        return;
    };
    let (mut bound, mut methods, mut missing) = (0, 0, Vec::new());
    for project in std::fs::read_dir(&root).unwrap().flatten() {
        if !project.path().is_dir() {
            continue;
        }
        let mut namespaces: std::collections::HashMap<String, Vec<String>> = Default::default();
        let mut interfaces = Vec::new();
        let mut stack = vec![project.path()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if !SKIP_DIRS.iter().any(|s| entry.file_name() == *s) {
                        stack.push(path);
                    }
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !matches!(ext, "xml" | "java") {
                    continue;
                }
                let Some(text) = text_file::read(&path)
                    .ok()
                    .and_then(|b| text_file::decode(&b).ok())
                else {
                    continue;
                };
                if ext == "java" {
                    interfaces.extend(
                        java::parse_file(&text.text)
                            .types
                            .into_iter()
                            .filter(|t| t.is_interface)
                            .map(|i| (path.clone(), i)),
                    );
                } else if let Some(m) = mapper::parse(&text.text)
                    && m.dialect == Dialect::MyBatis
                    && let Some(namespace) = m.namespace.clone()
                {
                    let ids = m
                        .defs
                        .iter()
                        .filter(|d| d.kind == mapper::Kind::Statement)
                        .map(|d| d.id.clone());
                    namespaces.entry(namespace).or_default().extend(ids);
                }
            }
        }
        for (path, interface) in interfaces {
            let Some(ids) = namespaces.get(&interface.fqn) else {
                continue;
            };
            bound += 1;
            for method in interface.methods.iter().filter(|m| m.needs_statement) {
                methods += 1;
                if !ids.contains(&method.name) {
                    missing.push(format!(
                        "{}::{} ({})",
                        interface.fqn,
                        method.name,
                        path.display()
                    ));
                }
            }
        }
    }
    eprintln!(
        "XML과 이어진 Mapper 인터페이스 {bound}개, XML이 필요한 메서드 {methods}개, 문장이 없는 메서드 {}개",
        missing.len()
    );
    for line in missing.iter().take(15) {
        eprintln!("  {line}");
    }
}

/// 매퍼의 parameterType(iBatis parameterClass)이 별칭을 거쳐 프로젝트 클래스로 풀리는 비율.
/// `#{}` 속성 완성이 실제 코드에서 얼마나 동작하는지 본다.
#[test]
#[ignore = "로컬 egov 코퍼스가 필요하다(FERRITE_EGOV_CORPUS)"]
fn 코퍼스의_parameter_type_해석_통계() {
    use ferrite_egov::java;

    let Some(root) = std::env::var_os("FERRITE_EGOV_CORPUS") else {
        eprintln!("FERRITE_EGOV_CORPUS가 없어 건너뜁니다");
        return;
    };
    let (mut total, mut maps, mut in_project, mut outside) = (0, 0, 0, 0);
    let mut unresolved: std::collections::BTreeMap<String, usize> = Default::default();
    for project in std::fs::read_dir(&root).unwrap().flatten() {
        if !project.path().is_dir() {
            continue;
        }
        let mut classes: Vec<String> = Vec::new();
        let mut aliases = Vec::new();
        let mut types: Vec<String> = Vec::new();
        for path in files(&project.path(), &["xml"])
            .into_iter()
            .chain(files(&project.path(), &["java"]))
        {
            let Some(text) = text_file::read(&path)
                .ok()
                .and_then(|b| text_file::decode(&b).ok())
            else {
                continue;
            };
            if path.extension().is_some_and(|e| e == "java") {
                classes.extend(
                    java::parse_file(&text.text)
                        .types
                        .into_iter()
                        .map(|t| t.fqn),
                );
                continue;
            }
            aliases.push(mapper::aliases(&text.text));
            if mapper::parse(&text.text).is_none() {
                continue;
            }
            for key in ["parameterType=\"", "parameterClass=\""] {
                for (at, _) in text.text.match_indices(key) {
                    let rest = &text.text[at + key.len()..];
                    if let Some(end) = rest.find('"') {
                        types.push(rest[..end].to_owned());
                    }
                }
            }
        }
        let alias_refs: Vec<&mapper::Aliases> = aliases.iter().collect();
        let class_refs: Vec<&str> = classes.iter().map(String::as_str).collect();
        for ty in types {
            total += 1;
            let lower = ty.to_lowercase();
            if [
                "map",
                "hashmap",
                "java.util.map",
                "java.util.hashmap",
                "egovmap",
            ]
            .contains(&lower.as_str())
                || lower.ends_with(".egovmap")
                || lower.ends_with("map") && !lower.contains('.')
            {
                maps += 1;
                continue;
            }
            let fqn = mapper::resolve_alias(&ty, &alias_refs, &class_refs);
            if class_refs.contains(&fqn.as_str()) {
                in_project += 1;
            } else if fqn.contains('.') {
                outside += 1;
            } else {
                *unresolved.entry(ty).or_default() += 1;
            }
        }
    }
    let unresolved_total: usize = unresolved.values().sum();
    eprintln!(
        "parameterType {total}개: 프로젝트 클래스로 풀림 {in_project}, Map 계열 {maps}, 프로젝트 밖 클래스(java.lang 등) {outside}, 못 풂 {unresolved_total}"
    );
    let mut top: Vec<_> = unresolved.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (ty, n) in top.iter().take(10) {
        eprintln!("  못 푼 이름 {ty}: {n}");
    }
}

/// `${`의 수를 따로 센다(EL 구간 찾기의 누락 검사). JSP 주석과 `<% %>` 안은 빼고, `\${`도 뺀다.
/// EL 안의 `${`(문자열 속)는 세지 않도록 닫는 `}`까지 건너뛴다.
fn count_el(text: &str) -> usize {
    let mut n = 0;
    let mut rest = text;
    while let Some(at) = rest.find(['<', '$']) {
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("<%--") {
            rest = after.find("--%>").map_or("", |end| &after[end + 4..]);
        } else if let Some(after) = tail.strip_prefix("<%") {
            rest = after.find("%>").map_or("", |end| &after[end + 2..]);
        } else if let Some(after) = tail.strip_prefix("${") {
            if !rest[..at].ends_with('\\') {
                n += 1;
            }
            rest = skip_el(after);
        } else {
            rest = &tail[1..];
        }
    }
    n
}

/// EL 안쪽을 건너뛴다(따옴표와 중괄호 짝을 센다).
fn skip_el(after: &str) -> &str {
    let mut depth = 0;
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in after.char_indices() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some(_) if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '{' => depth += 1,
                '}' if depth == 0 => return &after[i + 1..],
                '}' => depth -= 1,
                _ => {}
            },
        }
    }
    ""
}

/// 코퍼스의 JSP 전부로 구간 찾기·구문 강조·XSS 진단이 죽지 않는지, `${`를 빠짐없이 찾는지 본다.
/// 진단이 실제 코드에서 얼마나 나오는지(잡음)도 센다.
#[test]
#[ignore = "로컬 egov 코퍼스가 필요하다(FERRITE_EGOV_CORPUS)"]
fn 코퍼스의_jsp_el을_빠짐없이_찾고_xss를_센다() {
    let Some(root) = std::env::var_os("FERRITE_EGOV_CORPUS") else {
        eprintln!("FERRITE_EGOV_CORPUS가 없어 건너뜁니다");
        return;
    };
    let (mut pages, mut el_total, mut warnings, mut infos) = (0, 0, 0, 0);
    let mut mismatches = Vec::new();
    let mut top: std::collections::HashMap<String, usize> = Default::default();
    let mut request_samples = Vec::new();
    // 단계별 누적 시간과 가장 오래 걸린 파싱(편집할 때마다 문서 전체를 다시 파싱하므로 본다).
    let mut spent = [Duration::ZERO; 3];
    let mut slowest = (Duration::ZERO, 0);
    for path in files(Path::new(&root), &["jsp", "jspf", "tag"]) {
        let Ok(bytes) = text_file::read(&path) else {
            continue;
        };
        let Ok(content) = text_file::decode(&bytes) else {
            continue;
        };
        let text = &content.text;
        pages += 1;
        let started = Instant::now();
        let regions = jsp::regions(text);
        spent[0] += started.elapsed();
        let found = regions
            .iter()
            .filter(|r| r.kind == Kind::El && text[r.range.start..].starts_with('$'))
            .count();
        el_total += found;
        let expected = count_el(text);
        if found != expected {
            mismatches.push((path.clone(), found, expected));
        }
        // 구문 강조가 문서 전체에서 죽지 않는지.
        let mut buffer = Buffer::new(text);
        buffer.set_track_syntax_edits(true);
        let mut syntax = Syntax::new(Lang::Jsp).unwrap();
        let parse = Instant::now();
        syntax.update(&mut buffer);
        let parsed = parse.elapsed();
        spent[1] += parsed;
        slowest = slowest.max((parsed, text.len()));
        let lines = buffer.rope().len_lines();
        let _ = syntax.highlight_lines(buffer.rope(), 0..lines);
        let diagnose = Instant::now();
        let findings = ferrite_egov::jsp::diagnose(text);
        spent[2] += diagnose.elapsed();
        for finding in findings {
            let shown = text[finding.range.clone()].to_owned();
            match finding.severity {
                Severity::Warning => {
                    warnings += 1;
                    if request_samples.len() < 15 {
                        request_samples.push(shown);
                    }
                }
                _ => {
                    infos += 1;
                    *top.entry(shown).or_default() += 1;
                }
            }
        }
    }
    eprintln!(
        "JSP {pages}개, EL {el_total}곳, 불일치 {}, 경고(요청 값) {warnings}, 정보(그 밖의 값) {infos}",
        mismatches.len()
    );
    eprintln!(
        "  시간: 구간 {:?}, 파싱 {:?}, 진단 {:?}. 가장 오래 걸린 파싱 {:?}({}바이트)",
        spent[0], spent[1], spent[2], slowest.0, slowest.1
    );
    for (path, found, expected) in mismatches.iter().take(10) {
        eprintln!(
            "  불일치 {}: 찾음 {found}, 단순 검사 {expected}",
            path.display()
        );
    }
    for shown in &request_samples {
        eprintln!("  경고 예: {shown}");
    }
    let mut top: Vec<_> = top.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (shown, n) in top.iter().take(20) {
        eprintln!("  정보 상위 {n}: {shown}");
    }
    assert!(mismatches.is_empty());
}
