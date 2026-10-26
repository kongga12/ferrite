//! 파일에서 찾기(Ctrl+Shift+F). 아래 패널에서 프로젝트 전체를 찾는다. 찾기는 별도 스레드에서
//! 돌고 찾은 파일부터 보여 준다. 결과는 보이는 줄만 그린다(일치가 수만 개여도 느려지지 않게).

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use eframe::egui::{self, Key, text::LayoutJob};
use ferrite_core::project_search::{self, FileHits, MAX_TEXT_MATCHES, TextQuery, TextStats};

enum Message {
    Hits(FileHits),
    Done(Result<TextStats, String>),
}

struct Job {
    rx: Receiver<Message>,
    cancel: Arc<AtomicBool>,
}

/// 결과 목록의 한 줄.
#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Line(usize, usize),
}

/// 고른 결과: 파일, 0부터 센 줄, 줄 안 바이트 위치.
pub struct Jump {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
}

#[derive(Default)]
pub struct FindInFiles {
    focus: bool,
    query: TextQuery,
    /// 결과를 찾은 프로젝트 루트(경로를 짧게 보여 줄 때 쓴다).
    root: PathBuf,
    results: Vec<FileHits>,
    /// 접은 파일(`results`의 번호).
    collapsed: HashSet<usize>,
    stats: Option<TextStats>,
    job: Option<Job>,
    error: Option<String>,
}

impl FindInFiles {
    /// 검색어 칸에 포커스를 준다. `selected`(에디터에서 고른 한 줄 글자)가 있으면 찾을 내용으로 넣는다.
    pub fn show(&mut self, selected: Option<String>) {
        self.focus = true;
        if let Some(text) = selected.filter(|t| !t.contains('\n')) {
            self.query.pattern = text;
        }
    }

    fn start(&mut self, root: &Path, ctx: &egui::Context) {
        self.stop();
        self.results.clear();
        self.collapsed.clear();
        self.stats = None;
        self.error = None;
        self.root = root.to_path_buf();
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (query, root, ctx, flag) = (
            self.query.clone(),
            root.to_path_buf(),
            ctx.clone(),
            cancel.clone(),
        );
        thread::spawn(move || {
            let hits_tx = tx.clone();
            let hits_ctx = ctx.clone();
            let result = project_search::search_text(&root, &query, &flag, &move |hits| {
                let _ = hits_tx.send(Message::Hits(hits));
                hits_ctx.request_repaint();
            });
            let _ = tx.send(Message::Done(result));
            ctx.request_repaint();
        });
        self.job = Some(Job { rx, cancel });
    }

