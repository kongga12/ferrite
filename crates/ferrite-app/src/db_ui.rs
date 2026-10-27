//! DB 연결 창(도구 > DB 연결). 연결을 추가·고치기·지우기·시험한다. 연결 목록은 설정 폴더의
//! `connections.toml`에, 비밀번호는 OS 키체인에 둔다(파일에 쓰지 않는다). JDBC 종류(Oracle·Tibero·
//! CUBRID)는 드라이버 jar를 고르거나, 로컬 Maven 저장소·프로젝트에서 찾거나, [받기]로 받는다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use eframe::egui;
use ferrite_db::config::{self, ConnectionConfig, Kind};
use ferrite_db::jdbc::{self, DOWNLOADS};
use ferrite_db::secret::SecretStore;
use ferrite_db::{Connection, Event};
use ferrite_lsp::install;

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
    /// 로컬 Maven 저장소·프로젝트에서 찾은 드라이버 jar(JDBC 종류).
    found: Vec<PathBuf>,
}

enum Test {
    Running(Connection),
    Passed(String),
    Failed(String),
}

/// [받기] 진행. 작업 스레드가 바꾼다.
enum Download {
    Running { received: u64, total: u64 },
    Done(Vec<PathBuf>),
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
    /// 드라이버 jar를 찾는 로컬 Maven 저장소.
    m2: Option<PathBuf>,
    /// [받기]로 받은 드라이버를 두는 곳. 없으면(시험) 받지 않는다.
    drivers_dir: Option<PathBuf>,
    /// 열린 프로젝트(`WEB-INF/lib` 등에서 드라이버 jar를 찾는다).
    project: Option<PathBuf>,
    download: Option<(Kind, Arc<Mutex<Download>>)>,
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
            m2: None,
            drivers_dir: None,
            project: None,
            download: None,
        }
    }

    /// 드라이버 jar를 찾을 로컬 Maven 저장소와 [받기]로 받은 jar를 둘 곳.
    pub fn with_driver_dirs(mut self, m2: Option<PathBuf>, drivers_dir: Option<PathBuf>) -> Self {
        self.m2 = m2;
        self.drivers_dir = drivers_dir;
        self
    }

    /// 창을 연다. `project`의 lib 폴더에서도 드라이버 jar를 찾는다.
    pub fn show(&mut self, project: Option<&Path>) {
        self.open = true;
        self.project = project.map(Path::to_path_buf);
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
        let found = jdbc::candidates(config.kind, self.m2.as_deref(), self.project.as_deref());
        self.editor = Some(Editor {
            config,
            password: String::new(),
            clear_password: false,
            has_saved_password,
            is_new,
            error: None,
            found,
        });
    }

    /// [받기]: 고정한 판의 드라이버를 받아(받아 둔 것은 그대로 쓴다) 고치는 연결에 넣는다.
    fn start_download(&mut self, kind: Kind, ctx: &egui::Context) {
        let Some(dir) = self.drivers_dir.clone() else {
            return;
        };
        let total = DOWNLOADS
            .iter()
            .filter(|d| d.kind == kind)
            .map(|d| d.size)
            .sum();
        let state = Arc::new(Mutex::new(Download::Running { received: 0, total }));
        let (shared, wake) = (state.clone(), ctx.clone());
        let set = move |next: Download| {
            *shared.lock().unwrap_or_else(PoisonError::into_inner) = next;
            wake.request_repaint();
        };
        std::thread::spawn(move || {
            let (mut done, mut files) = (0, Vec::new());
            for download in DOWNLOADS.iter().filter(|d| d.kind == kind) {
                let dest = dir.join(download.file);
                // 받은 파일은 크기·해시를 맞춘 뒤에만 이 이름이 된다.
                if !dest.is_file() {
                    let result = install::download(
                        download.url,
                        download.size,
                        download.sha256,
                        &dest,
                        &mut |received, _| {
                            set(Download::Running {
                                received: done + received,
                                total,
                            });
                        },
                    );
                    if let Err(e) = result {
                        set(Download::Failed(format!("{}: {e}", download.file)));
                        return;
                    }
                }
                done += download.size;
                files.push(dest);
            }
            set(Download::Done(files));
        });
        self.download = Some((kind, state));
    }

    /// 받기가 끝났으면 고치는 연결(같은 종류)에 jar를 넣는다.
    fn poll_download(&mut self) {
        let Some((kind, state)) = &self.download else {
            return;
        };
        let files = match &*state.lock().unwrap_or_else(PoisonError::into_inner) {
            Download::Done(files) => files.clone(),
            _ => return,
        };
        if let Some(editor) = self.editor.as_mut().filter(|e| e.config.kind == *kind) {
            for file in files {
                if !editor.config.drivers.contains(&file) {
                    editor.config.drivers.push(file);
                }
            }
        }
        self.download = None;
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
        self.poll_download();
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
                ui.horizontal_wrapped(|ui| {
                    ui.label("새 연결");
                    for kind in Kind::ALL {
                        if ui.button(kind.label()).clicked() {
                            action = Some(Action::New(kind));
                        }
                    }
                });
                if let Some(editor) = &mut self.editor {
                    ui.separator();
                    let download = Downloads {
                        enabled: self.drivers_dir.is_some(),
                        state: self
                            .download
                            .as_ref()
                            .filter(|(kind, _)| *kind == editor.config.kind)
                            .map(|(_, state)| state.as_ref()),
                    };
                    if let Some(chosen) =
                        editor_ui(ui, editor, self.tests.get(&editor.config.id), download)
                    {
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
            Some(Action::Download(kind)) => self.start_download(kind, ctx),
            None => {}
        }
        None
    }
}

