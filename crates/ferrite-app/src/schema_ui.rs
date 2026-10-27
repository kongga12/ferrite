//! 스키마 브라우저(도구 > 스키마 브라우저). 고른 연결의 표·뷰와 열을 보인다. SQL 콘솔의 연결을
//! 건드리지 않게 따로 연결을 열어 읽고 닫는다. [SQL 콘솔에서 조회]는 그 표를 콘솔에서 조회한다.

use eframe::egui;
use ferrite_db::{Connection, ConnectionConfig, Event, Table};

use crate::db_ui::DbWindow;

/// 스키마 브라우저가 앱에 맡기는 일.
pub enum Action {
    /// 이 연결로 SQL 콘솔을 열고 문장을 실행한다.
    Query(ConnectionConfig, String),
}

#[derive(Default)]
pub struct SchemaBrowser {
    config: Option<ConnectionConfig>,
    /// 읽는 중인 연결. 다 읽으면 버린다(작업 스레드가 닫는다).
    loading: Option<Connection>,
    tables: Vec<Table>,
    error: Option<String>,
    filter: String,
    /// 표 목록이 바뀔 때마다 는다(매퍼 SQL 검사를 다시 한다).
    generation: u64,
}

impl SchemaBrowser {
    /// 읽은 표와 뷰(매퍼 SQL 자동 완성·검증이 쓴다). 읽지 않았으면 비어 있다.
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 스키마를 읽은 연결 이름.
    pub fn connection_name(&self) -> &str {
        self.config.as_ref().map_or("", |c| c.name.as_str())
    }

    fn load(&mut self, config: ConnectionConfig, db: &DbWindow, ctx: &egui::Context) {
        self.tables.clear();
        self.generation += 1;
        self.error = None;
        match db.password(&config.id) {
            Ok(password) => {
                let wake = ctx.clone();
                let connection =
                    Connection::open(&config, password, move || wake.request_repaint());
                connection.load_schema();
                self.loading = Some(connection);
            }
            Err(e) => self.error = Some(e),
        }
        self.config = Some(config);
    }

    fn poll(&mut self) {
        let Some(connection) = &self.loading else {
            return;
        };
        for event in connection.poll() {
            match event {
                Event::Schema(tables) => {
                    self.tables = tables;
                    self.generation += 1;
                }
                Event::Error(e) | Event::Disconnected(e) => self.error = Some(e),
                _ => continue,
            }
            self.loading = None;
            break;
        }
    }

    /// 표 이름이나 열 이름에 `filter`가 든 표(대소문자 무시).
    fn matches(table: &Table, filter: &str) -> bool {
        filter.is_empty()
            || table.label().to_lowercase().contains(filter)
            || table
                .columns
                .iter()
                .any(|c| c.name.to_lowercase().contains(filter))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, db: &DbWindow) -> Option<Action> {
        self.poll();
        let ctx = ui.ctx().clone();
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            let label = ui.label("연결");
            let current = self
                .config
                .as_ref()
                .map_or("연결 고르기", |c| c.name.as_str());
            egui::ComboBox::from_id_salt("schema_connection")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for config in db.connections() {
                        let selected = self.config.as_ref().is_some_and(|c| c.id == config.id);
                        if ui.selectable_label(selected, &config.name).clicked() {
                            chosen = Some(config.clone());
                        }
                    }
                    if db.connections().is_empty() {
                        ui.weak("도구 > DB 연결…에서 연결을 추가하세요");
                    }
                })
                .response
                .labelled_by(label.id);
            let refresh = ui.add_enabled(
                self.config.is_some() && self.loading.is_none(),
                egui::Button::new("새로 고침"),
            );
            if refresh.clicked() {
                chosen = self.config.clone();
            }
            if self.loading.is_some() {
                ui.spinner();
                ui.label("읽는 중…");
            }
        });
        if let Some(config) = chosen {
            self.load(config, db, &ctx);
        }
        ui.horizontal(|ui| {
            let label = ui.label("찾기");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("표·열 이름")
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
        });
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let filter = self.filter.trim().to_lowercase();
        let shown: Vec<&Table> = self
            .tables
            .iter()
            .filter(|t| Self::matches(t, &filter))
            .collect();
        if self.config.is_some() && self.loading.is_none() && self.error.is_none() {
            let views = self.tables.iter().filter(|t| t.view).count();
            ui.weak(format!(
                "표 {}개 · 뷰 {views}개{}",
                self.tables.len() - views,
                if filter.is_empty() {
                    String::new()
                } else {
                    format!(" · 찾은 것 {}개", shown.len())
                }
            ));
        }
        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for table in shown {
                    let title = if table.view {
                        format!("{} (뷰)", table.label())
                    } else {
                        table.label()
                    };
                    // 열 이름으로 찾았으면 펼쳐 보인다.
                    let open = (!filter.is_empty()
                        && !table.label().to_lowercase().contains(&filter))
                    .then_some(true);
                    egui::CollapsingHeader::new(title)
                        .id_salt(table.label())
                        .open(open)
                        .show(ui, |ui| {
                            if ui.small_button("SQL 콘솔에서 조회").clicked()
                                && let Some(config) = &self.config
                            {
                                let sql = format!("SELECT * FROM {}", table.sql_name(config.kind));
                                action = Some(Action::Query(config.clone(), sql));
                            }
                            egui::Grid::new(("schema_columns", table.label()))
                                .num_columns(2)
                                .spacing([16.0, 2.0])
                                .show(ui, |ui| {
                                    for column in &table.columns {
                                        ui.label(&column.name);
                                        ui.weak(&column.ty);
                                        ui.end_row();
                                    }
                                });
                        });
                }
            });
        action
    }
}
