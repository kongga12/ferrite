//! 편집기 확장 폴더 읽기. `package.json`의 `contributes.languages`(id, aliases, extensions,
//! filenames, configuration)와 `contributes.grammars`(language, scopeName, path)를 읽는다. 확장의
//! 코드(JS)는 쓰지 않는다. 문법은 JSON(`.tmLanguage.json`)만 읽는다.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::grammar::{Grammar, compile_grammar};

pub struct Language {
    pub id: String,
    /// 보이는 이름(aliases의 첫째, 없으면 id).
    pub name: String,
    pub line_comment: Option<String>,
    /// 한 글자 괄호 짝.
    pub brackets: Vec<(char, char)>,
    /// 문법의 scopeName과, 처음 쓸 때 옮긴 문법. 다른 문법을 많이 부르는 문법(마크다운 등)은
    /// 옮기는 데 시간이 걸리므로 그 언어 파일을 처음 열 때 옮긴다.
    scope: Option<String>,
    grammars: Arc<HashMap<String, Value>>,
    grammar: OnceLock<Option<Arc<Grammar>>>,
    /// 소문자 확장자(`.foo`).
    extensions: Vec<String>,
    filenames: Vec<String>,
}

impl Language {
    /// 이 언어의 TextMate 문법(없거나 옮기지 못했으면 없음).
    pub fn grammar(&self) -> Option<Arc<Grammar>> {
        self.grammar
            .get_or_init(|| {
                let scope = self.scope.as_deref()?;
                compile_grammar(scope, &self.grammars).ok().map(Arc::new)
            })
            .clone()
    }
}

/// 읽은 언어 확장들.
#[derive(Default)]
pub struct Registry {
    languages: Vec<Arc<Language>>,
}

/// 문법을 옮기기 전의 언어.
struct Pending {
    language: Language,
    scope: Option<String>,
}

impl Registry {
    /// `root`의 하위 폴더 중 `package.json`이 있는 것을 이름 순으로 읽는다. 같은 언어 id는 먼저 읽은
    /// 것에 확장자·파일 이름을 더한다. 읽지 못한 항목은 오류로 모으고 나머지는 계속 읽는다.
    pub fn load(root: &Path) -> (Self, Vec<String>) {
        let mut errors = Vec::new();
        let mut dirs: Vec<PathBuf> = match fs::read_dir(root) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join("package.json").is_file())
                .collect(),
            Err(_) => Vec::new(),
        };
        dirs.sort();
        let mut pending: Vec<Pending> = Vec::new();
        let mut grammars: HashMap<String, Value> = HashMap::new();
        for dir in dirs {
            for e in read_extension(&dir, &mut pending, &mut grammars) {
                errors.push(format!("{}: {e}", dir.display()));
            }
        }
        let grammars = Arc::new(grammars);
        let languages = pending
            .into_iter()
            .map(
                |Pending {
                     mut language,
                     scope,
                 }| {
                    language.scope = scope;
                    language.grammars = Arc::clone(&grammars);
                    Arc::new(language)
                },
            )
            .collect();
        (Self { languages }, errors)
    }

    pub fn len(&self) -> usize {
        self.languages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.languages.is_empty()
    }

    /// 파일 이름이 같은 언어, 없으면 가장 긴 확장자가 맞는 언어.
    pub fn language_for(&self, path: &Path) -> Option<Arc<Language>> {
        let name = path.file_name()?.to_string_lossy();
        if let Some(l) = self
            .languages
            .iter()
            .find(|l| l.filenames.iter().any(|f| *f == name))
        {
            return Some(Arc::clone(l));
        }
        let lower = name.to_lowercase();
        self.languages
            .iter()
            .filter_map(|l| {
                let ext = l
                    .extensions
                    .iter()
                    .filter(|e| lower.ends_with(e.as_str()))
                    .max_by_key(|e| e.len())?;
                Some((ext.len(), l))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, l)| Arc::clone(l))
    }
}

