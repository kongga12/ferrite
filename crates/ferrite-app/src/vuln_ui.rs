//! 문제 창의 "취약점": 프로젝트 의존성(Maven·Gradle·npm·PyPI·Cargo)을 공개 취약점 DB(OSV)와 견줘
//! 의존성 파일별로 보이고, 누르면 그 의존성을 적은 줄로 간다. 열린 파일에는 물결선으로도 보인다.
//! 온라인은 이름·버전만 api.osv.dev에 묻고, 오프라인은 받아 둔 생태계별 DB 묶음을 읽는다.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use ferrite_core::vuln::{self, Ecosystem, Finding, manifest, osv};
use ferrite_lsp::problem::{Problem, Severity};
use ropey::Rope;

/// 앱이 할 일.
pub enum VulnAction {
    /// 이 파일의 이 줄(0부터)로 간다.
    Go(PathBuf, usize),
}

struct Scan {
    dependencies: usize,
    findings: Vec<Finding>,
    notes: Vec<String>,
}

enum Msg {
    Step(String),
    Done(Result<Scan, String>),
}

enum DbMsg {
    Progress(u64, u64),
    Done(Result<(), String>),
}

struct Download {
    ecosystem: Ecosystem,
    progress: (u64, u64),
    rx: Receiver<DbMsg>,
}

pub struct VulnPanel {
    /// 오프라인 DB를 두는 곳(생태계마다 `<이름>/all.zip`). 테스트는 사용자 폴더를 쓰지 않는다.
    pub db_dir: Option<PathBuf>,
    online: bool,
    /// 검사한 프로젝트 루트(실제 경로).
    root: Option<PathBuf>,
    findings: Vec<Finding>,
    notes: Vec<String>,
    /// 결과가 바뀔 때마다 하나씩 는다(열린 문서의 물결선을 다시 만들 때).
    generation: u64,
    status: Option<(String, bool)>,
    scanning: Option<Receiver<Msg>>,
    download: Option<Download>,
}

impl VulnPanel {
    pub fn new(db_dir: Option<PathBuf>) -> Self {
        Self {
            db_dir,
            online: true,
            root: None,
            findings: Vec::new(),
            notes: Vec::new(),
            generation: 0,
            status: None,
            scanning: None,
            download: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    #[cfg(test)]
    pub fn set_online(&mut self, online: bool) {
        self.online = online;
    }

    #[cfg(test)]
    pub fn is_scanning(&self) -> bool {
        self.scanning.is_some()
    }

    fn database(&self, ecosystem: Ecosystem) -> Option<PathBuf> {
        Some(self.db_dir.as_ref()?.join(ecosystem.osv()).join("all.zip"))
    }

    /// 검사를 띄운다(빌드 도구·네트워크를 쓰므로 따로 돈다).
    pub fn start(&mut self, ctx: &egui::Context, root: &Path) {
        if self.scanning.is_some() {
            return;
        }
        let root = crate::app::real_path(root);
        let online = self.online;
        let databases: Vec<(Ecosystem, Option<PathBuf>)> = Ecosystem::ALL
            .into_iter()
            .map(|e| (e, self.database(e)))
            .collect();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let scanned = root.clone();
        thread::spawn(move || {
            let step = |text: String| {
                let _ = tx.send(Msg::Step(text));
                ctx.request_repaint();
            };
            let result = scan(&scanned, online, &databases, &step);
            let _ = tx.send(Msg::Done(result));
            ctx.request_repaint();
        });
        self.root = Some(root);
        self.scanning = Some(rx);
        self.status = Some(("의존성 파일을 찾는 중…".into(), false));
    }

    /// 검사·받기의 결과를 받는다. 매 프레임 부른다.
    pub fn poll(&mut self) {
        if let Some(rx) = &self.scanning {
            loop {
                match rx.try_recv() {
                    Ok(Msg::Step(text)) => self.status = Some((text, false)),
                    Ok(Msg::Done(result)) => {
                        self.finish(result);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.finish(Err("검사가 멈췄습니다".into()));
                        break;
                    }
                }
            }
        }
        if let Some(download) = &mut self.download {
            loop {
                match download.rx.try_recv() {
                    Ok(DbMsg::Progress(got, total)) => download.progress = (got, total),
                    Ok(DbMsg::Done(result)) => {
                        let name = download.ecosystem.osv();
                        self.status = Some(match result {
                            Ok(()) => (format!("{name} DB를 받았습니다"), false),
                            Err(e) => (format!("{name} DB: {e}"), true),
                        });
                        self.download = None;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.download = None;
                        break;
                    }
                }
            }
        }
    }

    fn finish(&mut self, result: Result<Scan, String>) {
        self.scanning = None;
        self.generation += 1;
        match result {
            Ok(scan) => {
                let count = |s| scan.findings.iter().filter(|f| f.severity == s).count();
                let mut text = format!(
                    "의존성 {}개 · 취약 {}건",
                    scan.dependencies,
                    scan.findings.len()
                );
                if !scan.findings.is_empty() {
                    let parts: Vec<String> = [
                        vuln::Severity::Critical,
                        vuln::Severity::High,
                        vuln::Severity::Moderate,
                        vuln::Severity::Low,
                        vuln::Severity::Unknown,
                    ]
                    .into_iter()
                    .filter(|&s| count(s) > 0)
                    .map(|s| format!("{} {}", s.label(), count(s)))
                    .collect();
                    text.push_str(&format!("({})", parts.join(" · ")));
                }
                text.push_str(if self.online {
                    " · 온라인"
                } else {
                    " · 오프라인 DB"
                });
                self.status = Some((text, false));
                self.findings = scan.findings;
                self.notes = scan.notes;
            }
            Err(e) => {
                self.status = Some((e, true));
                self.findings.clear();
                self.notes.clear();
            }
        }
    }

    fn start_download(&mut self, ctx: &egui::Context, ecosystem: Ecosystem) {
        let Some(dest) = self.database(ecosystem) else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let mut shown = Instant::now() - Duration::from_secs(1);
            let result = osv::download_database(ecosystem, &dest, &mut |got, total| {
                if shown.elapsed() >= Duration::from_millis(100) {
                    shown = Instant::now();
                    let _ = tx.send(DbMsg::Progress(got, total));
                    ctx.request_repaint();
                }
            });
            let _ = tx.send(DbMsg::Done(result));
            ctx.request_repaint();
        });
        self.download = Some(Download {
            ecosystem,
            progress: (0, 0),
            rx,
        });
    }

