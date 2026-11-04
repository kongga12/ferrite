//! 파일 변경 감시. 화면용(`FsWatcher`)은 열린 문서의 폴더와 펼친 트리 폴더만 비재귀로 감시한다
//! (저장소 전체를 재귀로 감시하면 Linux inotify 감시 개수 한도에 걸릴 수 있다).
//! 언어 서버에 알릴 프로젝트 전체 감시는 `ProjectWatcher`가 따로 한다.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;
use ferrite_lsp::watch::FileChange;
use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub struct FsWatcher {
    watcher: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    watched: HashSet<PathBuf>,
}

impl FsWatcher {
    /// 변경이 생기면 화면을 다시 그리도록 `ctx`를 깨운다.
    pub fn new(ctx: egui::Context) -> notify::Result<Self> {
        let (tx, events) = channel();
        let watcher = notify::recommended_watcher(move |event| {
            // 받는 쪽이 이미 없으면(앱 종료 중) 버린다.
            let _ = tx.send(event);
            ctx.request_repaint();
        })?;
        Ok(Self {
            watcher,
            events,
            watched: HashSet::new(),
        })
    }

    /// 감시 목록을 `dirs`와 같게 맞춘다. 감시에 실패한 폴더(삭제됨 등)도 목록에 남겨 매번 다시 시도하지 않는다.
    // ponytail: 매 프레임 호출. 폴더가 수천 개로 늘면 변경이 있을 때만 맞추도록.
    pub fn sync(&mut self, dirs: HashSet<PathBuf>) {
        for gone in self.watched.difference(&dirs).cloned().collect::<Vec<_>>() {
            let _ = self.watcher.unwatch(&gone);
            self.watched.remove(&gone);
        }
        for dir in dirs {
            if !self.watched.contains(&dir) {
                let _ = self.watcher.watch(&dir, RecursiveMode::NonRecursive);
                self.watched.insert(dir);
            }
        }
    }

    /// 마지막 호출 이후 내용·이름이 바뀐 경로들. 읽기 접근 이벤트는 뺀다.
    pub fn changed_paths(&self) -> HashSet<PathBuf> {
        self.events
            .try_iter()
            .filter_map(Result::ok)
            .filter(|e| !matches!(e.kind, EventKind::Access(_)))
            .flat_map(|e| e.paths)
            .collect()
    }
}

/// 프로젝트 전체에서 감시하지 않는 폴더(빌드 결과물·의존성·도구). 바뀜이 많고 언어 서버가 볼 일이 없다.
const SKIP_DIRS: [&str; 7] = [
    ".git",
    ".gradle",
    "bin",
    "build",
    "node_modules",
    "out",
    "target",
];

/// 프로젝트 밖에서 이보다 얕은 폴더(홈 아래면 홈부터, 아니면 드라이브부터 센 깊이)는 감시하지 않는다.
/// TS 서버는 프로젝트 밖 파일들의 공통 부모로 `C:\Users\<이름>\AppData` 같은 넓은 폴더를 등록하기도 한다.
/// 타입 캐시는 충분히 깊다(`AppData\Local\Microsoft\TypeScript\7.0`, `~/.cache/typescript/7.0`).
const MIN_OUTSIDE_DEPTH: usize = 3;

/// `path`가 `dir` 아래(같은 것 포함)인지. Windows는 대소문자를 가리지 않는다(TS 서버는 소문자 경로도 보낸다).
fn under(path: &Path, dir: &Path) -> bool {
    if cfg!(windows) {
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        lower(path).starts_with(lower(dir))
    } else {
        path.starts_with(dir)
    }
}

/// 홈 아래면 홈부터, 아니면 드라이브(루트)부터 센 폴더 깊이.
fn outside_depth(dir: &Path) -> usize {
    let depth = |p: &Path| {
        p.components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count()
    };
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    match home.map(PathBuf::from).filter(|home| under(dir, home)) {
        Some(home) => depth(dir).saturating_sub(depth(&home)),
        None => depth(dir),
    }
}

