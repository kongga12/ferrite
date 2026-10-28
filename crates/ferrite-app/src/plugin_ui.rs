//! WASM 플러그인: [플러그인] 메뉴의 명령과 플러그인 창(패널). 시작할 때 설정 폴더의
//! `ferrite/plugins/`에서 읽는다. 플러그인이 시킨 일(선택 영역 바꾸기, 알림, 패널 열기)은
//! 앱이 창을 다 그린 뒤 한꺼번에 한다(`take_actions`).

use std::path::PathBuf;

use eframe::egui;
use ferrite_plugin::{Action, Editor, Event, Plugin, Widget};

/// 플러그인에 넘길 활성 에디터(앱이 만든다).
pub struct Snapshot {
    pub path: PathBuf,
    pub language: Option<String>,
    pub text: String,
    pub selection: String,
}

impl Snapshot {
    fn editor(&self) -> Editor<'_> {
        Editor {
            path: &self.path,
            language: self.language.as_deref(),
            text: &self.text,
            selection: &self.selection,
        }
    }
}

/// 패널(플러그인 번호, 패널 id).
type PanelKey = (usize, String);
/// 활성 문서와 그 판.
type DocKey = Option<(PathBuf, u64)>;

/// 메뉴에서 고른 것.
pub enum MenuChoice {
    Command(usize, String),
    Panel(usize, String),
    Window,
    Reload,
}

#[derive(Default)]
pub struct Plugins {
    root: Option<PathBuf>,
    plugins: Vec<Plugin>,
    /// 같은 폴더의 언어 확장(package.json).
    languages: ferrite_textmate::Registry,
    errors: Vec<String>,
    /// 플러그인 창의 패널(플러그인 번호, 패널 id).
    panel: Option<PanelKey>,
    /// 패널이 마지막으로 돌려준 위젯과 오류.
    widgets: Vec<Widget>,
    panel_error: Option<String>,
    /// 위젯을 그린 때의 (패널, 활성 문서와 그 판). 바뀌면 플러그인에 다시 묻는다.
    drawn: Option<(PanelKey, DocKey)>,
    /// 앱이 할 일(플러그인 번호, 일).
    actions: Vec<(usize, Action)>,
}

impl Plugins {
    /// `root`(없으면 플러그인을 쓰지 않는다)의 플러그인을 읽는다.
    pub fn load(root: Option<PathBuf>) -> Self {
        let mut plugins = Self {
            root,
            ..Default::default()
        };
        plugins.reload();
        plugins
    }

    /// 다시 읽는다. 플러그인 상태는 처음부터다. 읽은 수와 오류 수를 알리는 글을 돌려준다.
    pub fn reload(&mut self) -> String {
        (self.plugins, self.errors) = match &self.root {
            Some(root) => ferrite_plugin::load_all(root),
            None => Default::default(),
        };
        let (languages, errors) = match &self.root {
            Some(root) => ferrite_textmate::Registry::load(root),
            None => Default::default(),
        };
        self.languages = languages;
        self.errors.extend(errors);
        self.widgets.clear();
        self.panel_error = None;
        self.drawn = None;
        self.panel = self
            .plugins
            .iter()
            .enumerate()
            .find_map(|(i, p)| p.manifest.panels.first().map(|panel| (i, panel.id.clone())));
        let read = format!(
            "플러그인 {}개, 언어 확장 {}개를 읽었습니다",
            self.plugins.len(),
            self.languages.len()
        );
        match self.errors.len() {
            0 => read,
            n => format!("{read}. 읽지 못한 것 {n}개(플러그인 창)"),
        }
    }

    pub fn languages(&self) -> &ferrite_textmate::Registry {
        &self.languages
    }

    pub fn name(&self, plugin: usize) -> &str {
        self.plugins
            .get(plugin)
            .map_or("", |p| p.manifest.name.as_str())
    }

    /// 앱이 할 일을 꺼낸다.
    pub fn take_actions(&mut self) -> Vec<(usize, Action)> {
        std::mem::take(&mut self.actions)
    }

    /// [플러그인] 메뉴의 내용.
    pub fn menu_ui(&self, ui: &mut egui::Ui) -> Option<MenuChoice> {
        let mut choice = None;
        if self.plugins.is_empty() {
            ui.weak("설치한 플러그인이 없습니다");
        }
        for (i, plugin) in self.plugins.iter().enumerate() {
            let m = &plugin.manifest;
            if m.commands.is_empty() && m.panels.is_empty() {
                continue;
            }
            ui.menu_button(&m.name, |ui| {
                for c in &m.commands {
                    if ui.button(&c.title).clicked() {
                        choice = Some(MenuChoice::Command(i, c.id.clone()));
                    }
                }
                if !m.commands.is_empty() && !m.panels.is_empty() {
                    ui.separator();
                }
                for p in &m.panels {
                    if ui.button(format!("{} 창", p.title)).clicked() {
                        choice = Some(MenuChoice::Panel(i, p.id.clone()));
                    }
                }
            });
        }
        ui.separator();
        if ui.button("플러그인 창").clicked() {
            choice = Some(MenuChoice::Window);
        }
        if ui.button("플러그인 다시 읽기").clicked() {
            choice = Some(MenuChoice::Reload);
        }
        if choice.is_some() {
            ui.close();
        }
        choice
    }

