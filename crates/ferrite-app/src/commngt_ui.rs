//! 공통컴포넌트 추가 창(도구 > 공통컴포넌트 추가). 공식 개발환경에서 가져온 목록에서 컴포넌트를
//! 고르면, 의존 컴포넌트까지 열린 프로젝트에 넣는다. 먼저 [미리 보기]로 무엇이 바뀌는지 보여 주고
//! [추가]로 쓴다. 가져오기(큰 개발환경 zip)·미리 보기·추가는 별도 스레드에서 돌린다.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use eframe::egui;
use ferrite_codegen::commngt::{self, Catalog, Outcome, Plan, State};
use ferrite_codegen::crud::{Db, Rte};
use ferrite_codegen::project;
use ferrite_core::text_file::Encoding;

/// 미리 보기에서 이름을 보여 줄 다른 파일 수.
const SHOWN_CONFLICTS: usize = 12;

enum Done {
    Imported(Result<Catalog, String>),
    Planned(Result<Plan, String>),
    Applied(Result<Outcome, String>),
}

pub struct CommngtWindow {
    open: bool,
    /// 가져온 목록을 두는 곳(`commngt-<버전>/`).
    root: PathBuf,
    project: Option<PathBuf>,
    rte: Rte,
    db: Db,
    encoding: Encoding,
    catalogs: Vec<Catalog>,
    /// `catalogs`의 번호.
    catalog: usize,
    /// 고른 패키지.
    selected: HashSet<String>,
    /// 프로젝트에 이미 있는 패키지.
    installed: HashSet<String>,
    overwrite: bool,
    plan: Option<Plan>,
    outcome: Option<Outcome>,
    job: Option<Receiver<Done>>,
    error: Option<String>,
}

impl CommngtWindow {
    pub fn new(root: PathBuf) -> Self {
        Self {
            open: false,
            root,
            project: None,
            rte: Rte::V4,
            db: Db::Oracle,
            encoding: Encoding::Utf8,
            catalogs: Vec::new(),
            catalog: 0,
            selected: HashSet::new(),
            installed: HashSet::new(),
            overwrite: false,
            plan: None,
            outcome: None,
            job: None,
            error: None,
        }
    }

    /// 창을 연다. 프로젝트가 바뀌었으면 실행환경·DB·인코딩을 다시 찾는다.
    pub fn show(&mut self, project: Option<&Path>) {
        self.open = true;
        let project = project.map(Path::to_path_buf);
        if project != self.project {
            self.project = project;
            if let Some(root) = &self.project {
                let found = project::defaults(root);
                self.rte = found.rte;
                self.db = found.db;
                self.encoding = found.encoding;
            }
            self.selected.clear();
            self.reset();
        }
        self.catalogs = commngt::imported(&self.root);
        // 프로젝트 실행환경과 주 번호가 같은 목록을 먼저 고른다(3.x 프로젝트 → 3.10).
        let major = match self.rte {
            Rte::V3 => 3,
            Rte::V4 => 4,
        };
        self.catalog = self
            .catalogs
            .iter()
            .position(|c| c.major() == Some(major))
            .unwrap_or(0);
        self.refresh_installed();
    }

    fn current(&self) -> Option<&Catalog> {
        self.catalogs.get(self.catalog)
    }

    fn refresh_installed(&mut self) {
        self.installed = match (&self.project, self.catalogs.get(self.catalog)) {
            (Some(root), Some(catalog)) => catalog
                .categories
                .iter()
                .flat_map(|c| &c.components)
                .filter(|c| commngt::installed(root, &c.package))
                .map(|c| c.package.clone())
                .collect(),
            _ => HashSet::new(),
        };
    }

    /// 고른 것이 바뀌면 미리 보기는 버린다.
    fn reset(&mut self) {
        self.plan = None;
        self.outcome = None;
        self.error = None;
    }

    fn run(&mut self, ctx: &egui::Context, work: impl FnOnce() -> Done + Send + 'static) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        self.job = Some(rx);
        self.error = None;
    }

