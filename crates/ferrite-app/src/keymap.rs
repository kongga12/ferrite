//! 단축키 배치: 명령마다 키(여럿일 수 있다). 기본 배치(VS Code식)와 Eclipse 배치(egov 개발환경에서
//! 넘어온 사용자)가 있고, 설정의 `[keys]`로 명령마다 덮는다. 키 글은 `Ctrl+Shift+F`처럼 적고, `Ctrl`은
//! macOS에서 Cmd다(실제 Ctrl은 `Control`).

use std::collections::BTreeMap;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use ferrite_core::settings::{self, Keys};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Command {
    NewFile,
    OpenFolder,
    Save,
    SaveAs,
    CloseTab,
    Find,
    Replace,
    FindNext,
    FindPrevious,
    GotoLine,
    DeleteLine,
    DuplicateDown,
    DuplicateUp,
    FindInFiles,
    QuickOpen,
    SymbolSearch,
    Outline,
    Terminal,
    SourceControl,
    Keys,
    Complete,
    Definition,
    References,
    Format,
    Rename,
    CodeAction,
    SendRequest,
    Run,
    Debug,
    DebugStop,
    DebugPause,
    StepOver,
    StepIn,
    StepOut,
    Breakpoint,
}

/// (명령, 이름, 보일 이름, 기본 배치, Eclipse 배치).
type Row = (
    Command,
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

const TABLE: &[Row] = &[
    (
        Command::NewFile,
        "file.new",
        "새 파일",
        &["Ctrl+N"],
        &["Ctrl+N"],
    ),
    // Eclipse의 Ctrl+O는 개요다. 폴더 열기는 메뉴로.
    (
        Command::OpenFolder,
        "file.openFolder",
        "폴더 열기",
        &["Ctrl+O"],
        &[],
    ),
    (Command::Save, "file.save", "저장", &["Ctrl+S"], &["Ctrl+S"]),
    (
        Command::SaveAs,
        "file.saveAs",
        "다른 이름으로 저장",
        &["Ctrl+Shift+S"],
        &["Ctrl+Shift+S"],
    ),
    (
        Command::CloseTab,
        "file.closeTab",
        "탭 닫기",
        &["Ctrl+W"],
        &["Ctrl+W"],
    ),
    // Eclipse의 Ctrl+F는 찾기·바꾸기 창이다.
    (Command::Find, "edit.find", "찾기", &["Ctrl+F"], &[]),
    (
        Command::Replace,
        "edit.replace",
        "바꾸기",
        &["Ctrl+H"],
        &["Ctrl+F"],
    ),
    (
        Command::FindNext,
        "edit.findNext",
        "다음 찾기",
        &["F3"],
        &["Ctrl+K"],
    ),
    (
        Command::FindPrevious,
        "edit.findPrevious",
        "이전 찾기",
        &["Shift+F3"],
        &["Ctrl+Shift+K"],
    ),
    (
        Command::GotoLine,
        "edit.gotoLine",
        "줄로 가기",
        &["Ctrl+G"],
        &["Ctrl+L"],
    ),
    (
        Command::DeleteLine,
        "edit.deleteLine",
        "줄 지우기",
        &["Ctrl+Shift+K"],
        &["Ctrl+D"],
    ),
    (
        Command::DuplicateDown,
        "edit.duplicateDown",
        "줄 복제(아래로)",
        &["Shift+Alt+Down"],
        &["Ctrl+Alt+Down"],
    ),
    (
        Command::DuplicateUp,
        "edit.duplicateUp",
        "줄 복제(위로)",
        &["Shift+Alt+Up"],
        &["Ctrl+Alt+Up"],
    ),
    (
        Command::FindInFiles,
        "search.inFiles",
        "파일에서 찾기",
        &["Ctrl+Shift+F"],
        &["Ctrl+H"],
    ),
    (
        Command::QuickOpen,
        "search.quickOpen",
        "파일로 이동",
        &["Ctrl+P"],
        &["Ctrl+Shift+R"],
    ),
    (
        Command::SymbolSearch,
        "search.symbol",
        "기호로 이동",
        &["Ctrl+T"],
        &["Ctrl+Shift+T"],
    ),
    (
        Command::Outline,
        "view.outline",
        "개요",
        &["Ctrl+Shift+O"],
        &["Ctrl+O"],
    ),
    // 터미널은 VS Code처럼 macOS에서도 실제 Ctrl+`다.
    (
        Command::Terminal,
        "view.terminal",
        "터미널",
        &["Control+`"],
        &["Ctrl+Alt+Shift+T"],
    ),
    (
        Command::SourceControl,
        "view.sourceControl",
        "소스 제어",
        &["Ctrl+Shift+G"],
        &[],
    ),
    (
        Command::Keys,
        "view.keys",
        "단축키 목록",
        &["Ctrl+Shift+L"],
        &["Ctrl+Shift+L"],
    ),
    // 자동완성은 macOS에서도 실제 Ctrl이다(Cmd+Space는 OS 검색).
    (
        Command::Complete,
        "code.complete",
        "자동완성",
        &["Control+Space"],
        &["Control+Space"],
    ),
    (
        Command::Definition,
        "code.definition",
        "정의로 이동",
        &["F12"],
        &["F3"],
    ),
    (
        Command::References,
        "code.references",
        "참조 찾기",
        &["Shift+F12"],
        &["Ctrl+Shift+G"],
    ),
    (
        Command::Format,
        "code.format",
        "문서 포맷",
        &["Shift+Alt+F"],
        &["Ctrl+Shift+F"],
    ),
    (
        Command::Rename,
        "code.rename",
        "기호 이름 바꾸기",
        &["F2"],
        &["Alt+Shift+R"],
    ),
    (
        Command::CodeAction,
        "code.action",
        "코드 액션",
        &["Ctrl+.", "Alt+Enter"],
        &["Ctrl+1"],
    ),
    (
        Command::SendRequest,
        "http.send",
        "HTTP 요청 보내기",
        &["Ctrl+Enter"],
        &["Ctrl+Enter"],
    ),
    // macOS의 Cmd+F5는 VoiceOver를 켜므로 기본 배치는 모든 OS에서 실제 Ctrl을 쓴다.
    (
        Command::Run,
        "run.run",
        "실행",
        &["Control+F5"],
        &["Ctrl+F11"],
    ),
    (
        Command::Debug,
        "run.debug",
        "디버그 시작·계속",
        &["F5"],
        &["F11", "F8"],
    ),
    (
        Command::DebugStop,
        "run.debugStop",
        "디버그 멈춤",
        &["Shift+F5"],
        &["Ctrl+F2"],
    ),
    (
        Command::DebugPause,
        "run.debugPause",
        "디버그 일시 정지",
        &["F6"],
        &[],
    ),
    (
        Command::StepOver,
        "run.stepOver",
        "한 줄씩",
        &["F10"],
        &["F6"],
    ),
    (Command::StepIn, "run.stepIn", "들어가기", &["F11"], &["F5"]),
    (
        Command::StepOut,
        "run.stepOut",
        "나오기",
        &["Shift+F11"],
        &["F7"],
    ),
    (
        Command::Breakpoint,
        "run.breakpoint",
        "중단점 켜기/끄기",
        &["F9"],
        &["Ctrl+Shift+B"],
    ),
];

impl Command {
    fn row(self) -> &'static Row {
        TABLE
            .iter()
            .find(|r| r.0 == self)
            .expect("모든 명령이 표에 있다")
    }

    /// 설정에 적는 이름(`"file.save"`).
    pub fn id(self) -> &'static str {
        self.row().1
    }

    pub fn label(self) -> &'static str {
        self.row().2
    }

    /// 에디터에 포커스가 있을 때만 받는다(다른 곳에서는 그 키를 그대로 둔다).
    pub fn editor_only(self) -> bool {
        matches!(
            self,
            Command::GotoLine
                | Command::DeleteLine
                | Command::DuplicateDown
                | Command::DuplicateUp
                | Command::SendRequest
        )
    }
}

