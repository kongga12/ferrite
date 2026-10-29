//! "테스트" 창: 실행 구성의 테스트 명령을 직접 띄워 출력을 보이고, 끝나면 보고서(JUnit XML, `go test
//! -json`)를 읽어 결과 트리로 보인다. 실패를 누르면 스택의 테스트 줄로 가고, 실패한 것만 다시 돌린다.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Instant, SystemTime};

use eframe::egui;
use ferrite_core::run::RunConfig;
use ferrite_core::test_report::{self, Case, Frame, Outcome, Tool};
use serde_json::Value;

/// 출력 창에 남길 최대 글자 수(넘으면 앞에서 버린다).
const OUTPUT_LIMIT: usize = 1 << 20;

/// pytest·CTest 보고서를 쓰는 폴더(이 앱 프로세스의 것). 앱을 닫을 때 지운다.
pub fn reports_dir() -> PathBuf {
    std::env::temp_dir().join(format!("ferrite-test-reports-{}", std::process::id()))
}

/// 앱이 할 일.
pub enum TestAction {
    /// 이 파일의 이 줄(0부터)을 보인다.
    Go(PathBuf, usize),
}

enum Msg {
    /// 출력 한 줄(표준 출력이면 true).
    Line(String, bool),
    Done(Option<i32>),
}

struct Running {
    rx: Receiver<Msg>,
    pid: u32,
    tool: Tool,
    /// pytest·CTest가 보고서를 쓰는 폴더.
    report: PathBuf,
    started: (SystemTime, Instant),
    /// 실패한 것만 다시 돌리는 중(결과를 앞 결과에 덮어 쓴다).
    rerun: bool,
    /// `go test -json`의 표준 출력.
    json: String,
    /// 사용자가 멈췄다(끝난 테스트의 결과만 있다).
    stopped: bool,
}

/// 창 아래쪽에 보일 것. 창이 낮아도 다 보이게 하나씩 보인다.
#[derive(Clone, Copy, Default, PartialEq)]
enum View {
    #[default]
    Results,
    Output,
}

#[derive(Default)]
pub struct TestPanel {
    view: View,
    /// 테스트 구성을 찾은 프로젝트 루트(바뀌면 다시 찾는다).
    root: Option<PathBuf>,
    configs: Vec<RunConfig>,
    chosen: usize,
    running: Option<Running>,
    /// 마지막으로 돌린 구성과 도구, 돌린 폴더(실패만 다시, 줄 찾기).
    last: Option<(RunConfig, Tool, PathBuf)>,
    cases: Vec<Case>,
    selected: Option<usize>,
    output: String,
    status: Option<(String, bool)>,
}

