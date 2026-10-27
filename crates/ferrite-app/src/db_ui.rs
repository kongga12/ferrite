//! DB 연결 창(도구 > DB 연결). 연결을 추가·고치기·지우기·시험한다. 연결 목록은 설정 폴더의
//! `connections.toml`에, 비밀번호는 OS 키체인에 둔다(파일에 쓰지 않는다).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use ferrite_db::config::{self, ConnectionConfig, Kind};
use ferrite_db::secret::SecretStore;
use ferrite_db::{Connection, Event};

/// 고치는 중인 연결.
struct Editor {
    config: ConnectionConfig,
    /// 새로 적은 비밀번호. 비워 두면 저장된 비밀번호를 그대로 둔다.
    password: String,
    /// 저장된 비밀번호를 지운다.
    clear_password: bool,
    has_saved_password: bool,
    is_new: bool,
    error: Option<String>,
}

enum Test {
    Running(Connection),
    Passed(String),
    Failed(String),
}

pub struct DbWindow {
    open: bool,
    /// 연결 목록 파일. 없으면(시험) 저장하지 않는다.
    path: Option<PathBuf>,
    secrets: Arc<dyn SecretStore>,
    connections: Vec<ConnectionConfig>,
    error: Option<String>,
    editor: Option<Editor>,
    /// 연결 고유 값 → 시험 결과.
    tests: HashMap<String, Test>,
    confirm_delete: Option<String>,
}

impl DbWindow {
    pub fn new(path: Option<PathBuf>, secrets: Arc<dyn SecretStore>) -> Self {
        let (connections, error) = match path.as_deref().map(config::load) {
            Some(Ok(list)) => (list, None),
            Some(Err(e)) => (Vec::new(), Some(e)),
            None => (Vec::new(), None),
        };
        Self {
            open: false,
            path,
            secrets,
            connections,
            error,
            editor: None,
            tests: HashMap::new(),
            confirm_delete: None,
        }
    }

    pub fn show(&mut self) {
        self.open = true;
    }

    pub fn connections(&self) -> &[ConnectionConfig] {
        &self.connections
    }

    /// 키체인에 저장된 비밀번호.
    pub fn password(&self, id: &str) -> Result<Option<String>, String> {
        self.secrets.get(id)
    }

    fn save_list(&mut self) -> Result<(), String> {
        match &self.path {
            Some(path) => config::save(path, &self.connections)
                .map_err(|e| format!("연결 목록을 저장하지 못했습니다: {e}")),
            None => Ok(()),
        }
    }

    fn edit(&mut self, config: ConnectionConfig, is_new: bool) {
        let has_saved_password = !is_new && matches!(self.secrets.get(&config.id), Ok(Some(_)));
        self.editor = Some(Editor {
            config,
            password: String::new(),
            clear_password: false,
            has_saved_password,
            is_new,
            error: None,
        });
    }

    /// 연결을 시험한다. 고치는 중이면 적은 값과 비밀번호로 한다.
    fn start_test(&mut self, config: &ConnectionConfig, typed: Option<&str>, ctx: &egui::Context) {
        let password = match typed.filter(|p| !p.is_empty()) {
            Some(password) => Some(password.to_owned()),
            None => match self.secrets.get(&config.id) {
                Ok(password) => password,
                Err(e) => {
                    self.tests.insert(config.id.clone(), Test::Failed(e));
                    return;
                }
            },
        };
        let wake = ctx.clone();
        let connection = Connection::open(config, password, move || wake.request_repaint());
        self.tests
            .insert(config.id.clone(), Test::Running(connection));
    }

    fn poll_tests(&mut self) {
        for test in self.tests.values_mut() {
            let Test::Running(connection) = test else {
                continue;
            };
            // 연결되면 바로 닫는다(시험 연결은 버리면 작업 스레드가 닫는다).
            for event in connection.poll() {
                match event {
                    Event::Connected { version } => *test = Test::Passed(version),
                    Event::Disconnected(e) => *test = Test::Failed(e),
                    _ => continue,
                }
                break;
            }
        }
    }