    fn poll(&mut self) {
        let Some(done) = self.job.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.job = None;
        match done {
            Done::Imported(Ok(catalog)) => {
                let version = catalog.version.clone();
                self.catalogs = commngt::imported(&self.root);
                self.catalog = self
                    .catalogs
                    .iter()
                    .position(|c| c.version == version)
                    .unwrap_or(0);
                self.selected.clear();
                self.reset();
                self.refresh_installed();
            }
            Done::Planned(Ok(plan)) => self.plan = Some(plan),
            Done::Applied(Ok(outcome)) => {
                self.plan = None;
                self.outcome = Some(outcome);
                self.selected.clear();
                self.refresh_installed();
            }
            Done::Imported(Err(e)) | Done::Planned(Err(e)) | Done::Applied(Err(e)) => {
                self.error = Some(e);
            }
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        self.poll();
        let mut open = self.open;
        egui::Window::new("공통컴포넌트 추가")
            .open(&mut open)
            .default_width(640.0)
            .default_height(640.0)
            .show(ctx, |ui| {
                self.source_ui(ui, ctx);
                if self.current().is_some() {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(ui.available_height().max(260.0) - 200.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| self.components_ui(ui));
                    ui.separator();
                    self.actions_ui(ui, ctx);
                }
                if self.job.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("처리 중…");
                    });
                }
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if let Some(outcome) = &self.outcome {
                    outcome_ui(ui, outcome);
                }
            });
        self.open = open;
    }

    fn source_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.job.is_some();
        ui.horizontal(|ui| {
            if !self.catalogs.is_empty() {
                let label = ui.label("목록");
                let mut chosen = self.catalog;
                egui::ComboBox::from_id_salt("commngt_catalog")
                    .selected_text(format!(
                        "공통컴포넌트 {}",
                        self.catalogs[self.catalog].version
                    ))
                    .show_ui(ui, |ui| {
                        for (i, catalog) in self.catalogs.iter().enumerate() {
                            ui.selectable_value(
                                &mut chosen,
                                i,
                                format!("공통컴포넌트 {}", catalog.version),
                            );
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if chosen != self.catalog && !busy {
                    self.catalog = chosen;
                    self.selected.clear();
                    self.reset();
                    self.refresh_installed();
                }
            }
            let pick = |dialog: rfd::FileDialog| {
                dialog.set_title("공식 개발환경(eGovFrameDev) 또는 공통컴포넌트 플러그인")
            };
            if ui
                .add_enabled(!busy, egui::Button::new("개발환경 폴더에서…"))
                .on_hover_text("eGovFrameDev 폴더(eclipse/plugins를 담은 곳)를 고릅니다")
                .clicked()
                && let Some(source) = pick(rfd::FileDialog::new()).pick_folder()
            {
                self.import(source, ctx);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("파일에서…"))
                .on_hover_text(
                    "플러그인 jar, 플러그인 업데이트 사이트 zip, 개발환경 zip을 고릅니다",
                )
                .clicked()
                && let Some(source) = pick(rfd::FileDialog::new())
                    .add_filter("jar·zip", &["jar", "zip"])
                    .pick_file()
            {
                self.import(source, ctx);
            }
        });
        if self.catalogs.is_empty() {
            ui.weak(
                "공식 개발환경(Eclipse)의 공통컴포넌트 플러그인(egovframework.dev.imp.commngt)에서 목록과 \
                 컴포넌트를 가져옵니다. 프로젝트와 같은 버전(3.x·4.x)의 개발환경을 고르세요.",
            );
        }
        let Some(catalog) = self.current() else {
            return;
        };
        if self.project.is_none() {
            ui.colored_label(ui.visuals().warn_fg_color, "먼저 프로젝트 폴더를 여세요");
        }
        let project_major = match self.rte {
            Rte::V3 => 3,
            Rte::V4 => 4,
        };
        if self.project.is_some() && catalog.major() != Some(project_major) {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "프로젝트는 {}인데 공통컴포넌트는 {}입니다(실행환경 패키지가 달라 컴파일되지 않습니다)",
                    self.rte.label(),
                    catalog.version
                ),
            );
        }
        if self.encoding == Encoding::Cp949 {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "프로젝트 인코딩이 MS949입니다. 공통컴포넌트 파일은 UTF-8이라 그대로 넣습니다",
            );
        }
    }

    fn import(&mut self, source: PathBuf, ctx: &egui::Context) {
        let root = self.root.clone();
        self.reset();
        self.run(ctx, move || Done::Imported(commngt::import(&source, &root)));
    }

    fn components_ui(&mut self, ui: &mut egui::Ui) {
        let Some(catalog) = self.catalogs.get(self.catalog) else {
            return;
        };
        let selected: Vec<String> = self.selected.iter().cloned().collect();
        // 고른 것이 기대어 함께 들어가는 패키지: 체크된 채로 막아 둔다.
        let implied: HashSet<String> = catalog
            .closure(&selected)
            .into_iter()
            .filter(|p| !self.selected.contains(p))
            .collect();
        let mut changed = false;
        for (index, category) in catalog.categories.iter().enumerate() {
            let chosen = category
                .components
                .iter()
                .filter(|c| self.selected.contains(&c.package))
                .count();
            let title = if chosen > 0 {
                format!("{} ({chosen})", category.name)
            } else {
                category.name.clone()
            };
            egui::CollapsingHeader::new(title)
                .id_salt(("commngt_category", index))
                .show(ui, |ui| {
                    ui.weak(&category.description);
                    for component in &category.components {
                        ui.horizontal(|ui| {
                            let package = &component.package;
                            let mut on =
                                self.selected.contains(package) || implied.contains(package);
                            let hover = format!(
                                "{}\n{}\n테이블: {}",
                                component.description,
                                package,
                                component.tables.join(", ")
                            );
                            let response = ui
                                .add_enabled(
                                    !implied.contains(package),
                                    egui::Checkbox::new(&mut on, &component.name),
                                )
                                .on_hover_text(&hover)
                                .on_disabled_hover_text(format!(
                                    "고른 컴포넌트가 기대므로 함께 넣습니다\n{hover}"
                                ));
                            if response.changed() {
                                if on {
                                    self.selected.insert(package.clone());
                                } else {
                                    self.selected.remove(package);
                                }
                                changed = true;
                            }
                            if self.installed.contains(package) {
                                ui.weak("설치됨");
                            }
                        });
                    }
                });
        }
        if changed {
            self.reset();
        }
    }

    fn actions_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(catalog) = self.current().cloned() else {
            return;
        };
        let selected: Vec<String> = self.selected.iter().cloned().collect();
        let packages = catalog.closure(&selected);
        ui.label(format!(
            "고른 컴포넌트 {}개, 의존을 더해 패키지 {}개",
            selected.len(),
            packages.len()
        ))
        .on_hover_text(packages.join("\n"));
        let busy = self.job.is_some();
        let ready = !busy && !selected.is_empty() && self.project.is_some();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(ready, egui::Button::new("미리 보기"))
                .clicked()
                && let Some(project) = self.project.clone()
            {
                self.outcome = None;
                self.run(ctx, move || {
                    Done::Planned(commngt::plan(&catalog, &selected, &project))
                });
            }
            let can_apply = ready && self.plan.is_some();
            if ui
                .add_enabled(can_apply, egui::Button::new("추가"))
                .on_disabled_hover_text("먼저 [미리 보기]로 바뀔 것을 확인하세요")
                .clicked()
                && let (Some(plan), Some(project)) = (self.plan.clone(), self.project.clone())
            {
                let (overwrite, db) = (self.overwrite, self.db);
                self.run(ctx, move || {
                    Done::Applied(commngt::apply(&plan, &project, overwrite, db))
                });
            }
        });
        ui.checkbox(
            &mut self.overwrite,
            "내용이 다른 기존 파일 덮어쓰기(.bak으로 백업, pom.xml은 빼고)",
        );
        if let Some(plan) = &self.plan {
            plan_ui(ui, plan, self.overwrite);
        }
    }
}