/// 확장 폴더 하나를 읽는다. 돌려주는 것은 오류들.
fn read_extension(
    dir: &Path,
    pending: &mut Vec<Pending>,
    grammars: &mut HashMap<String, Value>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let manifest = match read_json(&dir.join("package.json")) {
        Ok(m) => m,
        Err(e) => return vec![format!("package.json: {e}")],
    };
    let contributes = &manifest["contributes"];
    for entry in contributes["languages"].as_array().into_iter().flatten() {
        let Some(id) = entry["id"].as_str() else {
            continue;
        };
        let strings = |key: &str| -> Vec<String> {
            entry[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let extensions: Vec<String> = strings("extensions")
            .iter()
            .map(|e| e.to_lowercase())
            .collect();
        let filenames = strings("filenames");
        let config = match entry["configuration"].as_str() {
            Some(rel) => match inside(dir, rel).and_then(|p| read_json(&p)) {
                Ok(config) => Some(config),
                Err(e) => {
                    errors.push(format!("{rel}: {e}"));
                    None
                }
            },
            None => None,
        };
        if let Some(p) = pending.iter_mut().find(|p| p.language.id == id) {
            p.language.extensions.extend(extensions);
            p.language.filenames.extend(filenames);
            if let Some(config) = &config
                && p.language.line_comment.is_none()
                && p.language.brackets.is_empty()
            {
                (p.language.line_comment, p.language.brackets) = language_config(config);
            }
            continue;
        }
        let (line_comment, brackets) = config.as_ref().map(language_config).unwrap_or_default();
        let name = strings("aliases")
            .into_iter()
            .next()
            .unwrap_or_else(|| id.to_owned());
        pending.push(Pending {
            language: Language {
                id: id.to_owned(),
                name,
                line_comment,
                brackets,
                scope: None,
                grammars: Arc::default(),
                grammar: OnceLock::new(),
                extensions,
                filenames,
            },
            scope: None,
        });
    }
    for entry in contributes["grammars"].as_array().into_iter().flatten() {
        let (Some(scope), Some(rel)) = (entry["scopeName"].as_str(), entry["path"].as_str()) else {
            continue;
        };
        if !rel.to_lowercase().ends_with(".json") {
            errors.push(format!("{rel}: JSON 문법(.tmLanguage.json)만 읽습니다"));
            continue;
        }
        match inside(dir, rel).and_then(|p| read_json(&p)) {
            Ok(grammar) => {
                grammars.entry(scope.to_owned()).or_insert(grammar);
            }
            Err(e) => {
                errors.push(format!("{rel}: {e}"));
                continue;
            }
        }
        if let Some(language) = entry["language"].as_str()
            && let Some(p) = pending.iter_mut().find(|p| p.language.id == language)
        {
            p.scope.get_or_insert_with(|| scope.to_owned());
        }
    }
    errors
}

/// 주석 기호와 괄호 짝(`language-configuration.json`).
fn language_config(config: &Value) -> (Option<String>, Vec<(char, char)>) {
    let comment = &config["comments"]["lineComment"];
    // 새 형식은 `{ "comment": "//" }`다.
    let line_comment = comment
        .as_str()
        .or_else(|| comment["comment"].as_str())
        .map(str::to_owned);
    let brackets = config["brackets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pair| {
            let one = |i: usize| {
                let s = pair.get(i)?.as_str()?;
                let mut chars = s.chars();
                let c = chars.next()?;
                chars.next().is_none().then_some(c)
            };
            Some((one(0)?, one(1)?))
        })
        .collect();
    (line_comment, brackets)
}

/// 확장 폴더 안의 경로. 폴더 밖(`..`, 절대 경로)은 받지 않는다.
fn inside(dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let path = Path::new(rel);
    if !path
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err("확장 폴더 밖의 파일입니다".into());
    }
    Ok(dir.join(path))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&strip_jsonc(&text)).map_err(|e| e.to_string())
}

/// 주석(`//`, `/* */`)과 끝 쉼표를 뺀다(language-configuration.json은 JSONC다). 문자열 안은 그대로 둔다.
pub(crate) fn strip_jsonc(text: &str) -> String {
    let mut code = String::with_capacity(text.len());
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            code.push(c);
            if c == '\\' {
                code.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                code.push(c);
            }
            ('/', Some('/')) => while chars.next_if(|&n| n != '\n').is_some() {},
            ('/', Some('*')) => {
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                code.push(' ');
            }
            _ => code.push(c),
        }
    }
    // 끝 쉼표: 공백 뒤에 `}`·`]`가 오는 쉼표(문자열 밖).
    let chars: Vec<char> = code.chars().collect();
    let mut out = String::with_capacity(code.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            if c == '\\' {
                out.push(c);
                i += 1;
                if let Some(&n) = chars.get(i) {
                    out.push(n);
                }
            } else {
                in_string = c != '"';
                out.push(c);
            }
            i += 1;
            continue;
        }
        in_string = c == '"';
        let trailing = c == ','
            && chars[i + 1..]
                .iter()
                .find(|n| !n.is_whitespace())
                .is_some_and(|n| matches!(n, '}' | ']'));
        if !trailing {
            out.push(c);
        }
        i += 1;
    }
    out
}
