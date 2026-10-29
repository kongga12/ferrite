//! HTTP 요청 파일(`.http`) 실행과 "HTTP 응답" 창. 요청은 따로 스레드에서 보내고, 쿠키는 앱이 켜진
//! 동안 둔다(egov 로그인 뒤 이어 보내는 요청에 세션 쿠키가 붙는다).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;
use ferrite_http::{Client, CookieJar, Environments, Response, env};

/// 응답 창에 보이는 본문 글의 상한. 더 길면 앞만 보인다(egui 글 상자가 한 프레임에 다 배치한다).
const SHOWN_TEXT: usize = 512 * 1024;

pub fn is_http_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("http") || e.eq_ignore_ascii_case("rest"))
}

/// 요청 줄(0부터). 거터에 ▶를 그린다.
pub fn run_lines(text: &str) -> Vec<usize> {
    ferrite_http::parse(text)
        .requests
        .iter()
        .map(|r| r.line)
        .collect()
}

/// 창을 다 그린 뒤에 할 일(응답 창을 앞으로 가져오느라 배치를 바꾼다).
pub enum Task {
    /// 이 파일의 이 줄(0부터)이 든 요청을 보낸다.
    Run { path: PathBuf, line: usize },
    /// 엔드포인트로 `http/<컨트롤러>.http`에 요청을 만든다.
    Make {
        controller: String,
        path: String,
        methods: String,
    },
}

/// 보낸 요청과 그 결과.
struct Sent {
    title: String,
    method: String,
    url: String,
    env: Option<String>,
    /// 아직 받는 중이면 None.
    result: Option<Result<Shown, String>>,
}

/// 받은 응답과, 보일 본문 글(길면 앞만).
struct Shown {
    response: Response,
    text: String,
}

