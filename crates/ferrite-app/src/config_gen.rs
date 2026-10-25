//! egov 설정 파일 생성 창(도구 > egov 설정 파일 생성). 종류(Datasource·Transaction·ID 생성·Logging·
//! Scheduling·Cache·Property)와 형식(XML·Java Config·YAML·properties)을 고르고, 표로 정의한 입력 칸을 채운다.

use std::path::{Path, PathBuf};

use eframe::egui;
use ferrite_codegen::config::{
    self, ConfigLayout, Format, InputKind, KINDS, Placeholders, Request,
};
use ferrite_codegen::crud::{Db, Rte};
use ferrite_codegen::project;
use ferrite_core::text_file::{Encoding, LineEnding};

pub struct ConfigWindow {
    open: bool,
    root: Option<PathBuf>,
    rte: Rte,
    db: Db,
    base_package: String,
    placeholders: Placeholders,
    layout: ConfigLayout,
    /// Java Config 파일의 인코딩(프로젝트 인코딩). XML·YAML·properties는 선언대로 UTF-8로 쓴다.
    java_encoding: Encoding,
    line_ending: LineEnding,
    /// `KINDS`의 번호.
    kind: usize,
    request: Request,
    overwrite: bool,
    /// (오류인지, 내용)
    message: Option<(bool, String)>,
}

impl Default for ConfigWindow {
    fn default() -> Self {
        let base_package = "egovframework.example".to_owned();
        let placeholders = Placeholders::default();
        Self {
            open: false,
            root: None,
            rte: Rte::V4,
            db: Db::Oracle,
            request: Request::new(&KINDS[0], Rte::V4, Db::Oracle, &base_package, &placeholders),
            base_package,
            placeholders,
            layout: ConfigLayout::default(),
            java_encoding: Encoding::Utf8,
            line_ending: LineEnding::Lf,
            kind: 0,
            overwrite: false,
            message: None,
        }
    }
}

impl ConfigWindow {
    /// 창을 연다. 프로젝트가 바뀌었으면 기본값을 다시 찾는다.
    pub fn show(&mut self, root: Option<&Path>) {
        self.open = true;
        let root = root.map(Path::to_path_buf);
        if root != self.root {
            self.root = root;
            if let Some(root) = &self.root {
                let found = project::defaults(root);
                self.rte = found.rte;
                self.db = found.db;
                self.base_package = found.base_package;
                self.placeholders = found.placeholders;
                self.layout = found.config_layout;
                self.java_encoding = found.encoding;
                self.line_ending = found.line_ending;
            }
            self.reset();
        }
    }

    /// 고른 종류의 기본값으로 다시 채운다.
    fn reset(&mut self) {
        self.request = Request::new(
            &KINDS[self.kind],
            self.rte,
            self.db,
            &self.base_package,
            &self.placeholders,
        );
        self.overwrite = false;
    }

    fn generate(&mut self) -> Option<PathBuf> {
        let Some(root) = self.root.clone() else {
            self.message = Some((true, "먼저 프로젝트 폴더를 여세요".into()));
            return None;
        };
        let overrides = root.join(".ferrite/templates/config");
        let encoding = match self.request.format {
            Format::JavaConfig => self.java_encoding,
            // XML·YAML·properties는 UTF-8로 쓴다(XML 선언이 UTF-8이다).
            _ => Encoding::Utf8,
        };
        let result = config::generate(
            &self.request,
            &self.layout,
            overrides.is_dir().then_some(overrides.as_path()),
        )
        .and_then(|file| {
            project::write(&root, &[file], encoding, self.line_ending, self.overwrite)
        });
        match result {
            Ok(mut written) => {
                let path = written.pop()?;
                self.message = Some((false, format!("만들었습니다: {}", path.display())));
                Some(path)
            }
            Err(e) => {
                self.message = Some((true, e));
                None
            }
        }
    }

