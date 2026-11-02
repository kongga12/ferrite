//! EditorConfig(`.editorconfig`): 파일의 폴더부터 위로 `root = true`인 파일까지 읽어, 그 파일에 맞는
//! 절(`[*.java]`)의 설정을 모은다. 가까운 파일과 뒤의 절이 앞의 것을 덮는다. 들여쓰기·탭 폭·줄바꿈·
//! 끝 공백·마지막 줄바꿈을 따르고, `charset`은 따르지 않는다(CP949 감지·보존을 지금대로 둔다).
// ponytail: 글롭은 globset으로 맞춘다. 숫자 범위(`{1..3}`)와 조각 한가운데의 `**`(`a**b`)는 다르게
// 맞춰질 수 있다. 쓰는 곳이 생기면 EditorConfig 글롭을 따로 옮긴다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use globset::GlobBuilder;

use crate::buffer::Indent;
use crate::text_file::LineEnding;

/// 한 파일에 적용되는 설정. 적지 않은 것은 None.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    /// 들여쓰기 방식(`indent_style`): 탭이면 true.
    pub tabs: Option<bool>,
    /// 들여쓰기 칸 수(`indent_size`, `tab`이면 `tab_width`).
    pub indent_size: Option<u8>,
    /// 탭 문자의 표시 폭(`tab_width`, 없으면 `indent_size`).
    pub tab_width: Option<usize>,
    pub end_of_line: Option<LineEnding>,
    pub trim_trailing_whitespace: Option<bool>,
    pub insert_final_newline: Option<bool>,
    /// 읽은 `.editorconfig`(가까운 것부터). 설정을 준 파일이 없으면 비어 있다.
    pub files: Vec<PathBuf>,
}

impl Properties {
    /// 정한 것이 하나라도 있는지.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_none()
            && self.indent_size.is_none()
            && self.tab_width.is_none()
            && self.end_of_line.is_none()
            && self.trim_trailing_whitespace.is_none()
            && self.insert_final_newline.is_none()
    }

    /// 들여쓰기: 방식을 정했으면 그것, 칸 수만 정했으면 감지한 공백 들여쓰기의 칸 수만 바꾼다.
    pub fn indent(&self, detected: Indent) -> Indent {
        match (self.tabs, detected) {
            (Some(true), _) => Indent::Tabs,
            (Some(false), Indent::Spaces(n)) => Indent::Spaces(self.indent_size.unwrap_or(n)),
            (Some(false), Indent::Tabs) => Indent::Spaces(self.indent_size.unwrap_or(4)),
            (None, Indent::Spaces(n)) => Indent::Spaces(self.indent_size.unwrap_or(n)),
            (None, Indent::Tabs) => Indent::Tabs,
        }
    }
}

/// 그 파일의 설정을 모은다. 읽지 못한 `.editorconfig`는 없는 것으로 본다.
pub fn properties(path: &Path) -> Properties {
    // 위(루트)부터 가까운 쪽으로 덮으므로 먼저 가까운 것부터 모아 뒤집는다.
    let mut configs = Vec::new();
    for dir in path.ancestors().skip(1) {
        let file = dir.join(".editorconfig");
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let config = parse(&text);
        let root = config.root;
        configs.push((dir.to_path_buf(), file, config));
        if root {
            break;
        }
    }
    let mut values: HashMap<String, String> = HashMap::new();
    let mut files = Vec::new();
    for (dir, file, config) in configs.into_iter().rev() {
        let Some(relative) = relative(&dir, path) else {
            continue;
        };
        let mut used = false;
        for (glob, pairs) in &config.sections {
            if matches(glob, &relative) {
                used = true;
                for (key, value) in pairs {
                    values.insert(key.clone(), value.clone());
                }
            }
        }
        if used {
            files.insert(0, file);
        }
    }
    values.retain(|_, v| v != "unset");
    let number = |key: &str| values.get(key).and_then(|v| v.parse::<usize>().ok());
    let tabs = match values.get("indent_style").map(String::as_str) {
        Some("tab") => Some(true),
        Some("space") => Some(false),
        _ => None,
    };
    let tab_width = number("tab_width").or_else(|| number("indent_size"));
    let indent_size = match values.get("indent_size").map(String::as_str) {
        Some("tab") => tab_width,
        _ => number("indent_size"),
    }
    .filter(|n| (1..=16).contains(n))
    .map(|n| n as u8);
    let flag = |key: &str| match values.get(key).map(String::as_str) {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    };
    Properties {
        tabs,
        indent_size,
        tab_width: tab_width.filter(|n| (1..=16).contains(n)),
        // CR만 쓰는 줄바꿈은 저장 형식에 없어 따르지 않는다.
        end_of_line: match values.get("end_of_line").map(String::as_str) {
            Some("lf") => Some(LineEnding::Lf),
            Some("crlf") => Some(LineEnding::CrLf),
            _ => None,
        },
        trim_trailing_whitespace: flag("trim_trailing_whitespace"),
        insert_final_newline: flag("insert_final_newline"),
        files,
    }
}

struct Config {
    root: bool,
    /// (절의 글롭, (이름, 값)들). 이름과 값은 소문자.
    sections: Vec<(String, Vec<(String, String)>)>,
}

fn parse(text: &str) -> Config {
    let mut config = Config {
        root: false,
        sections: Vec::new(),
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            config.sections.push((glob.to_owned(), Vec::new()));
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim().to_lowercase(), value.trim().to_lowercase());
        match config.sections.last_mut() {
            Some((_, pairs)) => pairs.push((key, value)),
            // 첫 절 앞(머리)에는 root만 있다.
            None if key == "root" => config.root = value == "true",
            None => {}
        }
    }
    config
}

