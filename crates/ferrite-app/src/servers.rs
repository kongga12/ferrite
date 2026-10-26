//! 언어 서버 관리 창과 설치 작업. 설치·삭제는 별도 스레드에서 돌리고 프레임마다 결과를 거둔다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::Duration;

use eframe::egui;
use ferrite_lsp::catalog::{self, SERVERS, Server, Source};
use ferrite_lsp::client::find_program;
use ferrite_lsp::install::{self, Installed};

/// 삭제할 때 서버 프로세스가 끝나기를 기다리며 다시 시도하는 횟수와 간격.
/// Windows는 실행 중인 파일을 지우지 못하는데, 서버 종료는 비동기로 최대 2초 걸린다.
const UNINSTALL_RETRIES: u32 = 10;
const UNINSTALL_RETRY_DELAY: Duration = Duration::from_millis(300);

/// 창이나 상태바에서 사용자가 고른 일. 앱이 언어 서버를 내린 뒤 실행한다.
pub enum ServerAction {
    /// `files`가 있으면 내려받지 않고 그 파일들로 설치한다(인터넷이 막힌 환경).
    Install {
        server: &'static Server,
        files: Vec<PathBuf>,
    },
    Uninstall(&'static Server),
}

/// 끝난 작업. 앱이 상태바에 알리고 서버를 새 실행 파일로 다시 띄운다.
pub struct Finished {
    pub server: &'static Server,
    pub verb: &'static str,
    pub result: Result<(), String>,
}

enum JobEvent {
    Progress(u64, u64),
    Done(Result<(), String>),
}

struct Job {
    verb: &'static str,
    rx: Receiver<JobEvent>,
    progress: Option<(u64, u64)>,
}

/// 창에 보여 줄 설치 상태. PATH 검색이 가볍지 않아 창을 열 때와 작업이 끝날 때만 다시 구한다.
enum Status {
    /// Ferrite에 들어 있다(설치·삭제 없음).
    Builtin,
    Managed(Installed),
    OnPath(PathBuf),
    Missing,
}

pub struct ServerManager {
    root: PathBuf,
    open: bool,
    jobs: HashMap<&'static str, Job>,
    errors: HashMap<&'static str, String>,
    /// `catalog::all()`(언어 서버 다음에 디버그 어댑터)과 같은 순서.
    statuses: Vec<Status>,
}

impl ServerManager {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            open: false,
            jobs: HashMap::new(),
            errors: HashMap::new(),
            statuses: Vec::new(),
        }
    }

    pub fn show(&mut self) {
        self.open = true;
        self.refresh();
    }

    fn refresh(&mut self) {
        self.statuses = catalog::all()
            .map(|s| match install::installed(&self.root, s) {
                _ if matches!(s.source, Source::Builtin(_)) => Status::Builtin,
                Some(installed) => Status::Managed(installed),
                // PATH의 python이 있다고 debugpy가 있는 것은 아니다.
                None if matches!(s.source, Source::Pip(_)) && s.program == "python" => {
                    Status::Missing
                }
                None => find_program(s.program).map_or(Status::Missing, Status::OnPath),
            })
            .collect();
    }

    /// 목록보다 예전 버전이 설치된 서버.
    pub fn outdated(&self) -> Vec<&'static str> {
        catalog::all()
            .filter(|s| install::installed(&self.root, s).is_some_and(|i| i.is_outdated(s)))
            .map(|s| s.name)
            .collect()
    }

    /// 진행 중인 작업 설명("설치 중 45%"). 작업이 없으면 None.
    pub fn progress_text(&self, name: &str) -> Option<String> {
        let job = self.jobs.get(name)?;
        Some(match job.progress {
            Some((done, total)) if total > 0 => {
                format!("{} 중 {}%", job.verb, done * 100 / total)
            }
            _ => format!("{} 중…", job.verb),
        })
    }

    /// 작업을 시작한다. 같은 서버에 이미 작업이 있으면 무시한다.
    pub fn start(&mut self, action: ServerAction, ctx: &egui::Context) {
        let (server, verb) = match &action {
            ServerAction::Install { server, .. } => (*server, "설치"),
            ServerAction::Uninstall(server) => (*server, "삭제"),
        };
        if self.jobs.contains_key(server.name) {
            return;
        }
        self.errors.remove(server.name);
        let (tx, rx) = channel();
        let root = self.root.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = match action {
                ServerAction::Install { files, .. } if !files.is_empty() => {
                    install::install_from_files(&root, server, &files).map_err(|e| e.to_string())
                }
                ServerAction::Install { .. } => {
                    let progress_tx = tx.clone();
                    let progress_ctx = ctx.clone();
                    install::install(&root, server, &mut |done, total| {
                        let _ = progress_tx.send(JobEvent::Progress(done, total));
                        progress_ctx.request_repaint();
                    })
                    .map_err(|e| e.to_string())
                }
                ServerAction::Uninstall(_) => uninstall_with_retry(&root, server),
            };
            let _ = tx.send(JobEvent::Done(result));
            ctx.request_repaint();
        });
        self.jobs.insert(
            server.name,
            Job {
                verb,
                rx,
                progress: None,
            },
        );
    }

    /// 끝난 작업을 거둔다. 매 프레임 부른다.
    pub fn poll(&mut self) -> Vec<Finished> {
        let mut finished = Vec::new();
        for (&name, job) in &mut self.jobs {
            for event in job.rx.try_iter() {
                match event {
                    JobEvent::Progress(done, total) => job.progress = Some((done, total)),
                    JobEvent::Done(result) => {
                        let server = catalog::all().find(|s| s.name == name);
                        finished.extend(server.map(|server| Finished {
                            server,
                            verb: job.verb,
                            result,
                        }));
                    }
                }
            }
        }
        for done in &finished {
            self.jobs.remove(done.server.name);
            if let Err(e) = &done.result {
                self.errors.insert(done.server.name, e.clone());
            }
        }
        if !finished.is_empty() && self.open {
            self.refresh();
        }
        finished
    }

    /// 관리 창. 사용자가 고른 일을 돌려준다.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<ServerAction> {
        let mut action = None;
        let mut open = self.open;
        egui::Window::new("언어 서버")
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                ui.weak(format!("설치 위치: {}", self.root.display()));
                ui.add_space(4.0);
                egui::Grid::new("servers")
                    .num_columns(4)
                    .striped(true)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (i, (server, status)) in catalog::all().zip(&self.statuses).enumerate()
                        {
                            if i == SERVERS.len() {
                                ui.strong("디버그 어댑터");
                                ui.end_row();
                            }
                            ui.strong(server.name);
                            ui.label(server.languages.join(", "));
                            self.status_cell(ui, server, status);
                            ui.horizontal(|ui| {
                                if !self.jobs.contains_key(server.name)
                                    && let Some(chosen) = buttons(ui, server, status)
                                {
                                    action = Some(chosen);
                                }
                            });
                            ui.end_row();
                        }
                    });
                for (name, error) in &self.errors {
                    ui.colored_label(ui.visuals().error_fg_color, format!("{name}: {error}"));
                }
            });
        self.open = open;
        action
    }

    fn status_cell(&self, ui: &mut egui::Ui, server: &Server, status: &Status) {
        if let Some(progress) = self.progress_text(server.name) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(progress);
            });
            return;
        }
        match status {
            Status::Builtin => {
                ui.label(format!("Ferrite 내장 {}", server.version));
            }
            Status::Managed(i) if i.is_outdated(server) => {
                ui.label(format!(
                    "업데이트 있음 ({} → {})",
                    i.version, server.version
                ));
            }
            Status::Managed(i) => {
                ui.label(format!("설치됨 {}", i.version))
                    .on_hover_text(i.bin.display().to_string());
            }
            Status::OnPath(path) => {
                ui.label("직접 설치됨 (PATH)")
                    .on_hover_text(path.display().to_string());
            }
            Status::Missing if !install::installable(server) => {
                ui.weak("이 OS·CPU용 배포본 없음");
            }
            Status::Missing => {
                ui.label("설치 안 됨");
            }
        }
    }
}

