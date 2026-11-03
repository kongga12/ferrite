//! "에이전트" 창(ACP): 코딩 에이전트를 띄워 대화한다. 답 글·생각·도구 호출·계획을 보이고, 권한 요청에는
//! 단추로 답한다(고칠 diff가 붙으면 보인다). 에이전트가 Ferrite에 파일 쓰기를 맡기면 diff를 보이고
//! [받기]를 눌러야 에디터 버퍼에 적용한다(저장은 사용자가 한다). 파일 읽기 요청은 앱이 열린 버퍼
//! (저장 안 한 내용)나 디스크에서 답한다. 에이전트의 고르기 설정(권한 모드·모델 등)은 창 위에서 바꾼다.

use std::path::{Path, PathBuf};

use eframe::egui;
use ferrite_acp::{Agent, Event, MessageKind, PermissionOption, PlanEntry, ToolCall};
use ferrite_core::settings::AgentSetting;
use serde_json::Value;

/// 기본으로 있는 에이전트(이름, 프로그램, 인자).
const BUILTIN: [(&str, &str, &[&str]); 2] = [
    ("Gemini CLI", "gemini", &["--acp"]),
    (
        "Claude Code",
        "npx",
        &["-y", "@agentclientprotocol/claude-agent-acp"],
    ),
];

/// diff 카드에 보일 줄 수의 한도.
const DIFF_LINES: usize = 400;

enum Entry {
    Message(MessageKind, String),
    Tool(ToolCall),
    Plan(Vec<PlanEntry>),
    Notice(String, bool),
}

struct PermissionAsk {
    id: Value,
    tool: ToolCall,
    options: Vec<PermissionOption>,
    /// 고칠 파일과 diff 줄(받을 때 한 번 계산한다).
    diffs: Vec<(PathBuf, Vec<(char, String)>)>,
}

struct WriteAsk {
    id: Value,
    path: PathBuf,
    content: String,
    /// 새 파일이면 true.
    new_file: bool,
    lines: Vec<(char, String)>,
}

/// 앱이 할 일.
pub enum AgentAction {
    /// 에이전트가 읽으려는 파일(열린 버퍼면 그 내용, 1부터 센 줄과 줄 수).
    Read {
        id: Value,
        path: PathBuf,
        line: Option<usize>,
        limit: Option<usize>,
    },
    /// 에이전트가 쓰려는 파일. 앱이 지금 내용을 [`AgentPanel::propose_write`]로 넘긴다.
    Write {
        id: Value,
        path: PathBuf,
        content: String,
    },
    /// 사용자가 받은 쓰기. 앱이 버퍼에 적용하고 [`AgentPanel::answer_write`]로 답한다.
    Apply {
        id: Value,
        path: PathBuf,
        content: String,
    },
}

#[derive(Default)]
pub struct AgentPanel {
    chosen: usize,
    agent: Option<Agent>,
    name: String,
    /// 세션이 열리면 맞출 권한 모드(설정 `agent_mode`, 맞춘 뒤 비운다).
    start_mode: String,
    entries: Vec<Entry>,
    input: String,
    busy: bool,
    permissions: Vec<PermissionAsk>,
    writes: Vec<WriteAsk>,
}

impl AgentPanel {
    /// 고를 수 있는 에이전트: 기본 둘과 설정의 것(같은 이름이면 설정의 것).
    fn choices(settings: &[AgentSetting]) -> Vec<AgentSetting> {
        let mut list: Vec<AgentSetting> = BUILTIN
            .iter()
            .filter(|(name, ..)| !settings.iter().any(|s| s.name == *name))
            .map(|(name, command, args)| AgentSetting {
                name: (*name).to_owned(),
                command: (*command).to_owned(),
                args: args.iter().map(|a| (*a).to_owned()).collect(),
                env: Default::default(),
            })
            .collect();
        list.extend(settings.iter().cloned());
        list
    }

    fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.entries.push(Entry::Notice(text.into(), error));
    }

    /// 에이전트를 띄운다(프로젝트 루트에서). `mcp`는 세션에 넘길 MCP 서버(Ferrite의 MCP).
    fn start(&mut self, setting: &AgentSetting, root: &Path, mcp: Vec<Value>, mode: &str) {
        let env: Vec<(String, String)> = setting.env.clone().into_iter().collect();
        match Agent::spawn(&setting.command, &setting.args, &env, root, mcp) {
            Ok(agent) => {
                self.connect(agent, &setting.name, mode);
            }
            Err(e) => self.notice(e, true),
        }
    }

    /// 열린 에이전트를 붙인다(시험은 가짜 에이전트를 붙인다). `mode`는 세션이 열리면 맞출 권한 모드.
    pub fn connect(&mut self, agent: Agent, name: &str, mode: &str) {
        self.agent = Some(agent);
        self.name = name.to_owned();
        self.start_mode = mode.trim().to_owned();
        self.busy = false;
        self.notice(format!("{name}을(를) 띄웠습니다. 세션을 여는 중…"), false);
    }

    /// 세션이 열리면 설정의 권한 모드로 바꾼다. 에이전트에 없는 모드면 알리고 그대로 둔다.
    fn apply_start_mode(&mut self) {
        let want = std::mem::take(&mut self.start_mode);
        let Some(agent) = &mut self.agent else {
            return;
        };
        if want.is_empty() {
            return;
        }
        let mode = agent
            .config()
            .iter()
            .find(|o| o.category.as_deref() == Some("mode"));
        let problem = match mode {
            None => Some(format!(
                "에이전트가 권한 모드를 고르게 하지 않아 agent_mode = \"{want}\"를 쓰지 않았습니다"
            )),
            Some(mode) if mode.current == want => None,
            Some(mode) if mode.values.iter().any(|v| v.value == want) => {
                let id = mode.id.clone();
                agent.set_config(&id, &want).err()
            }
            Some(mode) => {
                let have: Vec<&str> = mode.values.iter().map(|v| v.value.as_str()).collect();
                Some(format!(
                    "에이전트에 \"{want}\" 모드가 없어 그대로 둡니다(있는 것: {})",
                    have.join(", ")
                ))
            }
        };
        if let Some(problem) = problem {
            self.notice(problem, true);
        }
    }

    /// 에이전트를 끝낸다(남은 요청은 물린다).
    fn stop(&mut self) {
        self.clear_asks("멈췄습니다");
        self.agent = None;
        self.busy = false;
    }

    fn clear_asks(&mut self, reason: &str) {
        if let Some(agent) = &mut self.agent {
            for ask in self.permissions.drain(..) {
                agent.answer_permission(&ask.id, None);
            }
            for ask in self.writes.drain(..) {
                agent.answer_write(&ask.id, Err(reason.to_owned()));
            }
        } else {
            self.permissions.clear();
            self.writes.clear();
        }
    }

    pub fn answer_read(&mut self, id: &Value, content: Result<String, String>) {
        if let Some(agent) = &mut self.agent {
            agent.answer_read(id, content);
        }
    }

    pub fn answer_write(&mut self, id: &Value, result: Result<(), String>) {
        if let Some(agent) = &mut self.agent {
            agent.answer_write(id, result);
        }
    }

    /// 쓰기 제안을 diff 카드로 보인다. `old`는 지금 내용(새 파일이면 None).
    pub fn propose_write(
        &mut self,
        id: Value,
        path: PathBuf,
        old: Option<String>,
        content: String,
    ) {
        let lines = ferrite_core::git::diff_lines(old.as_deref().unwrap_or_default(), &content);
        self.writes.push(WriteAsk {
            id,
            path,
            content,
            new_file: old.is_none(),
            lines,
        });
    }

    /// 에이전트에서 온 것을 처리한다. 매 프레임 부른다. 앱이 답할 읽기·쓰기를 돌려준다.
    pub fn poll(&mut self) -> Vec<AgentAction> {
        let Some(agent) = &mut self.agent else {
            return Vec::new();
        };
        let events = agent.poll();
        let mut actions = Vec::new();
        for event in events {
            match event {
                Event::Ready => {
                    self.notice("세션이 열렸습니다. 글을 보내세요.", false);
                    self.apply_start_mode();
                }
                Event::Message(kind, text) => match self.entries.last_mut() {
                    Some(Entry::Message(last, body)) if *last == kind => body.push_str(&text),
                    _ => self.entries.push(Entry::Message(kind, text)),
                },
                Event::ToolCall(call) => self.entries.push(Entry::Tool(call)),
                Event::ToolCallUpdate(update) => {
                    let found = self.entries.iter_mut().rev().find_map(|e| match e {
                        Entry::Tool(call) if call.id == update.id => Some(call),
                        _ => None,
                    });
                    match found {
                        Some(call) => call.merge(update),
                        None => self.entries.push(Entry::Tool(update)),
                    }
                }
                Event::Plan(entries) => {
                    // 계획은 새로 온 것이 앞의 것을 바꾼다.
                    self.entries.retain(|e| !matches!(e, Entry::Plan(_)));
                    self.entries.push(Entry::Plan(entries));
                }
                Event::Permission { id, tool, options } => {
                    let diffs = tool
                        .diffs
                        .iter()
                        .map(|d| {
                            let old = d.old_text.as_deref().unwrap_or_default();
                            (
                                d.path.clone(),
                                ferrite_core::git::diff_lines(old, &d.new_text),
                            )
                        })
                        .collect();
                    self.permissions.push(PermissionAsk {
                        id,
                        tool,
                        options,
                        diffs,
                    });
                }
                Event::ReadFile {
                    id,
                    path,
                    line,
                    limit,
                } => actions.push(AgentAction::Read {
                    id,
                    path,
                    line,
                    limit,
                }),
                Event::WriteFile { id, path, content } => {
                    actions.push(AgentAction::Write { id, path, content });
                }
                Event::TurnEnded(reason) => {
                    self.busy = false;
                    if reason != "end_turn" {
                        let why = match reason.as_str() {
                            "cancelled" => "멈췄습니다",
                            "max_tokens" => "최대 토큰에 닿았습니다",
                            "max_turn_requests" => "한 차례의 요청 수 한도에 닿았습니다",
                            "refusal" => "에이전트가 거절했습니다",
                            _ => "끝났습니다",
                        };
                        self.notice(format!("차례가 끝났습니다: {why}"), false);
                    }
                }
                Event::Error(e) => self.notice(e, true),
                Event::Exited(tail) => {
                    let name = self.name.clone();
                    self.notice(
                        if tail.trim().is_empty() {
                            format!("{name}이(가) 끝났습니다")
                        } else {
                            format!("{name}이(가) 끝났습니다: {tail}")
                        },
                        true,
                    );
                    self.agent = None;
                    self.busy = false;
                    self.permissions.clear();
                    self.writes.clear();
                    break;
                }
            }
        }
        actions
    }

    fn send(&mut self) {
        let text = self.input.trim().to_owned();
        let Some(agent) = &mut self.agent else {
            return;
        };
        if text.is_empty() {
            return;
        }
        match agent.prompt(&text) {
            Ok(()) => {
                self.entries.push(Entry::Message(MessageKind::User, text));
                self.input.clear();
                self.busy = true;
            }
            Err(e) => self.notice(e, true),
        }
    }

    /// 창을 그린다. `root`가 없으면 시작할 수 없다. `mode`는 설정의 처음 권한 모드(`agent_mode`),
    /// `mcp`는 세션에 넘길 Ferrite MCP 서버.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        root: Option<&Path>,
        settings: &[AgentSetting],
        mode: &str,
        mcp: Option<Value>,
    ) -> Option<AgentAction> {
        let choices = Self::choices(settings);
        self.chosen = self.chosen.min(choices.len().saturating_sub(1));
        let ready = self.agent.as_ref().is_some_and(Agent::is_ready);
        ui.horizontal(|ui| {
            let label = ui.label("에이전트");
            ui.add_enabled_ui(self.agent.is_none(), |ui| {
                egui::ComboBox::from_id_salt("agent_choice")
                    .selected_text(choices.get(self.chosen).map_or("", |c| c.name.as_str()))
                    .show_ui(ui, |ui| {
                        for (i, choice) in choices.iter().enumerate() {
                            ui.selectable_value(&mut self.chosen, i, &choice.name);
                        }
                    })
                    .response
                    .labelled_by(label.id);
            });
            if self.agent.is_none() {
                if ui
                    .add_enabled(root.is_some(), egui::Button::new("시작"))
                    .on_disabled_hover_text("폴더를 열고 시작하세요")
                    .clicked()
                    && let (Some(root), Some(choice)) = (root, choices.get(self.chosen))
                {
                    self.start(choice, root, mcp.into_iter().collect(), mode);
                }
            } else if ui.button("끝내기").clicked() {
                self.stop();
                let name = self.name.clone();
                self.notice(format!("{name}을(를) 끝냈습니다"), false);
            }
            if ui
                .add_enabled(!self.entries.is_empty(), egui::Button::new("지우기"))
                .clicked()
            {
                self.entries.clear();
            }
        });
        self.config_ui(ui);
        if self.agent.is_none() && self.entries.is_empty() {
            ui.weak(
                "에이전트는 프로젝트 폴더에서 돌고, 파일을 고치기 전에 diff를 보이고 묻습니다. \
                 묻는지는 에이전트의 권한 모드에 달려 있어 세션이 열리면 창 위에서 고르고, \
                 처음 모드는 settings.toml의 agent_mode로 정합니다. \
                 Ferrite는 에이전트를 설치하거나 로그인하지 않습니다(터미널에서 한 번 실행해 로그인하세요). \
                 다른 ACP 에이전트는 settings.toml의 [[agent]]로 더합니다.",
            );
        }
        ui.separator();
        let mut action = None;
        let input_height = 90.0;
        egui::ScrollArea::vertical()
            .id_salt("agent_transcript")
            .auto_shrink(false)
            .stick_to_bottom(true)
            .max_height((ui.available_height() - input_height).max(80.0))
            .show(ui, |ui| {
                for entry in &self.entries {
                    entry_ui(ui, entry);
                }
                action = self.asks_ui(ui);
            });
        ui.separator();
        let label = ui.weak("보낼 글");
        let field = ui
            .add(
                egui::TextEdit::multiline(&mut self.input)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text("에이전트에게 보낼 글 (Ctrl+Enter)"),
            )
            .labelled_by(label.id);
        let send_key = field.has_focus()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        ui.horizontal(|ui| {
            let can_send = ready && !self.busy && !self.input.trim().is_empty();
            if ui
                .add_enabled(can_send, egui::Button::new("보내기"))
                .clicked()
                || (send_key && can_send)
            {
                self.send();
            }
            if ui
                .add_enabled(self.busy, egui::Button::new("멈춤"))
                .clicked()
                && let Some(agent) = &mut self.agent
            {
                agent.cancel();
                self.clear_asks("멈췄습니다");
            }
            if self.busy {
                ui.weak("에이전트가 일하는 중…");
            } else if self.agent.is_some() && !ready {
                ui.weak("세션을 여는 중…");
            }
        });
        action
    }

    /// 에이전트의 고르기 설정(권한 모드·모델 등)을 에이전트가 준 차례대로 보이고 바꾼다.
    fn config_ui(&mut self, ui: &mut egui::Ui) {
        let Some(agent) = self.agent.as_mut().filter(|a| a.is_ready()) else {
            return;
        };
        if agent.config().is_empty() {
            return;
        }
        let mut change = None;
        ui.horizontal_wrapped(|ui| {
            for option in agent.config() {
                let label = ui.label(&option.name);
                let current = option.values.iter().find(|v| v.value == option.current);
                let combo = egui::ComboBox::from_id_salt(("agent_config", &option.id))
                    .selected_text(current.map_or(option.current.as_str(), |v| v.name.as_str()))
                    .show_ui(ui, |ui| {
                        for value in &option.values {
                            let item =
                                ui.selectable_label(value.value == option.current, &value.name);
                            let item = match &value.description {
                                Some(text) => item.on_hover_text(text),
                                None => item,
                            };
                            if item.clicked() && value.value != option.current {
                                change = Some((option.id.clone(), value.value.clone()));
                            }
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if let Some(text) = &option.description {
                    combo.on_hover_text(text);
                }
            }
        });
        if let Some((id, value)) = change
            && let Err(e) = agent.set_config(&id, &value)
        {
            self.notice(e, true);
        }
    }

    /// 권한 요청과 쓰기 제안 카드. 받은 쓰기는 앱이 적용한다.
    fn asks_ui(&mut self, ui: &mut egui::Ui) -> Option<AgentAction> {
        let mut answered = None;
        for (i, ask) in self.permissions.iter().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong(format!(
                    "권한 요청: {}",
                    ask.tool.title.as_deref().unwrap_or("도구 실행")
                ));
                if let Some(text) = &ask.tool.text {
                    ui.label(text);
                }
                for (path, lines) in &ask.diffs {
                    ui.weak(format!("고칠 파일: {}", path.display()));
                    diff_ui(ui, lines);
                }
                ui.horizontal_wrapped(|ui| {
                    for option in &ask.options {
                        if ui.button(&option.name).clicked() {
                            answered = Some((i, Some(option.id.clone())));
                        }
                    }
                    if ui.button("물리기").clicked() {
                        answered = Some((i, None));
                    }
                });
            });
        }
        if let Some((i, option)) = answered {
            let ask = self.permissions.remove(i);
            if let Some(agent) = &mut self.agent {
                agent.answer_permission(&ask.id, option.as_deref());
            }
        }
        let mut decided = None;
        for (i, ask) in self.writes.iter().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong(format!(
                    "{}: {}",
                    if ask.new_file {
                        "새 파일 제안"
                    } else {
                        "파일 쓰기 제안"
                    },
                    ask.path.display()
                ));
                diff_ui(ui, &ask.lines);
                ui.horizontal(|ui| {
                    if ui
                        .button("받기")
                        .on_hover_text("에디터 버퍼에 적용합니다(저장은 직접)")
                        .clicked()
                    {
                        decided = Some((i, true));
                    }
                    if ui.button("버리기").clicked() {
                        decided = Some((i, false));
                    }
                });
            });
        }
        let (i, accept) = decided?;
        let ask = self.writes.remove(i);
        if accept {
            return Some(AgentAction::Apply {
                id: ask.id,
                path: ask.path,
                content: ask.content,
            });
        }
        if let Some(agent) = &mut self.agent {
            agent.answer_write(&ask.id, Err("사용자가 버렸습니다".into()));
        }
        None
    }
}

