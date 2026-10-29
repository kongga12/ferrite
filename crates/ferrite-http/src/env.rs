//! 환경 파일: `http-client.env.json`(환경별 변수, 저장소에 둔다)과 `http-client.private.env.json`(비밀 값,
//! git에서 뺀다). `.http` 파일의 폴더부터 프로젝트 루트까지 올라가며 처음 찾은 폴더의 것을 쓴다.
//!
//! ```json
//! { "dev": { "baseUrl": "http://localhost:8080" }, "prod": { "baseUrl": "https://example.com" } }
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub const PUBLIC: &str = "http-client.env.json";
pub const PRIVATE: &str = "http-client.private.env.json";

#[derive(Debug, Default, PartialEq)]
pub struct Environments {
    /// 환경 이름 → 변수. 개인 파일의 값이 이긴다.
    pub values: BTreeMap<String, BTreeMap<String, String>>,
    /// 환경 파일이 있는 폴더.
    pub dir: Option<PathBuf>,
}

impl Environments {
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    pub fn get(&self, env: &str, name: &str) -> Option<&str> {
        self.values.get(env)?.get(name).map(String::as_str)
    }

    /// 개인 파일(있으면).
    pub fn private_file(&self) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|d| d.join(PRIVATE))
            .filter(|p| p.is_file())
    }
}

/// `dir`부터 `root`(포함, 없으면 `dir`만)까지 올라가며 환경 파일을 찾는다. 읽지 못한 파일은 오류로.
pub fn load(dir: &Path, root: Option<&Path>) -> (Environments, Vec<String>) {
    let mut errors = Vec::new();
    let mut current = Some(dir);
    while let Some(d) = current {
        if d.join(PUBLIC).is_file() || d.join(PRIVATE).is_file() {
            let mut envs = Environments {
                dir: Some(d.to_path_buf()),
                ..Default::default()
            };
            for name in [PUBLIC, PRIVATE] {
                let path = d.join(name);
                if !path.is_file() {
                    continue;
                }
                match read(&path) {
                    Ok(values) => {
                        for (env, vars) in values {
                            envs.values.entry(env).or_default().extend(vars);
                        }
                    }
                    Err(e) => errors.push(format!("{}: {e}", path.display())),
                }
            }
            return (envs, errors);
        }
        if root.is_none_or(|r| d == r) {
            break;
        }
        current = d.parent().filter(|p| root.is_none_or(|r| p.starts_with(r)));
    }
    (Environments::default(), errors)
}

fn read(path: &Path) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let json: Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let object = json
        .as_object()
        .ok_or("환경 이름을 키로 둔 객체여야 합니다")?;
    Ok(object
        .iter()
        .filter_map(|(env, vars)| {
            let vars = vars
                .as_object()?
                .iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), v)
                })
                .collect();
            Some((env.clone(), vars))
        })
        .collect())
}

/// 프로젝트의 `.gitignore`가 개인 환경 파일을 빼는가(이름이나 `*.private.env.json`).
pub fn ignored_by_git(root: &Path) -> bool {
    fs::read_to_string(root.join(".gitignore")).is_ok_and(|text| {
        text.lines().map(str::trim).any(|l| {
            let l = l.trim_start_matches("**/").trim_start_matches('/');
            l == PRIVATE || l == "*.private.env.json"
        })
    })
}

/// `.gitignore`에 개인 환경 파일을 더한다(없으면 만든다).
pub fn add_to_gitignore(root: &Path) -> std::io::Result<()> {
    let path = root.join(".gitignore");
    let mut text = fs::read_to_string(&path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# HTTP 요청의 비밀 값(Ferrite)\n");
    text.push_str(PRIVATE);
    text.push('\n');
    fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 위로_올라가며_찾고_개인_값이_이긴다() {
        let root = std::env::temp_dir().join(format!("ferrite-http-env-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let sub = root.join("http/board");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            root.join("http").join(PUBLIC),
            r#"{"dev": {"baseUrl": "http://localhost:8080", "port": 8080, "user": "x"}, "prod": {"baseUrl": "https://a"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("http").join(PRIVATE),
            r#"{"dev": {"user": "관리자", "pw": "비밀"}}"#,
        )
        .unwrap();
        let (envs, errors) = load(&sub, Some(&root));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(envs.names().collect::<Vec<_>>(), ["dev", "prod"]);
        assert_eq!(envs.get("dev", "port"), Some("8080"), "숫자도 글로");
        assert_eq!(envs.get("dev", "user"), Some("관리자"));
        assert_eq!(envs.get("dev", "pw"), Some("비밀"));
        assert_eq!(envs.private_file(), Some(root.join("http").join(PRIVATE)));
        // 루트 밖으로는 올라가지 않는다.
        assert_eq!(load(&root, Some(&root)).0, Environments::default());

        assert!(!ignored_by_git(&root));
        fs::write(root.join(".gitignore"), "target/").unwrap();
        add_to_gitignore(&root).unwrap();
        assert!(ignored_by_git(&root));
        assert!(
            fs::read_to_string(root.join(".gitignore"))
                .unwrap()
                .starts_with("target/\n")
        );
        fs::write(root.join("http").join(PUBLIC), "{ 깨짐").unwrap();
        assert_eq!(load(&sub, Some(&root)).1.len(), 1);
        fs::remove_dir_all(&root).unwrap();
    }
}
