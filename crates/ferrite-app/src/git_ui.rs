//! Git 화면: 상태바의 브랜치, 소스 제어 창(Ctrl+Shift+G), 열린 문서의 거터 변경 표시에 쓸 기준 내용.
//! git status와 인덱스 내용은 5초마다(저장·git 작업 뒤에는 바로) 별도 스레드에서 다시 읽는다.
//! 푸시는 네트워크를 쓰므로 별도 스레드에서 돌린다. 나머지 git 작업은 짧아 누를 때 바로 돈다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, text::LayoutJob};
use ferrite_core::git::{Change, Entry, Repo, Status};
use ferrite_core::text_file;

/// 상태를 다시 읽는 간격(밖에서 git을 쓴 것도 따라잡게).
const REFRESH_EVERY: Duration = Duration::from_secs(5);
/// diff에서 보여 줄 최대 줄 수.
const MAX_DIFF_LINES: usize = 3000;

struct Snapshot {
    status: Result<Status, String>,
    bases: HashMap<PathBuf, Option<String>>,
}

#[derive(Default)]
pub struct GitPanel {
    repo: Option<Repo>,
    status: Option<Status>,
    error: Option<String>,
    /// 열린 문서의 인덱스 내용(거터 비교 기준). 인덱스에 없는 파일은 None.
    bases: HashMap<PathBuf, Option<String>>,
    /// 기준 내용을 다시 읽을 때마다 늘어난다(문서가 거터 표시를 다시 계산할지 판단한다).
    generation: u64,
    refreshing: Option<Receiver<Snapshot>>,
    last_refresh: Option<Instant>,
    refresh_requested: bool,
    open: bool,
    branches: Vec<String>,
    /// diff를 보는 항목(경로, 스테이지된 쪽인지).
    selected: Option<(String, bool)>,
    diff: String,
    message: String,
    pushing: Option<Receiver<Result<String, String>>>,
    /// (오류인지, 내용)
    notice: Option<(bool, String)>,
}

impl GitPanel {
    /// 프로젝트가 바뀌면 그 폴더를 담은 저장소를 찾는다.
    pub fn set_root(&mut self, root: Option<&Path>) {
        self.repo = root.and_then(Repo::discover);
        self.status = None;
        self.error = None;
        self.bases.clear();
        self.generation += 1;
        self.selected = None;
        self.diff.clear();
        self.refresh_requested = true;
    }