    pub fn stop(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn poll(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let mut done = None;
        for message in job.rx.try_iter() {
            match message {
                Message::Hits(hits) => self.results.push(hits),
                Message::Done(result) => done = Some(result),
            }
        }
        if let Some(result) = done {
            self.job = None;
            self.results.sort_by(|a, b| a.path.cmp(&b.path));
            match result {
                Ok(stats) => self.stats = Some(stats),
                Err(e) => self.error = Some(e),
            }
        }
    }

    /// 검색 창을 그린다. 결과를 누르면 그 위치.
    pub fn ui(&mut self, ui: &mut egui::Ui, root: Option<&Path>) -> Option<Jump> {
        self.poll();
        let ctx = ui.ctx().clone();
        let mut jump = None;
        ui.horizontal(|ui| {
            let label = ui.label("파일에서 찾기");
            let field = ui
                .add(egui::TextEdit::singleline(&mut self.query.pattern).desired_width(280.0))
                .labelled_by(label.id);
            if std::mem::take(&mut self.focus) {
                field.request_focus();
            }
            let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            for (on, text, hover) in [
                (&mut self.query.case_sensitive, "Aa", "대소문자 구분"),
                (&mut self.query.whole_word, "단어", "단어 단위"),
                (&mut self.query.regex, ".*", "정규식"),
            ] {
                if ui
                    .selectable_label(*on, text)
                    .on_hover_text(hover)
                    .clicked()
                {
                    *on = !*on;
                }
            }
            let label = ui.label("파일");
            ui.add(
                egui::TextEdit::singleline(&mut self.query.files)
                    .hint_text("*.java, *.xml, !*Test.java")
                    .desired_width(200.0),
            )
            .labelled_by(label.id);
            let running = self.job.is_some();
            if running {
                if ui.button("멈춤").clicked() {
                    self.stop();
                }
            } else if (ui
                .add_enabled(root.is_some(), egui::Button::new("찾기"))
                .clicked()
                || submitted)
                && let Some(root) = root
            {
                self.start(root, &ctx);
            }
        });
        self.status_ui(ui, root.is_none());
        self.results_ui(ui, &mut jump);
        jump
    }

    fn status_ui(&self, ui: &mut egui::Ui, no_project: bool) {
        let found: usize = self.results.iter().map(FileHits::count).sum();
        if no_project {
            ui.weak("먼저 프로젝트 폴더를 여세요");
        } else if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        } else if self.job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "찾는 중… 파일 {}개에서 {found}개",
                    self.results.len()
                ));
            });
        } else if let Some(stats) = self.stats {
            let text = format!(
                "파일 {}개에서 {}개 찾음 (파일 {}개를 읽음)",
                self.results.len(),
                stats.matches,
                stats.files
            );
            if stats.truncated {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "{text}. {MAX_TEXT_MATCHES}개에서 멈췄습니다. 검색어나 파일 패턴을 좁히세요"
                    ),
                );
            } else {
                ui.label(text);
            }
        }
    }

    fn results_ui(&mut self, ui: &mut egui::Ui, jump: &mut Option<Jump>) {
        let rows: Vec<Row> = self
            .results
            .iter()
            .enumerate()
            .flat_map(|(f, hits)| {
                let lines = if self.collapsed.contains(&f) {
                    0
                } else {
                    hits.lines.len()
                };
                std::iter::once(Row::File(f)).chain((0..lines).map(move |l| Row::Line(f, l)))
            })
            .collect();
        let height =
            ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().item_spacing.y + 2.0;
        let mut toggle = None;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            height,
            rows.len(),
            |ui, range| {
                for row in &rows[range] {
                    match *row {
                        Row::File(f) => {
                            let hits = &self.results[f];
                            let shown = hits.path.strip_prefix(&self.root).unwrap_or(&hits.path);
                            let arrow = if self.collapsed.contains(&f) {
                                "+"
                            } else {
                                "-"
                            };
                            let text = format!(
                                "{arrow} {} ({})",
                                shown.to_string_lossy().replace('\\', "/"),
                                hits.count()
                            );
                            if ui
                                .selectable_label(false, egui::RichText::new(text).strong())
                                .clicked()
                            {
                                toggle = Some(f);
                            }
                        }
                        Row::Line(f, l) => {
                            let line = &self.results[f].lines[l];
                            let job = highlighted(ui, line.line + 1, &line.text, &line.ranges);
                            if ui.selectable_label(false, job).clicked() {
                                *jump = Some(Jump {
                                    path: self.results[f].path.clone(),
                                    line: line.line,
                                    column: line.column,
                                });
                            }
                        }
                    }
                }
            },
        );
        if let Some(f) = toggle
            && !self.collapsed.remove(&f)
        {
            self.collapsed.insert(f);
        }
    }
}

/// `줄번호: 내용`. 일치한 부분은 강조색 배경으로 칠한다.
fn highlighted(ui: &egui::Ui, line: usize, text: &str, ranges: &[Range<usize>]) -> LayoutJob {
    let visuals = ui.visuals();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let plain = egui::TextFormat {
        font_id: font.clone(),
        color: visuals.text_color(),
        ..Default::default()
    };
    let hit = egui::TextFormat {
        background: visuals.selection.bg_fill,
        color: visuals.strong_text_color(),
        ..plain.clone()
    };
    let mut job = LayoutJob::default();
    job.append(
        &format!("{line:>5}: "),
        0.0,
        egui::TextFormat {
            color: visuals.weak_text_color(),
            ..plain.clone()
        },
    );
    let mut at = 0;
    for range in ranges {
        let (start, end) = (range.start.max(at), range.end.min(text.len()));
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        job.append(&text[at..start], 0.0, plain.clone());
        job.append(&text[start..end], 0.0, hit.clone());
        at = end;
    }
    job.append(&text[at..], 0.0, plain);
    job
}
