//! 프로젝트 트리. 디렉터리를 펼칠 때만 읽는 지연 로딩 방식이라 대형 저장소도 즉시 열린다.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use eframe::egui;

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

pub struct FileTree {
    root: PathBuf,
    /// 펼친 적 있는 디렉터리의 목록 캐시. 읽기 실패는 메시지로 보관해 트리에 표시한다.
    children: HashMap<PathBuf, Result<Vec<Entry>, String>>,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            children: HashMap::new(),
        }
    }

    pub fn refresh(&mut self) {
        self.children.clear();
    }

    /// 한 폴더의 목록 캐시를 버린다. 다음에 그릴 때 다시 읽는다.
    pub fn invalidate(&mut self, dir: &Path) {
        self.children.remove(dir);
    }

    /// 목록을 읽어 둔(펼친 적 있는) 폴더들. 파일 감시 대상이다.
    pub fn loaded_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.children.keys()
    }

    /// 트리를 그리고, 클릭된 파일 경로를 돌려준다.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut clicked = None;
        let root = self.root.clone();
        let name = root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        egui::CollapsingHeader::new(name)
            .id_salt(&root)
            .default_open(true)
            .show(ui, |ui| self.dir_ui(ui, &root, &mut clicked));
        clicked
    }

    fn dir_ui(&mut self, ui: &mut egui::Ui, dir: &Path, clicked: &mut Option<PathBuf>) {
        let listing = self
            .children
            .entry(dir.to_path_buf())
            .or_insert_with(|| read_dir_sorted(dir).map_err(|e| e.to_string()))
            .clone();
        let entries = match listing {
            Ok(entries) => entries,
            Err(e) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
                return;
            }
        };
        for entry in entries {
            if entry.is_dir {
                egui::CollapsingHeader::new(&entry.name)
                    .id_salt(&entry.path)
                    .show(ui, |ui| self.dir_ui(ui, &entry.path, clicked));
            } else if ui.selectable_label(false, &entry.name).clicked() {
                *clicked = Some(entry.path);
            }
        }
    }
}

/// 디렉터리 먼저, 그다음 이름순(대소문자 무시)으로 정렬한다.
fn read_dir_sorted(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        // .git과 저장 중 잠깐 생기는 임시 파일은 숨긴다.
        if name == ".git" || name.ends_with(".ferrite-tmp") {
            continue;
        }
        let path = item.path();
        // file_type()은 심볼릭 링크 자체를 가리키므로, 링크 대상 기준으로 판정한다.
        let is_dir = path.is_dir();
        entries.push(Entry { path, name, is_dir });
    }
    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
    Ok(entries)
}
