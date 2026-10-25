//! 새 egov 프로젝트 창(도구 > 새 egov 프로젝트). 공식 확장의 프로젝트 템플릿(egov 5)을 고르고
//! 이름·groupId·artifactId·버전·위치를 넣으면 템플릿을 받거나(GitHub) 가져와서(vsix 등) 만든다.
//! 받기·풀기는 별도 스레드에서 돌리고 프레임마다 결과를 거둔다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use eframe::egui;
use ferrite_codegen::scaffold::{self, RELEASE, Request, TEMPLATES, Template};
use ferrite_lsp::install;

/// 템플릿 파일을 어디서 얻는가.
enum Source {
    /// 받아 둔 것만 쓴다.
    Cache,
    /// 없거나 맞지 않는 파일을 GitHub에서 받는다.
    Download,
    /// 다른 PC에서 가져온 파일(공식 확장 vsix, 또는 템플릿 zip·pom 틀).
    Files(Vec<PathBuf>),
}

enum Event {
    Progress(u64, u64),
    Unpacking,
    Done(Result<PathBuf, String>),
}

struct Job {
    rx: Receiver<Event>,
    progress: Option<(u64, u64)>,
    unpacking: bool,
}

pub struct NewProjectWindow {
    open: bool,
    /// 받은 템플릿을 두는 곳.
    cache: PathBuf,
    /// `TEMPLATES`의 번호.
    template: usize,
    request: Request,
    /// artifactId를 직접 고쳤으면 이름을 바꿔도 따라 바꾸지 않는다.
    artifact_edited: bool,
    /// 받아 둔 템플릿이 있는가(크기만 본다). 고를 때와 작업이 끝날 때 다시 구한다.
    cached: bool,
    job: Option<Job>,
    /// (오류인지, 내용)
    message: Option<(bool, String)>,
    /// 마지막으로 만든 프로젝트([열기]).
    created: Option<PathBuf>,
}

impl NewProjectWindow {
    pub fn new(cache: PathBuf) -> Self {
        Self {
            open: false,
            cache,
            template: 0,
            request: Request::new(&TEMPLATES[0], PathBuf::new()),
            artifact_edited: false,
            cached: false,
            job: None,
            message: None,
            created: None,
        }
    }

    /// 창을 연다. 처음이면 지금 프로젝트 옆(없으면 홈 폴더)을 만들 위치로 한다.
    pub fn show(&mut self, project: Option<&Path>) {
        self.open = true;
        if self.request.output.as_os_str().is_empty() {
            self.request.output = project
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .or_else(dirs::home_dir)
                .unwrap_or_default();
        }
        self.cached = self.current().cached(&self.cache);
    }