/// 프로젝트 전체 감시. 에이전트·git·다른 도구가 디스크에 쓴 것을 언어 서버에 알리는 데 쓴다.
/// Windows·macOS는 재귀 감시 하나로 보고, Linux는 inotify 감시 수 한도 때문에 뺄 폴더를 건너뛰며
/// 폴더마다 감시한다(새로 생긴 폴더도 더한다).
/// 서버가 감시를 등록한 프로젝트 밖 폴더(TS 서버의 타입 캐시 등)도 재귀로 본다(`watch_outside`).
pub struct ProjectWatcher {
    watcher: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    root: PathBuf,
    outside: BTreeSet<PathBuf>,
}

impl ProjectWatcher {
    pub fn new(root: &Path, ctx: egui::Context) -> notify::Result<Self> {
        let (tx, events) = channel();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
            ctx.request_repaint();
        })?;
        let mut this = Self {
            watcher,
            events,
            root: root.to_path_buf(),
            outside: BTreeSet::new(),
        };
        if cfg!(target_os = "linux") {
            this.watch_tree(root);
        } else {
            this.watcher.watch(root, RecursiveMode::Recursive)?;
        }
        Ok(this)
    }

    /// 이 폴더와 그 아래 폴더마다 감시를 건다(뺄 폴더 아래로는 들어가지 않는다).
    // ponytail: 화면 스레드에서 폴더를 다 돈다. 폴더가 수만 개인 저장소에서 느리면 별도 스레드로.
    fn watch_tree(&mut self, dir: &Path) {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            if self
                .watcher
                .watch(&dir, RecursiveMode::NonRecursive)
                .is_err()
            {
                continue;
            }
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir())
                    && !SKIP_DIRS.iter().any(|s| entry.file_name() == *s)
                {
                    stack.push(entry.path());
                }
            }
        }
    }

    /// 서버가 등록한 감시 폴더(`LanguageServer::watched_dirs`) 중 프로젝트 밖의 것도 재귀로 감시한다(바뀐
    /// 것만 다시 건다). TS 서버는 받은 타입을 프로젝트 밖 캐시에 두고, 그 바뀜을 알려야 받는 중에 읽은 타입
    /// 파일을 다시 읽는다. 프로젝트 안(이미 본다), 프로젝트를 품은 폴더와 너무 얕은 폴더(넓다)는 뺀다.
    // ponytail: 감시를 걸지 못한 폴더(아직 없음)는 등록이 바뀔 때까지 다시 걸지 않는다.
    pub fn watch_outside(&mut self, dirs: impl IntoIterator<Item = PathBuf>) {
        let dirs: BTreeSet<PathBuf> = dirs
            .into_iter()
            .filter(|d| {
                !under(d, &self.root)
                    && !under(&self.root, d)
                    && outside_depth(d) >= MIN_OUTSIDE_DEPTH
            })
            .collect();
        // 다른 것 아래의 폴더는 재귀 감시가 이미 덮는다.
        let dirs: BTreeSet<PathBuf> = dirs
            .iter()
            .filter(|d| !dirs.iter().any(|o| o != *d && under(d, o)))
            .cloned()
            .collect();
        if dirs == self.outside {
            return;
        }
        for gone in self.outside.difference(&dirs) {
            let _ = self.watcher.unwatch(gone);
        }
        for new in dirs.difference(&self.outside) {
            let _ = self.watcher.watch(new, RecursiveMode::Recursive);
        }
        self.outside = dirs;
    }

    fn ignored(&self, path: &Path) -> bool {
        match path.strip_prefix(&self.root) {
            Ok(relative) => relative
                .components()
                .any(|c| SKIP_DIRS.iter().any(|s| c.as_os_str() == *s)),
            Err(_) => !self.outside.iter().any(|d| under(path, d)),
        }
    }

    /// 마지막 호출 이후 바뀐 것(같은 경로는 한 번). 종류는 지금 디스크 상태로 정한다:
    /// 없으면 지움, 만들기·이름 바꾸기 알림이 있었으면 만듦, 아니면 바뀜.
    pub fn changes(&mut self) -> Vec<(PathBuf, FileChange)> {
        let mut paths = BTreeSet::new();
        let mut created = HashSet::new();
        for event in self.events.try_iter().filter_map(Result::ok) {
            if matches!(event.kind, EventKind::Access(_)) {
                continue;
            }
            let create = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
            );
            for path in event.paths {
                if self.ignored(&path) {
                    continue;
                }
                if create {
                    created.insert(path.clone());
                }
                paths.insert(path);
            }
        }
        let mut changes = Vec::new();
        for path in paths {
            let change = if !path.exists() {
                FileChange::Deleted
            } else if created.contains(&path) {
                FileChange::Created
            } else {
                FileChange::Changed
            };
            // 프로젝트 밖은 재귀 감시라 새 폴더를 notify가 더한다.
            if cfg!(target_os = "linux")
                && change == FileChange::Created
                && path.starts_with(&self.root)
                && path.is_dir()
            {
                self.watch_tree(&path);
            }
            changes.push((path, change));
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// `want`가 나올 때까지 모은다.
    fn wait_for(
        watcher: &mut ProjectWatcher,
        want: &(PathBuf, FileChange),
    ) -> Vec<(PathBuf, FileChange)> {
        let started = Instant::now();
        let mut seen = Vec::new();
        while !seen.contains(want) {
            seen.extend(watcher.changes());
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "{want:?}가 오지 않음: {seen:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        seen
    }

    #[test]
    fn 프로젝트의_바뀐_파일을_모으고_빌드_결과물은_뺀다() {
        let dir = std::env::temp_dir().join(format!("ferrite-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src/a")).unwrap();
        fs::create_dir_all(dir.join("target/classes")).unwrap();
        let root = crate::app::real_path(&dir);
        let mut watcher = ProjectWatcher::new(&root, egui::Context::default()).unwrap();

        let java = root.join("src").join("a").join("A.java");
        fs::write(&java, "class A {}").unwrap();
        fs::write(root.join("target/classes/A.class"), "x").unwrap();
        let seen = wait_for(&mut watcher, &(java.clone(), FileChange::Created));
        assert!(
            seen.iter()
                .all(|(p, _)| !p.starts_with(root.join("target"))),
            "{seen:?}"
        );

        // 새로 만든 폴더 안의 파일도 본다(Linux는 폴더마다 감시를 더한다).
        let new_dir = root.join("src").join("b");
        fs::create_dir_all(&new_dir).unwrap();
        wait_for(&mut watcher, &(new_dir.clone(), FileChange::Created));
        let mapper = new_dir.join("BoardMapper.xml");
        fs::write(&mapper, "<mapper/>").unwrap();
        wait_for(&mut watcher, &(mapper, FileChange::Created));

        fs::remove_file(&java).unwrap();
        wait_for(&mut watcher, &(java, FileChange::Deleted));
        drop(watcher);
        crate::app::remove_project(&dir);
    }

    #[test]
    fn 서버가_등록한_프로젝트_밖_폴더도_감시하되_넓은_폴더는_뺀다() {
        let dir = std::env::temp_dir().join(format!("ferrite-watch-out-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let base = crate::app::real_path(&dir);
        let root = base.join("project");
        // TS 서버의 타입 캐시처럼 node_modules 아래에 받은 타입이 있다.
        let cache = base.join("cache");
        let types = cache.join("node_modules").join("@types").join("jquery");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(&types).unwrap();
        let mut watcher = ProjectWatcher::new(&root, egui::Context::default()).unwrap();
        let shallow = base.ancestors().last().unwrap().join("ferrite-shallow");
        watcher.watch_outside([
            cache.join("node_modules"), // 캐시 감시가 덮는다
            cache.clone(),
            base.clone(),     // 프로젝트를 품었다
            root.join("src"), // 이미 본다
            shallow,          // 너무 얕다
        ]);
        assert_eq!(watcher.outside, BTreeSet::from([cache.clone()]));

        fs::write(base.join("other.txt"), "x").unwrap();
        let typing = types.join("index.d.ts");
        fs::write(&typing, "declare var $: any;").unwrap();
        let seen = wait_for(&mut watcher, &(typing.clone(), FileChange::Created));
        assert!(seen.iter().all(|(p, _)| p.starts_with(&cache)), "{seen:?}");
        fs::write(&typing, "declare var $: JQueryStatic;").unwrap();
        wait_for(&mut watcher, &(typing, FileChange::Changed));

        // 등록이 사라지면 감시를 푼다.
        watcher.watch_outside([]);
        assert!(watcher.outside.is_empty());
        drop(watcher);
        crate::app::remove_project(&dir);
    }
}