    /// 명령을 부른다. 시킨 일은 `take_actions`로 꺼낸다.
    pub fn run_command(&mut self, plugin: usize, id: &str, snapshot: Option<&Snapshot>) {
        let Some(p) = self.plugins.get_mut(plugin) else {
            return;
        };
        let editor = snapshot.map(Snapshot::editor);
        match p.command(id, editor.as_ref()) {
            Ok(reply) => self
                .actions
                .extend(reply.actions.into_iter().map(|a| (plugin, a))),
            Err(e) => self.actions.push((plugin, Action::Message { text: e })),
        }
    }

    /// 플러그인 창에 보일 패널을 고른다. 그 플러그인에 없는 패널이면 false.
    pub fn select(&mut self, plugin: usize, panel: &str) -> bool {
        let known = self
            .plugins
            .get(plugin)
            .is_some_and(|p| p.manifest.panels.iter().any(|c| c.id == panel));
        if known {
            self.panel = Some((plugin, panel.to_owned()));
        }
        known
    }

    /// 플러그인 창. `key`는 활성 문서와 그 판이고, 바뀌었을 때만 `snapshot`을 만들어 다시 묻는다.
    pub fn panel_ui(
        &mut self,
        ui: &mut egui::Ui,
        key: DocKey,
        snapshot: impl Fn() -> Option<Snapshot>,
    ) {
        for e in &self.errors {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if self.plugins.iter().all(|p| p.manifest.panels.is_empty()) {
            self.empty_ui(ui);
            return;
        }
        self.panel_picker(ui);
        let Some(panel) = self.panel.clone() else {
            return;
        };
        let drawn = Some((panel.clone(), key));
        if self.drawn != drawn {
            self.drawn = drawn;
            self.ask(&panel, None, snapshot().as_ref());
        }
        if let Some(e) = &self.panel_error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        let mut event = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for w in &mut self.widgets {
                match w {
                    Widget::Heading { text } => {
                        ui.heading(text.as_str());
                    }
                    Widget::Label { text } => {
                        ui.label(text.as_str());
                    }
                    Widget::Code { text } => {
                        ui.code(text.as_str());
                    }
                    Widget::Separator => {
                        ui.separator();
                    }
                    Widget::Button { id, text } => {
                        if ui.button(text.as_str()).clicked() {
                            event = Some(Event::Click { id: id.clone() });
                        }
                    }
                    Widget::Input { id, value, hint } => {
                        // 고친 값을 바로 보이게 여기에 쓰고, 플러그인 답이 오면 그것으로 바꾼다.
                        let edit = egui::TextEdit::singleline(value)
                            .hint_text(hint.as_str())
                            .id_salt(("plugin-input", id.as_str()));
                        if ui.add(edit).changed() {
                            event = Some(Event::Change {
                                id: id.clone(),
                                value: value.clone(),
                            });
                        }
                    }
                    Widget::Unknown => {}
                }
            }
        });
        if let Some(event) = event {
            self.ask(&panel, Some(&event), snapshot().as_ref());
        }
    }

    fn ask(&mut self, (plugin, panel): &PanelKey, event: Option<&Event>, s: Option<&Snapshot>) {
        let Some(p) = self.plugins.get_mut(*plugin) else {
            return;
        };
        let editor = s.map(Snapshot::editor);
        match p.panel(panel, event, editor.as_ref()) {
            Ok(reply) => {
                self.widgets = reply.ui;
                self.panel_error = None;
                self.actions
                    .extend(reply.actions.into_iter().map(|a| (*plugin, a)));
            }
            Err(e) => {
                self.widgets.clear();
                self.panel_error = Some(e);
            }
        }
    }

    /// 패널이 둘 이상이면 고르는 상자.
    fn panel_picker(&mut self, ui: &mut egui::Ui) {
        let panels: Vec<(PanelKey, String)> = self
            .plugins
            .iter()
            .enumerate()
            .flat_map(|(i, p)| {
                p.manifest.panels.iter().map(move |c| {
                    (
                        (i, c.id.clone()),
                        format!("{} · {}", p.manifest.name, c.title),
                    )
                })
            })
            .collect();
        if panels.len() < 2 {
            return;
        }
        let current = panels
            .iter()
            .find(|(key, _)| Some(key) == self.panel.as_ref())
            .map_or("", |(_, title)| title.as_str());
        egui::ComboBox::from_id_salt("plugin-panel")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (key, title) in &panels {
                    ui.selectable_value(&mut self.panel, Some(key.clone()), title);
                }
            });
        ui.separator();
    }

    fn empty_ui(&self, ui: &mut egui::Ui) {
        ui.label("패널을 더하는 플러그인이 없습니다.");
        let Some(root) = &self.root else {
            return;
        };
        ui.label("플러그인 폴더에 플러그인마다 폴더를 만들고 plugin.toml과 .wasm을 넣은 뒤 [플러그인 > 플러그인 다시 읽기]를 누르세요.");
        ui.horizontal(|ui| {
            ui.code(root.display().to_string());
            if ui.button("경로 복사").clicked() {
                ui.ctx().copy_text(root.display().to_string());
            }
        });
    }
}

/// 설정 폴더의 플러그인 폴더. 테스트는 사용자 플러그인을 읽지 않는다.
pub fn plugins_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(dirs::config_dir()?.join("ferrite").join("plugins"))
}