    /// 창을 그린다. 파일을 만들었으면 그 경로.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        if !self.open {
            return None;
        }
        let mut written = None;
        let mut open = self.open;
        egui::Window::new("egov 설정 파일 생성")
            .open(&mut open)
            .default_width(680.0)
            .default_height(600.0)
            .show(ctx, |ui| {
                // 입력 칸은 스크롤하고, [생성]과 결과는 늘 보이게 아래에 둔다.
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height().max(240.0) - 48.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| self.form_ui(ui));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("생성").clicked() {
                        written = self.generate();
                    }
                    if let Some((error, text)) = &self.message {
                        let color = if *error {
                            ui.visuals().error_fg_color
                        } else {
                            ui.visuals().text_color()
                        };
                        ui.colored_label(color, text);
                    }
                });
            });
        self.open = open;
        written
    }

    fn form_ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("config_kind")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                let label = ui.label("설정 종류");
                let current = &KINDS[self.kind];
                let mut chosen = self.kind;
                egui::ComboBox::from_id_salt("config_kind_combo")
                    .width(360.0)
                    .selected_text(format!("{} > {}", current.category, current.label))
                    .show_ui(ui, |ui| {
                        for (i, kind) in KINDS.iter().enumerate() {
                            ui.selectable_value(
                                &mut chosen,
                                i,
                                format!("{} > {}", kind.category, kind.label),
                            );
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if chosen != self.kind {
                    self.kind = chosen;
                    self.reset();
                }
                ui.end_row();

                let label = ui.label("실행환경");
                let mut rte = self.request.rte;
                egui::ComboBox::from_id_salt("config_rte")
                    .selected_text(rte.label())
                    .show_ui(ui, |ui| {
                        for option in [Rte::V3, Rte::V4] {
                            ui.selectable_value(&mut rte, option, option.label());
                        }
                    })
                    .response
                    .labelled_by(label.id);
                self.request.rte = rte;
                ui.end_row();

                ui.label("형식");
                let kind = self.request.kind;
                ui.horizontal(|ui| {
                    for format in kind.formats {
                        // 글자가 있는 버튼이라 이름은 그 글자다(묶음 이름표는 붙이지 않는다).
                        let response =
                            ui.selectable_label(self.request.format == *format, format.label());
                        if response.clicked() && self.request.format != *format {
                            // 이름이 기본값이면 형식에 맞는 기본값으로 바꾼다.
                            let defaults = [kind.xml_name, kind.java_name];
                            if defaults.contains(&self.request.name.as_str()) {
                                self.request.name = match format {
                                    Format::JavaConfig => kind.java_name,
                                    _ => kind.xml_name,
                                }
                                .to_owned();
                            }
                            self.request.format = *format;
                        }
                    }
                });
                ui.end_row();

                let name_label = match self.request.format {
                    Format::JavaConfig => "클래스 이름",
                    _ => "파일 이름(확장자 빼고)",
                };
                let label = ui.label(name_label);
                ui.text_edit_singleline(&mut self.request.name)
                    .labelled_by(label.id);
                ui.end_row();
                if self.request.format == Format::JavaConfig {
                    let label = ui.label("패키지");
                    ui.text_edit_singleline(&mut self.request.package)
                        .labelled_by(label.id);
                    ui.end_row();
                }
            });
        ui.weak(KINDS[self.kind].description);
        ui.separator();
        egui::Grid::new("config_inputs")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for input in self.request.kind.inputs {
                    if !self.request.visible(input) {
                        continue;
                    }
                    let label = ui.label(input.label);
                    let value = self.request.values.entry(input.key.to_owned()).or_default();
                    match input.kind {
                        InputKind::Text => {
                            ui.add(egui::TextEdit::singleline(value).desired_width(380.0))
                                .labelled_by(label.id);
                        }
                        InputKind::Choice(options) => {
                            egui::ComboBox::from_id_salt(("config_choice", input.key))
                                .selected_text(value.as_str())
                                .show_ui(ui, |ui| {
                                    for option in options {
                                        ui.selectable_value(value, (*option).to_owned(), *option);
                                    }
                                })
                                .response
                                .labelled_by(label.id);
                        }
                        InputKind::Check => {
                            let mut on = value == "true";
                            if ui.checkbox(&mut on, "").labelled_by(label.id).changed() {
                                *value = on.to_string();
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        ui.separator();
        let path = self.request.path(&self.layout);
        let exists = self.root.as_ref().is_some_and(|r| r.join(&path).exists());
        if exists {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("{} (있음, 덮어씀)", path.display()),
            );
            ui.checkbox(&mut self.overwrite, "이미 있는 파일 덮어쓰기");
        } else {
            ui.monospace(path.display().to_string());
        }
    }
}
