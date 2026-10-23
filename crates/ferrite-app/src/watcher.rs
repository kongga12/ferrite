//! 파일 변경 감시. 열린 문서의 폴더와 펼친 트리 폴더만 비재귀로 감시한다
//! (저장소 전체를 재귀로 감시하면 Linux inotify 감시 개수 한도에 걸릴 수 있다).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;
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
