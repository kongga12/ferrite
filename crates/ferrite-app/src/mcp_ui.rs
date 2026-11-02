//! "AI 에이전트 연결(MCP)" 창과 서버. 켜면 127.0.0.1에서 듣고, 에이전트에 넣을 설정(Claude Code 명령,
//! JSON)을 복사한다. 앱 상태는 0.2초마다 스냅샷으로 넣는다(서버는 그것만 읽는다). 토큰은 설정 폴더에
//! 두어 에이전트 설정이 앱을 다시 켜도 맞는다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use ferrite_mcp::{Server, Snapshot};

/// 스냅샷을 새로 만드는 간격.
const REFRESH: Duration = Duration::from_millis(200);

pub struct McpPanel {
    open: bool,
    /// 토큰 파일. 없으면(시험) 켤 때마다 새 토큰.
    token_path: Option<PathBuf>,
    token: Option<String>,
    server: Option<Server>,
    error: Option<String>,
    published: Option<Instant>,
    /// 파일 → (버퍼 버전, 내용). 바뀐 문서만 다시 옮긴다.
    texts: HashMap<PathBuf, (u64, Arc<str>)>,
}

impl McpPanel {
    pub fn new(token_path: Option<PathBuf>) -> Self {
        Self {
            open: false,
            token_path,
            token: None,
            server: None,
            error: None,
            published: None,
            texts: HashMap::new(),
        }
    }

    pub fn show(&mut self) {
        self.open = true;
    }

    #[cfg(test)]
    pub fn address(&self) -> Option<std::net::SocketAddr> {
        self.server.as_ref().map(Server::address)
    }

    #[cfg(test)]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// 토큰 파일을 읽거나 새로 만든다.
    fn load_token(&mut self, renew: bool) -> Result<String, String> {
        if !renew && let Some(token) = &self.token {
            return Ok(token.clone());
        }
        let Some(path) = self.token_path.clone() else {
            return Ok(ferrite_mcp::new_token());
        };
        if !renew
            && let Ok(text) = std::fs::read_to_string(&path)
            && text.trim().len() == 64
            && text.trim().bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Ok(text.trim().to_owned());
        }
        let token = ferrite_mcp::new_token();
        write_private(&path, &token)
            .map_err(|e| format!("토큰을 {}에 쓰지 못했습니다: {e}", path.display()))?;
        Ok(token)
    }

    /// 그 포트에서 듣는다(0이면 빈 포트).
    pub fn start(&mut self, port: u16) -> Result<(), String> {
        let token = self.load_token(false)?;
        let server = Server::start(port, &token)
            .map_err(|e| format!("127.0.0.1:{port}에서 듣지 못했습니다: {e}"))?;
        self.token = Some(token);
        self.server = Some(server);
        self.published = None;
        self.error = None;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.server = None;
        self.texts.clear();
    }

    /// 스냅샷을 새로 넣을 때인지(켜져 있고 간격이 지났다).
    pub fn due(&self) -> bool {
        self.server.is_some() && self.published.is_none_or(|at| at.elapsed() >= REFRESH)
    }

    /// 문서 내용(버전이 같으면 앞서 옮긴 것을 다시 쓴다).
    pub fn text(&mut self, path: &Path, version: u64, text: impl FnOnce() -> String) -> Arc<str> {
        match self.texts.get(path) {
            Some((v, text)) if *v == version => text.clone(),
            _ => {
                let text: Arc<str> = Arc::from(text());
                self.texts
                    .insert(path.to_path_buf(), (version, text.clone()));
                text
            }
        }
    }

    pub fn publish(&mut self, snapshot: Snapshot) {
        // 닫은 문서의 내용은 버린다.
        self.texts.retain(|path, _| {
            snapshot
                .documents
                .iter()
                .any(|d| Path::new(&d.path) == path)
        });
        if let Some(server) = &self.server {
            server.set_snapshot(snapshot);
        }
        self.published = Some(Instant::now());
    }

    pub fn ui(&mut self, ctx: &egui::Context, port: u16) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut action = None;
        egui::Window::new("AI 에이전트 연결(MCP)")
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    match &self.server {
                        Some(server) => {
                            ui.label(format!("듣는 중: http://{}/mcp", server.address()));
                            if ui.button("끄기").clicked() {
                                action = Some(Action::Stop);
                            }
                        }
                        None => {
                            ui.label("꺼짐");
                            if ui.button("켜기").clicked() {
                                action = Some(Action::Start);
                            }
                        }
                    }
                    if ui
                        .button("토큰 새로 만들기")
                        .on_hover_text("지금 토큰을 쓰는 에이전트는 다시 설정해야 합니다")
                        .clicked()
                    {
                        action = Some(Action::Renew);
                    }
                });
                ui.weak(
                    "이 PC의 프로그램만 붙을 수 있고, 토큰을 아는 에이전트는 열린 파일(저장하지 않은 내용 포함)·\
                     진단·디버거 상태를 읽습니다. 파일은 바꾸지 않습니다. 앱을 켤 때 함께 켜려면 settings.toml에 \
                     mcp_server = true, 포트는 mcp_port로 바꿉니다.",
                );
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                let (Some(server), Some(token)) = (&self.server, &self.token) else {
                    return;
                };
                let url = format!("http://{}/mcp", server.address());
                let claude = format!(
                    "claude mcp add --transport http ferrite {url} --header \"Authorization: Bearer {token}\""
                );
                let json = serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": { "ferrite": {
                        "type": "http", "url": url, "headers": { "Authorization": format!("Bearer {token}") },
                    } },
                }))
                .unwrap_or_default();
                ui.separator();
                for (label, text) in [("Claude Code 명령", claude), ("JSON 설정(다른 에이전트)", json)] {
                    ui.horizontal(|ui| {
                        ui.strong(label);
                        if ui.button("복사").clicked() {
                            ui.ctx().copy_text(text.clone());
                        }
                    });
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).monospace())
                            .wrap(),
                    );
                }
            });
        if !open {
            self.open = false;
        }
        match action {
            Some(Action::Start) => {
                if let Err(e) = self.start(port) {
                    self.error = Some(e);
                }
            }
            Some(Action::Stop) => self.stop(),
            Some(Action::Renew) => match self.load_token(true) {
                Ok(token) => {
                    self.token = Some(token);
                    if self.server.is_some() {
                        // 새 토큰으로 다시 듣는다.
                        self.stop();
                        if let Err(e) = self.start(port) {
                            self.error = Some(e);
                        }
                    }
                }
                Err(e) => self.error = Some(e),
            },
            None => {}
        }
    }
}

enum Action {
    Start,
    Stop,
    Renew,
}

/// 토큰을 쓴다. 유닉스는 나만 읽게 둔다.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