    /// 고친 연결을 저장한다. 비밀번호는 키체인에 둔다.
    fn save_editor(&mut self) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let problems = editor.config.problems();
        if !problems.is_empty() {
            editor.error = Some(problems.join(", "));
            return;
        }
        let id = editor.config.id.clone();
        let secret = if editor.clear_password {
            self.secrets.delete(&id)
        } else if !editor.password.is_empty() && editor.config.kind.is_server() {
            self.secrets.set(&id, &editor.password)
        } else {
            Ok(())
        };
        if let Err(e) = secret {
            editor.error = Some(e);
            return;
        }
        let config = editor.config.clone();
        match self.connections.iter_mut().find(|c| c.id == id) {
            Some(existing) => *existing = config,
            None => self.connections.push(config),
        }
        match self.save_list() {
            Ok(()) => {
                self.editor = None;
                self.tests.remove(&id);
            }
            Err(e) => {
                if let Some(editor) = &mut self.editor {
                    editor.error = Some(e);
                }
            }
        }
    }

    fn delete(&mut self, id: &str) {
        self.connections.retain(|c| c.id != id);
        self.tests.remove(id);
        // 비밀번호를 못 지워도 목록에서는 뺀다(알리기만 한다).
        let secret = self.secrets.delete(id);
        self.error = self.save_list().err().or(secret.err());
    }

    /// [SQL 콘솔]을 누르면 그 연결을 돌려준다.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<ConnectionConfig> {
        if !self.open {
            return None;
        }
        self.poll_tests();
        let mut open = self.open;
        let mut action = None;
        egui::Window::new("DB 연결")
            .open(&mut open)
            // 메뉴 줄을 가리지 않게 그 아래에 처음 띄운다.
            .default_pos([120.0, 80.0])
            .default_width(560.0)
            .show(ctx, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if self.connections.is_empty() {
                    ui.weak("연결이 없습니다. [새 연결]로 추가하세요.");
                }
                for connection in &self.connections {
                    ui.horizontal(|ui| {
                        ui.strong(&connection.name);
                        ui.label(connection.kind.label());
                        ui.weak(connection.summary());
                    });
                    ui.horizontal(|ui| {
                        if ui.button("SQL 콘솔").clicked() {
                            action = Some(Action::Console(connection.clone()));
                        }
                        if ui.button("연결 시험").clicked() {
                            action = Some(Action::Test(connection.clone()));
                        }
                        if ui.button("고치기").clicked() {
                            action = Some(Action::Edit(connection.clone()));
                        }
                        if self.confirm_delete.as_deref() == Some(&connection.id) {
                            if ui.button("정말 지우기").clicked() {
                                action = Some(Action::Delete(connection.id.clone()));
                            }
                            if ui.button("그만두기").clicked() {
                                action = Some(Action::CancelDelete);
                            }
                        } else if ui.button("지우기").clicked() {
                            action = Some(Action::ConfirmDelete(connection.id.clone()));
                        }
                        test_label(ui, self.tests.get(&connection.id));
                    });
                    ui.separator();
                }
                ui.horizontal(|ui| {
                    ui.label("새 연결");
                    for kind in Kind::ALL {
                        if ui.button(kind.label()).clicked() {
                            action = Some(Action::New(kind));
                        }
                    }
                });
                if let Some(editor) = &mut self.editor {
                    ui.separator();
                    if let Some(chosen) = editor_ui(ui, editor, self.tests.get(&editor.config.id)) {
                        action = Some(chosen);
                    }
                }
            });
        self.open = open;
        match action {
            Some(Action::Test(config)) => self.start_test(&config, None, ctx),
            Some(Action::TestEditor) => {
                if let Some(editor) = &self.editor {
                    let (config, password) = (editor.config.clone(), editor.password.clone());
                    self.start_test(&config, Some(&password), ctx);
                }
            }
            Some(Action::Edit(config)) => self.edit(config, false),
            Some(Action::New(kind)) => self.edit(ConnectionConfig::new(kind), true),
            Some(Action::Save) => self.save_editor(),
            Some(Action::CancelEdit) => self.editor = None,
            Some(Action::ConfirmDelete(id)) => self.confirm_delete = Some(id),
            Some(Action::CancelDelete) => self.confirm_delete = None,
            Some(Action::Delete(id)) => {
                self.confirm_delete = None;
                self.delete(&id);
            }
            Some(Action::Console(config)) => return Some(config),
            None => {}
        }
        None
    }
}

