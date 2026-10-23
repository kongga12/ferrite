//! 메인 창: 메뉴바, 프로젝트 트리, 탭 에디터, 상태바, 미저장 변경 확인.

use std::path::PathBuf;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, ViewportCommand};

use crate::file_tree::FileTree;
use crate::text_file::{self, Encoding, LineEnding, TextContent};

// Modifiers::COMMAND는 macOS에서 Cmd, 그 외 OS에서 Ctrl로 동작한다.
const OPEN_FOLDER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);

struct Document {
    path: PathBuf,
    content: TextContent,
    dirty: bool,
}

impl Document {
    fn title(&self) -> String {
        let name = self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        if self.dirty {
            format!("● {name}")
        } else {
            name
        }
    }
}

/// 저장하지 않은 변경 때문에 사용자 확인을 기다리는 닫기 요청.
enum PendingClose {
    Tab(usize),
    App,
}

enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

pub struct FerriteApp {
    tree: Option<FileTree>,
    docs: Vec<Document>,
    active: Option<usize>,
    pending_close: Option<PendingClose>,
    /// 사용자가 종료를 확정했으면 true. 이후의 창 닫기 요청은 막지 않는다.
    allow_close: bool,
    status: String,
}

impl FerriteApp {
    /// `project`는 명령줄로 받은 시작 폴더(`ferrite <폴더>`).
    pub fn new(cc: &eframe::CreationContext<'_>, project: Option<PathBuf>) -> Self {
        crate::fonts::install_korean_font(&cc.egui_ctx);
        let (tree, status) = match project {
            Some(dir) if dir.is_dir() => (
                Some(FileTree::new(dir.clone())),
                format!("프로젝트: {}", dir.display()),
            ),
            Some(path) => (None, format!("폴더가 아닙니다: {}", path.display())),
            None => (
                None,
                format!(
                    "폴더를 열어 시작하세요 ({})",
                    cc.egui_ctx.format_shortcut(&OPEN_FOLDER)
                ),
            ),
        };
        Self {
            tree,
            docs: Vec::new(),
            active: None,
            pending_close: None,
            allow_close: false,
            status,
        }
    }