/// 키 글(`Ctrl+Shift+F`, `Control+Space`, `Alt+Up`, `Ctrl+.`)을 읽는다.
pub fn parse(text: &str) -> Result<KeyboardShortcut, String> {
    let text = text.trim();
    // `Ctrl++`처럼 키가 `+`이면 나누기 전에 떼어 둔다.
    let (mods, key) = match text.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None => text.rsplit_once('+').unwrap_or(("", text)),
    };
    let key = key.trim();
    // 대소문자를 가리지 않는다(`space`·`f5`·`DOWN`). egui 이름은 첫 글자만 대문자다.
    let mut chars = key.chars();
    let capitalized: String = chars
        .next()
        .map(|c| c.to_ascii_uppercase())
        .into_iter()
        .chain(chars.map(|c| c.to_ascii_lowercase()))
        .collect();
    let key = Key::from_name(key)
        .or_else(|| Key::from_name(&capitalized))
        .ok_or_else(|| format!("모르는 키입니다: {key}"))?;
    let mut modifiers = Modifiers::NONE;
    for part in mods.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        modifiers = modifiers.plus(match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => Modifiers::COMMAND,
            "control" => Modifiers::CTRL,
            "alt" | "option" => Modifiers::ALT,
            "shift" => Modifiers::SHIFT,
            _ => return Err(format!("모르는 조합 키입니다: {part}")),
        });
    }
    Ok(KeyboardShortcut::new(modifiers, key))
}