fn buttons(ui: &mut egui::Ui, server: &'static Server, status: &Status) -> Option<ServerAction> {
    let managed = match status {
        Status::Managed(installed) => Some(installed),
        _ => None,
    };
    let mut action = None;
    if managed.is_none_or(|i| i.is_outdated(server)) && install::installable(server) {
        let verb = if managed.is_some() {
            "업데이트"
        } else {
            "설치"
        };
        if ui
            .button(format!("{verb} ({})", install_hint(server)))
            .clicked()
        {
            action = Some(ServerAction::Install {
                server,
                files: Vec::new(),
            });
        }
        // 인터넷이 막힌 환경: 다른 PC에서 받아 온 파일들로 설치한다. 해시가 목록과 맞아야 한다.
        let assets = install::assets(server);
        if !assets.is_empty() {
            let urls: Vec<&str> = assets.iter().map(|a| a.url).collect();
            let urls = urls.join("\n");
            if ui.button("주소 복사").on_hover_text(&urls).clicked() {
                ui.ctx().copy_text(urls);
            }
            if ui
                .button("파일에서…")
                .on_hover_text("다른 PC에서 받아 온 배포 파일(위 주소의 파일 모두)로 설치합니다")
                .clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .set_title(format!("{} 배포 파일", server.name))
                    .pick_files()
            {
                action = Some(ServerAction::Install { server, files });
            }
        }
    }
    if managed.is_some() && ui.button("삭제").clicked() {
        action = Some(ServerAction::Uninstall(server));
    }
    action
}

/// 설치 버튼 옆에 붙이는 설명: 내려받을 크기 또는 필요한 도구.
pub fn install_hint(server: &Server) -> String {
    match server.source {
        Source::Release(_) => {
            let size: u64 = install::assets(server).iter().map(|a| a.size).sum();
            format!("{:.1}MB", size as f64 / 1_000_000.0)
        }
        Source::Go(_) => "go 필요".into(),
        Source::Pip(_) => "python 필요".into(),
        Source::Builtin(_) => String::new(),
    }
}

fn uninstall_with_retry(root: &Path, server: &Server) -> Result<(), String> {
    let mut attempt = 0;
    loop {
        match install::uninstall(root, server) {
            Ok(()) => return Ok(()),
            Err(_) if attempt < UNINSTALL_RETRIES => {
                attempt += 1;
                thread::sleep(UNINSTALL_RETRY_DELAY);
            }
            Err(e) => return Err(format!("{e} (서버가 아직 실행 중일 수 있습니다)")),
        }
    }
}
