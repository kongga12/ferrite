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

/// 응답이 올 때까지 기다린다(그사이 온 진단은 버린다).
fn response(lsp: &ferrite_lsp::client::LanguageServer, id: i64) -> Option<serde_json::Value> {
    use ferrite_lsp::client::ServerEvent;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        for event in lsp.poll_events() {
            match event {
                ServerEvent::Response { id: got, result } if got == id => return result.ok(),
                ServerEvent::Exited(reason) => panic!("서버가 끝남: {reason}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("응답이 오지 않음");
}

/// 프로젝트(코퍼스 바로 아래 폴더)마다 실제 서버를 띄워, JSP에서 쓰는 taglib 접두사의 태그 목록이
/// 나오는지, include 경로와 컨트롤러의 뷰 이름이 파일로 이어지는지 센다.
#[test]
#[ignore = "로컬 egov 코퍼스가 필요하다(FERRITE_EGOV_CORPUS)"]
fn 코퍼스의_taglib_include_뷰_이름_연결_통계() {
    use std::collections::HashMap;

    use ferrite_egov::{java, web};
    use ferrite_lsp::client::{LanguageServer, ServerCommand};
    use ferrite_lsp::features::{
        completion_params, parse_completion, parse_locations, position_params,
    };
    use ropey::Rope;

    let Some(root) = std::env::var_os("FERRITE_EGOV_CORPUS") else {
        eprintln!("FERRITE_EGOV_CORPUS가 없어 건너뜁니다");
        return;
    };
    let read = |path: &Path| {
        text_file::read(path)
            .ok()
            .and_then(|b| text_file::decode(&b).ok())
            .map(|c| c.text)
    };
    // (접두사 → (풀림, 못 풂)), include·뷰는 (풀림, 전체)와 못 푼 예.
    let mut prefixes: HashMap<String, (usize, usize)> = HashMap::new();
    let (mut includes, mut views) = ((0, 0), (0, 0));
    let (mut missed_includes, mut missed_views) = (Vec::new(), Vec::new());
    // 못 푼 뷰 중 같은 이름의 JSP가 프로젝트에 있는 것(진짜 놓친 것일 수 있다). 없으면 JSON·엑셀
    // 같은 빈 뷰, Thymeleaf, 뷰가 아닌 문자열이다.
    let mut views_with_jsp = 0;
    let started = Instant::now();
    for project in std::fs::read_dir(&root).unwrap().flatten() {
        let project = project.path();
        let jsps = files(&project, &["jsp"]);
        if !project.is_dir() || jsps.is_empty() {
            continue;
        }
        let command = ServerCommand {
            program: env!("CARGO_BIN_EXE_ferrite-egov-ls").into(),
            ..Default::default()
        };
        let mut lsp = LanguageServer::start(&command, &project, || {}).unwrap();
        let encoding = lsp.encoding();
        for path in &jsps {
            let Some(text) = read(path) else { continue };
            // 쓰는 접두사(`<c:`). `jsp:`는 표준 액션이라 taglib이 아니다.
            let mut used: Vec<String> = text
                .match_indices('<')
                .filter_map(|(at, _)| {
                    let rest = &text[at + 1..];
                    let len = rest
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                        .unwrap_or(rest.len());
                    (len > 0 && rest[len..].starts_with(':')).then(|| rest[..len].to_owned())
                })
                .filter(|p| p != "jsp")
                .collect();
            used.sort();
            used.dedup();
            // 문서 끝에 `<접두사:`를 줄마다 붙여 그 자리에서 자동완성한다.
            let mut probe = text.clone();
            let mut sites = Vec::new();
            for prefix in &used {
                probe.push_str(&format!("\n<{prefix}:"));
                sites.push((prefix.clone(), probe.len()));
            }
            lsp.did_open(path, "jsp", 1, &probe);
            let rope = Rope::from_str(&probe);
            for (prefix, offset) in sites {
                let params =
                    completion_params(path, &rope, rope.byte_to_char(offset), encoding, None)
                        .unwrap();
                let id = lsp.request("textDocument/completion", params);
                let found = response(&lsp, id)
                    .map(|r| !parse_completion(&r, &rope, encoding).items.is_empty())
                    .unwrap_or(false);
                let entry = prefixes.entry(prefix).or_default();
                if found {
                    entry.0 += 1;
                } else {
                    entry.1 += 1;
                }
            }
            // include 경로(EL·URL이 섞인 것은 뺀다). 태그·지시자의 첫 따옴표 안에서 이동해 본다.
            let starts = ["<%@", "<jsp:include", "<jsp:forward", "<c:import"]
                .iter()
                .flat_map(|needle| text.match_indices(needle).map(|(at, _)| at));
            for start in starts.collect::<Vec<_>>() {
                let tag_end = text[start..].find('>').map_or(text.len(), |e| start + e);
                let Some(quote) = text[start..tag_end].find(['"', '\'']).map(|q| start + q) else {
                    continue;
                };
                let Some(value) = web::include_at(&probe, quote + 1) else {
                    continue;
                };
                if value.contains("${") || value.contains("<%") || value.contains("://") {
                    continue;
                }
                let params =
                    position_params(path, &rope, rope.byte_to_char(quote + 1), encoding).unwrap();
                let id = lsp.request("textDocument/definition", params);
                includes.1 += 1;
                if response(&lsp, id).is_some_and(|r| !parse_locations(&r).is_empty()) {
                    includes.0 += 1;
                } else if missed_includes.len() < 10 {
                    missed_includes.push(value.to_owned());
                }
            }
            lsp.did_close(path);
        }
        // 컨트롤러의 `return "뷰";`
        for path in files(&project, &["java"]) {
            let Some(text) = read(&path) else { continue };
            if !text.contains("@Controller") {
                continue;
            }
            let returns: Vec<(usize, String)> = text
                .match_indices("return \"")
                .filter_map(|(at, m)| {
                    let start = at + m.len();
                    let end = start + text[start..].find('"')?;
                    let name = &text[start..end];
                    // 주석 처리한 코드의 return은 문자열이 아니다(Java 파서로 거른다).
                    (!name.is_empty()
                        && !name.contains(':')
                        && !name.contains(char::is_whitespace)
                        && java::target_at(&text, start + 1).is_some())
                    .then(|| (start, name.to_owned()))
                })
                .collect();
            if returns.is_empty() {
                continue;
            }
            lsp.did_open(&path, "java", 1, &text);
            let rope = Rope::from_str(&text);
            for (start, name) in returns {
                let params =
                    position_params(&path, &rope, rope.byte_to_char(start + 1), encoding).unwrap();
                let id = lsp.request("textDocument/definition", params);
                views.1 += 1;
                let found = response(&lsp, id).is_some_and(|r| {
                    parse_locations(&r)
                        .iter()
                        .any(|t| t.path.extension().is_some_and(|e| e == "jsp"))
                });
                let tail = format!(
                    "{}.jsp",
                    name.trim_matches('/').rsplit('/').next().unwrap_or("")
                );
                let has_jsp = jsps.iter().any(|j| {
                    j.file_name()
                        .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(&tail))
                });
                if found {
                    views.0 += 1;
                } else if has_jsp {
                    views_with_jsp += 1;
                    if missed_views.len() < 15 {
                        missed_views.push(format!(
                            "{}: {name}",
                            project.file_name().unwrap().to_string_lossy()
                        ));
                    }
                }
            }
            lsp.did_close(&path);
        }
    }
    let mut ranked: Vec<_> = prefixes.into_iter().collect();
    ranked.sort_by_key(|(_, (ok, miss))| std::cmp::Reverse(ok + miss));
    eprintln!("시간 {:?}", started.elapsed());
    for (prefix, (ok, miss)) in ranked.iter().take(20) {
        eprintln!("  접두사 {prefix}: 태그 목록 나옴 {ok}, 안 나옴 {miss} (파일 수)");
    }
    eprintln!("include {}/{} 풀림", includes.0, includes.1);
    for value in &missed_includes {
        eprintln!("  못 푼 include: {value}");
    }
    eprintln!(
        "뷰 이름 {}/{} 풀림. 못 푼 것 중 같은 이름 JSP가 있는 것 {views_with_jsp}",
        views.0, views.1
    );
    for value in &missed_views {
        eprintln!("  못 푼 뷰: {value}");
    }
}

/// 프로젝트마다 실제 서버를 띄워 Spring XML 설정을 하나씩 열고, 나오는 진단을 종류별로 센다
/// (잡음 확인). `${키}` 이동이 프로퍼티 파일로 이어지는지도 센다.
#[test]
#[ignore = "로컬 egov 코퍼스가 필요하다(FERRITE_EGOV_CORPUS)"]
fn 코퍼스의_spring_xml_진단_통계() {
    use std::collections::HashMap;

    use ferrite_egov::spring;
    use ferrite_lsp::client::{LanguageServer, ServerCommand, ServerEvent, parse_diagnostics};
    use ferrite_lsp::features::{parse_locations, position_params};
    use ropey::Rope;

    let Some(root) = std::env::var_os("FERRITE_EGOV_CORPUS") else {
        eprintln!("FERRITE_EGOV_CORPUS가 없어 건너뜁니다");
        return;
    };
    let read = |path: &Path| {
        text_file::read(path)
            .ok()
            .and_then(|b| text_file::decode(&b).ok())
            .map(|c| c.text)
    };
    let (mut configs, mut entities) = (0, 0);
    let mut by_code: HashMap<String, (usize, Vec<String>)> = HashMap::new();
    let (mut placeholders, mut resolved) = (0, 0);
    let started = Instant::now();
    for project in std::fs::read_dir(&root).unwrap().flatten() {
        let project = project.path();
        if !project.is_dir() {
            continue;
        }
        let xmls: Vec<(PathBuf, String, spring::Config)> = files(&project, &["xml"])
            .into_iter()
            .filter_map(|path| {
                let text = read(&path)?;
                let config = spring::parse(&text)?;
                Some((path, text, config))
            })
            .collect();
        // `@GeneratedValue(IDENTITY)`를 쓰는 엔티티(DB에 따라 알림이 나와야 한다).
        let identity: Vec<(PathBuf, String)> = files(&project, &["java"])
            .into_iter()
            .filter_map(|path| Some((path.clone(), read(&path)?)))
            .filter(|(_, text)| text.contains("@GeneratedValue") && text.contains("IDENTITY"))
            .collect();
        if xmls.is_empty() && identity.is_empty() {
            continue;
        }
        let command = ServerCommand {
            program: env!("CARGO_BIN_EXE_ferrite-egov-ls").into(),
            ..Default::default()
        };
        let mut lsp = LanguageServer::start(&command, &project, || {}).unwrap();
        let encoding = lsp.encoding();
        let documents = xmls
            .iter()
            .map(|(path, text, config)| (path, text, Some(config), "xml"))
            .chain(
                identity
                    .iter()
                    .map(|(path, text)| (path, text, None, "java")),
            );
        for (path, text, config, language) in documents {
            if config.is_some() {
                configs += 1;
            } else {
                entities += 1;
            }
            lsp.did_open(path, language, 1, text);
            // 진단이 없으면 알림도 없다. 뒤이은 요청의 응답이 오기 전까지 온 진단을 모은다
            // (서버는 메시지를 차례로 처리한다).
            let probe = position_params(path, &Rope::from_str(text), 0, encoding).unwrap();
            let id = lsp.request("textDocument/definition", probe);
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut diagnostics = Vec::new();
            'wait: loop {
                for event in lsp.poll_events() {
                    match event {
                        ServerEvent::Notification { method, params }
                            if method == "textDocument/publishDiagnostics" =>
                        {
                            if let Some((got, found)) = parse_diagnostics(&params)
                                && got.file_name() == path.file_name()
                            {
                                diagnostics = found;
                            }
                        }
                        ServerEvent::Response { id: got, .. } if got == id => break 'wait,
                        _ => {}
                    }
                }
                assert!(Instant::now() < deadline, "응답이 오지 않음");
                std::thread::sleep(Duration::from_millis(2));
            }
            let rope = Rope::from_str(text);
            for d in diagnostics {
                let code = match &d.code {
                    Some(lsp_types::NumberOrString::String(code)) => code.clone(),
                    _ => String::new(),
                };
                let start = ferrite_lsp::position::from_lsp(&rope, d.range.start, encoding);
                let end = ferrite_lsp::position::from_lsp(&rope, d.range.end, encoding);
                let entry = by_code.entry(code).or_default();
                entry.0 += 1;
                if entry.1.len() < 8 {
                    entry.1.push(format!(
                        "{}/{}: {}",
                        project.file_name().unwrap().to_string_lossy(),
                        path.file_name().unwrap().to_string_lossy(),
                        rope.slice(start..end)
                    ));
                }
            }
            for usage in config.iter().flat_map(|c| c.placeholders.iter()) {
                placeholders += 1;
                let char_idx = rope.byte_to_char(usage.range.start + 1);
                let params = position_params(path, &rope, char_idx, encoding).unwrap();
                let id = lsp.request("textDocument/definition", params);
                if response(&lsp, id).is_some_and(|r| !parse_locations(&r).is_empty()) {
                    resolved += 1;
                }
            }
            lsp.did_close(path);
        }
    }
    eprintln!(
        "시간 {:?}, Spring XML 설정 {configs}개, IDENTITY를 쓰는 엔티티 {entities}개",
        started.elapsed()
    );
    let mut codes: Vec<_> = by_code.into_iter().collect();
    codes.sort();
    for (code, (count, samples)) in &codes {
        eprintln!("  {code}: {count}");
        for sample in samples {
            eprintln!("    {sample}");
        }
    }
    eprintln!("플레이스홀더 {resolved}/{placeholders} 이동");
}
