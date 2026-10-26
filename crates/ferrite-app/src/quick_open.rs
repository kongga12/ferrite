//! 파일로 이동(Ctrl+P). 프로젝트 파일 이름을 퍼지 검색해 연다. 파일 목록은 창을 열 때마다 별도
//! 스레드에서 다시 만들고(파일 감시는 열린 폴더만 보므로), 그동안은 이전 목록으로 찾는다.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use eframe::egui::{self, Key, Modifiers, text::LayoutJob};
use ferrite_core::project_search::{self, FileMatch};

/// 보여 줄 결과 수.
const SHOWN: usize = 50;

#[derive(Default)]
pub struct QuickOpen {
    open: bool,
    root: Option<PathBuf>,
    query: String,
    files: Vec<String>,
    matches: Vec<FileMatch>,
    selected: usize,
    listing: Option<Receiver<Vec<String>>>,
    focus: bool,
}

impl QuickOpen {
    pub fn show(&mut self, root: &Path, ctx: &egui::Context) {
        if self.root.as_deref() != Some(root) {
            self.root = Some(root.to_path_buf());
            self.files.clear();
        }
        self.open = true;
        self.focus = true;
        self.query.clear();
        let (tx, rx) = channel();
        let (root, ctx) = (root.to_path_buf(), ctx.clone());
        thread::spawn(move || {
            let _ = tx.send(project_search::list_files(&root, &AtomicBool::new(false)));
            ctx.request_repaint();
        });
        self.listing = Some(rx);
        self.refresh();
    }

    fn refresh(&mut self) {
        self.matches = project_search::fuzzy_files(&self.files, &self.query, SHOWN);
        self.selected = 0;
    }

    /// 창을 그린다. 고른 파일의 절대 경로.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        if !self.open {
            return None;
        }
        if let Some(files) = self.listing.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.listing = None;
            self.files = files;
            self.refresh();
        }
        let (down, up, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        let last = self.matches.len().saturating_sub(1);
        if down {
            self.selected = (self.selected + 1).min(last);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let mut chosen = enter.then_some(self.selected);
        let mut open = !escape;
        egui::Window::new("파일로 이동")
            .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(640.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("파일 이름");
                    let field = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.query)
                                .hint_text("예: SampleController, sample/web/egov")
                                .desired_width(f32::INFINITY),
                        )
                        .labelled_by(label.id);
                    if std::mem::take(&mut self.focus) {
                        field.request_focus();
                    }
                    if field.changed() {
                        self.matches = project_search::fuzzy_files(&self.files, &self.query, SHOWN);
                        self.selected = 0;
                    }
                });
                if self.listing.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("파일 목록을 읽는 중…");
                    });
                } else {
                    ui.weak(format!("파일 {}개", self.files.len()));
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (i, m) in self.matches.iter().enumerate() {
                            let row = ui.selectable_label(
                                i == self.selected,
                                highlighted(ui, &self.files[m.index], &m.positions),
                            );
                            if i == self.selected && (down || up) {
                                row.scroll_to_me(None);
                            }
                            if row.clicked() {
                                chosen = Some(i);
                            }
                        }
                    });
            });
        let path = chosen
            .and_then(|i| self.matches.get(i))
            .zip(self.root.as_ref())
            .map(|(m, root)| root.join(&self.files[m.index]));
        self.open = open && path.is_none();
        path
    }
}

/// 파일 이름(굵게) 다음에 폴더(흐리게). 일치한 글자는 강조색으로 칠한다.
fn highlighted(ui: &egui::Ui, path: &str, positions: &[u32]) -> LayoutJob {
    let visuals = ui.visuals();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let name_start = path.rfind('/').map_or(0, |i| path[..=i].chars().count());
    let chars: Vec<char> = path.chars().collect();
    let mut job = LayoutJob::default();
    let push = |job: &mut LayoutJob, range: std::ops::Range<usize>, base: egui::Color32| {
        for (i, c) in chars[range.clone()].iter().enumerate() {
            let hit = positions.binary_search(&((range.start + i) as u32)).is_ok();
            let format = egui::TextFormat {
                font_id: font.clone(),
                color: if hit { visuals.hyperlink_color } else { base },
                underline: if hit {
                    egui::Stroke::new(1.0, visuals.hyperlink_color)
                } else {
                    egui::Stroke::NONE
                },
                ..Default::default()
            };
            job.append(&c.to_string(), 0.0, format);
        }
    };
    push(
        &mut job,
        name_start..chars.len(),
        visuals.strong_text_color(),
    );
    if name_start > 0 {
        job.append(
            "  ",
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                ..Default::default()
            },
        );
        push(&mut job, 0..name_start - 1, visuals.weak_text_color());
    }
    job
}