    /// 오프라인 DB 상태와 [받기]·[파일에서…].
    fn databases_ui(&mut self, ui: &mut egui::Ui) {
        let mut fetch = None;
        for ecosystem in Ecosystem::ALL {
            let path = self.database(ecosystem);
            ui.horizontal(|ui| {
                ui.label(ecosystem.osv());
                let state = match &self.download {
                    Some(d) if d.ecosystem == ecosystem => {
                        let (got, total) = d.progress;
                        format!("받는 중 {:.1}/{:.1}MB", mb(got), mb(total))
                    }
                    _ => path
                        .as_deref()
                        .and_then(database_state)
                        .unwrap_or("없음".into()),
                };
                ui.weak(state);
                if ui
                    .add_enabled(
                        path.is_some() && self.download.is_none(),
                        egui::Button::new("받기"),
                    )
                    .on_hover_text(format!(
                        "{}(약 {}MB)을 받습니다",
                        osv::database_url(ecosystem),
                        approximate_mb(ecosystem)
                    ))
                    .clicked()
                {
                    fetch = Some(ecosystem);
                }
                if ui
                    .add_enabled(path.is_some(), egui::Button::new("파일에서…"))
                    .on_hover_text("받아 둔 all.zip을 들입니다(인터넷이 막힌 곳)")
                    .clicked()
                    && let Some(dest) = &path
                    && let Some(source) = rfd::FileDialog::new()
                        .add_filter("OSV DB", &["zip"])
                        .pick_file()
                {
                    self.status = Some(match osv::import_database(&source, dest) {
                        Ok(()) => (format!("{} DB를 들였습니다", ecosystem.osv()), false),
                        Err(e) => (e, true),
                    });
                }
            });
        }
        if let Some(ecosystem) = fetch {
            self.start_download(ui.ctx(), ecosystem);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, root: Option<&Path>) -> Option<VulnAction> {
        let Some(root) = root else {
            ui.weak("폴더를 열면 프로젝트 의존성의 알려진 취약점을 검사합니다");
            return None;
        };
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.scanning.is_none(), egui::Button::new("검사"))
                .clicked()
            {
                self.start(ui.ctx(), root);
            }
            ui.radio_value(&mut self.online, true, "온라인");
            ui.radio_value(&mut self.online, false, "오프라인 DB");
        });
        ui.weak(if self.online {
            "온라인: 의존성 이름과 버전만 api.osv.dev에 보냅니다(코드는 보내지 않습니다)."
        } else {
            "오프라인: 받아 두거나 들인 OSV DB 묶음만 읽습니다."
        });
        if !self.online {
            self.databases_ui(ui);
        }
        if let Some((text, bad)) = &self.status {
            let color = if *bad {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().weak_text_color()
            };
            ui.colored_label(color, text);
        }
        for note in &self.notes {
            ui.colored_label(ui.visuals().warn_fg_color, note);
        }
        let Some(scanned) = self.root.clone() else {
            ui.weak("Maven·Gradle은 빌드 도구로 전이 의존까지 풀고, npm·PyPI·Cargo는 잠금 파일과 고정한 버전을 읽습니다.");
            return None;
        };
        if self.findings.is_empty() {
            return None;
        }
        ui.separator();
        let mut files: BTreeMap<String, Vec<&Finding>> = BTreeMap::new();
        for finding in &self.findings {
            files
                .entry(shown(&finding.dependency.file, &scanned))
                .or_default()
                .push(finding);
        }
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("vuln_findings")
            .auto_shrink(false)
            .show(ui, |ui| {
                for (file, mut findings) in files {
                    findings.sort_by_key(|f| (f.dependency.line, f.severity));
                    egui::CollapsingHeader::new(format!("{file} ({})", findings.len()))
                        .id_salt(("vuln_file_group", file.as_str()))
                        .default_open(true)
                        .show(ui, |ui| {
                            for finding in findings {
                                let text = egui::RichText::new(row(finding)).color(
                                    crate::editor_view::severity_color(
                                        ui.visuals(),
                                        severity(finding.severity),
                                    ),
                                );
                                if ui
                                    .selectable_label(false, text)
                                    .on_hover_text(format!("{} · {}", finding.id, finding.summary))
                                    .clicked()
                                {
                                    action = Some(VulnAction::Go(
                                        finding.dependency.file.clone(),
                                        finding.dependency.line,
                                    ));
                                }
                            }
                        });
                }
            });
        action
    }
}