impl Shown {
    fn new(response: Response) -> Self {
        let content_type = response
            .header("content-type")
            .unwrap_or("")
            .to_ascii_lowercase();
        let binary = [
            "image/",
            "audio/",
            "video/",
            "application/octet-stream",
            "application/pdf",
            "application/zip",
        ]
        .iter()
        .any(|t| content_type.starts_with(t));
        let text = if binary {
            format!("(이진 본문 {}, {content_type})", size(response.body.len()))
        } else if response.text.len() > SHOWN_TEXT {
            let end = (0..=SHOWN_TEXT)
                .rev()
                .find(|&i| response.text.is_char_boundary(i))
                .unwrap_or(0);
            format!(
                "{}\n\n… (본문이 커서 앞 {}만 보입니다. 전체 {})",
                &response.text[..end],
                size(SHOWN_TEXT),
                size(response.text.len())
            )
        } else {
            response.text.clone()
        };
        Self { response, text }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Tab {
    #[default]
    Body,
    Headers,
}

#[derive(Default)]
pub struct HttpPanel {
    /// 처음 보낼 때 만든다(OS 인증서 저장소를 읽는다).
    client: Option<Arc<Client>>,
    /// 보내는 동안에는 보내는 스레드가 가졌다가 돌려준다.
    jar: CookieJar,
    /// 고른 환경. 환경 파일에 없으면 첫 환경을 고른다.
    env: Option<String>,
    envs: Environments,
    env_errors: Vec<String>,
    /// 개인 환경 파일을 `.gitignore`가 빼지 않는 프로젝트 루트(경고하고 더하기를 권한다).
    unignored: Option<PathBuf>,
    sent: Option<Sent>,
    rx: Option<Receiver<(Result<Response, String>, CookieJar)>>,
    tab: Tab,
    status: Option<String>,
}

impl HttpPanel {
    /// `line`(0부터)이 든 요청을 보낸다. 오류(요청이 없음, 채우지 못한 변수)도 창에 보인다.
    pub fn run(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        text: &str,
        line: usize,
        root: Option<&Path>,
    ) {
        // 쿠키 저장소는 보내는 스레드가 가지고 있다. 겹쳐 보내면 한쪽이 받은 쿠키를 잃는다.
        if self.running() {
            self.status = Some("앞 요청의 응답을 기다리는 중입니다".into());
            return;
        }
        let file = ferrite_http::parse(text);
        let Some(request) = file.request_at(line) else {
            self.status = Some("이 줄에는 보낼 요청이 없습니다".into());
            return;
        };
        let dir = path.parent().unwrap_or(Path::new("."));
        let root = root.filter(|r| dir.starts_with(r));
        (self.envs, self.env_errors) = env::load(dir, root);
        if !self
            .env
            .as_ref()
            .is_some_and(|e| self.envs.values.contains_key(e))
        {
            self.env = self.envs.names().next().map(str::to_owned);
        }
        self.unignored = root
            .filter(|r| {
                self.envs.private_file().is_some()
                    && r.join(".git").exists()
                    && !env::ignored_by_git(r)
            })
            .map(Path::to_path_buf);
        self.status = None;
        self.tab = Tab::Body;
        let env = self.env.as_deref().map(|e| (&self.envs, e));
        let mut sent = Sent {
            title: request
                .name
                .clone()
                .unwrap_or_else(|| format!("{} 줄", request.line + 1)),
            method: request.method.clone(),
            url: request.url.clone(),
            env: self.env.clone(),
            result: None,
        };
        match ferrite_http::prepare(&file, request, env) {
            Err(e) => {
                sent.result = Some(Err(e));
                self.rx = None;
            }
            Ok(prepared) => {
                sent.url.clone_from(&prepared.url);
                let client = self
                    .client
                    .get_or_insert_with(|| Arc::new(Client::default()))
                    .clone();
                let mut jar = std::mem::take(&mut self.jar);
                let (tx, rx) = mpsc::channel();
                let ctx = ctx.clone();
                let spawned = std::thread::Builder::new()
                    .name("http-request".into())
                    .spawn(move || {
                        let result = client.send(&prepared, &mut jar);
                        let _ = tx.send((result, jar));
                        ctx.request_repaint();
                    });
                match spawned {
                    Ok(_) => self.rx = Some(rx),
                    Err(e) => {
                        sent.result = Some(Err(format!("보내는 스레드를 띄우지 못했습니다: {e}")))
                    }
                }
            }
        }
        self.sent = Some(sent);
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    #[cfg(test)]
    pub fn done(&self) -> bool {
        self.sent.as_ref().is_some_and(|s| s.result.is_some())
    }

    #[cfg(test)]
    pub fn body(&self) -> Option<&str> {
        match &self.sent.as_ref()?.result {
            Some(Ok(shown)) => Some(&shown.text),
            _ => None,
        }
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        let received = match rx.try_recv() {
            Ok((result, jar)) => {
                self.jar = jar;
                Some(result.map(Shown::new))
            }
            Err(TryRecvError::Empty) => return,
            // 스레드가 패닉했다. 쿠키는 잃는다.
            Err(TryRecvError::Disconnected) => Some(Err("보내다가 멈췄습니다".into())),
        };
        self.rx = None;
        if let Some(sent) = &mut self.sent {
            sent.result = received;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        let running = self.running();
        ui.horizontal(|ui| {
            let names: Vec<String> = self.envs.names().map(str::to_owned).collect();
            ui.label("환경");
            egui::ComboBox::from_id_salt("http_env")
                .selected_text(self.env.as_deref().unwrap_or("(없음)"))
                .show_ui(ui, |ui| {
                    for name in names {
                        let selected = self.env.as_deref() == Some(name.as_str());
                        if ui.selectable_label(selected, &name).clicked() {
                            self.env = Some(name);
                        }
                    }
                })
                .response
                .on_hover_text(format!("{} · 다음 요청부터 씁니다", env::PUBLIC));
            let cookies = self.jar.len();
            if ui
                .add_enabled(
                    !running && cookies > 0,
                    egui::Button::new(format!("쿠키 {cookies}개 지우기")),
                )
                .clicked()
            {
                self.jar.clear();
            }
            if running {
                ui.spinner();
            }
        });
        for error in &self.env_errors {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("환경 파일을 읽지 못했습니다: {error}"),
            );
        }
        if let Some(root) = self.unignored.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "{}이 .gitignore에 없습니다. 비밀 값이 커밋될 수 있습니다.",
                        env::PRIVATE
                    ),
                );
                if ui.button(".gitignore에 더하기").clicked() {
                    match env::add_to_gitignore(&root) {
                        Ok(()) => self.unignored = None,
                        Err(e) => {
                            self.status = Some(format!(".gitignore에 더하지 못했습니다: {e}"))
                        }
                    }
                }
            });
        }
        if let Some(status) = &self.status {
            ui.colored_label(ui.visuals().warn_fg_color, status);
        }
        ui.separator();
        let Some(sent) = &self.sent else {
            ui.weak(".http 파일에서 요청 줄의 ▶를 누르거나 Ctrl+Enter로 보냅니다");
            return;
        };
        ui.strong(&sent.title);
        let env = sent
            .env
            .as_deref()
            .map(|e| format!(" · 환경 {e}"))
            .unwrap_or_default();
        ui.weak(format!("{} {}{env}", sent.method, sent.url));
        let shown = match &sent.result {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("보내는 중…");
                });
                return;
            }
            Some(Err(e)) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
                return;
            }
            Some(Ok(shown)) => shown,
        };
        let r = &shown.response;
        ui.horizontal_wrapped(|ui| {
            let color = match r.status {
                200..=299 => egui::Color32::from_rgb(60, 170, 90),
                300..=399 => ui.visuals().hyperlink_color,
                _ => ui.visuals().error_fg_color,
            };
            ui.colored_label(
                color,
                egui::RichText::new(format!("{} {}", r.status, r.reason)).strong(),
            );
            ui.weak(format!(
                "{} ms · {} · {}",
                r.elapsed.as_millis(),
                size(r.body.len()),
                r.charset
            ));
        });
        for (status, url) in &r.redirects {
            ui.weak(format!("→ {status} {url}"));
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Body, "본문");
            ui.selectable_value(
                &mut self.tab,
                Tab::Headers,
                format!("헤더 ({})", r.headers.len()),
            );
        });
        egui::ScrollArea::both()
            .id_salt("http_response")
            .show(ui, |ui| match self.tab {
                Tab::Body => {
                    ui.add(
                        egui::TextEdit::multiline(&mut shown.text.as_str())
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                }
                Tab::Headers => {
                    egui::Grid::new("http_headers")
                        .striped(true)
                        .show(ui, |ui| {
                            for (name, value) in &r.headers {
                                ui.monospace(name);
                                ui.add(
                                    egui::Label::new(egui::RichText::new(value).monospace()).wrap(),
                                );
                                ui.end_row();
                            }
                        });
                }
            });
    }
}