    fn current(&self) -> &'static Template {
        &TEMPLATES[self.template]
    }

    fn select(&mut self, index: usize) {
        let old = self.current();
        self.template = index;
        let new = self.current();
        if self.request.name == old.project_name {
            self.request.name = new.project_name.to_owned();
        }
        if !self.artifact_edited {
            self.request.artifact_id = artifact_for(&self.request.name);
        }
        self.cached = new.cached(&self.cache);
        self.message = None;
    }

    fn start(&mut self, source: Source, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let template = self.current();
        let request = self.request.clone();
        let cache = self.cache.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let send = |event| {
                let _ = tx.send(event);
                ctx.request_repaint();
            };
            let result = run(template, &request, &cache, source, &send);
            send(Event::Done(result));
        });
        self.job = Some(Job {
            rx,
            progress: None,
            unpacking: false,
        });
        self.message = None;
        self.created = None;
    }

    fn poll(&mut self) {
        let Some(job) = &mut self.job else {
            return;
        };
        let mut done = None;
        for event in job.rx.try_iter() {
            match event {
                Event::Progress(received, total) => job.progress = Some((received, total)),
                Event::Unpacking => job.unpacking = true,
                Event::Done(result) => done = Some(result),
            }
        }
        if let Some(result) = done {
            self.job = None;
            self.cached = self.current().cached(&self.cache);
            match result {
                Ok(path) => {
                    self.message = Some((false, format!("만들었습니다: {}", path.display())));
                    self.created = Some(path);
                }
                Err(e) => self.message = Some((true, e)),
            }
        }
    }

    /// 창을 그린다. [열기]를 누르면 그 프로젝트 폴더.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        if !self.open {
            return None;
        }
        self.poll();
        let mut opened = None;
        let mut open = self.open;
        egui::Window::new("새 egov 프로젝트")
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                self.form_ui(ui);
                ui.separator();
                self.actions_ui(ui, ctx);
                if let Some((error, text)) = &self.message {
                    let color = if *error {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.colored_label(color, text);
                }
                if let Some(path) = &self.created
                    && ui.button("열기").clicked()
                {
                    opened = Some(path.clone());
                }
            });
        self.open = open;
        if opened.is_some() {
            self.open = false;
        }
        opened
    }

    fn form_ui(&mut self, ui: &mut egui::Ui) {
        let busy = self.job.is_some();
        let template = self.current();
        egui::Grid::new("new_project")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                let label = ui.label("템플릿");
                let mut chosen = self.template;
                egui::ComboBox::from_id_salt("new_project_template")
                    .width(360.0)
                    .selected_text(title(template))
                    .show_ui(ui, |ui| {
                        for (i, t) in TEMPLATES.iter().enumerate() {
                            ui.selectable_value(&mut chosen, i, title(t));
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if chosen != self.template && !busy {
                    self.select(chosen);
                }
                ui.end_row();

                ui.label("");
                ui.weak(template.description);
                ui.end_row();

                let label = ui.label("프로젝트 이름");
                let name = ui
                    .text_edit_singleline(&mut self.request.name)
                    .labelled_by(label.id);
                if name.changed() && !self.artifact_edited {
                    self.request.artifact_id = artifact_for(&self.request.name);
                }
                ui.end_row();

                if template.pom.is_some() {
                    let label = ui.label("groupId");
                    ui.text_edit_singleline(&mut self.request.group_id)
                        .labelled_by(label.id);
                    ui.end_row();

                    let label = ui.label("artifactId");
                    if ui
                        .text_edit_singleline(&mut self.request.artifact_id)
                        .labelled_by(label.id)
                        .changed()
                    {
                        self.artifact_edited = true;
                    }
                    ui.end_row();

                    let label = ui.label("버전");
                    ui.text_edit_singleline(&mut self.request.version)
                        .labelled_by(label.id);
                    ui.end_row();
                }

                let label = ui.label("만들 위치");
                ui.horizontal(|ui| {
                    let mut output = self.request.output.to_string_lossy().into_owned();
                    if ui
                        .add(egui::TextEdit::singleline(&mut output).desired_width(300.0))
                        .labelled_by(label.id)
                        .changed()
                    {
                        self.request.output = PathBuf::from(output);
                    }
                    if ui.button("찾아보기…").clicked()
                        && let Some(dir) = rfd::FileDialog::new()
                            .set_directory(&self.request.output)
                            .pick_folder()
                    {
                        self.request.output = dir;
                    }
                });
                ui.end_row();
            });
        if !self.request.output.as_os_str().is_empty() && !self.request.name.is_empty() {
            let target = self.request.target();
            if target.exists() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("{} (이미 있음)", target.display()),
                );
            } else {
                ui.monospace(target.display().to_string());
            }
        }
        for problem in self.request.problems(template) {
            ui.colored_label(ui.visuals().error_fg_color, problem);
        }
        ui.weak(format!(
            "공식 확장 {RELEASE}의 egov 5 템플릿입니다. 빌드에는 Java 17 이상과 Maven이 필요합니다."
        ));
    }

    fn actions_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(job) = &self.job {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(match job.progress {
                    _ if job.unpacking => "프로젝트를 만드는 중…".to_owned(),
                    Some((received, total)) if total > 0 => {
                        format!("템플릿 받는 중 {}%", received * 100 / total)
                    }
                    _ => "템플릿 준비 중…".to_owned(),
                });
            });
            return;
        }
        let template = self.current();
        let ready = self.request.problems(template).is_empty();
        ui.horizontal(|ui| {
            if self.cached {
                if ui.add_enabled(ready, egui::Button::new("만들기")).clicked() {
                    self.start(Source::Cache, ctx);
                }
            } else if ui
                .add_enabled(
                    ready,
                    egui::Button::new(format!("내려받아 만들기 ({})", megabytes(template.size()))),
                )
                .on_hover_text("GitHub eGovFramework/egovframe-vscode-initializr에서 받습니다")
                .clicked()
            {
                self.start(Source::Download, ctx);
            }
            // 인터넷이 막힌 PC: 다른 PC에서 받은 공식 vsix나 템플릿 파일로 만든다.
            let urls: Vec<String> = template.files().into_iter().map(|(_, url)| url).collect();
            let urls = urls.join("\n");
            if ui.button("주소 복사").on_hover_text(&urls).clicked() {
                ctx.copy_text(urls);
            }
            if ui
                .add_enabled(ready, egui::Button::new("파일에서…"))
                .on_hover_text(format!(
                    "공식 확장 {RELEASE}의 vsix, 또는 위 주소의 파일들로 만듭니다"
                ))
                .clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .set_title("egov 템플릿 파일(vsix, zip, pom)")
                    .pick_files()
            {
                self.start(Source::Files(files), ctx);
            }
        });
        let note = if self.cached {
            "받아 둔 템플릿을 씁니다".to_owned()
        } else {
            format!(
                "처음 한 번 템플릿 {}를 받아 {}에 둡니다",
                megabytes(template.size()),
                self.cache.display()
            )
        };
        ui.weak(note);
    }
}

/// 템플릿 파일을 마련하고 프로젝트를 만든다(작업 스레드).
fn run(
    template: &'static Template,
    request: &Request,
    cache: &Path,
    source: Source,
    send: &dyn Fn(Event),
) -> Result<PathBuf, String> {
    match source {
        Source::Cache => {}
        Source::Files(files) => scaffold::import(template, &files, cache)?,
        Source::Download => {
            let total = template.size();
            let mut done = 0;
            for (pinned, url) in template.files() {
                let dest = cache.join(pinned.file);
                if scaffold::verify(&dest, pinned).is_err() {
                    install::download(
                        &url,
                        pinned.size,
                        pinned.sha256,
                        &dest,
                        &mut |received, _| send(Event::Progress(done + received, total)),
                    )
                    .map_err(|e| format!("{}: {e}", pinned.file))?;
                }
                done += pinned.size;
            }
        }
    }
    send(Event::Unpacking);
    let result = scaffold::create(template, request, cache);
    if result.is_err() {
        // 손상된 캐시 파일은 지워, 다음에 [내려받아 만들기]가 보이게 한다.
        for (pinned, _) in template.files() {
            let path = cache.join(pinned.file);
            if path.exists() && scaffold::verify(&path, pinned).is_err() {
                let _ = fs::remove_file(path);
            }
        }
    }
    result
}

fn title(template: &Template) -> String {
    format!("{} > {}", template.category, template.name)
}

/// 프로젝트 이름에서 artifactId 기본값(소문자, `.`·`_`는 `-`).
fn artifact_for(name: &str) -> String {
    name.to_ascii_lowercase().replace(['.', '_'], "-")
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}MB", bytes as f64 / 1_000_000.0)
}
