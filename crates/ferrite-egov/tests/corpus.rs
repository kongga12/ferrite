//! 실제 egov 프로젝트 코퍼스로 문자열 치환(`${}`·`$x$`) 탐지에 빠진 것이 없는지 본다.
//! 고객사 소스는 저장소에 넣지 않으므로 로컬 경로를 환경 변수로만 받는다.
//! `FERRITE_EGOV_CORPUS=D:\works cargo test -p ferrite-egov --test corpus -- --ignored --nocapture`
//!
//! 파서(tree-sitter-xml)와 따로 짠 단순 검사기로 같은 파일을 세어 맞춰 본다. 단순 검사기는 주석을
//! 지우고 CDATA·태그 밖 텍스트만 남겨 센다. 둘이 다르면 파서가 놓쳤거나 잘못 잡은 것이다.

use std::path::{Path, PathBuf};

use ferrite_core::text_file;
use ferrite_egov::mapper::{self, Dialect};

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

fn xml_files(root: &Path) -> Vec<PathBuf> {
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
                        .is_some_and(|e| e.eq_ignore_ascii_case("xml")) =>
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
    for path in xml_files(Path::new(&root)) {
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