    fn open_folder_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.status = format!("프로젝트: {}", dir.display());
            self.tree = Some(FileTree::new(dir));
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            self.active = Some(i);
            return;
        }
        match text_file::load(&path) {
            Ok(content) => {
                self.docs.push(Document {
                    path,
                    content,
                    dirty: false,
                });
                self.active = Some(self.docs.len() - 1);
            }
            Err(e) => self.status = format!("열기 실패: {}: {e}", path.display()),
        }
    }

    /// 저장 성공 여부를 돌려준다. 실패 사유는 상태바에 표시한다.
    fn save(&mut self, index: usize) -> bool {
        let doc = &mut self.docs[index];
        match text_file::save(&doc.path, &doc.content) {
            Ok(()) => {
                doc.dirty = false;
                self.status = format!("저장됨: {}", doc.path.display());
                true
            }
            Err(e) => {
                self.status = format!("저장 실패: {}: {e}", doc.path.display());
                false
            }
        }
    }

    fn request_close_tab(&mut self, index: usize) {
        if self.docs[index].dirty {
            self.pending_close = Some(PendingClose::Tab(index));
        } else {
            self.close_tab(index);
        }
    }

    fn close_tab(&mut self, index: usize) {
        self.docs.remove(index);
        self.active = match self.active {
            _ if self.docs.is_empty() => None,
            Some(a) if a > index => Some(a - 1),
            Some(a) if a == index => Some(index.min(self.docs.len() - 1)),
            other => other,
        };
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_shortcut(&OPEN_FOLDER)) {
            self.open_folder_dialog();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE))
            && let Some(i) = self.active
        {
            self.save(i);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&CLOSE_TAB))
            && let Some(i) = self.active
        {
            self.request_close_tab(i);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("파일", |ui| {
                    let open = egui::Button::new("폴더 열기…")
                        .shortcut_text(ctx.format_shortcut(&OPEN_FOLDER));
                    if ui.add(open).clicked() {
                        ui.close();
                        self.open_folder_dialog();
                    }
                    let save = egui::Button::new("저장").shortcut_text(ctx.format_shortcut(&SAVE));
                    if ui.add_enabled(self.active.is_some(), save).clicked()
                        && let Some(i) = self.active
                    {
                        ui.close();
                        self.save(i);
                    }
                    let close =
                        egui::Button::new("탭 닫기").shortcut_text(ctx.format_shortcut(&CLOSE_TAB));
                    if ui.add_enabled(self.active.is_some(), close).clicked()
                        && let Some(i) = self.active
                    {
                        ui.close();
                        self.request_close_tab(i);
                    }
                    if ui
                        .add_enabled(self.tree.is_some(), egui::Button::new("트리 새로고침"))
                        .clicked()
                        && let Some(tree) = &mut self.tree
                    {
                        ui.close();
                        tree.refresh();
                    }
                    ui.separator();
                    if ui.button("종료").clicked() {
                        ui.close();
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                let Some(doc) = self.active.map(|i| &mut self.docs[i]) else {
                    return;
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let before = (doc.content.encoding, doc.content.line_ending);
                    egui::ComboBox::new("line_ending", "줄바꿈")
                        .selected_text(doc.content.line_ending.label())
                        .show_ui(ui, |ui| {
                            for le in LineEnding::ALL {
                                ui.selectable_value(&mut doc.content.line_ending, le, le.label());
                            }
                        });
                    egui::ComboBox::new("encoding", "인코딩")
                        .selected_text(doc.content.encoding.label())
                        .show_ui(ui, |ui| {
                            for enc in Encoding::ALL {
                                ui.selectable_value(&mut doc.content.encoding, enc, enc.label());
                            }
                        })
                        .response
                        .on_hover_text("저장할 때 사용할 인코딩");
                    if (doc.content.encoding, doc.content.line_ending) != before {
                        doc.dirty = true;
                    }
                });
            });
        });
    }

    fn project_panel(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        let mut pick_folder = false;
        egui::Panel::left("project")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink(false)
                    .show(ui, |ui| match &mut self.tree {
                        Some(tree) => {
                            open = tree.ui(ui);
                        }
                        None => {
                            pick_folder = ui.button("폴더 열기…").clicked();
                        }
                    });
            });
        if pick_folder {
            self.open_folder_dialog();
        }
        if let Some(path) = open {
            self.open_file(path);
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        // 스크린리더가 에디터를 "현재 탭 이름"으로 읽도록 에디터를 활성 탭에 연결한다.
        let mut active_tab = None;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, doc) in self.docs.iter().enumerate() {
                            let tab = ui
                                .selectable_label(self.active == Some(i), doc.title())
                                .on_hover_text(doc.path.display().to_string());
                            if self.active == Some(i) {
                                active_tab = Some(tab.id);
                            }
                            if tab.clicked() {
                                self.active = Some(i);
                            }
                            if tab.middle_clicked() || ui.small_button("×").clicked() {
                                close = Some(i);
                            }
                        }
                    });
                });
            ui.separator();

            let Some(doc) = self.active.map(|i| &mut self.docs[i]) else {
                ui.centered_and_justified(|ui| ui.label("열린 파일이 없습니다"));
                return;
            };
            // ponytail: TextEdit는 매 프레임 전체 텍스트를 레이아웃한다. Phase 1 가상화 에디터로 교체.
            egui::ScrollArea::both()
                .id_salt(&doc.path)
                .auto_shrink(false)
                .show(ui, |ui| {
                    let edit = egui::TextEdit::multiline(&mut doc.content.text)
                        .id_salt(&doc.path)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .min_size(ui.available_size());
                    let mut response = ui.add(edit);
                    if let Some(tab) = active_tab {
                        response = response.labelled_by(tab);
                    }
                    if response.changed() {
                        doc.dirty = true;
                    }
                });
        });
        if let Some(i) = close {
            self.request_close_tab(i);
        }
    }

    /// 창 닫기 요청을 가로채, 저장하지 않은 문서가 있으면 확인 대화상자로 넘긴다.
    fn intercept_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.docs.iter().any(|d| d.dirty)
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.pending_close = Some(PendingClose::App);
        }
    }

    fn pending_close_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_close else {
            return;
        };
        let message = match pending {
            PendingClose::Tab(i) => {
                format!("'{}'의 변경 사항을 저장할까요?", self.docs[*i].title())
            }
            PendingClose::App => format!(
                "저장하지 않은 파일이 {}개 있습니다. 저장할까요?",
                self.docs.iter().filter(|d| d.dirty).count()
            ),
        };
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::unique("pending_close")).show(ctx, |ui| {
            ui.heading("저장하지 않은 변경 사항");
            ui.label(message);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("저장").clicked() {
                    choice = Some(CloseChoice::Save);
                }
                if ui.button("저장 안 함").clicked() {
                    choice = Some(CloseChoice::Discard);
                }
                if ui.button("취소").clicked() {
                    choice = Some(CloseChoice::Cancel);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(CloseChoice::Cancel);
        }
        let Some(choice) = choice else {
            return;
        };

        match (self.pending_close.take(), choice) {
            (_, CloseChoice::Cancel) | (None, _) => {}
            (Some(PendingClose::Tab(i)), CloseChoice::Save) => {
                if self.save(i) {
                    self.close_tab(i);
                }
            }
            (Some(PendingClose::Tab(i)), CloseChoice::Discard) => self.close_tab(i),
            (Some(PendingClose::App), CloseChoice::Save) => {
                // 하나라도 저장에 실패하면 종료하지 않는다(실패 사유는 상태바에 남는다).
                let dirty: Vec<usize> = (0..self.docs.len())
                    .filter(|&i| self.docs[i].dirty)
                    .collect();
                let mut all_saved = true;
                for i in dirty {
                    all_saved &= self.save(i);
                }
                if all_saved {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
            (Some(PendingClose::App), CloseChoice::Discard) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }
}

impl eframe::App for FerriteApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.intercept_close(ctx);
        // 확인 대화상자가 떠 있는 동안에는 단축키로 탭 목록이 바뀌지 않게 한다.
        if self.pending_close.is_none() {
            self.handle_shortcuts(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.menu_bar(ui);
        self.status_bar(ui);
        self.project_panel(ui);
        self.editor(ui);
        self.pending_close_dialog(ui.ctx());
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn request_window_close(harness: &mut Harness<'_, FerriteApp>) {
        harness
            .input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
    }

    #[test]
    fn 미저장_문서가_있으면_창_닫기_전에_저장을_확인한다() {
        let dir = std::env::temp_dir().join(format!("ferrite-app-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "hello").unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));

        // 트리에서 파일 열기
        harness.get_by_label("a.txt").click();
        harness.run();
        assert_eq!(harness.state().docs.len(), 1);

        // 편집하면 탭에 미저장 표시가 붙는다
        let editor = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
        editor.focus();
        editor.type_text("!");
        harness.run();
        assert!(harness.state().docs[0].dirty);
        harness.get_by_label("● a.txt");

        // 창 닫기 요청은 막히고 확인 대화상자가 뜬다
        request_window_close(&mut harness);
        harness.run();
        assert!(!harness.state().allow_close);
        harness.get_by_label("저장하지 않은 변경 사항");

        // 저장을 고르면 파일에 기록되고 종료가 허용된다
        harness.get_by_label("저장").click();
        harness.run();
        assert!(harness.state().allow_close);
        assert!(!harness.state().docs[0].dirty);
        let saved = fs::read_to_string(&file).unwrap();
        assert!(
            saved.contains('!') && saved.contains("hello"),
            "저장 내용: {saved}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
