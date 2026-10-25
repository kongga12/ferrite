//! egov CRUD 코드 생성 창(도구 > egov CRUD 코드 생성). 표 정의(DDL을 붙여 넣거나 필드를 직접 입력)로
//! VO·Mapper·Service·Controller·JSP를 만든다. 기본값(실행환경 버전, DB, 인코딩, 폴더)은 프로젝트에서 찾는다.

use std::path::{Path, PathBuf};

use eframe::egui;
use ferrite_codegen::crud::{self, Db, Field, JUnit, Layout, Part, Rte, Spec};
use ferrite_codegen::{ddl, project};
use ferrite_core::text_file::{Encoding, LineEnding};

/// 화면에서 고르는 Java 타입.
const JAVA_TYPES: [&str; 12] = [
    "java.lang.String",
    "java.lang.Integer",
    "java.lang.Long",
    "java.math.BigDecimal",
    "java.lang.Double",
    "java.lang.Boolean",
    "java.sql.Date",
    "java.sql.Timestamp",
    "java.util.Date",
    "java.lang.Short",
    "java.lang.Float",
    "byte[]",
];

pub struct CodegenWindow {
    open: bool,
    root: Option<PathBuf>,
    ddl: String,
    table: String,
    table_comment: String,
    class_name: String,
    package: String,
    /// 패키지를 직접 고쳤으면 클래스 이름을 바꿔도 패키지를 다시 정하지 않는다.
    package_edited: bool,
    base_package: String,
    author: String,
    date: String,
    rte: Rte,
    db: Db,
    junit: JUnit,
    encoding: Encoding,
    line_ending: LineEnding,
    layout: Layout,
    fields: Vec<Field>,
    parts: Vec<(Part, bool)>,
    overwrite: bool,
    /// 프로젝트를 훑으며 알게 된 주의 사항.
    notes: Vec<String>,
    /// (오류인지, 내용)
    message: Option<(bool, String)>,
}

impl Default for CodegenWindow {
    fn default() -> Self {
        Self {
            open: false,
            root: None,
            ddl: String::new(),
            table: String::new(),
            table_comment: String::new(),
            class_name: String::new(),
            package: String::new(),
            package_edited: false,
            base_package: "egovframework.example".into(),
            author: String::new(),
            date: today(),
            rte: Rte::V4,
            db: Db::Oracle,
            junit: JUnit::V5,
            encoding: Encoding::Utf8,
            line_ending: LineEnding::Lf,
            layout: Layout::default(),
            fields: Vec::new(),
            parts: Part::ALL.iter().map(|p| (*p, true)).collect(),
            overwrite: false,
            notes: Vec::new(),
            message: None,
        }
    }
}