/// 조합 키 수. 많은 것부터 맞춘다(egui는 덧붙은 Shift·Alt를 무시하고 맞추므로 `Shift+F5`를 `F5`보다
/// 먼저 본다).
fn weight(shortcut: &KeyboardShortcut) -> usize {
    let m = shortcut.modifiers;
    [m.alt, m.shift, m.ctrl || m.command, m.mac_cmd]
        .into_iter()
        .filter(|&b| b)
        .count()
}

pub struct Keymap {
    /// (명령, 키). 조합 키가 많은 것부터.
    bindings: Vec<(Command, KeyboardShortcut)>,
    /// 설정의 잘못된 이름·키.
    pub errors: Vec<String>,
    pub preset: settings::Keymap,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::new(settings::Keymap::Default, &BTreeMap::new())
    }
}

impl Keymap {
    pub fn new(preset: settings::Keymap, overrides: &BTreeMap<String, Keys>) -> Keymap {
        let mut errors = Vec::new();
        let mut bindings = Vec::new();
        for &(command, id, _, default, eclipse) in TABLE {
            let keys: Vec<&str> = match overrides.get(id) {
                Some(keys) => keys.list(),
                None if preset == settings::Keymap::Eclipse => eclipse.to_vec(),
                None => default.to_vec(),
            };
            for key in keys {
                match parse(key) {
                    Ok(shortcut) => bindings.push((command, shortcut)),
                    Err(e) => errors.push(format!("[keys] \"{id}\": {e}")),
                }
            }
        }
        for id in overrides.keys() {
            if !TABLE.iter().any(|r| r.1 == id) {
                errors.push(format!("[keys] 모르는 명령입니다: \"{id}\""));
            }
        }
        // 정렬은 안정적이라 같은 무게에서는 표 순서가 남는다.
        bindings.sort_by_key(|(_, s)| std::cmp::Reverse(weight(s)));
        Keymap {
            bindings,
            errors,
            preset,
        }
    }

    /// 그 명령의 키들.
    pub fn keys(&self, command: Command) -> impl Iterator<Item = &KeyboardShortcut> {
        self.bindings
            .iter()
            .filter(move |(c, _)| *c == command)
            .map(|(_, s)| s)
    }

    /// 메뉴에 보일 키(첫째 키, 없으면 빈 글).
    pub fn text(&self, ctx: &egui::Context, command: Command) -> String {
        self.keys(command)
            .next()
            .map(|s| ctx.format_shortcut(s))
            .unwrap_or_default()
    }

    /// 이번 프레임에 눌린 명령을 입력에서 꺼낸다. `allow`가 false인 것은 꺼내지 않는다(그 키는 위젯에
    /// 간다).
    pub fn pressed(
        &self,
        ctx: &egui::Context,
        allow: impl Fn(Command, &KeyboardShortcut) -> bool,
    ) -> Vec<Command> {
        let mut found = Vec::new();
        for (command, shortcut) in &self.bindings {
            if allow(*command, shortcut) && ctx.input_mut(|i| i.consume_shortcut(shortcut)) {
                found.push(*command);
            }
        }
        found
    }

    /// 그 명령의 키를 꺼낸다(창 안에서 따로 받는 명령).
    pub fn consume(&self, input: &mut egui::InputState, command: Command) -> bool {
        self.keys(command).any(|s| input.consume_shortcut(s))
    }

    /// 목록 창에 보일 (명령, 키들, 같은 키를 쓰는 다른 명령).
    pub fn rows(&self) -> Vec<(Command, Vec<KeyboardShortcut>, Vec<Command>)> {
        TABLE
            .iter()
            .map(|r| {
                let keys: Vec<KeyboardShortcut> = self.keys(r.0).copied().collect();
                let clashes = self
                    .bindings
                    .iter()
                    .filter(|(c, s)| *c != r.0 && keys.contains(s))
                    .map(|(c, _)| *c)
                    .collect();
                (r.0, keys, clashes)
            })
            .collect()
    }
}