fn size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// 엔드포인트의 요청 블록. 파일에 이미 있으면 그 요청 줄만 찾는다.
pub fn request_block(controller: &str, path: &str, methods: &str) -> (String, String) {
    let method = methods
        .split(',')
        .map(str::trim)
        .find(|m| !m.is_empty())
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let line = format!("{method} {{{{baseUrl}}}}{path}");
    let mut block = format!("### {controller} {path}\n{line}\n");
    if matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
        block.push_str("Content-Type: application/x-www-form-urlencoded\n\n");
    }
    (block, line)
}

/// 컨트롤러 이름으로 만든 파일 이름(경로 문자를 뺀다).
pub fn request_file_name(controller: &str) -> String {
    let name: String = controller
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_matches('.');
    format!("{}.http", if name.is_empty() { "requests" } else { name })
}

/// 환경 파일이 없을 때 만드는 견본.
pub const ENV_TEMPLATE: &str =
    "{\n  \"dev\": {\n    \"baseUrl\": \"http://localhost:8080\"\n  }\n}\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 엔드포인트로_요청_블록과_파일_이름을_만든다() {
        let (block, line) = request_block("BoardController", "/board/save.do", "post,GET");
        assert_eq!(line, "POST {{baseUrl}}/board/save.do");
        assert_eq!(
            block,
            "### BoardController /board/save.do\nPOST {{baseUrl}}/board/save.do\nContent-Type: application/x-www-form-urlencoded\n\n"
        );
        assert_eq!(request_block("C", "/a", "").1, "GET {{baseUrl}}/a");
        assert_eq!(
            run_lines(&format!(
                "@x = 1\n{block}\n{}",
                request_block("C", "/a", "").0
            )),
            [2, 7]
        );
        assert_eq!(request_file_name("BoardController"), "BoardController.http");
        assert_eq!(request_file_name("../a/b c"), "_a_b_c.http");
        assert_eq!(request_file_name("(이름 없음)"), "_이름_없음_.http");
        assert_eq!(request_file_name(".."), "requests.http");
        assert!(is_http_file(Path::new("a/B.HTTP")) && is_http_file(Path::new("x.rest")));
        assert!(!is_http_file(Path::new("x.html")));
        assert_eq!(
            (size(10), size(2048), size(3 << 20)),
            ("10 B".into(), "2.0 KB".into(), "3.0 MB".into())
        );
    }
}