/// 목록의 한 줄: `줄  [심각도] CVE · 이름 버전 → 고친 버전 · 끌어온 것`.
fn row(finding: &Finding) -> String {
    let dependency = &finding.dependency;
    let mut text = format!(
        "{}  [{}] {} · {} {}",
        dependency.line + 1,
        finding.severity.label(),
        finding.title(),
        dependency.name,
        dependency.version
    );
    if let Some(fixed) = &finding.fixed {
        text.push_str(&format!(" → {fixed}"));
    }
    if let Some(via) = &dependency.via {
        // Maven 이름은 길어서 아티팩트만(`그룹:아티팩트` → `아티팩트`).
        let short = via.rsplit(':').next().unwrap_or(via);
        text.push_str(&format!(" · {short}이(가) 끌어옴"));
    }
    text
}

/// 프로젝트의 의존성을 모아 견준다(검사 스레드).
fn scan(
    root: &Path,
    online: bool,
    databases: &[(Ecosystem, Option<PathBuf>)],
    step: &dyn Fn(String),
) -> Result<Scan, String> {
    let mut dependencies = Vec::new();
    let mut notes = Vec::new();
    for (i, manifest) in manifest::find(root).into_iter().enumerate() {
        let name = shown(&manifest.path, root);
        step(format!("의존성 읽는 중: {name}"));
        let output =
            std::env::temp_dir().join(format!("ferrite-deps-{}-{i}.txt", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let resolved = match manifest.command(root, &output) {
            None => Ok(None),
            Some(line) => run(manifest.path.parent().unwrap_or(root), &line).map(|stdout| {
                Some(if manifest.kind == manifest::Kind::Maven {
                    std::fs::read_to_string(&output).unwrap_or_default()
                } else {
                    stdout
                })
            }),
        };
        let _ = std::fs::remove_file(&output);
        match resolved.and_then(|r| manifest.read(r.as_deref())) {
            Ok(found) => dependencies.extend(found),
            Err(e) => notes.push(format!("{name}: {e}")),
        }
    }
    let unique = dependencies
        .iter()
        .map(|d| (d.ecosystem, &d.name, &d.version))
        .collect::<HashSet<_>>()
        .len();
    if dependencies.is_empty() {
        return Ok(Scan {
            dependencies: 0,
            findings: Vec::new(),
            notes,
        });
    }
    let advisories = if online {
        step(format!("OSV에 묻는 중: 의존성 {unique}개"));
        osv::query_online(&dependencies)?
    } else {
        let mut all = Vec::new();
        for (ecosystem, database) in databases {
            let mine: Vec<_> = dependencies
                .iter()
                .filter(|d| d.ecosystem == *ecosystem)
                .cloned()
                .collect();
            if mine.is_empty() {
                continue;
            }
            match database.as_deref().filter(|p| p.is_file()) {
                Some(database) => {
                    step(format!("{} DB를 읽는 중…", ecosystem.osv()));
                    all.extend(osv::query_offline(database, &mine)?);
                }
                None => notes.push(format!(
                    "{} DB가 없어 의존성 {}개를 검사하지 않았습니다",
                    ecosystem.osv(),
                    mine.len()
                )),
            }
        }
        all
    };
    Ok(Scan {
        dependencies: unique,
        findings: osv::findings(&advisories, &dependencies),
        notes,
    })
}

/// 빌드 도구를 셸로 돌려 표준 출력을 받는다. 실패하면 출력의 끝 몇 줄을 알린다.
fn run(dir: &Path, line: &str) -> Result<String, String> {
    let output = crate::debug_ui::run_hidden(line, dir, &Default::default())
        .map_err(|e| format!("`{line}`을(를) 띄우지 못했습니다: {e}"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let text = output.stdout + &output.stderr;
    Err(format!(
        "`{line}` 실패: {}",
        crate::debug_ui::tail(&text, 3)
    ))
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.0
}

/// 받을 DB 묶음의 대략 크기(2026년 11월).
fn approximate_mb(ecosystem: Ecosystem) -> u32 {
    match ecosystem {
        Ecosystem::Maven => 11,
        Ecosystem::Npm => 220,
        Ecosystem::PyPI => 38,
        Ecosystem::Crates => 4,
    }
}

/// 받아 둔 DB의 크기와 받은 때.
fn database_state(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let days = meta
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map_or(0, |d| d.as_secs() / 86_400);
    let age = if days == 0 {
        "오늘 받음".to_owned()
    } else {
        format!("{days}일 전 받음")
    };
    Some(format!("{:.1}MB · {age}", mb(meta.len())))
}

fn shown(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 물결선의 심각도: 치명·높음은 오류, 보통·모름은 경고, 낮음은 참고.
fn severity(severity: vuln::Severity) -> Severity {
    match severity {
        vuln::Severity::Critical | vuln::Severity::High => Severity::Error,
        vuln::Severity::Moderate | vuln::Severity::Unknown => Severity::Warning,
        vuln::Severity::Low => Severity::Info,
    }
}

/// 이 문서(의존성 파일)에 해당하는 결과를 에디터의 진단(그 줄의 글자, 위치 순)으로 바꾼다.
pub fn problems_for(findings: &[Finding], path: &Path, rope: &Rope) -> Vec<Problem> {
    let mut problems: Vec<Problem> = findings
        .iter()
        .filter(|f| f.dependency.file == path)
        .map(|f| {
            let mut message = format!(
                "{} [{}] {} {}: {}",
                f.title(),
                f.severity.label(),
                f.dependency.name,
                f.dependency.version,
                f.summary
            );
            if let Some(fixed) = &f.fixed {
                message.push_str(&format!(" ({fixed}에서 고침)"));
            }
            Problem {
                range: crate::sarif_ui::line_range(rope, f.dependency.line),
                severity: severity(f.severity),
                message: message.clone(),
                source: "OSV".into(),
                raw: lsp_types::Diagnostic {
                    message,
                    ..Default::default()
                },
            }
        })
        .collect();
    problems.sort_by_key(|p| (p.range.start, std::cmp::Reverse(p.severity)));
    problems
}