impl TestPanel {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// 프로젝트의 테스트 구성(자동 감지와 사용자 구성 중 테스트 도구로 알 수 있는 것).
    fn refresh(&mut self, root: &Path) {
        let mut configs = ferrite_core::run::detect(root);
        configs.extend(ferrite_core::run::load_custom(root).unwrap_or_default());
        configs.retain(|c| Tool::of(&c.command).is_some());
        self.configs = configs;
        self.chosen = self.chosen.min(self.configs.len().saturating_sub(1));
        self.root = Some(root.to_path_buf());
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        root: &Path,
        config: RunConfig,
        rerun: Option<String>,
    ) {
        let Some(tool) = Tool::of(&config.command) else {
            return;
        };
        let dir = config.working_directory(root);
        let report = reports_dir().join(format!(
            "{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        if let Err(e) = std::fs::create_dir_all(&report) {
            self.status = Some((format!("보고서 폴더를 만들지 못했습니다: {e}"), true));
            return;
        }
        let line =
            test_report::with_report(tool, rerun.as_deref().unwrap_or(&config.command), &report);
        let mut command = crate::debug_ui::shell(&line);
        command
            .current_dir(&dir)
            .envs(&config.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        ferrite_lsp::client::hide_console(&mut command);
        // 멈출 때 셸이 띄운 자식(Maven의 JVM 등)까지 끝내도록 프로세스 묶음을 따로 만든다.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.status = Some((format!("테스트를 띄우지 못했습니다: {e}"), true));
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        let readers = [
            child
                .stdout
                .take()
                .map(|o| read_lines(o, true, tx.clone(), ctx.clone())),
            child
                .stderr
                .take()
                .map(|e| read_lines(e, false, tx.clone(), ctx.clone())),
        ];
        let pid = child.id();
        let wake = ctx.clone();
        thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code());
            // 출력을 다 읽은 뒤에 끝을 알린다.
            for reader in readers.into_iter().flatten() {
                let _ = reader.join();
            }
            let _ = tx.send(Msg::Done(code));
            wake.request_repaint();
        });
        self.output = format!("> {line}\n");
        self.view = View::Output;
        self.status = Some((format!("실행 중: {}", config.name), false));
        if rerun.is_none() {
            self.cases.clear();
            self.selected = None;
        }
        self.running = Some(Running {
            rx,
            pid,
            tool,
            report: report.clone(),
            started: (SystemTime::now(), Instant::now()),
            rerun: rerun.is_some(),
            json: String::new(),
            stopped: false,
        });
        self.last = Some((config, tool, dir));
    }

    /// 돌고 있는 테스트를 끝낸다(셸이 띄운 자식까지).
    pub fn stop(&mut self) {
        if let Some(running) = &mut self.running {
            running.stopped = true;
            kill_tree(running.pid);
        }
    }

    fn poll(&mut self, root: &Path) {
        let Some(running) = &mut self.running else {
            return;
        };
        let mut done = None;
        loop {
            match running.rx.try_recv() {
                Ok(Msg::Line(line, stdout)) => {
                    // `go test -json`의 표준 출력은 사건(JSON)이다. 모아 두고 그 안의 출력만 보인다.
                    let shown = if running.tool == Tool::Go && stdout {
                        running.json.push_str(&line);
                        running.json.push('\n');
                        match serde_json::from_str::<Value>(&line) {
                            Ok(event) => event
                                .get("Output")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            Err(_) => format!("{line}\n"),
                        }
                    } else {
                        format!("{line}\n")
                    };
                    push_limited(&mut self.output, &shown);
                }
                Ok(Msg::Done(code)) => {
                    done = Some(code);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = Some(None);
                    break;
                }
            }
        }
        let Some(code) = done else {
            return;
        };
        let Some(running) = self.running.take() else {
            return;
        };
        let found = match running.tool {
            Tool::Go => test_report::parse_go_json(&running.json),
            tool => {
                let mut cases = Vec::new();
                for file in
                    test_report::report_files(tool, root, &running.report, running.started.0)
                {
                    let text = std::fs::read(&file)
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default();
                    match test_report::parse_junit(&text) {
                        Ok(found) => cases.extend(found),
                        Err(e) => push_limited(
                            &mut self.output,
                            &format!("[보고서를 읽지 못했습니다: {}: {e}]\n", file.display()),
                        ),
                    }
                }
                cases
            }
        };
        let found_any = !found.is_empty();
        if found_any {
            self.view = View::Results;
        }
        if running.rerun {
            for case in found {
                match self
                    .cases
                    .iter_mut()
                    .find(|c| c.suite == case.suite && c.name == case.name)
                {
                    Some(old) => *old = case,
                    None => self.cases.push(case),
                }
            }
        } else {
            self.cases = found;
            self.selected = self.cases.iter().position(|c| c.outcome == Outcome::Failed);
        }
        let count = |o| self.cases.iter().filter(|c| c.outcome == o).count();
        let (passed, failed, skipped) = (
            count(Outcome::Passed),
            count(Outcome::Failed),
            count(Outcome::Skipped),
        );
        let seconds = running.started.1.elapsed().as_secs_f64();
        let code = code.map_or_else(|| "끊김".to_owned(), |c| c.to_string());
        let stopped = if running.stopped { "멈춤 · " } else { "" };
        self.status = Some(if found_any {
            (
                format!(
                    "{stopped}통과 {passed} · 실패 {failed} · 건너뜀 {skipped} · {seconds:.1}초 · 종료 코드 {code}"
                ),
                failed > 0,
            )
        } else {
            (
                format!(
                    "{stopped}결과 보고서가 없습니다(컴파일 실패 등, 출력을 보세요) · 종료 코드 {code}"
                ),
                true,
            )
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, root: Option<&Path>) -> Option<TestAction> {
        let Some(root) = root else {
            ui.weak("폴더를 열면 프로젝트의 테스트 명령(mvn test·gradle test·pytest·ctest·go test)을 돌립니다");
            return None;
        };
        if self.root.as_deref() != Some(root) {
            self.refresh(root);
        }
        self.poll(root);
        let running = self.is_running();
        let mut run = None;
        ui.horizontal(|ui| {
            let names: Vec<String> = self.configs.iter().map(|c| c.name.clone()).collect();
            let label = ui.label("구성");
            egui::ComboBox::from_id_salt("test_config")
                .selected_text(
                    names
                        .get(self.chosen)
                        .cloned()
                        .unwrap_or_else(|| "(테스트 구성 없음)".into()),
                )
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.chosen, i, name);
                    }
                })
                .response
                .labelled_by(label.id);
            if ui
                .add_enabled(!running && !names.is_empty(), egui::Button::new("실행"))
                .clicked()
            {
                run = Some(false);
            }
            let failed: Vec<&Case> = self
                .cases
                .iter()
                .filter(|c| c.outcome == Outcome::Failed)
                .collect();
            if ui
                .add_enabled(
                    !running && !failed.is_empty() && self.last.is_some(),
                    egui::Button::new("실패만 다시"),
                )
                .clicked()
            {
                run = Some(true);
            }
            if ui.add_enabled(running, egui::Button::new("멈춤")).clicked() {
                self.stop();
            }
            if ui
                .small_button("새로 고침")
                .on_hover_text("실행 구성을 다시 찾는다")
                .clicked()
            {
                self.refresh(root);
            }
            if running {
                ui.spinner();
            }
            ui.separator();
            ui.selectable_value(&mut self.view, View::Results, "결과");
            ui.selectable_value(&mut self.view, View::Output, "출력");
        });
        match run {
            Some(false) => {
                self.refresh(root);
                if let Some(config) = self.configs.get(self.chosen).cloned() {
                    self.start(ui.ctx(), root, config, None);
                }
            }
            Some(true) => {
                if let Some((config, tool, _)) = self.last.clone() {
                    let failed: Vec<&Case> = self
                        .cases
                        .iter()
                        .filter(|c| c.outcome == Outcome::Failed)
                        .collect();
                    if let Some(line) = test_report::rerun(tool, &config.command, &failed) {
                        self.start(ui.ctx(), root, config, Some(line));
                    }
                }
            }
            None => {}
        }
        if let Some((text, bad)) = &self.status {
            let color = if *bad {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().weak_text_color()
            };
            ui.colored_label(color, text);
        }
        ui.separator();

        let mut action = None;
        match self.view {
            View::Results if self.cases.is_empty() => {
                ui.weak("결과가 없습니다. 테스트 구성을 골라 [실행]을 누르세요.");
            }
            View::Results => {
                ui.columns(2, |columns| {
                    egui::ScrollArea::vertical()
                        .id_salt("test_tree")
                        .auto_shrink([false, false])
                        .show(&mut columns[0], |ui| {
                            if let Some(go) = self.tree_ui(ui, root) {
                                action = Some(go);
                            }
                        });
                    egui::ScrollArea::vertical()
                        .id_salt("test_detail")
                        .auto_shrink([false, false])
                        .show(&mut columns[1], |ui| {
                            if let Some(go) = self.detail_ui(ui, root) {
                                action = Some(go);
                            }
                        });
                });
            }
            View::Output => {
                egui::ScrollArea::vertical()
                    .id_salt("test_output_scroll")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.output.as_str())
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
            }
        }
        action
    }

    /// 결과 트리: 묶음(클래스·모듈·패키지)마다 테스트. 실패가 있는 묶음은 펼쳐 둔다. 실패를 두 번 누르면
    /// 테스트 줄로 간다.
    fn tree_ui(&mut self, ui: &mut egui::Ui, root: &Path) -> Option<TestAction> {
        let mut suites: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, case) in self.cases.iter().enumerate() {
            suites.entry(&case.suite).or_default().push(i);
        }
        let mut picked = None;
        let mut jump = None;
        for (suite, indices) in suites {
            let failed = indices
                .iter()
                .filter(|&&i| self.cases[i].outcome == Outcome::Failed)
                .count();
            let title = if failed > 0 {
                egui::RichText::new(format!("{suite} (실패 {failed}/{})", indices.len()))
                    .color(ui.visuals().error_fg_color)
            } else {
                egui::RichText::new(format!("{suite} ({})", indices.len()))
            };
            egui::CollapsingHeader::new(title)
                .id_salt(("test_suite", suite))
                .default_open(failed > 0)
                .show(ui, |ui| {
                    for i in indices {
                        let case = &self.cases[i];
                        ui.horizontal(|ui| {
                            let (word, color) = outcome_label(ui, case.outcome);
                            ui.colored_label(color, word);
                            let time = case
                                .seconds
                                .map(|s| format!("  {s:.3}초"))
                                .unwrap_or_default();
                            let row = ui.selectable_label(
                                self.selected == Some(i),
                                format!("{}{time}", case.name),
                            );
                            if row.clicked() {
                                picked = Some(i);
                            }
                            if row.double_clicked() {
                                jump = Some(i);
                            }
                        });
                    }
                });
        }
        if let Some(i) = picked {
            self.selected = Some(i);
        }
        let i = jump?;
        let case = &self.cases[i];
        let frames = test_report::frames(case);
        let frame = &frames[test_report::test_frame(case, &frames)?];
        self.go(root, frame)
    }

    fn detail_ui(&mut self, ui: &mut egui::Ui, root: &Path) -> Option<TestAction> {
        let case = self.selected.and_then(|i| self.cases.get(i))?.clone();
        ui.strong(&case.name);
        ui.weak(&case.suite);
        ui.horizontal(|ui| {
            let (word, color) = outcome_label(ui, case.outcome);
            ui.colored_label(color, word);
            if let Some(s) = case.seconds {
                ui.weak(format!("{s:.3}초"));
            }
        });
        if let Some(message) = &case.message {
            ui.label(message);
        }
        let frames = test_report::frames(&case);
        let first = test_report::test_frame(&case, &frames);
        let mut go = None;
        if !frames.is_empty() {
            ui.separator();
            ui.weak("스택(눌러서 그 줄로)");
            // 긴 프레임은 한 줄로 자른다(전체는 아래 "전체").
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            for (i, frame) in frames.iter().enumerate() {
                let text = egui::RichText::new(&frame.text).monospace();
                let text = if Some(i) == first {
                    text.strong()
                } else {
                    text
                };
                if ui.selectable_label(false, text).clicked() {
                    go = self.go(root, frame);
                }
            }
        }
        if !case.detail.trim().is_empty() {
            egui::CollapsingHeader::new("전체")
                .id_salt(("test_detail_text", case.suite.as_str(), case.name.as_str()))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut case.detail.as_str())
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                });
        }
        go
    }

    /// 프레임의 파일을 찾아 그 줄로 간다. 못 찾으면 상태에 알린다.
    fn go(&mut self, root: &Path, frame: &Frame) -> Option<TestAction> {
        let dir = self
            .last
            .as_ref()
            .map_or_else(|| root.to_path_buf(), |(_, _, dir)| dir.clone());
        match test_report::resolve(root, &dir, frame) {
            Some(path) => Some(TestAction::Go(
                crate::app::real_path(&path),
                frame.line.saturating_sub(1) as usize,
            )),
            None => {
                self.status = Some((
                    format!("프로젝트에서 {}을(를) 찾지 못했습니다", frame.file),
                    true,
                ));
                None
            }
        }
    }

    /// 시험용: 이 결과를 받은 것처럼 보인다.
    #[cfg(test)]
    pub fn show_results(&mut self, config: RunConfig, dir: PathBuf, cases: Vec<Case>) {
        let tool = Tool::of(&config.command).expect("테스트 명령");
        self.selected = cases.iter().position(|c| c.outcome == Outcome::Failed);
        self.cases = cases;
        self.last = Some((config, tool, dir));
    }
}

