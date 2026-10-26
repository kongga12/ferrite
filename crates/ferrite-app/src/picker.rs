//! 구성 고르기 창(실행 Ctrl+F5, 디버그 F5). 사용자 구성과 자동 구성을 나눠 보여 주고, 고른 구성을
//! 돌려준다. 다시 열면 마지막으로 고른 구성을 고른 채로 연다(Enter로 다시 실행).

use eframe::egui::{self, Key, Modifiers};

/// 고르기 창에 보일 구성.
pub trait Choice: Clone {
    fn name(&self) -> &str;
    /// 이름 옆에 흐리게 보일 설명(명령 줄, 어댑터 등). 이름과 같으면 보이지 않는다.
    fn detail(&self) -> String;
}

impl Choice for ferrite_core::run::RunConfig {
    fn name(&self) -> &str {
        &self.name
    }

    fn detail(&self) -> String {
        self.command.clone()
    }
}

impl Choice for ferrite_core::debug_config::DebugConfig {
    fn name(&self) -> &str {
        &self.name
    }

    fn detail(&self) -> String {
        format!("{} · {}", self.adapter, self.request)
    }
}

pub enum PickerAction<T> {
    Pick(T),
    /// 사용자 구성 파일을 연다(없으면 견본으로 만든다).
    Edit,
}

pub struct Picker<T> {
    title: &'static str,
    /// 구성 편집 단추에 보일 파일 이름.
    file: &'static str,
    open: bool,
    /// 사용자 구성 다음에 자동 구성.
    items: Vec<T>,
    /// 앞에서부터 이만큼이 사용자 구성.
    custom: usize,
    error: Option<String>,
    selected: usize,
    last: Option<String>,
}

impl<T: Choice> Picker<T> {
    pub fn new(title: &'static str, file: &'static str) -> Self {
        Self {
            title,
            file,
            open: false,
            items: Vec::new(),
            custom: 0,
            error: None,
            selected: 0,
            last: None,
        }
    }

    /// 창을 연다. 구성 파일은 사용자가 고칠 수 있으므로 부르는 쪽이 열 때마다 다시 읽어 넘긴다.
    /// 창에는 글자 칸이 없으므로, 고르는 키가 에디터·터미널로 가지 않게 키보드 포커스를 거둔다.
    pub fn show(
        &mut self,
        custom: Vec<T>,
        detected: Vec<T>,
        error: Option<String>,
        ctx: &egui::Context,
    ) {
        ctx.memory_mut(|m| m.stop_text_input());
        self.custom = custom.len();
        self.items = custom;
        self.items.extend(detected);
        self.error = error;
        self.selected = self
            .last
            .as_ref()
            .and_then(|last| self.items.iter().position(|c| c.name() == last))
            .unwrap_or(0);
        self.open = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<PickerAction<T>> {
        if !self.open {
            return None;
        }
        let (down, up, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        let last = self.items.len().saturating_sub(1);
        if down {
            self.selected = (self.selected + 1).min(last);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let mut chosen = enter.then_some(self.selected);
        let mut edit = false;
        let mut open = !escape;
        egui::Window::new(self.title)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if self.items.is_empty() {
                    ui.weak("구성을 찾지 못했습니다. [구성 편집…]에서 직접 적으세요.");
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (i, item) in self.items.iter().enumerate() {
                            if i == 0 && self.custom > 0 {
                                ui.weak("사용자 구성");
                            }
                            if i == self.custom {
                                ui.weak("자동 감지");
                            }
                            let row = ui
                                .horizontal(|ui| {
                                    let row = ui.selectable_label(i == self.selected, item.name());
                                    let detail = item.detail();
                                    if detail != item.name() {
                                        ui.weak(detail);
                                    }
                                    row
                                })
                                .inner;
                            if i == self.selected && (down || up) {
                                row.scroll_to_me(None);
                            }
                            if row.clicked() {
                                chosen = Some(i);
                            }
                        }
                    });
                ui.separator();
                edit = ui.button("구성 편집…").on_hover_text(self.file).clicked();
            });
        let action = if edit {
            Some(PickerAction::Edit)
        } else {
            chosen
                .and_then(|i| self.items.get(i))
                .map(|c| PickerAction::Pick(c.clone()))
        };
        if let Some(PickerAction::Pick(item)) = &action {
            self.last = Some(item.name().to_owned());
        }
        self.open = open && action.is_none();
        action
    }
}