/// `.editorconfig` 폴더에서 파일까지의 상대 경로(`/`로 잇는다).
fn relative(dir: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(dir).ok()?;
    let parts: Vec<String> = rest
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}

/// 절의 글롭이 그 파일에 맞는지. `/`가 있으면 `.editorconfig` 폴더 기준, 없으면 어느 깊이든.
fn matches(glob: &str, relative: &str) -> bool {
    let glob = if glob.contains('/') {
        glob.trim_start_matches('/').to_owned()
    } else {
        format!("**/{glob}")
    };
    GlobBuilder::new(&double_star(&glob))
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .is_ok_and(|g| g.compile_matcher().is_match(relative))
}

/// globset은 `**`를 경로 조각 하나로만 받는다. EditorConfig의 `**`는 `/`를 넘는 아무 글이므로, 조각
/// 앞의 `**.min.js`는 `**/*.min.js`로, 조각 끝의 `src**`는 `src*/**`로 바꾼다.
fn double_star(glob: &str) -> String {
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            let starts = i == 0 || chars[i - 1] == '/';
            let ends = chars.get(i + 2).is_none_or(|&c| c == '/');
            out.push_str(match (starts, ends) {
                (true, false) => "**/*",
                (false, true) => "*/**",
                _ => "**",
            });
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ferrite-editorconfig-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, path: &str, text: &str) -> PathBuf {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        file
    }

    #[test]
    fn 가까운_파일과_뒤의_절이_덮고_root에서_멈춘다() {
        let root = temp_root("merge");
        // root 위의 것은 읽지 않는다.
        write(&root, ".editorconfig", "[*]\nindent_style = tab\n");
        write(
            &root,
            "app/.editorconfig",
            "root = true\n\n# 모든 파일\n[*]\nindent_style = space\nindent_size = 4\nend_of_line = crlf\n\
             insert_final_newline = true\ntrim_trailing_whitespace = true\n\n\
             [*.{js,jsp}]\nindent_size = 2\n\n[*.md]\ntrim_trailing_whitespace = false\n\n\
             [src/main/webapp/**.xml]\nend_of_line = lf\n",
        );
        write(
            &root,
            "app/web/.editorconfig",
            "[*.js]\nINDENT_SIZE = 3\n[Makefile]\nindent_style = tab\n",
        );
        let java = properties(&write(&root, "app/src/A.java", ""));
        assert_eq!(java.indent(Indent::Tabs), Indent::Spaces(4));
        assert_eq!(java.tab_width, Some(4));
        assert_eq!(java.end_of_line, Some(LineEnding::CrLf));
        assert_eq!(
            (java.trim_trailing_whitespace, java.insert_final_newline),
            (Some(true), Some(true))
        );
        assert_eq!(java.files, [root.join("app/.editorconfig")]);
        // 절의 `{a,b}`, 가까운 파일이 덮음, 이름은 대소문자를 가리지 않음
        let jsp = properties(&write(&root, "app/web/main.jsp", ""));
        assert_eq!(jsp.indent_size, Some(2));
        let js = properties(&write(&root, "app/web/main.js", ""));
        assert_eq!(js.indent_size, Some(3));
        assert_eq!(js.files.len(), 2);
        let make = properties(&write(&root, "app/web/Makefile", ""));
        assert_eq!(make.indent(Indent::Spaces(2)), Indent::Tabs);
        assert_eq!(
            properties(&write(&root, "app/README.md", "")).trim_trailing_whitespace,
            Some(false)
        );
        // `/`가 있는 글롭은 그 폴더 기준
        let xml = properties(&write(&root, "app/src/main/webapp/WEB-INF/web.xml", ""));
        assert_eq!(xml.end_of_line, Some(LineEnding::Lf));
        let other = properties(&write(&root, "app/web/src/main/webapp/a.xml", ""));
        assert_eq!(other.end_of_line, Some(LineEnding::CrLf));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn 절의_글롭() {
        assert!(matches("*.java", "src/main/A.java"));
        assert!(matches("/*.java", "A.java"));
        assert!(!matches("/*.java", "src/A.java"), "/로 시작하면 그 폴더만");
        assert!(matches("**.min.js", "webapp/js/jquery.min.js"));
        assert!(matches("src/**.xml", "src/main/webapp/WEB-INF/web.xml"));
        assert!(matches("lib/**", "lib/a/b.js"));
        assert!(matches("{package.json,.travis.yml}", "web/package.json"));
        assert!(matches("[!a]*.txt", "b.txt"));
        assert!(!matches("[!a]*.txt", "a.txt"));
        assert!(matches("*.{js,jsp}", "main.jsp"));
        assert!(!matches("*.js", "main.jsp"));
    }

    #[test]
    fn 탭_폭과_unset과_모르는_값() {
        let root = temp_root("values");
        write(
            &root,
            ".editorconfig",
            "root = true\n[*]\nindent_style = tab\nindent_size = tab\ntab_width = 8\nend_of_line = cr\n\
             [*.txt]\nindent_style = unset\nindent_size = 99\ninsert_final_newline = maybe\n",
        );
        let c = properties(&write(&root, "a.c", ""));
        assert_eq!(
            (c.tabs, c.indent_size, c.tab_width),
            (Some(true), Some(8), Some(8))
        );
        assert_eq!(c.end_of_line, None, "CR은 따르지 않는다");
        let txt = properties(&write(&root, "notes.txt", ""));
        assert_eq!(txt.tabs, None);
        assert_eq!(txt.indent_size, None, "범위 밖");
        assert_eq!(txt.insert_final_newline, None);
        assert_eq!(txt.indent(Indent::Spaces(2)), Indent::Spaces(2));
        // 없는 폴더
        let none = properties(&std::env::temp_dir().join("ferrite-no-editorconfig/a.rs"));
        assert!(none.is_empty() && none.files.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
