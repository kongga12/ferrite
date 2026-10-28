//! 자동 업데이트 화면. 시작할 때(설정 `update_check`) 따로 스레드에서 새 판을 묻고, 있으면 상태
//! 표시줄에 알린다. [받아 설치]는 설치 파일을 받아 서명을 검증한다. NSIS로 설치한 Windows는 Ferrite를
//! 닫을 때 설치 파일을 돌려 새 판을 띄우고, AppImage는 파일을 바꾼다. 나머지(MSI·dmg·deb·개발 빌드)는
//! 내려받기 페이지를 연다.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui;
use ferrite_update::{Installer, RELEASES_PAGE, Release};

/// 업데이트 서명을 검증할 공개 키(`cargo packager signer generate`가 만든 `.pub`). 비밀 키는 릴리스
/// 워크플로의 GitHub 비밀 값에만 있다.
const PUBLIC_KEY: &str = include_str!("../updater.pub");
const CURRENT: &str = env!("CARGO_PKG_VERSION");

enum Message {
    Checked(Result<Option<Release>, String>),
    Progress(u64, Option<u64>),
    Downloaded(Result<PathBuf, String>),
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading {
        release: Release,
        installer: Installer,
        received: u64,
        total: Option<u64>,
    },
    /// 받아 검증했다. NSIS는 앱이 끝날 때 설치하고, AppImage는 이미 바꿨다.
    Ready {
        release: Release,
        installer: Installer,
    },
    Failed(String),
}

#[derive(Default)]
pub struct Updater {
    state: State,
    events: Option<Receiver<Message>>,
    /// 업데이트 창을 보인다.
    pub open: bool,
    /// 앱이 끝날 때 돌릴 NSIS 설치 파일.
    pending: Option<PathBuf>,
}

impl Updater {
    /// 새 판을 묻는다. `manual`이면 창을 열어 결과(최신 판, 오류)도 보인다. 시작할 때의 확인은
    /// 새 판이 있을 때만 상태 표시줄에 알린다.
    pub fn check(&mut self, url: &str, manual: bool, ctx: &egui::Context) {
        if matches!(
            self.state,
            State::Checking | State::Downloading { .. } | State::Ready { .. }
        ) {
            self.open |= manual;
            return;
        }
        self.state = State::Checking;
        self.open = manual;
        let url = url.to_owned();
        self.spawn(ctx, move |send| {
            send(Message::Checked(ferrite_update::check(&url, CURRENT)));
        });
    }

    fn spawn(&mut self, ctx: &egui::Context, work: impl FnOnce(&dyn Fn(Message)) + Send + 'static) {
        let (sender, events): (Sender<Message>, _) = channel();
        self.events = Some(events);
        let ctx = ctx.clone();
        thread::spawn(move || {
            work(&|message| {
                let _ = sender.send(message);
                ctx.request_repaint();
            })
        });
    }

    /// 작업 스레드의 소식을 받는다.
    pub fn poll(&mut self) {
        let Some(events) = &self.events else {
            return;
        };
        let messages: Vec<Message> = events.try_iter().collect();
        for message in messages {
            let state = std::mem::take(&mut self.state);
            self.state = match (message, state) {
                (Message::Checked(Ok(Some(release))), _) => State::Available(release),
                (Message::Checked(Ok(None)), _) => State::UpToDate,
                (Message::Checked(Err(e)), _) => State::Failed(e),
                (
                    Message::Progress(received, total),
                    State::Downloading {
                        release, installer, ..
                    },
                ) => State::Downloading {
                    release,
                    installer,
                    received,
                    total,
                },
                (
                    Message::Downloaded(Ok(file)),
                    State::Downloading {
                        release, installer, ..
                    },
                ) => self.finish(release, installer, file),
                (Message::Downloaded(Err(e)), _) => {
                    self.open = true;
                    State::Failed(e)
                }
                (_, state) => state,
            };
        }
    }

    /// 받아 검증한 설치 파일을 쓴다.
    fn finish(&mut self, release: Release, installer: Installer, file: PathBuf) -> State {
        match &installer {
            Installer::Nsis => self.pending = Some(file),
            Installer::AppImage(target) => {
                if let Err(e) = ferrite_update::replace_appimage(&file, target) {
                    self.open = true;
                    return State::Failed(format!("{}: {e}", target.display()));
                }
            }
        }
        State::Ready { release, installer }
    }

    fn download(&mut self, release: Release, installer: Installer, ctx: &egui::Context) {
        let Some(asset) = release.asset.clone() else {
            return;
        };
        let name = asset
            .url
            .rsplit('/')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or("ferrite-update")
            .to_owned();
        let dest = download_dir().join(name);
        self.state = State::Downloading {
            release,
            installer,
            received: 0,
            total: None,
        };
        self.spawn(ctx, move |send| {
            let result = ferrite_update::download(&asset, PUBLIC_KEY, &dest, &mut |r, t| {
                send(Message::Progress(r, t))
            });
            send(Message::Downloaded(result.map(|()| dest)));
        });
    }