    pub fn request_refresh(&mut self) {
        self.refresh_requested = true;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 열린 문서의 거터 비교 기준(인덱스 내용).
    pub fn base(&self, path: &Path) -> Option<&str> {
        self.bases.get(path)?.as_deref()
    }

    /// 매 프레임 부른다. 끝난 갱신을 거두고, 때가 되면 새로 시작한다.
    pub fn tick(&mut self, ctx: &egui::Context, open_docs: Vec<PathBuf>) {
        if let Some(snapshot) = self.refreshing.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.refreshing = None;
            match snapshot.status {
                Ok(status) => {
                    self.status = Some(status);
                    self.error = None;
                }
                Err(e) => self.error = Some(e),
            }
            self.bases = snapshot.bases;
            self.generation += 1;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let due = self
            .last_refresh
            .is_none_or(|t| t.elapsed() >= REFRESH_EVERY);
        if self.refreshing.is_none() && (self.refresh_requested || due) {
            self.refresh_requested = false;
            self.last_refresh = Some(Instant::now());
            let docs: Vec<PathBuf> = open_docs
                .into_iter()
                .filter(|p| repo.relative(p).is_some())
                .collect();
            let (tx, rx) = channel();
            let ctx = ctx.clone();
            thread::spawn(move || {
                let _ = tx.send(snapshot(&repo, &docs));
                ctx.request_repaint();
            });
            self.refreshing = Some(rx);
        }
        // 아무 입력이 없어도 다음 갱신 때 깨어나게 한다.
        ctx.request_repaint_after(REFRESH_EVERY);
    }

    /// 상태바 항목. 누르면 소스 제어 창을 연다.
    pub fn status_bar(&mut self, ui: &mut egui::Ui) {
        let (Some(repo), Some(status)) = (&self.repo, &self.status) else {
            return;
        };
        let branch = status.branch.as_deref().unwrap_or("(분리된 HEAD)");
        let changes = status.entries.len();
        let text = if changes > 0 {
            format!("Git: {branch} · 변경 {changes}")
        } else {
            format!("Git: {branch}")
        };
        if ui
            .selectable_label(self.open, text)
            .on_hover_text(format!("{} (소스 제어 열기)", repo.root().display()))
            .clicked()
        {
            self.toggle();
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.reload_branches();
            self.refresh_requested = true;
        }
    }

    fn reload_branches(&mut self) {
        if let Some(repo) = &self.repo {
            self.branches = repo.branches().unwrap_or_default();
        }
    }

    /// git 작업의 결과를 알리고 상태를 바로 다시 읽는다.
    fn done(&mut self, result: Result<String, String>) {
        self.notice = Some(match result {
            Ok(text) => (false, text),
            Err(e) => (true, e),
        });
        self.refresh_requested = true;
        self.reload_diff();
    }

    fn reload_diff(&mut self) {
        let (Some(repo), Some((path, staged))) = (&self.repo, &self.selected) else {
            self.diff.clear();
            return;
        };
        let entry = self
            .status
            .as_ref()
            .and_then(|s| s.entries.iter().find(|e| &e.path == path))
            .cloned()
            .unwrap_or_else(|| Entry {
                path: path.clone(),
                staged: None,
                unstaged: Some(Change::Modified),
            });
        self.diff = repo
            .diff(&entry, *staged)
            .unwrap_or_else(|e| format!("diff를 읽지 못했습니다: {e}"));
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.pushing.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pushing = None;
            self.done(result.map(|out| {
                if out.is_empty() {
                    "올렸습니다".to_owned()
                } else {
                    format!("올렸습니다: {out}")
                }
            }));
        }
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("소스 제어")
            .open(&mut open)
            .default_width(680.0)
            .default_height(620.0)
            .show(ctx, |ui| {
                if self.repo.is_none() {
                    ui.label("열린 프로젝트가 git 저장소가 아닙니다(또는 git을 찾을 수 없습니다).");
                    return;
                }
                self.header_ui(ui, ctx);
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.separator();
                self.changes_ui(ui);
                ui.separator();
                diff_ui(ui, &self.diff);
                ui.separator();
                self.commit_ui(ui);
                if let Some((error, text)) = &self.notice {
                    let color = if *error {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.colored_label(color, text);
                }
            });
        self.open = open;
    }

    fn header_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(status) = self.status.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("상태를 읽는 중…");
            });
            return;
        };
        ui.horizontal(|ui| {
            let label = ui.label("브랜치");
            let current = status
                .branch
                .clone()
                .unwrap_or_else(|| "(분리된 HEAD)".into());
            let mut chosen = current.clone();
            egui::ComboBox::from_id_salt("git_branch")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for branch in &self.branches {
                        ui.selectable_value(&mut chosen, branch.clone(), branch);
                    }
                })
                .response
                .labelled_by(label.id);
            if chosen != current
                && let Some(repo) = &self.repo
            {
                let result = repo
                    .switch(&chosen)
                    .map(|()| format!("{chosen} 브랜치로 바꿨습니다"));
                self.done(result);
            }
            if let Some(upstream) = &status.upstream {
                ui.weak(format!(
                    "{upstream}: 올릴 커밋 {}, 받을 커밋 {}",
                    status.ahead, status.behind
                ));
            }
            if ui.button("새로 고침").clicked() {
                self.reload_branches();
                self.refresh_requested = true;
            }
            if self.pushing.is_some() {
                ui.spinner();
                ui.label("올리는 중…");
            } else if ui
                .button("푸시")
                .on_hover_text("추적 브랜치가 없으면 origin에 같은 이름으로 만든다")
                .clicked()
                && let Some(repo) = self.repo.clone()
            {
                let (tx, rx) = channel();
                let ctx = ctx.clone();
                thread::spawn(move || {
                    let _ = tx.send(repo.push(&status));
                    ctx.request_repaint();
                });
                self.pushing = Some(rx);
                self.notice = None;
            }
        });
    }

    fn changes_ui(&mut self, ui: &mut egui::Ui) {
        let Some(status) = self.status.clone() else {
            return;
        };
        let staged: Vec<&Entry> = status
            .entries
            .iter()
            .filter(|e| e.staged.is_some())
            .collect();
        let unstaged: Vec<&Entry> = status
            .entries
            .iter()
            .filter(|e| e.unstaged.is_some())
            .collect();
        let mut action: Option<(bool, Vec<String>)> = None;
        let mut select = None;
        egui::ScrollArea::vertical()
            .id_salt("git_changes")
            .max_height(220.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (title, entries, staged_side) in [
                    ("스테이지됨", &staged, true),
                    ("변경 사항", &unstaged, false),
                ] {
                    ui.horizontal(|ui| {
                        ui.strong(format!("{title} {}", entries.len()));
                        let all = if staged_side {
                            "모두 내리기"
                        } else {
                            "모두 스테이지"
                        };
                        if !entries.is_empty() && ui.small_button(all).clicked() {
                            action = Some((
                                !staged_side,
                                entries.iter().map(|e| e.path.clone()).collect(),
                            ));
                        }
                    });
                    for entry in entries.iter() {
                        let change = if staged_side {
                            entry.staged
                        } else {
                            entry.unstaged
                        };
                        let letter = change.map_or(' ', Change::letter);
                        ui.horizontal(|ui| {
                            let (sign, hover) = if staged_side {
                                ("-", "스테이지에서 내리기")
                            } else {
                                ("+", "스테이지에 올리기")
                            };
                            if ui
                                .small_button(sign)
                                .on_hover_text(format!("{hover}: {}", entry.path))
                                .clicked()
                            {
                                action = Some((!staged_side, vec![entry.path.clone()]));
                            }
                            let selected =
                                self.selected.as_ref() == Some(&(entry.path.clone(), staged_side));
                            if ui
                                .selectable_label(
                                    selected,
                                    egui::RichText::new(format!("{letter} {}", entry.path))
                                        .monospace(),
                                )
                                .clicked()
                            {
                                select = Some((entry.path.clone(), staged_side));
                            }
                        });
                    }
                }
            });
        if let Some(selected) = select {
            self.selected = Some(selected);
            self.reload_diff();
        }
        if let (Some((stage, paths)), Some(repo)) = (action, &self.repo) {
            let count = paths.len();
            let result = if stage {
                repo.stage(&paths)
                    .map(|()| format!("파일 {count}개를 스테이지에 올렸습니다"))
            } else {
                repo.unstage(&paths)
                    .map(|()| format!("파일 {count}개를 스테이지에서 내렸습니다"))
            };
            self.done(result);
        }
    }

    fn commit_ui(&mut self, ui: &mut egui::Ui) {
        let staged = self
            .status
            .as_ref()
            .is_some_and(|s| s.entries.iter().any(|e| e.staged.is_some()));
        let label = ui.label("커밋 메시지");
        ui.add(
            egui::TextEdit::multiline(&mut self.message)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(label.id);
        let ready = staged && !self.message.trim().is_empty();
        if ui
            .add_enabled(ready, egui::Button::new("커밋"))
            .on_disabled_hover_text("스테이지된 변경과 메시지가 있어야 합니다")
            .clicked()
            && let Some(repo) = &self.repo
        {
            let result = repo.commit(&self.message);
            if result.is_ok() {
                self.message.clear();
                self.selected = None;
            }
            self.done(result.map(|summary| format!("커밋했습니다: {summary}")));
        }
    }
}

