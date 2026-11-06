//! HTTP 요청 파일(`.http`) 실행과 "HTTP 응답" 창. 요청은 따로 스레드에서 보내고 응답 처리 스크립트도
//! 그 스레드에서 돌린다. 쿠키, 스크립트가 정한 변수, 요청 기록은 앱이 켜진 동안 둔다(egov 로그인 뒤
//! 이어 보내는 요청에 세션 쿠키가 붙고, 받은 토큰을 다음 요청의 `{{token}}`에 쓴다).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;
use ferrite_http::script::{self, Outcome};
use ferrite_http::{Client, CookieJar, Environments, Globals, Response, env};

/// 응답 창에 보이는 본문 글의 상한. 더 길면 앞만 보인다(egui 글 상자가 한 프레임에 다 배치한다).
const SHOWN_TEXT: usize = 512 * 1024;

/// 요청 기록의 수. 기록은 디스크에 남기지 않는다(채운 토큰·세션 값이 들 수 있다).
const HISTORY: usize = 30;

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
    /// 기록의 요청을 지금 파일에서 찾아([`locate`]) 다시 보내거나(`send`) 그 줄로 간다.
    History {
        path: PathBuf,
        line: usize,
        name: Option<String>,
        send: bool,
    },
    /// 엔드포인트로 `http/<컨트롤러>.http`에 요청을 만든다.
    Make {
        controller: String,
        path: String,
        methods: String,
    },
}

/// 기록의 요청이 지금 파일에서 든 줄. 이름이 있으면 이름으로 찾아 위에 줄을 더해도 따라가고, 없거나
/// 찾지 못하면 보낸 때의 줄이다.
pub fn locate(text: &str, line: usize, name: Option<&str>) -> usize {
    name.and_then(|name| {
        ferrite_http::parse(text)
            .requests
            .into_iter()
            .find(|r| r.name.as_deref() == Some(name))
            .map(|r| r.line)
    })
    .unwrap_or(line)
}

/// 보낸 요청과 그 결과.
struct Sent {
    title: String,
    method: String,
    url: String,
    env: Option<String>,
    /// 요청이 든 파일과 줄(0부터), 이름. 기록에서 다시 보내거나 그 줄로 갈 때 쓴다.
    path: PathBuf,
    line: usize,
    name: Option<String>,
    /// 보낸 차례(1부터). 같은 요청을 여러 번 보내도 기록에서 가려 본다.
    number: usize,
    at: Instant,
    /// 아직 받는 중이면 None.
    result: Option<Result<Shown, String>>,
    /// `>> 경로`로 응답 본문을 쓴 파일, 또는 쓰지 못한 까닭.
    saved: Option<Result<PathBuf, String>>,
    /// 응답 처리 스크립트를 돌린 결과. 스크립트가 없거나 응답을 받지 못했으면 None.
    script: Option<Outcome>,
    /// 요청 전 스크립트(`< {% %}`)가 있었다(돌리지 않는다).
    pre_script: bool,
}

/// 보내는 스레드가 돌려주는 것. 쿠키 저장소와 변수는 그 스레드가 가졌다가 돌려준다.
struct Received {
    result: Result<Response, String>,
    jar: CookieJar,
    globals: Globals,
    saved: Option<Result<PathBuf, String>>,
    script: Option<Outcome>,
}

impl Sent {
    /// 기록 목록의 한 줄: 결과, 메서드, 이름, 보낸 때.
    fn label(&self) -> String {
        let outcome = match &self.result {
            None => "보내는 중".to_owned(),
            Some(Ok(shown)) => shown.response.status.to_string(),
            Some(Err(_)) => "오류".to_owned(),
        };
        format!(
            "#{} {outcome} {} {} · {}",
            self.number,
            self.method,
            self.title,
            ago(self.at.elapsed())
        )
    }
}

/// 받은 응답과, 보일 본문 글(길면 앞만).
struct Shown {
    /// 본문(`body`·`text`)은 비운다. 기록에 여럿 두므로 보일 글만 남긴다(본문은 20MB까지 받는다).
    response: Response,
    text: String,
    /// 받은 본문의 바이트 수.
    bytes: usize,
}