/// 터미널이 셸 제어 문자로 쓰는 키(Ctrl+글자, Shift·Alt 없이). 터미널에 포커스가 있으면 단축키로
/// 가로채지 않는다(Ctrl+D 입력 끝, Ctrl+L 화면 지우기, Ctrl+K 줄 지우기 등). macOS의 Cmd는 상관없다.
pub fn is_terminal_control(shortcut: &KeyboardShortcut) -> bool {
    let m = shortcut.modifiers;
    let ctrl = m.ctrl || (m.command && !cfg!(target_os = "macos"));
    ctrl && !m.shift
        && !m.alt
        && shortcut.logical_key.name().len() == 1
        && shortcut
            .logical_key
            .name()
            .chars()
            .all(|c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 키_글을_읽는다() {
        let s = |m, k| KeyboardShortcut::new(m, k);
        assert_eq!(
            parse("Ctrl+Shift+F"),
            Ok(s(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F))
        );
        assert_eq!(parse("control+space"), Ok(s(Modifiers::CTRL, Key::Space)));
        assert_eq!(
            parse("Shift+Alt+Down"),
            Ok(s(Modifiers::SHIFT.plus(Modifiers::ALT), Key::ArrowDown))
        );
        assert_eq!(parse(" F5 "), Ok(s(Modifiers::NONE, Key::F5)));
        assert_eq!(parse("Ctrl+."), Ok(s(Modifiers::COMMAND, Key::Period)));
        assert_eq!(parse("Ctrl+d"), Ok(s(Modifiers::COMMAND, Key::D)));
        assert_eq!(parse("Ctrl++"), Ok(s(Modifiers::COMMAND, Key::Plus)));
        assert!(parse("Ctrl+Hyper+F").is_err());
        assert!(parse("Ctrl+Nope").is_err());
    }

    #[test]
    fn 두_배치_모두_모든_명령을_읽고_서로_겹치지_않는다() {
        for preset in [settings::Keymap::Default, settings::Keymap::Eclipse] {
            let keymap = Keymap::new(preset, &BTreeMap::new());
            assert!(keymap.errors.is_empty(), "{preset:?}: {:?}", keymap.errors);
            for (command, _, clashes) in keymap.rows() {
                assert!(clashes.is_empty(), "{preset:?}: {command:?}와 {clashes:?}");
            }
        }
        let eclipse = Keymap::new(settings::Keymap::Eclipse, &BTreeMap::new());
        let keys = |c| eclipse.keys(c).copied().collect::<Vec<_>>();
        assert_eq!(keys(Command::DeleteLine), [parse("Ctrl+D").unwrap()]);
        assert_eq!(keys(Command::StepOver), [parse("F6").unwrap()]);
        assert_eq!(
            keys(Command::Debug),
            [parse("F11").unwrap(), parse("F8").unwrap()]
        );
        assert!(keys(Command::OpenFolder).is_empty());
    }

    #[test]
    fn 설정으로_덮고_잘못된_것은_알린다() {
        let overrides = BTreeMap::from([
            ("edit.deleteLine".to_owned(), Keys::One("Ctrl+Y".into())),
            ("code.action".to_owned(), Keys::Many(vec![])),
            ("run.stepOver".to_owned(), Keys::One("Ctrl+Bogus".into())),
            ("edit.nothing".to_owned(), Keys::One("F1".into())),
        ]);
        let keymap = Keymap::new(settings::Keymap::Default, &overrides);
        assert_eq!(
            keymap
                .keys(Command::DeleteLine)
                .copied()
                .collect::<Vec<_>>(),
            [parse("Ctrl+Y").unwrap()]
        );
        assert_eq!(keymap.keys(Command::CodeAction).count(), 0);
        assert_eq!(keymap.keys(Command::StepOver).count(), 0);
        assert_eq!(keymap.errors.len(), 2, "{:?}", keymap.errors);
        // 조합 키가 많은 것을 먼저 맞춘다.
        let order: Vec<Command> = keymap.bindings.iter().map(|(c, _)| *c).collect();
        let at = |c| order.iter().position(|x| *x == c).unwrap();
        assert!(at(Command::DebugStop) < at(Command::Debug));
        assert!(at(Command::FindInFiles) < at(Command::Find));
        // 같은 키를 두 명령에 주면 목록에 겹침으로 보인다.
        let clash = BTreeMap::from([("edit.gotoLine".to_owned(), Keys::One("Ctrl+S".into()))]);
        let keymap = Keymap::new(settings::Keymap::Default, &clash);
        let rows = keymap.rows();
        let save = rows.iter().find(|r| r.0 == Command::Save).unwrap();
        assert_eq!(save.2, [Command::GotoLine]);
    }

    #[test]
    fn 터미널은_ctrl_글자를_받는다() {
        let t = |k| is_terminal_control(&parse(k).unwrap());
        assert!(t("Control+D"));
        assert_eq!(t("Ctrl+L"), !cfg!(target_os = "macos"));
        assert!(!t("Ctrl+Shift+F"));
        assert!(!t("Control+`"));
        assert!(!t("F5"));
        assert!(!t("Ctrl+1"));
    }
}
