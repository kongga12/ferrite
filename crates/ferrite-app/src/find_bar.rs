//! 찾기·바꾸기 막대. 결과는 (문서, 버퍼 버전, 검색어)가 바뀔 때만 다시 계산한다.

use std::ops::Range;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Key};
use ferrite_core::buffer::Buffer;
use ferrite_core::search::{self, SearchQuery};

pub enum FindAction {
    Next,
    Prev,
    ReplaceOne,
    ReplaceAll,
    Close,
}

pub struct FindBar {
    pub query: SearchQuery,
    pub replacement: String,
    pub show_replace: bool,
    focus_query: bool,
    cache_key: Option<(PathBuf, u64, SearchQuery)>,
    matches: Vec<Range<usize>>,
    error: Option<String>,
}

impl FindBar {
    /// `selected`가 한 줄짜리면 검색어로 미리 채운다.
    pub fn new(show_replace: bool, selected: Option<String>) -> Self {
        let pattern = selected.filter(|s| !s.contains('\n')).unwrap_or_default();
        Self {
            query: SearchQuery {
                pattern,
                ..Default::default()
            },
            replacement: String::new(),
            show_replace,
            focus_query: true,
            cache_key: None,
            matches: Vec::new(),
            error: None,
        }
    }

    pub fn focus(&mut self, show_replace: bool) {
        self.focus_query = true;
        self.show_replace |= show_replace;
    }

    pub fn matches(&self) -> &[Range<usize>] {
        &self.matches
    }

    /// 문서나 검색어가 바뀌었으면 다시 찾는다.
    pub fn refresh(&mut self, path: &Path, buffer: &Buffer) {
        let fresh = self
            .cache_key
            .as_ref()
            .is_some_and(|(p, v, q)| p == path && *v == buffer.version() && *q == self.query);
        if fresh {
            return;
        }
        self.cache_key = Some((path.to_path_buf(), buffer.version(), self.query.clone()));
        self.matches.clear();
        self.error = None;
        if self.query.pattern.is_empty() {
            return;
        }
        match self.query.compile() {
            Ok(re) => self.matches = search::find_all(buffer.rope(), &re),
            Err(e) => self.error = Some(e),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, buffer: &Buffer) -> Option<FindAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            let label = ui.label("찾기");
            let field = ui
                .add(
                    egui::TextEdit::singleline(&mut self.query.pattern)
                        .desired_width(260.0)
                        .hint_text("검색어"),
                )
                .labelled_by(label.id);
            if std::mem::take(&mut self.focus_query) {
                field.request_focus();
            }
            // 한 줄 입력칸은 Enter에서 포커스를 잃으므로 다시 잡아 연속으로 다음을 찾게 한다.
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let shift = ui.input(|i| i.modifiers.shift);
                action = Some(if shift {
                    FindAction::Prev
                } else {
                    FindAction::Next
                });
                self.focus_query = true;
            }
            ui.toggle_value(&mut self.query.case_sensitive, "Aa")
                .on_hover_text("대소문자 구분");
            ui.toggle_value(&mut self.query.regex, ".*")
                .on_hover_text("정규식");
            if ui.button("↑").on_hover_text("이전 (Shift+F3)").clicked() {
                action = Some(FindAction::Prev);
            }
            if ui.button("↓").on_hover_text("다음 (F3)").clicked() {
                action = Some(FindAction::Next);
            }
            ui.label(self.summary(buffer));
            if ui.button("×").on_hover_text("닫기 (Esc)").clicked() {
                action = Some(FindAction::Close);
            }
        });
        if self.show_replace {
            ui.horizontal(|ui| {
                let label = ui.label("바꾸기");
                ui.add(
                    egui::TextEdit::singleline(&mut self.replacement)
                        .desired_width(260.0)
                        .hint_text(if self.query.regex {
                            "바꿀 내용 ($1 등 그룹 참조 가능)"
                        } else {
                            "바꿀 내용"
                        }),
                )
                .labelled_by(label.id);
                if ui.button("바꾸기").clicked() {
                    action = Some(FindAction::ReplaceOne);
                }
                if ui.button("모두 바꾸기").clicked() {
                    action = Some(FindAction::ReplaceAll);
                }
            });
        }
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            action = Some(FindAction::Close);
        }
        action
    }

    fn summary(&self, buffer: &Buffer) -> egui::RichText {
        if let Some(e) = &self.error {
            let first_line = e.lines().last().unwrap_or(e);
            return egui::RichText::new(format!("정규식 오류: {first_line}"))
                .color(egui::Color32::RED);
        }
        if self.query.pattern.is_empty() {
            return egui::RichText::new("");
        }
        let total = self.matches.len();
        if total == 0 {
            return egui::RichText::new("결과 없음");
        }
        let sel = buffer.selection().range();
        let more = if total == search::MAX_MATCHES {
            "+"
        } else {
            ""
        };
        match self.matches.iter().position(|m| *m == sel) {
            Some(i) => egui::RichText::new(format!("{}/{total}{more}", i + 1)),
            None => egui::RichText::new(format!("{total}{more}개")),
        }
    }
}