impl Shown {
    fn new(mut response: Response) -> Self {
        let bytes = response.body.len();
        response.body = Vec::new();
        let full = std::mem::take(&mut response.text);
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
            format!("(이진 본문 {}, {content_type})", size(bytes))
        } else if full.len() > SHOWN_TEXT {
            let end = (0..=SHOWN_TEXT)
                .rev()
                .find(|&i| full.is_char_boundary(i))
                .unwrap_or(0);
            format!(
                "{}\n\n… (본문이 커서 앞 {}만 보입니다. 전체 {})",
                &full[..end],
                size(SHOWN_TEXT),
                size(full.len())
            )
        } else {
            full
        };
        Self {
            response,
            text,
            bytes,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Tab {
    #[default]
    Body,
    Headers,
    Script,
}

#[derive(Default)]
pub struct HttpPanel {
    /// 처음 보낼 때 만든다(OS 인증서 저장소를 읽는다).
    client: Option<Arc<Client>>,
    /// 보내는 동안에는 보내는 스레드가 가졌다가 돌려준다.
    jar: CookieJar,
    /// 응답 처리 스크립트가 `client.global.set`으로 정한 변수. 쿠키처럼 보내는 스레드가 가졌다 돌려준다.
    globals: Globals,
    /// 고른 환경. 환경 파일에 없으면 첫 환경을 고른다.
    env: Option<String>,
    envs: Environments,
    env_errors: Vec<String>,
    /// 개인 환경 파일을 `.gitignore`가 빼지 않는 프로젝트 루트(경고하고 더하기를 권한다).
    unignored: Option<PathBuf>,
    /// 보낸 요청, 새것이 앞. 받는 중인 요청은 늘 맨 앞이다(받는 동안에는 새로 보내거나 지우지 않는다).
    history: VecDeque<Sent>,
    /// 창에 보이는 기록.
    selected: usize,
    /// 지금까지 보낸 수(기록을 지워도 이어 센다).
    sent_count: usize,
    rx: Option<Receiver<Received>>,
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
            path: path.to_path_buf(),
            line: request.line,
            name: request.name.clone(),
            number: self.sent_count + 1,
            at: Instant::now(),
            result: None,
            saved: None,
            script: None,
            pre_script: request.pre_script,
        };
        match ferrite_http::prepare(&file, request, env, &self.globals, dir) {
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
                let mut globals = std::mem::take(&mut self.globals);
                let (tx, rx) = mpsc::channel();
                let ctx = ctx.clone();
                let spawned = std::thread::Builder::new()
                    .name("http-request".into())
                    .spawn(move || {
                        let result = client.send(&prepared, &mut jar);
                        let saved = match (&result, &prepared.save) {
                            (Ok(response), Some((path, overwrite))) => Some(
                                ferrite_http::save_body(path, *overwrite, &response.body)
                                    .map_err(|e| format!("{} ({e})", path.display())),
                            ),
                            _ => None,
                        };
                        let script = match &result {
                            Ok(response) if !prepared.scripts.is_empty() => {
                                Some(script::run(&prepared.scripts, response, &mut globals))
                            }
                            _ => None,
                        };
                        let _ = tx.send(Received {
                            result,
                            jar,
                            globals,
                            saved,
                            script,
                        });
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
        self.sent_count += 1;
        self.history.push_front(sent);
        self.history.truncate(HISTORY);
        self.selected = 0;
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    /// 가장 최근에 보낸 요청의 응답을 받았다.
    #[cfg(test)]
    pub fn done(&self) -> bool {
        self.history.front().is_some_and(|s| s.result.is_some())
    }

    #[cfg(test)]
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    #[cfg(test)]
    pub fn globals(&self) -> &Globals {
        &self.globals
    }

    /// 창에 보이는 기록의 본문.
    #[cfg(test)]
    pub fn body(&self) -> Option<&str> {
        match &self.history.get(self.selected)?.result {
            Some(Ok(shown)) => Some(&shown.text),
            _ => None,
        }
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        let (received, saved, script) = match rx.try_recv() {
            Ok(received) => {
                self.jar = received.jar;
                self.globals = received.globals;
                (
                    received.result.map(Shown::new),
                    received.saved,
                    received.script,
                )
            }
            Err(TryRecvError::Empty) => return,
            // 스레드가 패닉했다. 쿠키와 변수는 잃는다.
            Err(TryRecvError::Disconnected) => (Err("보내다가 멈췄습니다".into()), None, None),
        };
        self.rx = None;
        if let Some(sent) = self.history.front_mut() {
            sent.result = Some(received);
            sent.saved = saved;
            sent.script = script;
        }
    }

    #[cfg(test)]
    pub fn saved(&self) -> Option<&Result<PathBuf, String>> {
        self.history.get(self.selected)?.saved.as_ref()
    }

    /// 창을 그린다. 기록에서 다시 보내거나 요청으로 가는 일을 돌려준다.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Task> {
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
            let globals = self.globals.len();
            if globals > 0 {
                let names: Vec<&str> = self.globals.keys().map(String::as_str).collect();
                if ui
                    .add_enabled(
                        !running,
                        egui::Button::new(format!("변수 {globals}개 지우기")),
                    )
                    .on_hover_text(format!(
                        "응답 스크립트가 정한 변수(client.global): {}",
                        names.join(", ")
                    ))
                    .clicked()
                {
                    self.globals.clear();
                }
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
        let mut task = None;
        if !self.history.is_empty() {
            ui.horizontal_wrapped(|ui| {
                let label = ui.label("기록");
                egui::ComboBox::from_id_salt("http_history")
                    .selected_text(self.history[self.selected].label())
                    .show_ui(ui, |ui| {
                        for (i, sent) in self.history.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, i, sent.label())
                                .on_hover_text(format!(
                                    "{} {}\n{}:{}",
                                    sent.method,
                                    sent.url,
                                    sent.path.display(),
                                    sent.line + 1
                                ));
                        }
                    })
                    .response
                    .labelled_by(label.id);
                let sent = &self.history[self.selected];
                let revisit = |send| Task::History {
                    path: sent.path.clone(),
                    line: sent.line,
                    name: sent.name.clone(),
                    send,
                };
                if ui
                    .add_enabled(!running, egui::Button::new("다시 보내기"))
                    .clicked()
                {
                    task = Some(revisit(true));
                }
                if ui.button("요청으로 가기").clicked() {
                    task = Some(revisit(false));
                }
                if ui
                    .add_enabled(!running, egui::Button::new("기록 지우기"))
                    .clicked()
                {
                    self.history.clear();
                    self.selected = 0;
                }
            });
        }
        let Some(sent) = self.history.get(self.selected) else {
            ui.weak(".http 파일에서 요청 줄의 ▶를 누르거나 Ctrl+Enter로 보냅니다");
            return task;
        };
        ui.strong(&sent.title);
        let env = sent
            .env
            .as_deref()
            .map(|e| format!(" · 환경 {e}"))
            .unwrap_or_default();
        ui.weak(format!("{} {}{env}", sent.method, sent.url));
        if sent.pre_script {
            ui.weak("요청 전 스크립트(< {% %})는 돌리지 않습니다");
        }
        match &sent.saved {
            Some(Ok(path)) => {
                ui.weak(format!("응답 본문을 저장했습니다: {}", path.display()));
            }
            Some(Err(e)) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("응답 본문을 저장하지 못했습니다: {e}"),
                );
            }
            None => {}
        }
        let shown = match &sent.result {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("보내는 중…");
                });
                return task;
            }
            Some(Err(e)) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
                return task;
            }
            Some(Ok(shown)) => shown,
        };
        let r = &shown.response;
        ui.horizontal_wrapped(|ui| {
            let color = match r.status {
                200..=299 => PASSED,
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
                size(shown.bytes),
                r.charset
            ));
        });
        for (status, url) in &r.redirects {
            ui.weak(format!("→ {status} {url}"));
        }
        if let Some(outcome) = &sent.script {
            let failed = outcome.tests.iter().filter(|t| t.error.is_some()).count();
            let total = outcome.tests.len();
            if outcome.error.is_some() {
                ui.colored_label(ui.visuals().error_fg_color, "응답 스크립트 오류");
            } else if failed > 0 {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("테스트 {total}개 중 {failed}개 실패"),
                );
            } else if total > 0 {
                ui.colored_label(PASSED, format!("테스트 {total}개 통과"));
            }
        }
        if sent.script.is_none() && self.tab == Tab::Script {
            self.tab = Tab::Body;
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Body, "본문");
            ui.selectable_value(
                &mut self.tab,
                Tab::Headers,
                format!("헤더 ({})", r.headers.len()),
            );
            if sent.script.is_some() {
                ui.selectable_value(&mut self.tab, Tab::Script, "스크립트");
            }
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
                Tab::Script => {
                    if let Some(outcome) = &sent.script {
                        script_ui(ui, outcome);
                    }
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
        task
    }
}

