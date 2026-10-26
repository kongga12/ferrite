//! 실행 구성 고르기(Ctrl+F5). 사용자 구성(`.ferrite/run.toml`)과 자동 감지 구성을 보여 주고, 고른
//! 구성을 터미널 탭에서 돌린다. 다시 열면 마지막으로 돌린 구성을 고른 채로 연다(Enter로 다시 실행).

use std::path::Path;

use eframe::egui::{self, Key, Modifiers};
use ferrite_core::run::{self, RunConfig};

pub enum RunAction {
    Run(RunConfig),
    /// 사용자 구성 파일을 연다(없으면 견본으로 만든다).
    Edit,
}

#[derive(Default)]
pub struct RunPicker {
    open: bool,
    /// 사용자 구성 다음에 자동 감지 구성.
    configs: Vec<RunConfig>,
    /// 앞에서부터 이만큼이 사용자 구성.
    custom: usize,
    error: Option<String>,
    selected: usize,
    last: Option<String>,
}

impl RunPicker {
    /// 구성을 다시 읽고 창을 연다. 구성 파일은 사용자가 고칠 수 있으므로 열 때마다 읽는다.
    /// 창에는 글자 칸이 없으므로, 고르는 키가 에디터·터미널로 가지 않게 키보드 포커스를 거둔다.
    pub fn show(&mut self, root: &Path, ctx: &egui::Context) {
        ctx.memory_mut(|m| m.stop_text_input());
        let (custom, error) = match run::load_custom(root) {
            Ok(custom) => (custom, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        self.custom = custom.len();
        self.configs = custom;
        self.configs.extend(run::detect(root));
        self.error = error;
        self.selected = self
            .last
            .as_ref()
            .and_then(|last| self.configs.iter().position(|c| &c.name == last))
            .unwrap_or(0);
        self.open = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<RunAction> {
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
        let last = self.configs.len().saturating_sub(1);
        if down {
            self.selected = (self.selected + 1).min(last);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let mut chosen = enter.then_some(self.selected);
        let mut edit = false;
        let mut open = !escape;
        egui::Window::new("실행")
            .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if self.configs.is_empty() {
                    ui.weak("실행할 구성을 찾지 못했습니다. [구성 편집…]에서 직접 적으세요.");
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (i, config) in self.configs.iter().enumerate() {
                            if i == 0 && self.custom > 0 {
                                ui.weak("사용자 구성");
                            }
                            if i == self.custom {
                                ui.weak("자동 감지");
                            }
                            let row = ui
                                .horizontal(|ui| {
                                    let row = ui.selectable_label(i == self.selected, &config.name);
                                    if config.command != config.name {
                                        ui.weak(&config.command);
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
                edit = ui
                    .button("구성 편집…")
                    .on_hover_text(run::CUSTOM_FILE)
                    .clicked();
            });
        let action = if edit {
            Some(RunAction::Edit)
        } else {
            chosen
                .and_then(|i| self.configs.get(i))
                .map(|c| RunAction::Run(c.clone()))
        };
        if let Some(RunAction::Run(config)) = &action {
            self.last = Some(config.name.clone());
        }
        self.open = open && action.is_none();
        action
    }
}
