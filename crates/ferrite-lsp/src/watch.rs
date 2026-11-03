//! 디스크 바뀜 알림(`workspace/didChangeWatchedFiles`)의 동적 등록. 서버가 `client/registerCapability`로
//! 등록한 패턴을 기억했다가, 앱이 본 바뀜 중 맞는 것만 그 서버에 보낸다.
//! 글 패턴은 전체 경로에, 상대 패턴(`baseUri` + `pattern`)은 그 폴더 아래 경로에 맞춘다.

use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use lsp_types::Uri;
use serde_json::Value;

use crate::uri::uri_to_path;

/// LSP `FileChangeType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileChange {
    Created = 1,
    Changed = 2,
    Deleted = 3,
}

impl FileChange {
    /// 등록의 `WatchKind` 비트(만들기 1, 바꾸기 2, 지우기 4).
    fn watch_kind(self) -> u64 {
        match self {
            FileChange::Created => 1,
            FileChange::Changed => 2,
            FileChange::Deleted => 4,
        }
    }
}

struct Watcher {
    glob: GlobMatcher,
    /// 상대 패턴의 기준 폴더.
    base: Option<PathBuf>,
    kind: u64,
}

impl Watcher {
    fn parse(value: &Value) -> Option<Watcher> {
        let (pattern, base) = match &value["globPattern"] {
            Value::String(pattern) => (pattern.as_str(), None),
            relative => {
                // baseUri는 URI 글이거나 작업 폴더({uri, name})다.
                let base = relative["baseUri"]
                    .as_str()
                    .or_else(|| relative["baseUri"]["uri"].as_str())?;
                let base = uri_to_path(&base.parse::<Uri>().ok()?)?;
                (relative["pattern"].as_str()?, Some(base))
            }
        };
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .case_insensitive(cfg!(windows))
            .build()
            .ok()?
            .compile_matcher();
        Some(Watcher {
            glob,
            base,
            kind: value["kind"].as_u64().unwrap_or(7),
        })
    }

    fn matches(&self, path: &Path, change: FileChange) -> bool {
        if self.kind & change.watch_kind() == 0 {
            return false;
        }
        let target = match &self.base {
            Some(base) => match path.strip_prefix(base) {
                Ok(relative) => relative,
                Err(_) => return false,
            },
            None => path,
        };
        self.glob
            .is_match(target.to_string_lossy().replace('\\', "/"))
    }
}

/// 서버 하나가 등록한 파일 감시들(등록 번호 → 패턴).
#[derive(Default)]
pub struct Watchers {
    registrations: Vec<(String, Vec<Watcher>)>,
}

impl Watchers {
    /// `client/registerCapability`의 params. 파일 감시 등록만 기억한다(같은 번호는 바꾼다).
    pub fn register(&mut self, params: &Value) {
        for registration in params["registrations"].as_array().into_iter().flatten() {
            if registration["method"] != "workspace/didChangeWatchedFiles" {
                continue;
            }
            let id = registration["id"].as_str().unwrap_or_default().to_owned();
            let watchers = registration["registerOptions"]["watchers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Watcher::parse)
                .collect();
            self.registrations.retain(|(known, _)| *known != id);
            self.registrations.push((id, watchers));
        }
    }

    /// `client/unregisterCapability`의 params(규격의 철자가 `unregisterations`다).
    pub fn unregister(&mut self, params: &Value) {
        for gone in params["unregisterations"].as_array().into_iter().flatten() {
            self.registrations
                .retain(|(id, _)| gone["id"].as_str() != Some(id));
        }
    }

    pub fn matches(&self, path: &Path, change: FileChange) -> bool {
        self.registrations
            .iter()
            .flat_map(|(_, watchers)| watchers)
            .any(|w| w.matches(path, change))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uri::path_to_uri;
    use serde_json::json;

    #[test]
    fn 등록한_패턴과_종류에_맞는_바뀜만_고른다() {
        let root = std::env::temp_dir().join("ferrite-watch");
        let base = path_to_uri(&root).unwrap();
        let mut watchers = Watchers::default();
        watchers.register(&json!({ "registrations": [
            { "id": "java", "method": "workspace/didChangeWatchedFiles", "registerOptions": { "watchers": [
                { "globPattern": "**/*.java" },
                // 지우기만, 기준 폴더 아래
                { "globPattern": { "baseUri": base.as_str(), "pattern": "src/**/*.xml" }, "kind": 4 },
                { "globPattern": { "baseUri": { "uri": base.as_str(), "name": "p" }, "pattern": "{pom.xml,build.gradle}" } },
            ] } },
            { "id": "other", "method": "workspace/didChangeConfiguration", "registerOptions": {} },
        ] }));
        let java = root.join("src").join("a").join("A.java");
        assert!(watchers.matches(&java, FileChange::Created));
        assert!(!watchers.matches(&root.join("A.txt"), FileChange::Changed));
        let mapper = root.join("src").join("m").join("B.xml");
        assert!(
            !watchers.matches(&mapper, FileChange::Changed),
            "지우기만 등록"
        );
        assert!(watchers.matches(&mapper, FileChange::Deleted));
        assert!(
            !watchers.matches(&root.join("B.xml"), FileChange::Deleted),
            "src 밖"
        );
        assert!(watchers.matches(&root.join("pom.xml"), FileChange::Changed));
        assert!(
            !watchers.matches(&root.join("sub").join("pom.xml"), FileChange::Changed),
            "*는 경로 구분자를 넘지 않는다"
        );

        watchers.unregister(&json!({ "unregisterations": [{ "id": "java", "method": "workspace/didChangeWatchedFiles" }] }));
        assert!(!watchers.matches(&java, FileChange::Created));
    }
}