/// 2xx 상태와 통과한 테스트의 색.
const PASSED: egui::Color32 = egui::Color32::from_rgb(60, 170, 90);

/// 응답 스크립트의 테스트(통과·실패와 까닭), 오류, 로그.
fn script_ui(ui: &mut egui::Ui, outcome: &Outcome) {
    if let Some(error) = &outcome.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    for test in &outcome.tests {
        ui.horizontal_wrapped(|ui| {
            match &test.error {
                None => ui.colored_label(PASSED, "통과"),
                Some(_) => ui.colored_label(ui.visuals().error_fg_color, "실패"),
            };
            ui.label(&test.name);
            if let Some(error) = &test.error {
                ui.weak(error);
            }
        });
    }
    if !outcome.log.is_empty() {
        ui.separator();
        for line in &outcome.log {
            ui.monospace(line);
        }
    }
    if outcome.error.is_none() && outcome.tests.is_empty() && outcome.log.is_empty() {
        ui.weak("테스트와 로그가 없습니다");
    }
}

/// 보낸 지 얼마나 되었나. 기록은 앱이 켜진 동안만 두므로 시각 대신 지난 시간을 보인다.
fn ago(elapsed: Duration) -> String {
    match elapsed.as_secs() {
        0..60 => "방금".to_owned(),
        s @ 60..3600 => format!("{}분 전", s / 60),
        s => format!("{}시간 전", s / 3600),
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

    #[test]
    fn 기록의_요청을_이름으로_다시_찾는다() {
        let text = "# 위에 더한 줄\n### 로그인\nGET /a\n\n### 내 정보\nGET /b\n";
        assert_eq!(locate(text, 3, Some("내 정보")), 5, "줄이 밀려도 이름으로");
        assert_eq!(locate(text, 2, None), 2, "이름이 없으면 그 줄");
        assert_eq!(locate(text, 7, Some("없음")), 7, "못 찾으면 그 줄");
        assert_eq!(
            [0, 59, 60, 3599, 7200].map(|s| ago(Duration::from_secs(s))),
            ["방금", "방금", "1분 전", "59분 전", "2시간 전"]
        );
    }
}