/// 상태와 열린 문서의 인덱스 내용을 읽는다(별도 스레드).
fn snapshot(repo: &Repo, docs: &[PathBuf]) -> Snapshot {
    let bases = docs
        .iter()
        .map(|path| {
            let base = repo
                .indexed(path)
                .and_then(|bytes| text_file::decode(&bytes).ok())
                .map(|content| content.text);
            (path.clone(), base)
        })
        .collect();
    Snapshot {
        status: repo.status(),
        bases,
    }
}

/// 통합 diff를 줄 머리글자(`+`, `-`, `@@`)에 따라 칠한다.
fn diff_ui(ui: &mut egui::Ui, diff: &str) {
    if diff.is_empty() {
        ui.weak("파일을 고르면 diff를 보여 줍니다");
        return;
    }
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let plain = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    for (shown, line) in diff.lines().enumerate() {
        if shown == MAX_DIFF_LINES {
            job.append(
                &format!("… {}줄 더 있음\n", diff.lines().count() - MAX_DIFF_LINES),
                0.0,
                egui::TextFormat::simple(font.clone(), ui.visuals().weak_text_color()),
            );
            break;
        }
        let color = if line.starts_with("+++") || line.starts_with("---") {
            plain
        } else if line.starts_with('+') {
            Color32::from_rgb(70, 170, 90)
        } else if line.starts_with('-') {
            Color32::from_rgb(210, 80, 80)
        } else if line.starts_with("@@") {
            Color32::from_rgb(70, 130, 210)
        } else {
            plain
        };
        job.append(
            &format!("{line}\n"),
            0.0,
            egui::TextFormat::simple(font.clone(), color),
        );
    }
    egui::ScrollArea::both()
        .id_salt("git_diff")
        .max_height(240.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.label(job);
        });
}