impl CodegenWindow {
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
                self.junit = found.junit;
                self.encoding = found.encoding;
                self.line_ending = found.line_ending;
                self.layout = found.layout;
                self.base_package = found.base_package;
                self.notes = found.notes;
                self.update_package();
            }
        }
    }

    fn update_package(&mut self) {
        if !self.package_edited {
            let module = self.class_name.to_lowercase();
            self.package = if module.is_empty() {
                self.base_package.clone()
            } else {
                format!("{}.{module}", self.base_package)
            };
        }
    }

    fn read_ddl(&mut self) {
        match ddl::parse(&self.ddl) {
            Ok(table) => {
                let spec = Spec::from_table(&table, &self.package, self.rte, self.db, self.junit);
                self.table = spec.table;
                self.table_comment = spec.table_comment.unwrap_or_default();
                self.class_name = spec.class_name;
                self.fields = spec.fields;
                self.update_package();
                self.message = Some((false, format!("열 {}개를 읽었습니다", self.fields.len())));
            }
            Err(e) => self.message = Some((true, format!("DDL을 읽지 못했습니다: {e}"))),
        }
    }

    fn spec(&self) -> Spec {
        Spec {
            table: self.table.trim().to_owned(),
            table_comment: Some(self.table_comment.trim().to_owned()).filter(|c| !c.is_empty()),
            fields: self.fields.clone(),
            package: self.package.trim().to_owned(),
            class_name: self.class_name.trim().to_owned(),
            author: self.author.trim().to_owned(),
            date: self.date.trim().to_owned(),
            rte: self.rte,
            db: self.db,
            junit: self.junit,
            encoding: self.encoding,
        }
    }

    fn selected_parts(&self) -> Vec<Part> {
        self.parts
            .iter()
            .filter(|(_, on)| *on)
            .map(|(p, _)| *p)
            .collect()
    }

    /// 파일을 만든다. 성공하면 쓴 파일들.
    fn generate(&mut self) -> Option<Vec<PathBuf>> {
        let Some(root) = self.root.clone() else {
            self.message = Some((true, "먼저 프로젝트 폴더를 여세요".into()));
            return None;
        };
        let overrides = root.join(".ferrite/templates/code");
        let result = crud::generate(
            &self.spec(),
            &self.layout,
            &self.selected_parts(),
            overrides.is_dir().then_some(overrides.as_path()),
        )
        .and_then(|files| {
            project::write(
                &root,
                &files,
                self.encoding,
                self.line_ending,
                self.overwrite,
            )
        });
        match result {
            Ok(written) => {
                self.message = Some((false, format!("파일 {}개를 만들었습니다", written.len())));
                Some(written)
            }
            Err(e) => {
                self.message = Some((true, e));
                None
            }
        }
    }

    /// 창을 그린다. 파일을 만들었으면 그 경로들.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Vec<PathBuf>> {
        if !self.open {
            return None;
        }
        let mut written = None;
        let mut open = self.open;
        egui::Window::new("egov CRUD 코드 생성")
            .open(&mut open)
            .default_width(860.0)
            .default_height(640.0)
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
        let intro = ui.label(
            "표 정의(CREATE TABLE)를 붙여 넣고 [DDL 읽기]를 누르세요. 필드를 직접 추가해도 됩니다.",
        );
        ui.add(
            egui::TextEdit::multiline(&mut self.ddl)
                .code_editor()
                .desired_rows(6)
                .desired_width(f32::INFINITY)
                .hint_text("CREATE TABLE TB_BOARD ( BOARD_ID NUMBER(10) PRIMARY KEY, TITLE VARCHAR2(200) NOT NULL, … );"),
        )
        .labelled_by(intro.id);
        ui.horizontal(|ui| {
            if ui.button("DDL 읽기").clicked() {
                self.read_ddl();
            }
            if ui.button("필드 추가").clicked() {
                self.fields.push(blank_field(self.fields.len()));
            }
        });
        ui.separator();
        self.settings_ui(ui);
        ui.separator();
        self.fields_ui(ui);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            for (part, on) in &mut self.parts {
                ui.checkbox(on, part.label());
            }
        });
        ui.separator();
        self.preview_ui(ui);
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("codegen_settings")
            .num_columns(4)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                labeled(ui, "표 이름", &mut self.table);
                labeled(ui, "표 설명", &mut self.table_comment);
                ui.end_row();
                if labeled(ui, "클래스 이름", &mut self.class_name).changed() {
                    self.update_package();
                }
                if labeled(ui, "패키지", &mut self.package).changed() {
                    self.package_edited = true;
                }
                ui.end_row();
                labeled(ui, "작성자", &mut self.author);
                labeled(ui, "작성일", &mut self.date);
                ui.end_row();
                let label = ui.label("실행환경");
                egui::ComboBox::from_id_salt("codegen_rte")
                    .selected_text(self.rte.label())
                    .show_ui(ui, |ui| {
                        for rte in [Rte::V3, Rte::V4] {
                            ui.selectable_value(&mut self.rte, rte, rte.label());
                        }
                    })
                    .response
                    .labelled_by(label.id);
                let label = ui.label("DB");
                egui::ComboBox::from_id_salt("codegen_db")
                    .selected_text(self.db.label())
                    .show_ui(ui, |ui| {
                        for db in Db::ALL {
                            ui.selectable_value(&mut self.db, db, db.label());
                        }
                    })
                    .response
                    .labelled_by(label.id);
                ui.end_row();
                let label = ui.label("JUnit");
                egui::ComboBox::from_id_salt("codegen_junit")
                    .selected_text(match self.junit {
                        JUnit::V4 => "JUnit 4",
                        JUnit::V5 => "JUnit 5",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.junit, JUnit::V4, "JUnit 4");
                        ui.selectable_value(&mut self.junit, JUnit::V5, "JUnit 5");
                    })
                    .response
                    .labelled_by(label.id);
                let label = ui.label("인코딩·줄바꿈");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("codegen_encoding")
                        .selected_text(self.encoding.label())
                        .show_ui(ui, |ui| {
                            for encoding in [Encoding::Utf8, Encoding::Cp949] {
                                ui.selectable_value(&mut self.encoding, encoding, encoding.label());
                            }
                        })
                        .response
                        .labelled_by(label.id);
                    egui::ComboBox::from_id_salt("codegen_line_ending")
                        .selected_text(self.line_ending.label())
                        .show_ui(ui, |ui| {
                            for ending in LineEnding::ALL {
                                ui.selectable_value(&mut self.line_ending, ending, ending.label());
                            }
                        })
                        .response
                        .labelled_by(label.id);
                });
                ui.end_row();
            });
    }

    fn fields_ui(&mut self, ui: &mut egui::Ui) {
        if self.fields.is_empty() {
            ui.weak("필드가 없습니다.");
            return;
        }
        let mut remove = None;
        egui::Grid::new("codegen_fields")
            .num_columns(8)
            .striped(true)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                // 칸마다 열 제목으로 이름을 붙인다(화면 읽기 프로그램용).
                let headers: Vec<egui::Id> =
                    ["열", "필드", "Java 타입", "키", "필수", "길이", "설명", ""]
                        .iter()
                        .map(|h| ui.strong(*h).id)
                        .collect();
                ui.end_row();
                for (i, field) in self.fields.iter_mut().enumerate() {
                    ui.add(egui::TextEdit::singleline(&mut field.column).desired_width(120.0))
                        .labelled_by(headers[0]);
                    ui.add(egui::TextEdit::singleline(&mut field.name).desired_width(110.0))
                        .labelled_by(headers[1]);
                    egui::ComboBox::from_id_salt(("codegen_type", i))
                        .width(150.0)
                        .selected_text(field.java_type.as_str())
                        .show_ui(ui, |ui| {
                            for ty in JAVA_TYPES {
                                ui.selectable_value(&mut field.java_type, ty.to_owned(), ty);
                            }
                        })
                        .response
                        .labelled_by(headers[2]);
                    ui.checkbox(&mut field.primary_key, "")
                        .labelled_by(headers[3]);
                    ui.checkbox(&mut field.not_null, "").labelled_by(headers[4]);
                    let mut length = field.max_length.map(|n| n.to_string()).unwrap_or_default();
                    if ui
                        .add(egui::TextEdit::singleline(&mut length).desired_width(44.0))
                        .labelled_by(headers[5])
                        .changed()
                    {
                        field.max_length = length.trim().parse().ok();
                    }
                    let mut comment = field.comment.clone().unwrap_or_default();
                    if ui
                        .add(egui::TextEdit::singleline(&mut comment).desired_width(160.0))
                        .labelled_by(headers[6])
                        .changed()
                    {
                        field.comment = Some(comment).filter(|c| !c.is_empty());
                    }
                    if ui
                        .small_button("삭제")
                        .on_hover_text("이 필드를 뺀다")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            self.fields.remove(i);
        }
    }

    /// 만들 파일 목록. 이미 있는 파일은 덮어쓰기를 허락해야 만든다.
    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        let spec = self.spec();
        let mut existing = 0;
        for part in self.selected_parts() {
            let path = part.path(&spec, &self.layout);
            let exists = self.root.as_ref().is_some_and(|r| r.join(&path).exists());
            if exists {
                existing += 1;
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("{} (있음, 덮어씀)", path.display()),
                );
            } else {
                ui.monospace(path.display().to_string());
            }
        }
        if existing > 0 {
            ui.checkbox(&mut self.overwrite, "이미 있는 파일 덮어쓰기");
        }
        for note in &self.notes {
            ui.colored_label(ui.visuals().warn_fg_color, note);
        }
    }
}

/// 이름표와, 그 이름표로 이름 붙인 입력란(화면 읽기 프로그램이 읽을 수 있게).
fn labeled(ui: &mut egui::Ui, label: &str, value: &mut String) -> egui::Response {
    let label = ui.label(label);
    ui.text_edit_singleline(value).labelled_by(label.id)
}

fn blank_field(index: usize) -> Field {
    Field {
        column: format!("COLUMN_{}", index + 1),
        name: format!("column{}", index + 1),
        java_type: "java.lang.String".into(),
        data_type: String::new(),
        primary_key: index == 0,
        not_null: index == 0,
        max_length: None,
        comment: None,
    }
}

/// 오늘 날짜(`2026-10-25`, UTC 기준). 화면에서 고칠 수 있다.
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    // 1970-01-01부터 지난 날 수 → 그레고리력 날짜(Howard Hinnant의 civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 오늘_날짜_모양() {
        let today = today();
        assert_eq!(today.len(), 10);
        assert!(today.starts_with("20"));
        assert_eq!(&today[4..5], "-");
    }
}