    /// 상태 표시줄의 알림(새 판이 있을 때만).
    pub fn status_ui(&mut self, ui: &mut egui::Ui) {
        let text = match &self.state {
            State::Available(release) => format!("새 판 {}", release.version),
            State::Downloading {
                received, total, ..
            } => match total {
                Some(total) if *total > 0 => {
                    format!("업데이트 받는 중 {}%", received * 100 / total)
                }
                _ => format!("업데이트 받는 중 {}KB", received / 1024),
            },
            State::Ready { .. } => "업데이트 준비됨".into(),
            _ => return,
        };
        if ui.button(text).on_hover_text("업데이트 창 열기").clicked() {
            self.open = true;
        }
    }

    /// 업데이트 창. 설치하려고 앱을 닫아야 하면 true.
    pub fn window(&mut self, ctx: &egui::Context) -> bool {
        if !self.open {
            return false;
        }
        let mut open = true;
        let mut close_app = false;
        let mut download = None;
        egui::Window::new("업데이트")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| match &self.state {
                State::Idle | State::Checking => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("새 판을 확인하는 중");
                    });
                }
                State::UpToDate => {
                    ui.label(format!("최신 판입니다({CURRENT})."));
                }
                State::Failed(error) => {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                State::Available(release) => {
                    ui.label(format!(
                        "새 판 {}이 있습니다(지금 {CURRENT}).",
                        release.version
                    ));
                    notes(ui, release);
                    let exe = std::env::current_exe().unwrap_or_default();
                    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
                    let installer = release
                        .asset
                        .as_ref()
                        .and_then(|a| ferrite_update::installer(&a.format, &exe, appimage));
                    ui.horizontal(|ui| match installer {
                        Some(installer) => {
                            if ui.button("받아 설치").clicked() {
                                download = Some((release.clone(), installer));
                            }
                        }
                        None => {
                            if ui.button("내려받기 페이지 열기").clicked() {
                                let url = release
                                    .asset
                                    .as_ref()
                                    .map_or(RELEASES_PAGE, |a| a.url.as_str());
                                ctx.open_url(egui::OpenUrl::new_tab(url));
                            }
                            ui.weak("이 설치 방식은 Ferrite가 직접 바꾸지 않습니다");
                        }
                    });
                }
                State::Downloading {
                    release,
                    received,
                    total,
                    ..
                } => {
                    ui.label(format!("{}을 받는 중", release.version));
                    match total {
                        Some(total) if *total > 0 => {
                            ui.add(egui::ProgressBar::new(*received as f32 / *total as f32));
                        }
                        _ => {
                            ui.spinner();
                        }
                    }
                }
                State::Ready {
                    release,
                    installer: Installer::Nsis,
                } => {
                    ui.label(format!(
                        "{}을 받아 서명을 확인했습니다. Ferrite를 닫으면 설치하고 다시 띄웁니다.",
                        release.version
                    ));
                    if ui.button("지금 닫고 설치").clicked() {
                        close_app = true;
                    }
                }
                State::Ready { release, .. } => {
                    ui.label(format!(
                        "{}으로 바꿨습니다. Ferrite를 다시 시작하면 새 판입니다.",
                        release.version
                    ));
                }
            });
        if let Some((release, installer)) = download {
            self.download(release, installer, ctx);
        }
        self.open = open && !close_app;
        close_app
    }

    /// 앱이 끝날 때: 받아 둔 NSIS 설치 파일을 돌린다.
    pub fn on_exit(&mut self) {
        if let Some(installer) = self.pending.take()
            && let Err(e) = ferrite_update::run_nsis(&installer)
        {
            eprintln!(
                "업데이트 설치 파일을 띄우지 못했습니다({}): {e}",
                installer.display()
            );
        }
    }
}

/// 받은 설치 파일을 두는 곳.
fn download_dir() -> PathBuf {
    std::env::temp_dir().join("ferrite-update")
}

/// 지난번에 받은 설치 파일을 지운다. 설치 파일은 앱이 끝난 뒤에 돌기 때문에 그때는 지우지 못한다.
/// 막 업데이트하고 다시 뜬 때에는 설치 파일이 아직 돌고 있어 못 지울 수 있다(다음 시작에 지운다).
pub fn clean_downloads() {
    let _ = std::fs::remove_dir_all(download_dir());
}

/// 릴리스 노트(길면 스크롤).
fn notes(ui: &mut egui::Ui, release: &Release) {
    if release.notes.trim().is_empty() {
        return;
    }
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            ui.label(release.notes.trim());
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 내장한_공개_키를_읽을_수_있다() {
        // 키를 읽으면 그다음 단계인 서명 읽기에서 실패한다.
        assert_eq!(
            ferrite_update::verify(b"", "", PUBLIC_KEY),
            Err("설치 파일의 서명을 읽지 못했습니다".into())
        );
    }
}