fn entry_ui(ui: &mut egui::Ui, entry: &Entry) {
    match entry {
        Entry::Message(MessageKind::User, text) => {
            ui.horizontal_wrapped(|ui| {
                ui.strong("나:");
                ui.label(text);
            });
        }
        Entry::Message(MessageKind::Agent, text) => {
            ui.label(text);
        }
        Entry::Message(MessageKind::Thought, text) => {
            egui::CollapsingHeader::new("생각")
                .id_salt(("agent_thought", text.len(), text.get(..8)))
                .show(ui, |ui| {
                    ui.weak(text);
                });
        }
        Entry::Tool(call) => {
            let status = match call.status.as_deref() {
                Some("completed") => "완료",
                Some("failed") => "실패",
                Some("in_progress") => "진행",
                _ => "대기",
            };
            let title = call.title.as_deref().unwrap_or(&call.id);
            let kind = call
                .kind
                .as_deref()
                .map(|k| format!(" ({k})"))
                .unwrap_or_default();
            ui.weak(format!("[{status}] {title}{kind}"));
            for diff in &call.diffs {
                let lines = ferrite_core::git::diff_lines(
                    diff.old_text.as_deref().unwrap_or_default(),
                    &diff.new_text,
                );
                let added = lines.iter().filter(|(t, _)| *t == '+').count();
                let removed = lines.iter().filter(|(t, _)| *t == '-').count();
                ui.weak(format!("    {} +{added} -{removed}", diff.path.display()));
            }
        }
        Entry::Plan(entries) => {
            ui.strong("계획");
            for entry in entries {
                let mark = match entry.status.as_str() {
                    "completed" => "[완료]",
                    "in_progress" => "[진행]",
                    _ => "[대기]",
                };
                ui.label(format!("{mark} {}", entry.content));
            }
        }
        Entry::Notice(text, error) => {
            if *error {
                ui.colored_label(ui.visuals().error_fg_color, text);
            } else {
                ui.weak(text);
            }
        }
    }
}

fn diff_ui(ui: &mut egui::Ui, lines: &[(char, String)]) {
    let (add, remove) = if ui.visuals().dark_mode {
        (
            egui::Color32::from_rgb(0x7e, 0xe7, 0x87),
            egui::Color32::from_rgb(0xff, 0x7b, 0x72),
        )
    } else {
        (
            egui::Color32::from_rgb(0x1a, 0x7f, 0x37),
            egui::Color32::from_rgb(0xcf, 0x22, 0x2e),
        )
    };
    if lines.is_empty() {
        ui.weak("(바뀐 내용 없음)");
    }
    for (tag, text) in lines.iter().take(DIFF_LINES) {
        let line = match tag {
            '~' => egui::RichText::new("...").weak(),
            '+' => egui::RichText::new(format!("+ {text}")).color(add),
            '-' => egui::RichText::new(format!("- {text}")).color(remove),
            _ => egui::RichText::new(format!("  {text}")),
        };
        ui.label(line.monospace());
    }
    if lines.len() > DIFF_LINES {
        ui.weak(format!("... {}줄 더", lines.len() - DIFF_LINES));
    }
}