fn plan_ui(ui: &mut egui::Ui, plan: &Plan, overwrite: bool) {
    ui.label(format!(
        "새 파일 {}개, 같은 파일 {}개, 다른 파일 {}개",
        plan.count(State::New),
        plan.count(State::Same),
        plan.count(State::Differs)
    ));
    let differs: Vec<&str> = plan
        .entries
        .iter()
        .filter(|e| e.state == State::Differs)
        .map(|e| e.path.as_str())
        .collect();
    if !differs.is_empty() {
        ui.weak(if overwrite {
            "다른 파일은 백업하고 바꿉니다(pom.xml은 그대로 두고 빠진 의존성을 알립니다):"
        } else {
            "다른 파일은 건너뜁니다:"
        });
        for path in differs.iter().take(SHOWN_CONFLICTS) {
            ui.monospace(*path);
        }
        if differs.len() > SHOWN_CONFLICTS {
            ui.weak(format!("외 {}개", differs.len() - SHOWN_CONFLICTS));
        }
    }
    if !plan.missing.is_empty() {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "목록에 zip이 없어 넣지 못합니다: {}",
                plan.missing.join(", ")
            ),
        );
    }
}

fn outcome_ui(ui: &mut egui::Ui, outcome: &Outcome) {
    ui.separator();
    ui.label(format!(
        "넣었습니다: 새 파일 {}개, 덮어씀 {}개, 건너뜀 {}개(같은 파일 {}개)",
        outcome.created,
        outcome.overwritten.len(),
        outcome.skipped.len(),
        outcome.same
    ));
    if !outcome.missing_dependencies.is_empty() {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "pom.xml에 없는 의존성 {}개(공통컴포넌트 pom.xml 기준, 직접 더하세요):",
                outcome.missing_dependencies.len()
            ),
        );
        ui.monospace(outcome.missing_dependencies.join("\n"));
    }
    for (path, keys) in &outcome.missing_keys {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!("{path}에 없는 키: {}", keys.join(", ")),
        );
    }
    if !outcome.scripts.is_empty() {
        ui.label("프로젝트 DB에 이 순서로 돌리세요(테이블 생성, 기초 자료):");
        ui.monospace(outcome.scripts.join("\n"));
    }
}