enum Action {
    Console(ConnectionConfig),
    Test(ConnectionConfig),
    TestEditor,
    Edit(ConnectionConfig),
    New(Kind),
    Save,
    CancelEdit,
    ConfirmDelete(String),
    CancelDelete,
    Delete(String),
}

fn test_label(ui: &mut egui::Ui, test: Option<&Test>) {
    match test {
        Some(Test::Running(_)) => {
            ui.spinner();
            ui.label("연결하는 중…");
        }
        Some(Test::Passed(version)) => {
            ui.colored_label(
                egui::Color32::from_rgb(60, 150, 80),
                format!("연결됨: {version}"),
            );
        }
        Some(Test::Failed(e)) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        None => {}
    }
}

/// 연결 고치기. 고른 일을 돌려준다.
fn editor_ui(ui: &mut egui::Ui, editor: &mut Editor, test: Option<&Test>) -> Option<Action> {
    let mut action = None;
    ui.strong(if editor.is_new {
        "새 연결"
    } else {
        "연결 고치기"
    });
    let config = &mut editor.config;
    egui::Grid::new("db_editor")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            let field = |ui: &mut egui::Ui, label: &str, value: &mut String| {
                let label = ui.label(label);
                ui.add(egui::TextEdit::singleline(value).desired_width(320.0))
                    .labelled_by(label.id);
                ui.end_row();
            };
            ui.label("종류");
            ui.label(config.kind.label());
            ui.end_row();
            field(ui, "이름", &mut config.name);
            if config.kind.is_server() {
                field(ui, "호스트", &mut config.host);
                let label = ui.label("포트");
                ui.add(egui::DragValue::new(&mut config.port).range(1..=65535))
                    .labelled_by(label.id);
                ui.end_row();
                field(ui, "데이터베이스", &mut config.database);
                field(ui, "사용자", &mut config.user);
                let label = ui.label("비밀번호");
                ui.horizontal(|ui| {
                    let hint = if editor.has_saved_password {
                        "저장된 비밀번호를 쓴다(바꾸려면 적기)"
                    } else {
                        "OS 키체인에 저장한다"
                    };
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.password)
                            .password(true)
                            .hint_text(hint)
                            .desired_width(240.0),
                    )
                    .labelled_by(label.id);
                    if editor.has_saved_password {
                        ui.checkbox(&mut editor.clear_password, "저장된 비밀번호 지우기");
                    }
                });
                ui.end_row();
            } else {
                let label = ui.label("파일");
                ui.horizontal(|ui| {
                    let mut text = config
                        .path
                        .as_ref()
                        .map_or_else(String::new, |p| p.display().to_string());
                    if ui
                        .add(egui::TextEdit::singleline(&mut text).desired_width(240.0))
                        .labelled_by(label.id)
                        .changed()
                    {
                        config.path = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
                    }
                    if ui.button("고르기…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("SQLite 파일")
                            .add_filter("SQLite", &["db", "sqlite", "sqlite3"])
                            .pick_file()
                    {
                        config.path = Some(path);
                    }
                });
                ui.end_row();
            }
        });
    if let Some(error) = &editor.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    ui.horizontal(|ui| {
        if ui.button("저장").clicked() {
            action = Some(Action::Save);
        }
        if ui.button("연결 시험").clicked() {
            action = Some(Action::TestEditor);
        }
        if ui.button("취소").clicked() {
            action = Some(Action::CancelEdit);
        }
        test_label(ui, test);
    });
    action
}