fn outcome_label(ui: &egui::Ui, outcome: Outcome) -> (&'static str, egui::Color32) {
    match outcome {
        Outcome::Passed => ("통과", egui::Color32::from_rgb(60, 170, 90)),
        Outcome::Failed => ("실패", ui.visuals().error_fg_color),
        Outcome::Skipped => ("건너뜀", ui.visuals().weak_text_color()),
    }
}

/// 출력을 줄마다 읽어 보낸다. 문자 집합은 줄마다 맞춘다(Windows의 Maven·javac는 CP949로 쓴다).
fn read_lines(
    output: impl Read + Send + 'static,
    stdout: bool,
    tx: Sender<Msg>,
    ctx: egui::Context,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(output);
        let mut bytes = Vec::new();
        while reader.read_until(b'\n', &mut bytes).is_ok_and(|n| n > 0) {
            let line = ferrite_core::text_file::decode(&bytes)
                .map(|c| c.text)
                .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
            if tx
                .send(Msg::Line(
                    line.trim_end_matches(['\r', '\n']).to_owned(),
                    stdout,
                ))
                .is_err()
            {
                return;
            }
            ctx.request_repaint();
            bytes.clear();
        }
    })
}

fn push_limited(output: &mut String, text: &str) {
    output.push_str(text);
    if output.len() > OUTPUT_LIMIT {
        let mut cut = output.len() - OUTPUT_LIMIT;
        while !output.is_char_boundary(cut) {
            cut += 1;
        }
        output.drain(..cut);
    }
}

/// 프로세스와 그 자식을 모두 끝낸다(셸 → Maven → JVM).
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("taskkill");
        command.args(["/T", "/F", "/PID", &pid.to_string()]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("kill");
        command.args(["-TERM", "--", &format!("-{pid}")]);
        command
    };
    ferrite_lsp::client::hide_console(&mut command);
    let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
}