/// 연결 고치기에 보일 [받기] 상태.
struct Downloads<'a> {
    /// 받을 곳이 있다(시험은 받지 않는다).
    enabled: bool,
    /// 이 종류를 받는 중이거나 실패했다.
    state: Option<&'a Mutex<Download>>,
}

enum Action {
    Download(Kind),
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
fn editor_ui(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    test: Option<&Test>,
    download: Downloads,
) -> Option<Action> {
    let mut action = None;
    ui.strong(if editor.is_new {
        "새 연결"
    } else {
        "연결 고치기"
    });
    let config = &mut editor.config;
    let built_url = {
        let mut built = config.clone();
        built.url.clear();
        built.jdbc_url()
    };
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
                field(ui, config.kind.database_label(), &mut config.database);
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
                if config.kind.is_jdbc() {
                    let label = ui.label("JDBC URL");
                    ui.add(
                        egui::TextEdit::singleline(&mut config.url)
                            .hint_text(format!("비우면 {built_url}"))
                            .desired_width(320.0),
                    )
                    .labelled_by(label.id)
                    .on_hover_text("적으면 호스트·포트·데이터베이스 대신 씁니다(Oracle SID 등)");
                    ui.end_row();
                    ui.label("드라이버 jar");
                    if let Some(chosen) = drivers_ui(ui, config, &editor.found, &download) {
                        action = Some(chosen);
                    }
                    ui.end_row();
                }
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

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// JDBC 드라이버 jar: 고른 것(빼기), [jar 고르기…], [받기], 찾은 것(추가).
fn drivers_ui(
    ui: &mut egui::Ui,
    config: &mut ConnectionConfig,
    found: &[PathBuf],
    download: &Downloads,
) -> Option<Action> {
    let mut action = None;
    ui.vertical(|ui| {
        let mut remove = None;
        for (index, jar) in config.drivers.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(file_name(jar))
                    .on_hover_text(jar.display().to_string());
                if ui.small_button("빼기").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            config.drivers.remove(index);
        }
        let add = |config: &mut ConnectionConfig, jar: PathBuf| {
            if !config.drivers.contains(&jar) {
                config.drivers.push(jar);
            }
        };
        ui.horizontal_wrapped(|ui| {
            if ui.button("jar 고르기…").clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .set_title("JDBC 드라이버 jar")
                    .add_filter("jar", &["jar"])
                    .pick_files()
            {
                for file in files {
                    add(config, file);
                }
            }
            let pinned: Vec<_> = DOWNLOADS.iter().filter(|d| d.kind == config.kind).collect();
            let state = download
                .state
                .map(|s| s.lock().unwrap_or_else(PoisonError::into_inner));
            let running = matches!(state.as_deref(), Some(Download::Running { .. }));
            if !pinned.is_empty() {
                let size: u64 = pinned.iter().map(|d| d.size).sum();
                let names: Vec<&str> = pinned.iter().map(|d| d.file).collect();
                let host = pinned[0].url.split('/').nth(2).unwrap_or_default();
                if ui
                    .add_enabled(
                        download.enabled && !running,
                        egui::Button::new(format!("받기({:.1}MB)", size as f64 / 1_000_000.0)),
                    )
                    .on_hover_text(format!(
                        "{host}에서 {}를 받습니다(크기·SHA-256 확인)",
                        names.join(", ")
                    ))
                    .clicked()
                {
                    action = Some(Action::Download(config.kind));
                }
            }
            match state.as_deref() {
                Some(Download::Running { received, total }) => {
                    ui.spinner();
                    ui.label(format!(
                        "받는 중 {:.1}/{:.1}MB",
                        *received as f64 / 1_000_000.0,
                        *total as f64 / 1_000_000.0
                    ));
                }
                Some(Download::Failed(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, format!("받지 못했습니다: {e}"));
                }
                _ => {}
            }
        });
        let rest: Vec<&PathBuf> = found
            .iter()
            .filter(|jar| !config.drivers.contains(jar))
            .collect();
        if !rest.is_empty() {
            ui.weak("찾은 jar(로컬 Maven 저장소·프로젝트)");
            for jar in rest {
                ui.horizontal(|ui| {
                    if ui.small_button("추가").clicked() {
                        add(config, jar.clone());
                    }
                    ui.label(file_name(jar))
                        .on_hover_text(jar.display().to_string());
                });
            }
        }
    });
    action
}
