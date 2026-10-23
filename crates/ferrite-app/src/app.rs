//! 메인 창: 메뉴바, 프로젝트 트리, 탭 에디터, 상태바, 미저장 변경 확인.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, ViewportCommand};
use ferrite_core::buffer::{Buffer, Indent, Selection, detect_indent};
use ferrite_core::settings::{self, Settings, Theme};
use ferrite_core::syntax::{Lang, MAX_SYNTAX_BYTES, Syntax};
use ferrite_core::text_file::{self, Encoding, LineEnding, TextContent, TextFileError};
use ferrite_core::{fs_ops, search};

use crate::editor_view::{self, EditorOptions, EditorState, Styler};
use crate::file_tree::{FileTree, TreeAction};
use crate::find_bar::{FindAction, FindBar};
use crate::lsp_hub::{DocSync, HubUpdate, LspHub, ServerState};
use crate::lsp_ui::{ActionMenu, CompletionPopup, Pending, ReferenceList, word_at, word_start};
use crate::servers::{ServerAction, ServerManager, install_hint};
use crate::watcher::FsWatcher;
use ferrite_lsp::client::RpcError;
use ferrite_lsp::edits::{
    FileEdit, code_action_params, formatting_params, parse_code_actions, parse_resolved_action,
    parse_text_edits, parse_workspace_edit, rename_params,
};
use ferrite_lsp::features::{
    char_range, completion_params, parse_completion, parse_hover, parse_locations, position_params,
    references_params,
};
use ferrite_lsp::position::PositionEncoding;
use ferrite_lsp::problem::{Severity, to_problems};
use serde_json::Value;

// Modifiers::COMMAND는 macOS에서 Cmd, 그 외 OS에서 Ctrl로 동작한다.
const OPEN_FOLDER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
const REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::H);
const FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
const FIND_PREV: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
const NEW_FILE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
// 자동완성은 macOS에서도 실제 Ctrl 키다(Cmd+Space는 OS 검색).
const COMPLETE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Space);
const GOTO_DEFINITION: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F12);
const FIND_REFERENCES: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F12);
const FORMAT: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::SHIFT.plus(Modifiers::ALT), Key::F);
const RENAME_SYMBOL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F2);
// 코드 액션: Ctrl+.와 Alt+Enter 두 가지 습관을 모두 받는다.
const CODE_ACTION: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Period);
const CODE_ACTION_ALT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::Enter);
/// 마우스가 한 자리에 이만큼 머물면 호버 정보를 요청한다.
const HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(400);

/// 언어 서버에 보낼 요청 종류. 위치(`pos`)와 함께 보내며, 위치를 쓰지 않는 요청도 있다.
#[derive(Clone, PartialEq)]
enum LspRequest {
    Completion(Option<char>),
    Hover,
    Definition,
    References,
    Format,
    Rename(String),
    CodeActions,
    /// 편집이 나중에 오는 코드 액션의 편집 받기(`codeAction/resolve`).
    Resolve(Value),
    /// 서버 명령 실행(`workspace/executeCommand`). 결과 편집은 서버가 applyEdit로 보낸다.
    Execute(Value),
}

impl LspRequest {
    /// 사용자가 직접 부른 요청이면 실패를 상태바에 알린다(호버·입력 중 자동완성은 조용히).
    fn is_explicit(&self) -> bool {
        !matches!(self, LspRequest::Hover | LspRequest::Completion(Some(_)))
    }
}

/// 서버가 버린 요청(ContentModified 등)을 다시 보내는 최대 횟수.
const MAX_LSP_RETRIES: u8 = 3;

/// 요청을 다시 보낼 때 필요한 정보.
struct Retry {
    kind: LspRequest,
    pos: usize,
    path: PathBuf,
    attempts: u8,
    /// 요청을 보낸 시점의 버퍼 버전. 편집 응답이 그 사이 바뀐 문서에 적용되지 않게 비교한다.
    version: u64,
}

/// 에디터를 그린 프레임에서 언어 서버 기능에 넘길 정보.
struct EditorFrame {
    path: PathBuf,
    focused: bool,
    cursor_rect: egui::Rect,
    hover_pos: Option<usize>,
    ctrl_click: Option<usize>,
    /// 이번 프레임에 입력된 글자.
    typed: String,
}

/// 마우스가 머문 위치와 그 호버 정보.
struct HoverState {
    path: PathBuf,
    pos: usize,
    since: Instant,
    requested: bool,
    text: Option<String>,
}

/// 이름을 입력받아 실행할 파일 작업.
enum NameTarget {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    /// 코드 기호의 이름 바꾸기(F2). 위치는 활성 문서의 커서.
    Symbol {
        pos: usize,
    },
}

struct NamePrompt {
    target: NameTarget,
    name: String,
    error: Option<String>,
    focus: bool,
}

/// OS별 설정 파일 위치(Windows `%APPDATA%`, macOS `~/Library/Application Support`, Linux `~/.config`).
/// 언어 서버 매니저가 서버를 설치하는 곳. Windows `%LOCALAPPDATA%\ferrite\servers`,
/// macOS `~/Library/Application Support/ferrite/servers`, Linux `~/.local/share/ferrite/servers`.
fn servers_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("ferrite")
        .join("servers")
}

fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("ferrite").join("settings.toml"))
}

/// 심볼릭 링크를 푼 실제 절대 경로. 파일 감시 알림은 실제 경로로 오므로(macOS의 `/var` →
/// `/private/var` 등) 문서·트리 경로를 이것으로 통일해야 알림과 비교할 수 있다.
/// 아직 없는 경로면 절대 경로만 만든다.
fn real_path(path: &Path) -> PathBuf {
    let Ok(real) = std::fs::canonicalize(path) else {
        return std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    };
    // Windows의 canonicalize는 `\\?\C:\…` 형태를 돌려준다. 드라이브 경로면 접두사를 떼어
    // 화면 표시와 감시 알림 경로(접두사 없음)에 맞춘다. 네트워크 경로(UNC)는 그대로 둔다.
    #[cfg(windows)]
    if let Some(plain) = real.to_str().and_then(|s| s.strip_prefix(r"\\?\"))
        && !plain.starts_with(r"UNC\")
    {
        return PathBuf::from(plain);
    }
    real
}

/// 설정 파일을 읽는다. 없으면 기본값, 잘못되었으면 기본값과 오류 메시지.
fn load_settings() -> (Settings, Option<String>) {
    let Some(path) = settings_path() else {
        return (Settings::default(), None);
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => match Settings::parse(&text) {
            Ok(s) => (s, None),
            Err(e) => (
                Settings::default(),
                Some(format!("설정 오류(기본값 사용): {e}")),
            ),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Settings::default(), None),
        Err(e) => (
            Settings::default(),
            Some(format!("설정 파일을 읽지 못했습니다: {e}")),
        ),
    }
}

fn apply_settings(ctx: &egui::Context, settings: &Settings) {
    ctx.set_theme(match settings.theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Dark => egui::ThemePreference::Dark,
        Theme::Light => egui::ThemePreference::Light,
    });
    let size = settings.font_size;
    ctx.all_styles_mut(|style| {
        if let Some(font) = style.text_styles.get_mut(&egui::TextStyle::Monospace) {
            font.size = size;
        }
    });
}

/// 문서 경로를 바꾼다(이름 바꾸기, 다른 이름으로 저장). 언어 서버에서는 옛 경로를 닫고
/// 다음 동기화 때 새 경로로 다시 연다.
fn move_document(hub: &mut LspHub, doc: &mut Document, path: PathBuf) {
    if let Some(lang) = doc.lang {
        hub.closed(&doc.path, lang);
    }
    doc.lsp = DocSync::default();
    doc.set_path(path);
}

struct Document {
    path: PathBuf,
    buffer: Buffer,
    encoding: Encoding,
    line_ending: LineEnding,
    indent: Indent,
    /// 마지막으로 열거나 저장한 시점의 버퍼 리비전.
    saved_revision: u64,
    /// 저장 인코딩·줄바꿈을 바꿨지만 아직 저장하지 않음.
    format_changed: bool,
    lang: Option<Lang>,
    /// 언어를 알고 파일이 크지 않을 때만 있다.
    syntax: Option<Syntax>,
    view: EditorState,
    /// 마지막으로 읽거나 쓴 디스크 내용의 해시. 자기 저장으로 생긴 변경 알림을 걸러낸다.
    disk_hash: u64,
    /// 저장하지 않은 변경이 있는데 디스크 파일이 밖에서 바뀌었다(사용자 선택 대기).
    external_change: bool,
    /// 언어 서버 동기화 상태와 진단.
    lsp: DocSync,
}

impl Document {
    fn open(path: PathBuf, default_indent: Indent) -> Result<Self, TextFileError> {
        let bytes = text_file::read(&path)?;
        let content = text_file::decode(&bytes)?;
        let lang = Lang::from_path(&path);
        let mut doc = Self {
            path,
            buffer: Buffer::new(""),
            encoding: content.encoding,
            line_ending: content.line_ending,
            indent: default_indent,
            saved_revision: 0,
            format_changed: false,
            lang,
            syntax: None,
            view: EditorState::default(),
            disk_hash: 0,
            external_change: false,
            lsp: DocSync::default(),
        };
        doc.replace_content(&content, &bytes, default_indent);
        Ok(doc)
    }

    /// 디스크에서 읽은 내용으로 버퍼를 새로 만든다(처음 열 때, 다시 불러올 때). 커서 위치는 최대한 유지한다.
    fn replace_content(&mut self, content: &TextContent, bytes: &[u8], default_indent: Indent) {
        let head = self.buffer.selection().head;
        let mut buffer = Buffer::new(&content.text);
        buffer.move_to(head.min(buffer.len_chars()), false);
        self.indent = detect_indent(buffer.rope(), default_indent);
        self.syntax = self
            .lang
            .filter(|_| buffer.rope().len_bytes() <= MAX_SYNTAX_BYTES)
            .and_then(|l| Syntax::new(l).ok());
        buffer.set_track_syntax_edits(self.syntax.is_some());
        self.saved_revision = buffer.revision();
        self.buffer = buffer;
        self.encoding = content.encoding;
        self.line_ending = content.line_ending;
        self.format_changed = false;
        self.disk_hash = text_file::content_hash(bytes);
        self.external_change = false;
        self.lsp.content_replaced();
    }

    fn reload(&mut self, default_indent: Indent) -> Result<(), TextFileError> {
        let bytes = text_file::read(&self.path)?;
        let content = text_file::decode(&bytes)?;
        self.replace_content(&content, &bytes, default_indent);
        Ok(())
    }

    fn save(&mut self) -> Result<(), TextFileError> {
        let content = TextContent {
            text: self.buffer.text(),
            encoding: self.encoding,
            line_ending: self.line_ending,
        };
        let bytes = text_file::encode(&content)?;
        text_file::write_atomic(&self.path, &bytes)?;
        self.saved_revision = self.buffer.revision();
        self.format_changed = false;
        self.disk_hash = text_file::content_hash(&bytes);
        self.external_change = false;
        Ok(())
    }

    /// 디스크 파일이 바뀌었다는 알림을 처리하고, 사용자에게 알릴 내용을 돌려준다.
    /// 저장하지 않은 변경이 없으면 다시 불러오고, 있으면 충돌로 표시해 사용자가 고르게 한다.
    fn on_disk_changed(&mut self, default_indent: Indent) -> Option<String> {
        let name = self.name();
        let bytes = match text_file::read(&self.path) {
            Ok(bytes) => bytes,
            Err(TextFileError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                return Some(format!(
                    "디스크에서 삭제되었습니다: {name} (저장하면 다시 만듭니다)"
                ));
            }
            Err(e) => return Some(format!("바뀐 파일을 읽지 못했습니다: {name}: {e}")),
        };
        let hash = text_file::content_hash(&bytes);
        if hash == self.disk_hash {
            return None;
        }
        if self.is_dirty() {
            self.disk_hash = hash;
            self.external_change = true;
            return Some(format!(
                "디스크의 {name}이(가) 바뀌었습니다. 저장하지 않은 변경과 충돌합니다"
            ));
        }
        match text_file::decode(&bytes) {
            Ok(content) => {
                self.replace_content(&content, &bytes, default_indent);
                Some(format!("디스크에서 다시 불러왔습니다: {name}"))
            }
            Err(e) => Some(format!("바뀐 파일을 읽지 못했습니다: {name}: {e}")),
        }
    }

    fn is_dirty(&self) -> bool {
        self.buffer.revision() != self.saved_revision || self.format_changed
    }

    fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    fn title(&self) -> String {
        let name = self.name();
        if self.is_dirty() {
            format!("● {name}")
        } else {
            name
        }
    }

    /// 이름 바꾸기·다른 이름으로 저장 뒤 경로를 바꾼다. 확장자가 바뀌어 언어가 달라지면 구문 분석도 새로 한다.
    fn set_path(&mut self, path: PathBuf) {
        let lang = Lang::from_path(&path);
        if lang != self.lang {
            self.lang = lang;
            self.syntax = lang
                .filter(|_| self.buffer.rope().len_bytes() <= MAX_SYNTAX_BYTES)
                .and_then(|l| Syntax::new(l).ok());
            self.buffer.set_track_syntax_edits(self.syntax.is_some());
        }
        self.path = path;
    }
}

/// 저장하지 않은 변경 때문에 사용자 확인을 기다리는 닫기 요청.
enum PendingClose {
    Tab(usize),
    App,
}

enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

pub struct FerriteApp {
    tree: Option<FileTree>,
    docs: Vec<Document>,
    active: Option<usize>,
    pending_close: Option<PendingClose>,
    name_prompt: Option<NamePrompt>,
    /// 휴지통으로 옮기기 전 확인을 기다리는 경로.
    confirm_delete: Option<PathBuf>,
    hub: LspHub,
    servers: ServerManager,
    /// 아래쪽 "문제" 목록을 보이는지.
    show_problems: bool,
    /// 응답을 기다리는 언어 서버 요청.
    pending: std::collections::HashMap<(Lang, i64), (Pending, Retry)>,
    completion: Option<CompletionPopup>,
    actions: Option<ActionMenu>,
    hover: Option<HoverState>,
    references: Option<ReferenceList>,
    /// 사용자가 종료를 확정했으면 true. 이후의 창 닫기 요청은 막지 않는다.
    allow_close: bool,
    status: String,
    find: Option<FindBar>,
    /// 다음 프레임에 에디터로 키보드 포커스를 돌려준다(찾기 막대를 닫은 뒤 등).
    focus_editor: bool,
    settings: Settings,
    /// 설정 파일을 저장해 다음 프레임에 다시 적용해야 한다.
    settings_changed: bool,
    /// 감시를 시작하지 못한 환경(권한 등)에서는 없다. 그래도 편집은 된다.
    watcher: Option<FsWatcher>,
}

impl FerriteApp {
    /// `start`는 명령줄로 받은 시작 경로. 폴더면 프로젝트로, 파일이면 상위 폴더를 프로젝트로 열고 파일을 연다.
    pub fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>) -> Self {
        crate::fonts::install_korean_font(&cc.egui_ctx);
        let (settings, settings_error) = load_settings();
        apply_settings(&cc.egui_ctx, &settings);
        let start = start.map(|p| real_path(&p));
        let file = start.as_ref().filter(|p| p.is_file()).cloned();
        let project = match &file {
            Some(f) => f.parent().map(PathBuf::from),
            None => start,
        };
        let (tree, status) = match project {
            Some(dir) if dir.is_dir() => (
                Some(FileTree::new(dir.clone())),
                format!("프로젝트: {}", dir.display()),
            ),
            Some(path) => (None, format!("열 수 없는 경로입니다: {}", path.display())),
            None => (
                None,
                format!(
                    "폴더를 열어 시작하세요 ({})",
                    cc.egui_ctx.format_shortcut(&OPEN_FOLDER)
                ),
            ),
        };
        let settings_failed = settings_error.is_some();
        let mut app = Self {
            tree,
            docs: Vec::new(),
            active: None,
            pending_close: None,
            allow_close: false,
            status: settings_error.unwrap_or(status),
            find: None,
            focus_editor: false,
            settings,
            settings_changed: false,
            watcher: FsWatcher::new(cc.egui_ctx.clone()).ok(),
            name_prompt: None,
            confirm_delete: None,
            hub: LspHub::new(cc.egui_ctx.clone(), servers_root()),
            servers: ServerManager::new(servers_root()),
            show_problems: false,
            pending: std::collections::HashMap::new(),
            completion: None,
            actions: None,
            hover: None,
            references: None,
        };
        let outdated = app.servers.outdated();
        // 설정 오류 안내가 더 급하므로 그때는 덮어쓰지 않는다.
        if !outdated.is_empty() && !settings_failed {
            app.status = format!(
                "언어 서버 업데이트 있음: {} (도구 > 언어 서버)",
                outdated.join(", ")
            );
        }
        if let Some(file) = file {
            app.open_file(file);
        }
        app
    }

    /// 대화상자가 떠 있으면 단축키로 문서 목록이 바뀌지 않게 막는다.
    fn modal_open(&self) -> bool {
        self.pending_close.is_some() || self.name_prompt.is_some() || self.confirm_delete.is_some()
    }

    fn handle_tree_action(&mut self, action: TreeAction) {
        let prompt = |target, name: String| {
            Some(NamePrompt {
                target,
                name,
                error: None,
                focus: true,
            })
        };
        match action {
            TreeAction::Open(path) => self.open_file(path),
            TreeAction::NewFile(dir) => {
                self.name_prompt = prompt(NameTarget::NewFile(dir), String::new())
            }
            TreeAction::NewFolder(dir) => {
                self.name_prompt = prompt(NameTarget::NewFolder(dir), String::new());
            }
            TreeAction::Rename(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.name_prompt = prompt(NameTarget::Rename(path), name);
            }
            TreeAction::Delete(path) => self.confirm_delete = Some(path),
        }
    }

    /// 트리의 폴더 목록 캐시를 버려 바로 다시 읽게 한다(감시 알림보다 먼저 반영).
    fn invalidate_tree(&mut self, dir: Option<&Path>) {
        if let (Some(tree), Some(dir)) = (&mut self.tree, dir) {
            tree.invalidate(dir);
        }
    }

    fn name_prompt_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.name_prompt else {
            return;
        };
        let title = match prompt.target {
            NameTarget::NewFile(_) => "새 파일",
            NameTarget::NewFolder(_) => "새 폴더",
            NameTarget::Rename(_) => "이름 바꾸기",
            NameTarget::Symbol { .. } => "기호 이름 바꾸기 (모든 사용처)",
        };
        let (mut submit, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("name_prompt")).show(ctx, |ui| {
            ui.heading(title);
            let label = ui.label("이름");
            let field = ui
                .add(egui::TextEdit::singleline(&mut prompt.name).desired_width(320.0))
                .labelled_by(label.id);
            if std::mem::take(&mut prompt.focus) {
                field.request_focus();
            }
            submit = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if let Some(e) = &prompt.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                submit |= ui.button("확인").clicked();
                cancel = ui.button("취소").clicked();
            });
        });
        if cancel || (modal.should_close() && !submit) {
            self.name_prompt = None;
            return;
        }
        if !submit {
            return;
        }
        let name = prompt.name.trim_end_matches(['\r', '\n']).to_owned();
        // 기호 이름 바꾸기는 파일 작업이 아니라 언어 서버에 맡긴다(결과는 응답에서 적용).
        if let NameTarget::Symbol { pos } = prompt.target {
            if name.trim().is_empty() {
                prompt.error = Some("이름을 입력하세요".into());
                prompt.focus = true;
                return;
            }
            self.name_prompt = None;
            self.lsp_request(LspRequest::Rename(name), pos);
            return;
        }
        let result = match &prompt.target {
            NameTarget::Symbol { .. } => return,
            NameTarget::NewFile(dir) => fs_ops::create_file(dir, &name).map(|p| (None, p)),
            NameTarget::NewFolder(dir) => fs_ops::create_dir(dir, &name).map(|p| (None, p)),
            NameTarget::Rename(old) => fs_ops::rename(old, &name).map(|p| (Some(old.clone()), p)),
        };
        let (renamed_from, path) = match result {
            Ok(done) => done,
            Err(e) => {
                prompt.error = Some(e);
                prompt.focus = true;
                return;
            }
        };
        let is_new_file = matches!(prompt.target, NameTarget::NewFile(_));
        self.name_prompt = None;
        self.invalidate_tree(path.parent());
        if let Some(old) = renamed_from {
            // 열린 문서(폴더 이름을 바꿨으면 그 안의 문서들)의 경로도 새 이름으로 옮긴다.
            for doc in &mut self.docs {
                if let Ok(rest) = doc.path.strip_prefix(&old) {
                    let moved = if rest.as_os_str().is_empty() {
                        path.clone()
                    } else {
                        path.join(rest)
                    };
                    move_document(&mut self.hub, doc, moved);
                }
            }
            self.status = format!("이름을 바꿨습니다: {}", path.display());
        } else if is_new_file {
            self.open_file(path);
        } else {
            self.status = format!("폴더를 만들었습니다: {}", path.display());
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = &self.confirm_delete else {
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let what = if path.is_dir() {
            format!("'{name}' 폴더와 그 안의 모든 항목을")
        } else {
            format!("'{name}'을(를)")
        };
        let (mut confirm, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("confirm_delete")).show(ctx, |ui| {
            ui.heading("휴지통으로 삭제");
            ui.label(format!(
                "{what} 휴지통으로 옮길까요? 휴지통에서 되살릴 수 있습니다."
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                confirm = ui.button("휴지통으로 이동").clicked();
                cancel = ui.button("취소").clicked();
            });
        });
        if cancel || (modal.should_close() && !confirm) {
            self.confirm_delete = None;
            return;
        }
        if !confirm {
            return;
        }
        let Some(path) = self.confirm_delete.take() else {
            return;
        };
        if let Err(e) = fs_ops::move_to_trash(&path) {
            self.status = e;
            return;
        }
        self.invalidate_tree(path.parent());
        // 지운 항목의 탭은 닫는다. 저장하지 않은 변경이 있는 탭은 내용을 잃지 않도록 남긴다.
        let mut kept = 0;
        for i in (0..self.docs.len()).rev() {
            if self.docs[i].path.starts_with(&path) {
                if self.docs[i].is_dirty() {
                    kept += 1;
                } else {
                    self.close_tab(i);
                }
            }
        }
        self.status = if kept > 0 {
            format!("휴지통으로 옮겼습니다: {name} (저장하지 않은 탭 {kept}개는 열어 둠)")
        } else {
            format!("휴지통으로 옮겼습니다: {name}")
        };
    }

    /// 저장 위치를 고르게 해 빈 파일을 만들고 연다. 이미 있는 파일을 고르면 덮어쓰지 않고 연다.
    fn new_file_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("새 파일");
        if let Some(tree) = &self.tree {
            dialog = dialog.set_directory(tree.root());
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        if !path.exists() {
            let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
                return;
            };
            if let Err(e) = fs_ops::create_file(dir, &name.to_string_lossy()) {
                self.status = e;
                return;
            }
            self.invalidate_tree(Some(dir));
        }
        self.open_file(path);
    }

    fn save_as_dialog(&mut self, index: usize) {
        let doc = &self.docs[index];
        let mut dialog = rfd::FileDialog::new()
            .set_title("다른 이름으로 저장")
            .set_file_name(doc.name());
        if let Some(dir) = doc.path.parent() {
            dialog = dialog.set_directory(dir);
        }
        let Some(target) = dialog.save_file() else {
            return;
        };
        let target = real_path(&target);
        if self
            .docs
            .iter()
            .enumerate()
            .any(|(i, d)| i != index && d.path == target)
        {
            self.status = format!("다른 탭에 이미 열려 있는 파일입니다: {}", target.display());
            return;
        }
        let doc = &mut self.docs[index];
        let old = std::mem::replace(&mut doc.path, target.clone());
        match doc.save() {
            Ok(()) => {
                // 이제 파일이 있으니 실제 경로로 확정하고 언어도 다시 고른다.
                move_document(&mut self.hub, doc, real_path(&target));
                self.invalidate_tree(target.parent());
                self.status = format!("저장됨: {}", target.display());
            }
            Err(e) => {
                doc.path = old;
                self.status = format!("저장 실패: {}: {e}", target.display());
            }
        }
    }

    /// 설정 파일이 없으면 템플릿으로 만들고 에디터로 연다.
    fn open_settings(&mut self) {
        let Some(path) = settings_path() else {
            self.status = "이 OS에서는 설정 폴더를 찾을 수 없습니다".into();
            return;
        };
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| text_file::write_atomic(&path, settings::TEMPLATE.as_bytes()));
            if let Err(e) = created {
                self.status = format!("설정 파일을 만들지 못했습니다: {e}");
                return;
            }
        }
        self.open_file(path);
    }

    /// 감시 알림을 처리하고 감시 목록을 지금 열린 문서·펼친 폴더에 맞춘다.
    fn process_fs_events(&mut self) {
        let Some(watcher) = &mut self.watcher else {
            return;
        };
        let changed = watcher.changed_paths();
        if let Some(tree) = &mut self.tree {
            for dir in changed.iter().filter_map(|p| p.parent()) {
                tree.invalidate(dir);
            }
        }
        let default_indent = self.settings.default_indent();
        for doc in self.docs.iter_mut().filter(|d| changed.contains(&d.path)) {
            if let Some(message) = doc.on_disk_changed(default_indent) {
                self.status = message;
            }
        }
        let mut dirs: HashSet<PathBuf> = self
            .docs
            .iter()
            .filter_map(|d| d.path.parent().map(Path::to_path_buf))
            .collect();
        if let Some(tree) = &self.tree {
            dirs.extend(tree.loaded_dirs().cloned());
        }
        watcher.sync(dirs);
    }

    fn open_folder_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            let dir = real_path(&dir);
            self.status = format!("프로젝트: {}", dir.display());
            self.tree = Some(FileTree::new(dir));
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        // 링크로 연 파일과 실제 파일을 같은 문서로 보고, 감시 알림 경로와도 맞춘다.
        let path = real_path(&path);
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            self.active = Some(i);
            return;
        }
        match Document::open(path.clone(), self.settings.default_indent()) {
            Ok(doc) => {
                self.docs.push(doc);
                self.active = Some(self.docs.len() - 1);
            }
            Err(e) => self.status = format!("열기 실패: {}: {e}", path.display()),
        }
    }

    /// 저장 성공 여부를 돌려준다. 실패 사유는 상태바에 표시한다.
    fn save(&mut self, index: usize) -> bool {
        let doc = &mut self.docs[index];
        match doc.save() {
            Ok(()) => {
                if let Some(lang) = doc.lang {
                    self.hub.saved(&doc.path, lang, &doc.buffer, &mut doc.lsp);
                }
                self.status = format!("저장됨: {}", doc.path.display());
                if settings_path().is_some_and(|p| real_path(&p) == doc.path) {
                    self.settings_changed = true;
                }
                true
            }
            Err(e) => {
                self.status = format!("저장 실패: {}: {e}", doc.path.display());
                false
            }
        }
    }

    fn request_close_tab(&mut self, index: usize) {
        if self.docs[index].is_dirty() {
            self.pending_close = Some(PendingClose::Tab(index));
        } else {
            self.close_tab(index);
        }
    }

    fn close_tab(&mut self, index: usize) {
        let doc = self.docs.remove(index);
        if let Some(lang) = doc.lang {
            self.hub.closed(&doc.path, lang);
        }
        self.active = match self.active {
            _ if self.docs.is_empty() => None,
            Some(a) if a > index => Some(a - 1),
            Some(a) if a == index => Some(index.min(self.docs.len() - 1)),
            other => other,
        };
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_shortcut(&OPEN_FOLDER)) {
            self.open_folder_dialog();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&NEW_FILE)) {
            self.new_file_dialog();
        }
        // Ctrl+S는 Shift가 더 눌린 입력과도 맞으므로, Shift가 붙은 단축키를 먼저 확인한다.
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE_AS))
            && let Some(i) = self.active
        {
            self.save_as_dialog(i);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE))
            && let Some(i) = self.active
        {
            self.save(i);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&CLOSE_TAB))
            && let Some(i) = self.active
        {
            self.request_close_tab(i);
        }
        for (shortcut, replace) in [(FIND, false), (REPLACE, true)] {
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.open_find(replace);
            }
        }
        if self.find.is_some() {
            if ctx.input_mut(|i| i.consume_shortcut(&FIND_PREV)) {
                self.run_find_action(FindAction::Prev);
            }
            if ctx.input_mut(|i| i.consume_shortcut(&FIND_NEXT)) {
                self.run_find_action(FindAction::Next);
            }
        }
        // Shift가 붙은 단축키를 먼저 확인한다(F12가 Shift+F12에도 맞으므로).
        if ctx.input_mut(|i| i.consume_shortcut(&FIND_REFERENCES)) {
            self.request_at_cursor(LspRequest::References);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&GOTO_DEFINITION)) {
            self.request_at_cursor(LspRequest::Definition);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&COMPLETE)) {
            self.request_at_cursor(LspRequest::Completion(None));
        }
        if ctx.input_mut(|i| i.consume_shortcut(&FORMAT)) {
            self.request_at_cursor(LspRequest::Format);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&RENAME_SYMBOL)) {
            self.open_symbol_rename();
        }
        if ctx
            .input_mut(|i| i.consume_shortcut(&CODE_ACTION) || i.consume_shortcut(&CODE_ACTION_ALT))
        {
            self.request_at_cursor(LspRequest::CodeActions);
        }
    }

    /// 커서 위치 기호의 새 이름을 묻는다(F2).
    fn open_symbol_rename(&mut self) {
        let Some(i) = self.active else {
            return;
        };
        let buffer = &self.docs[i].buffer;
        let pos = buffer.selection().head;
        let name = word_at(buffer, pos);
        if name.is_empty() {
            self.status = "이름을 바꿀 기호 위에 커서를 두세요".into();
            return;
        }
        self.name_prompt = Some(NamePrompt {
            target: NameTarget::Symbol { pos },
            name,
            error: None,
            focus: true,
        });
    }

    fn request_at_cursor(&mut self, kind: LspRequest) {
        if let Some(i) = self.active {
            let pos = self.docs[i].buffer.selection().head;
            self.lsp_request(kind, pos);
        }
    }

    /// 활성 문서의 `pos` 위치로 언어 서버 요청을 보낸다. 응답은 `on_lsp_response`에서 처리한다.
    fn lsp_request(&mut self, kind: LspRequest, pos: usize) {
        self.send_lsp_request(kind, pos, 0);
    }

    fn send_lsp_request(&mut self, kind: LspRequest, pos: usize, attempts: u8) {
        let Some(i) = self.active else {
            return;
        };
        let doc = &mut self.docs[i];
        let Some(lang) = doc.lang else {
            return;
        };
        let path = doc.path.clone();
        // 로프 복제는 내부 노드를 공유하므로 싸다.
        let rope = doc.buffer.rope().clone();
        let (method, pending) = match &kind {
            LspRequest::Completion(_) => (
                "textDocument/completion",
                Pending::Completion {
                    path: path.clone(),
                    anchor: word_start(&doc.buffer, pos),
                },
            ),
            LspRequest::Hover => (
                "textDocument/hover",
                Pending::Hover {
                    path: path.clone(),
                    pos,
                },
            ),
            LspRequest::Definition => ("textDocument/definition", Pending::Definition),
            LspRequest::References => (
                "textDocument/references",
                Pending::References {
                    name: word_at(&doc.buffer, pos),
                },
            ),
            LspRequest::Format => (
                "textDocument/formatting",
                Pending::Format { path: path.clone() },
            ),
            LspRequest::Rename(_) => ("textDocument/rename", Pending::Rename),
            LspRequest::CodeActions => (
                "textDocument/codeAction",
                Pending::CodeActions { path: path.clone() },
            ),
            LspRequest::Resolve(_) => ("codeAction/resolve", Pending::Resolve),
            LspRequest::Execute(_) => ("workspace/executeCommand", Pending::Execute),
        };
        // 코드 액션은 선택 범위(없으면 커서)에 걸친 진단 원본을 함께 보낸다.
        let selection = doc.buffer.selection().range();
        let diagnostics: Vec<lsp_types::Diagnostic> = doc
            .lsp
            .problems
            .iter()
            .filter(|p| p.range.start <= selection.end && selection.start <= p.range.end)
            .map(|p| p.raw.clone())
            .collect();
        let (tab_size, insert_spaces) =
            (self.settings.tab_size, !matches!(doc.indent, Indent::Tabs));
        let params = |encoding: PositionEncoding| match &kind {
            LspRequest::Completion(trigger) => {
                completion_params(&path, &rope, pos, encoding, *trigger)
            }
            LspRequest::References => references_params(&path, &rope, pos, encoding),
            LspRequest::Hover | LspRequest::Definition => {
                position_params(&path, &rope, pos, encoding)
            }
            LspRequest::Format => formatting_params(&path, tab_size, insert_spaces),
            LspRequest::Rename(name) => rename_params(&path, &rope, pos, encoding, name),
            LspRequest::CodeActions => {
                code_action_params(&path, &rope, selection.clone(), encoding, &diagnostics)
            }
            LspRequest::Resolve(action) => Some(action.clone()),
            LspRequest::Execute(command) => Some(command.clone()),
        };
        match self
            .hub
            .request(&doc.path, lang, &doc.buffer, &mut doc.lsp, method, params)
        {
            Some(id) => {
                let retry = Retry {
                    kind,
                    pos,
                    path: doc.path.clone(),
                    attempts,
                    version: doc.buffer.version(),
                };
                self.pending.insert((lang, id), (pending, retry));
            }
            None if kind.is_explicit() => {
                self.status = "언어 서버가 준비되지 않았습니다".into();
            }
            None => {}
        }
    }

    fn on_lsp_response(
        &mut self,
        lang: Lang,
        id: i64,
        result: Result<Value, RpcError>,
        encoding: PositionEncoding,
    ) {
        let Some((pending, retry)) = self.pending.remove(&(lang, id)) else {
            return;
        };
        let result = match result {
            Ok(result) => result,
            // 서버가 "내용이 바뀌었다"며 요청을 버리면 다시 보낸다. 단, 처음 요청한 문서 버전
            // 그대로일 때만. 그 사이 문서가 바뀌었으면 처음 위치가 지금 문서와 맞지 않고,
            // 지금 버전을 붙여 보내면 오래된 편집 거부까지 우회해 엉뚱한 곳을 고친다.
            Err(e) if e.is_retryable() && retry.attempts < MAX_LSP_RETRIES => {
                if self.request_still_current(&retry)
                    && self.active.is_some_and(|i| self.docs[i].path == retry.path)
                {
                    self.send_lsp_request(retry.kind, retry.pos, retry.attempts + 1);
                }
                return;
            }
            Err(e) => {
                // 호버·자동완성 실패는 흔하고(위치가 맞지 않음 등) 알릴 필요가 없다.
                if !matches!(pending, Pending::Hover { .. } | Pending::Completion { .. }) {
                    self.status = format!("언어 서버 오류: {e}");
                }
                return;
            }
        };
        let active = self.active.map(|i| &self.docs[i]);
        match pending {
            Pending::Completion { path, anchor } => {
                // 응답이 오기 전에 다른 문서로 옮겼으면 버린다.
                let Some(doc) = active.filter(|d| d.path == path) else {
                    return;
                };
                let list = parse_completion(&result, doc.buffer.rope(), encoding);
                let mut popup = CompletionPopup::new(path, anchor, list.items, list.incomplete);
                self.completion = popup.refilter(&doc.buffer).then_some(popup);
            }
            Pending::Hover { path, pos } => {
                if let Some(hover) = &mut self.hover
                    && hover.path == path
                    && hover.pos == pos
                {
                    hover.text = parse_hover(&result);
                }
            }
            Pending::Definition => match parse_locations(&result).into_iter().next() {
                Some(target) => self.navigate_to(&target.path, target.range, encoding),
                None => self.status = "정의를 찾지 못했습니다".into(),
            },
            Pending::References { name } => {
                let targets = parse_locations(&result);
                if targets.is_empty() {
                    self.status = format!("'{name}'의 참조가 없습니다");
                } else {
                    let root = self.tree.as_ref().map(FileTree::root);
                    self.references = Some(ReferenceList::new(&name, targets, encoding, root));
                }
            }
            Pending::Format { path } => {
                let edits = parse_text_edits(&result);
                if edits.is_empty() {
                    self.status = "포맷할 내용이 없습니다".into();
                } else if self.request_still_current(&retry) {
                    let file = FileEdit {
                        path,
                        edits,
                        version: None,
                    };
                    if self
                        .apply_workspace_edit(lang, vec![file], encoding)
                        .is_some()
                    {
                        self.status = "포맷했습니다".into();
                    }
                }
            }
            Pending::Rename => {
                if !self.request_still_current(&retry) {
                    return;
                }
                let edits = parse_workspace_edit(&result);
                match self.apply_workspace_edit(lang, edits, encoding) {
                    Some((_, 0)) => self.status = "바꿀 곳이 없습니다".into(),
                    Some((files, changes)) => {
                        self.status = format!(
                            "이름을 바꿨습니다: 파일 {files}개, {changes}곳(저장하지 않음)"
                        );
                    }
                    None => {}
                }
            }
            Pending::CodeActions { path } => {
                let items = parse_code_actions(&result);
                if items.is_empty() {
                    self.status = "가능한 코드 액션이 없습니다".into();
                } else {
                    self.actions = Some(ActionMenu::new(path, items, encoding, retry.version));
                }
            }
            Pending::Resolve => {
                if !self.request_still_current(&retry) {
                    return;
                }
                let (edits, command) = parse_resolved_action(&result);
                if self.apply_workspace_edit(lang, edits, encoding).is_some()
                    && let Some(command) = command
                {
                    self.execute_command(lang, command, encoding);
                }
            }
            // 명령의 결과 편집은 서버가 workspace/applyEdit로 따로 보낸다.
            Pending::Execute => {}
        }
    }

    /// 요청한 뒤 그 문서가 바뀌지 않았는지. 바뀌었으면 서버가 옛 내용 기준으로 계산한 결과라
    /// 적용하면 엉뚱한 곳을 고치게 되므로(실제로 겹친 응답에서 코드가 깨졌다) 버린다.
    /// 사용자가 직접 한 요청일 때만 알린다(타이핑 중 자동완성·호버가 버려지는 것은 흔하다).
    fn request_still_current(&mut self, retry: &Retry) -> bool {
        let unchanged = self
            .docs
            .iter()
            .find(|d| d.path == retry.path)
            .is_some_and(|d| d.buffer.version() == retry.version);
        if !unchanged && retry.kind.is_explicit() {
            self.status =
                "요청 뒤에 문서가 바뀌어 결과를 적용하지 않았습니다. 다시 시도하세요".into();
        }
        unchanged
    }

    /// 고른 코드 액션을 실행한다: 편집이 있으면 적용하고, 명령이 있으면 서버에 실행을 요청하고,
    /// 편집이 아직 없으면 resolve로 받아 온다.
    fn run_code_action(&mut self, menu: ActionMenu) {
        let (path, encoding, version) = (menu.path.clone(), menu.encoding, menu.version);
        let Some(action) = menu.take_selected() else {
            return;
        };
        let doc = self.docs.iter().find(|d| d.path == path);
        let Some(lang) = doc.and_then(|d| d.lang) else {
            return;
        };
        if doc.is_some_and(|d| d.buffer.version() != version) {
            self.status = "메뉴를 연 뒤 문서가 바뀌었습니다. 코드 액션을 다시 여세요".into();
            return;
        }
        if !action.edit.is_empty()
            && self
                .apply_workspace_edit(lang, action.edit, encoding)
                .is_none()
        {
            return;
        }
        if let Some(unresolved) = action.unresolved {
            self.request_at_cursor(LspRequest::Resolve(unresolved));
        } else if let Some(command) = action.command {
            self.execute_command(lang, command, encoding);
        }
    }

    /// 코드 액션의 명령을 실행한다. jdt.ls의 `java.apply.workspaceEdit`는 클라이언트가 처리하는
    /// 명령이라(서버로 보내면 처리기가 없다는 오류) 인자로 온 편집을 직접 적용한다.
    fn execute_command(&mut self, lang: Lang, command: Value, encoding: PositionEncoding) {
        if command.get("command").and_then(Value::as_str) != Some("java.apply.workspaceEdit") {
            self.request_at_cursor(LspRequest::Execute(command));
            return;
        }
        let arguments = command.get("arguments").and_then(Value::as_array);
        for edit in arguments.into_iter().flatten() {
            self.apply_workspace_edit(lang, parse_workspace_edit(edit), encoding);
        }
    }

    /// 여러 파일 편집을 적용한다. 열려 있지 않은 파일은 탭으로 열어 버퍼에만 반영하고
    /// 저장하지 않는다(사용자가 확인하고 저장하도록). 원래 보던 탭으로 돌아온다.
    /// 서버가 편집에 붙인 문서 버전이 지금과 다르면 하나도 적용하지 않고 None.
    /// 적용했으면 (바뀐 파일 수, 바뀐 곳 수).
    fn apply_workspace_edit(
        &mut self,
        lang: Lang,
        edits: Vec<FileEdit>,
        encoding: PositionEncoding,
    ) -> Option<(usize, usize)> {
        let stale = edits.iter().any(|f| {
            f.version.is_some_and(|v| {
                self.hub
                    .document_version(lang, &real_path(&f.path))
                    .is_some_and(|current| current != v)
            })
        });
        if stale {
            self.status =
                "서버의 편집이 지금 문서와 맞지 않아 적용하지 않았습니다. 다시 시도하세요".into();
            return None;
        }
        let original = self.active.map(|i| self.docs[i].path.clone());
        let (mut files, mut changes) = (0, 0);
        for file in edits.into_iter().filter(|f| !f.edits.is_empty()) {
            self.open_file(file.path.clone());
            let target = real_path(&file.path);
            let Some(doc) = self.docs.iter_mut().find(|d| d.path == target) else {
                continue;
            };
            let rope = doc.buffer.rope().clone();
            let ranges: Vec<(std::ops::Range<usize>, String)> = file
                .edits
                .into_iter()
                .map(|(range, text)| (char_range(&rope, range, encoding), text))
                .collect();
            files += 1;
            changes += ranges.len();
            doc.buffer.replace_ranges(ranges, Instant::now());
        }
        if let Some(original) = original {
            self.active = self.docs.iter().position(|d| d.path == original);
        }
        Some((files, changes))
    }

    /// 다른 파일일 수도 있는 위치로 이동한다(열려 있지 않으면 연다).
    fn navigate_to(&mut self, path: &Path, range: lsp_types::Range, encoding: PositionEncoding) {
        self.open_file(path.to_path_buf());
        let target = real_path(path);
        let Some(i) = self.docs.iter().position(|d| d.path == target) else {
            return;
        };
        self.active = Some(i);
        let doc = &mut self.docs[i];
        let start = char_range(doc.buffer.rope(), range, encoding).start;
        doc.buffer.set_selection(Selection::caret(start));
        doc.view.reveal_cursor();
        self.focus_editor = true;
    }

    fn accept_completion(&mut self) {
        let (Some(popup), Some(i)) = (self.completion.take(), self.active) else {
            return;
        };
        let doc = &mut self.docs[i];
        if let Some(item) = popup.selected()
            && doc.path == popup.path
        {
            let edits = popup.edits(item, &doc.buffer);
            doc.buffer.replace_ranges(edits, Instant::now());
            doc.view.reveal_cursor();
        }
    }

    fn open_find(&mut self, replace: bool) {
        match &mut self.find {
            Some(find) => find.focus(replace),
            None => {
                let selected = self
                    .active
                    .map(|i| self.docs[i].buffer.selected_text())
                    .filter(|s| !s.is_empty());
                self.find = Some(FindBar::new(replace, selected));
            }
        }
    }

    fn run_find_action(&mut self, action: FindAction) {
        if matches!(action, FindAction::Close) {
            self.find = None;
            self.focus_editor = true;
            return;
        }
        let (Some(find), Some(i)) = (&mut self.find, self.active) else {
            return;
        };
        let doc = &mut self.docs[i];
        let now = Instant::now();
        match action {
            FindAction::ReplaceOne => {
                if let Err(e) =
                    search::replace_selection(&mut doc.buffer, &find.query, &find.replacement, now)
                {
                    self.status = format!("바꾸기 실패: {e}");
                    return;
                }
                find.refresh(&doc.path, &doc.buffer);
            }
            FindAction::ReplaceAll => {
                self.status =
                    match search::replace_all(&mut doc.buffer, &find.query, &find.replacement, now)
                    {
                        Ok(n) => format!("{n}개를 바꿨습니다"),
                        Err(e) => format!("바꾸기 실패: {e}"),
                    };
                return;
            }
            _ => {}
        }
        let sel = doc.buffer.selection().range();
        let target = if matches!(action, FindAction::Prev) {
            search::prev_match(find.matches(), sel.start)
        } else {
            search::next_match(find.matches(), sel.end)
        };
        if let Some(m) = target {
            doc.buffer.set_selection(Selection {
                anchor: m.start,
                head: m.end,
            });
            doc.view.reveal_cursor();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("파일", |ui| {
                    let new_file =
                        egui::Button::new("새 파일…").shortcut_text(ctx.format_shortcut(&NEW_FILE));
                    if ui.add(new_file).clicked() {
                        ui.close();
                        self.new_file_dialog();
                    }
                    let open = egui::Button::new("폴더 열기…")
                        .shortcut_text(ctx.format_shortcut(&OPEN_FOLDER));
                    if ui.add(open).clicked() {
                        ui.close();
                        self.open_folder_dialog();
                    }
                    ui.separator();
                    let save = egui::Button::new("저장").shortcut_text(ctx.format_shortcut(&SAVE));
                    if ui.add_enabled(self.active.is_some(), save).clicked()
                        && let Some(i) = self.active
                    {
                        ui.close();
                        self.save(i);
                    }
                    let save_as = egui::Button::new("다른 이름으로 저장…")
                        .shortcut_text(ctx.format_shortcut(&SAVE_AS));
                    if ui.add_enabled(self.active.is_some(), save_as).clicked()
                        && let Some(i) = self.active
                    {
                        ui.close();
                        self.save_as_dialog(i);
                    }
                    let close =
                        egui::Button::new("탭 닫기").shortcut_text(ctx.format_shortcut(&CLOSE_TAB));
                    if ui.add_enabled(self.active.is_some(), close).clicked()
                        && let Some(i) = self.active
                    {
                        ui.close();
                        self.request_close_tab(i);
                    }
                    if ui
                        .add_enabled(self.tree.is_some(), egui::Button::new("트리 새로고침"))
                        .clicked()
                        && let Some(tree) = &mut self.tree
                    {
                        ui.close();
                        tree.refresh();
                    }
                    ui.separator();
                    if ui.button("설정 열기").clicked() {
                        ui.close();
                        self.open_settings();
                    }
                    ui.separator();
                    if ui.button("종료").clicked() {
                        ui.close();
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
                ui.menu_button("편집", |ui| {
                    for (label, shortcut, replace) in
                        [("찾기", FIND, false), ("바꾸기", REPLACE, true)]
                    {
                        let item =
                            egui::Button::new(label).shortcut_text(ctx.format_shortcut(&shortcut));
                        if ui.add_enabled(self.active.is_some(), item).clicked() {
                            ui.close();
                            self.open_find(replace);
                        }
                    }
                });
                ui.menu_button("코드", |ui| {
                    let items: [(&str, KeyboardShortcut, Option<LspRequest>); 6] = [
                        ("정의로 이동", GOTO_DEFINITION, Some(LspRequest::Definition)),
                        ("참조 찾기", FIND_REFERENCES, Some(LspRequest::References)),
                        ("자동완성", COMPLETE, Some(LspRequest::Completion(None))),
                        (
                            "코드 액션 (Alt+Enter)",
                            CODE_ACTION,
                            Some(LspRequest::CodeActions),
                        ),
                        ("문서 포맷", FORMAT, Some(LspRequest::Format)),
                        ("기호 이름 바꾸기", RENAME_SYMBOL, None),
                    ];
                    for (label, shortcut, request) in items {
                        let item =
                            egui::Button::new(label).shortcut_text(ctx.format_shortcut(&shortcut));
                        if ui.add_enabled(self.active.is_some(), item).clicked() {
                            ui.close();
                            match request {
                                Some(request) => self.request_at_cursor(request),
                                None => self.open_symbol_rename(),
                            }
                        }
                    }
                });
                ui.menu_button("도구", |ui| {
                    if ui.button("언어 서버…").clicked() {
                        ui.close();
                        self.servers.show();
                    }
                });
            });
        });
    }

    /// 언어 서버 설치·삭제를 시작한다. 지울 때는 실행 중인 서버를 먼저 내린다(Windows는
    /// 실행 중인 파일을 못 지운다). 설치는 그대로 두고 끝난 뒤 새 실행 파일로 다시 띄운다.
    fn start_server_action(&mut self, action: ServerAction, ctx: &egui::Context) {
        if let ServerAction::Uninstall(server) = &action {
            self.hub.stop(server.name);
        }
        self.servers.start(action, ctx);
    }

    /// 끝난 설치·삭제를 알리고, 그 서버를 쓰던 언어를 다시 띄우게 한다.
    fn finish_server_jobs(&mut self) {
        for done in self.servers.poll() {
            self.hub.stop(done.server.name);
            self.status = match done.result {
                Ok(()) => format!("{} {}를 마쳤습니다", done.server.name, done.verb),
                Err(e) => format!("{} {} 실패: {e}", done.server.name, done.verb),
            };
        }
    }

    /// 열린 문서를 언어 서버와 맞추고, 서버에서 온 진단·메시지를 반영한다.
    fn sync_language_servers(&mut self) {
        self.hub.set_root(self.tree.as_ref().map(FileTree::root));
        for doc in &mut self.docs {
            if let Some(lang) = doc.lang {
                self.hub.sync(&doc.path, lang, &doc.buffer, &mut doc.lsp);
            }
        }
        for update in self.hub.process_events() {
            match update {
                HubUpdate::Diagnostics {
                    path,
                    diagnostics,
                    encoding,
                } => {
                    if let Some(doc) = self.docs.iter_mut().find(|d| d.path == path) {
                        doc.lsp.problems = to_problems(doc.buffer.rope(), &diagnostics, encoding);
                    }
                }
                HubUpdate::Message(message) => self.status = message,
                HubUpdate::Response {
                    lang,
                    id,
                    result,
                    encoding,
                } => self.on_lsp_response(lang, id, result, encoding),
                HubUpdate::ApplyEdit {
                    lang,
                    id,
                    edits,
                    encoding,
                } => {
                    self.apply_workspace_edit(lang, edits, encoding);
                    self.hub
                        .reply(lang, id, serde_json::json!({ "applied": true }));
                }
            }
        }
    }

    /// 아래쪽 "문제" 목록. 항목을 누르면 그 위치로 이동한다.
    fn problems_panel(&mut self, ui: &mut egui::Ui) {
        let (true, Some(index)) = (self.show_problems, self.active) else {
            return;
        };
        let mut jump = None;
        egui::Panel::bottom("problems")
            .resizable(true)
            .default_size(140.0)
            .show(ui, |ui| {
                let doc = &self.docs[index];
                ui.strong(format!("문제 — {}", doc.name()));
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if doc.lsp.problems.is_empty() {
                            ui.weak("문제가 없습니다");
                        }
                        for problem in &doc.lsp.problems {
                            let start = problem.range.start.min(doc.buffer.len_chars());
                            let (line, col) = doc.buffer.pos_to_line_col(start);
                            let message = problem.message.lines().next().unwrap_or_default();
                            let text = egui::RichText::new(format!(
                                "{}:{}  [{}] {message}",
                                line + 1,
                                col + 1,
                                problem.severity.label()
                            ))
                            .color(editor_view::severity_color(ui.visuals(), problem.severity));
                            if ui.selectable_label(false, text).clicked() {
                                jump = Some(start);
                            }
                        }
                    });
            });
        if let Some(pos) = jump {
            let doc = &mut self.docs[index];
            doc.buffer.move_to(pos, false);
            doc.view.reveal_cursor();
            self.focus_editor = true;
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let mut restart = None;
        let mut install = None;
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(doc) = self.active.map(|i| &mut self.docs[i]) {
                    let count = |s| doc.lsp.problems.iter().filter(|p| p.severity == s).count();
                    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
                    if ui
                        .selectable_label(
                            self.show_problems,
                            format!("오류 {errors} · 경고 {warnings}"),
                        )
                        .on_hover_text("문제 목록 보기/숨기기")
                        .clicked()
                    {
                        self.show_problems = !self.show_problems;
                    }
                    if let Some(lang) = doc.lang
                        && let Some((server, state)) = self.hub.status(lang)
                    {
                        let name = server.name;
                        match state {
                            ServerState::Starting => {
                                ui.label(format!("{name}: 준비 중…"));
                            }
                            ServerState::Ready => {
                                ui.label(format!("{name}: 준비됨"));
                            }
                            ServerState::NotInstalled => {
                                if let Some(progress) = self.servers.progress_text(name) {
                                    ui.label(format!("{name}: {progress}"));
                                } else if ferrite_lsp::install::installable(server) {
                                    let hint = install_hint(server);
                                    if ui
                                        .button(format!("{name} 설치"))
                                        .on_hover_text(format!(
                                            "언어 서버가 없습니다. Ferrite가 {name} {}을(를) \
                                             설치합니다({hint}). 도구 > 언어 서버에서 관리",
                                            server.version
                                        ))
                                        .clicked()
                                    {
                                        install = Some(server);
                                    }
                                } else {
                                    ui.label(format!("{name}: 설치 안 됨"))
                                        .on_hover_text(format!(
                                            "PATH에서 {name}을(를) 찾지 못했고, \
                                             이 OS·CPU용 배포본도 없습니다"
                                        ));
                                }
                            }
                            ServerState::Failed(reason) => {
                                let text =
                                    egui::RichText::new(format!("{name}: 오류 (눌러서 다시 시작)"))
                                        .color(ui.visuals().error_fg_color);
                                if ui.link(text).on_hover_text(reason.as_str()).clicked() {
                                    restart = Some(lang);
                                }
                            }
                        }
                    }
                    let before = (doc.encoding, doc.line_ending);
                    egui::ComboBox::new("line_ending", "줄바꿈")
                        .selected_text(doc.line_ending.label())
                        .show_ui(ui, |ui| {
                            for le in LineEnding::ALL {
                                ui.selectable_value(&mut doc.line_ending, le, le.label());
                            }
                        });
                    egui::ComboBox::new("encoding", "인코딩")
                        .selected_text(doc.encoding.label())
                        .show_ui(ui, |ui| {
                            for enc in Encoding::ALL {
                                ui.selectable_value(&mut doc.encoding, enc, enc.label());
                            }
                        })
                        .response
                        .on_hover_text("저장할 때 사용할 인코딩");
                    if (doc.encoding, doc.line_ending) != before {
                        doc.format_changed = true;
                    }
                    ui.label(match doc.indent {
                        Indent::Tabs => "탭".to_owned(),
                        Indent::Spaces(n) => format!("공백: {n}"),
                    });
                    ui.label(match (doc.lang, &doc.syntax) {
                        (Some(lang), Some(_)) => lang.name().to_owned(),
                        (Some(lang), None) => format!("{} (대용량: 구문 강조 꺼짐)", lang.name()),
                        (None, _) => "일반 텍스트".to_owned(),
                    });
                    let (line, col) = doc.buffer.pos_to_line_col(doc.buffer.selection().head);
                    ui.label(format!("줄 {}, 열 {}", line + 1, col + 1));
                }
                // 남은 폭에 상태 메시지를 둔다. 길면 말줄임으로 잘라 오른쪽 항목과 겹치지 않게 한다.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(&self.status).truncate());
                });
            });
        });
        if let Some(lang) = restart {
            self.hub.restart(lang);
        }
        if let Some(server) = install {
            let ctx = ui.ctx().clone();
            let action = ServerAction::Install {
                server,
                files: Vec::new(),
            };
            self.start_server_action(action, &ctx);
        }
    }

    fn project_panel(&mut self, ui: &mut egui::Ui) {
        let mut tree_action = None;
        let mut pick_folder = false;
        egui::Panel::left("project")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink(false)
                    .show(ui, |ui| match &mut self.tree {
                        Some(tree) => {
                            tree_action = tree.ui(ui);
                        }
                        None => {
                            pick_folder = ui.button("폴더 열기…").clicked();
                        }
                    });
            });
        if pick_folder {
            self.open_folder_dialog();
        }
        if let Some(action) = tree_action {
            self.handle_tree_action(action);
        }
    }

    /// 자동완성 팝업·코드 액션 메뉴가 열려 있으면 이동·적용·닫기 키를 에디터보다 먼저 가로챈다.
    fn handle_popup_keys(&mut self, ctx: &egui::Context) {
        let pressed = |key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
        if let Some(menu) = &mut self.actions {
            if pressed(Key::ArrowDown) {
                menu.move_selection(true);
            }
            if pressed(Key::ArrowUp) {
                menu.move_selection(false);
            }
            if pressed(Key::Escape) {
                self.actions = None;
            } else if pressed(Key::Enter)
                && let Some(menu) = self.actions.take()
            {
                self.run_code_action(menu);
            }
            return;
        }
        if self.completion.is_none() {
            return;
        }
        if pressed(Key::Escape) {
            self.completion = None;
            return;
        }
        if pressed(Key::Enter) || pressed(Key::Tab) {
            self.accept_completion();
            return;
        }
        if let Some(popup) = &mut self.completion {
            if pressed(Key::ArrowDown) {
                popup.move_selection(true);
            }
            if pressed(Key::ArrowUp) {
                popup.move_selection(false);
            }
        }
    }

    /// 에디터를 그린 뒤 언어 서버 기능을 이어 붙인다: Ctrl+클릭 정의 이동, 입력에 따른
    /// 자동완성 요청·다시 거르기, 팝업 그리기, 마우스가 머물면 호버 요청.
    fn after_editor(&mut self, ctx: &egui::Context, frame: EditorFrame) {
        let Some(i) = self.active else {
            return;
        };
        if let Some(pos) = frame.ctrl_click {
            self.lsp_request(LspRequest::Definition, pos);
        }
        let lang = self.docs[i].lang;
        let server_ready =
            lang.is_some_and(|l| matches!(self.hub.status(l), Some((_, ServerState::Ready))));
        if frame.focused
            && server_ready
            && let (Some(lang), Some(last)) = (lang, frame.typed.chars().last())
        {
            let pos = self.docs[i].buffer.selection().head;
            let popup_open = self
                .completion
                .as_ref()
                .is_some_and(|p| p.path == frame.path);
            let incomplete = self.completion.as_ref().is_some_and(|p| p.incomplete);
            if self.hub.trigger_characters(lang).contains(&last) {
                self.lsp_request(LspRequest::Completion(Some(last)), pos);
            } else if crate::lsp_ui::is_identifier_char(last) && (!popup_open || incomplete) {
                self.lsp_request(LspRequest::Completion(None), pos);
            }
        }
        if let Some(popup) = &mut self.completion {
            let doc = &self.docs[i];
            if popup.path != doc.path || !popup.refilter(&doc.buffer) {
                self.completion = None;
            }
        }
        if let Some(popup) = &mut self.completion
            && popup.ui(ctx, frame.cursor_rect.left_bottom())
        {
            self.accept_completion();
            self.focus_editor = true;
        }
        if self.actions.as_ref().is_some_and(|m| m.path != frame.path) {
            self.actions = None;
        }
        if let Some(menu) = &mut self.actions
            && menu.ui(ctx, frame.cursor_rect.left_bottom())
            && let Some(menu) = self.actions.take()
        {
            self.run_code_action(menu);
            self.focus_editor = true;
        }
        match frame.hover_pos {
            None => self.hover = None,
            Some(pos) => {
                let same = self
                    .hover
                    .as_ref()
                    .is_some_and(|h| h.path == frame.path && h.pos == pos);
                if !same {
                    self.hover = Some(HoverState {
                        path: frame.path.clone(),
                        pos,
                        since: Instant::now(),
                        requested: false,
                        text: None,
                    });
                }
                if let Some(hover) = &mut self.hover
                    && !hover.requested
                {
                    if hover.since.elapsed() >= HOVER_DELAY {
                        hover.requested = true;
                        self.lsp_request(LspRequest::Hover, pos);
                    } else {
                        ctx.request_repaint_after(HOVER_DELAY);
                    }
                }
            }
        }
    }

    /// 아래쪽 "참조" 목록. 항목을 누르면 그 위치로 이동한다.
    fn references_panel(&mut self, ui: &mut egui::Ui) {
        let Some(list) = &self.references else {
            return;
        };
        let (mut jump, mut close) = (None, false);
        egui::Panel::bottom("references")
            .resizable(true)
            .default_size(160.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(&list.title);
                    close = ui.small_button("×").on_hover_text("닫기").clicked();
                });
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        for (n, item) in list.items.iter().enumerate() {
                            if ui
                                .selectable_label(
                                    false,
                                    egui::RichText::new(&item.label).monospace(),
                                )
                                .clicked()
                            {
                                jump = Some(n);
                            }
                        }
                    });
            });
        if close {
            self.references = None;
        } else if let Some(n) = jump {
            let item = &list.items[n];
            let (path, range, encoding) =
                (item.target.path.clone(), item.target.range, list.encoding);
            self.navigate_to(&path, range, encoding);
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        self.handle_popup_keys(ui.ctx());
        // 에디터가 처리하기 전에 이번 프레임에 입력된 글자를 본다(자동완성 트리거 판단용).
        let typed: String = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect()
        });
        let mut frame = None;
        let mut close = None;
        let mut find_action = None;
        let mut reload = false;
        // 스크린리더가 에디터를 "현재 탭 이름"으로 읽도록 에디터를 활성 탭에 연결한다.
        let mut active_tab = None;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, doc) in self.docs.iter().enumerate() {
                            let tab = ui
                                .selectable_label(self.active == Some(i), doc.title())
                                .on_hover_text(doc.path.display().to_string());
                            if self.active == Some(i) {
                                active_tab = Some(tab.id);
                            }
                            if tab.clicked() {
                                self.active = Some(i);
                            }
                            if tab.middle_clicked() || ui.small_button("×").clicked() {
                                close = Some(i);
                            }
                        }
                    });
                });
            ui.separator();

            let Some(doc) = self.active.map(|i| &mut self.docs[i]) else {
                ui.centered_and_justified(|ui| ui.label("열린 파일이 없습니다"));
                return;
            };
            if doc.external_change {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "디스크의 파일이 바뀌었습니다. 저장하지 않은 변경이 있습니다.",
                    );
                    reload = ui
                        .button("디스크 내용으로 다시 불러오기 (내 변경 버림)")
                        .clicked();
                    if ui.button("무시 (저장하면 디스크 내용을 덮어씀)").clicked() {
                        doc.external_change = false;
                    }
                });
                ui.separator();
            }
            if let Some(find) = &mut self.find {
                find.refresh(&doc.path, &doc.buffer);
                find_action = find.ui(ui, &doc.buffer);
                ui.separator();
            }
            let opts = EditorOptions {
                tab_size: self.settings.tab_size,
                indent: doc.indent,
                comment_token: doc.lang.and_then(Lang::line_comment),
                search_matches: self.find.as_ref().map_or(&[], |f| f.matches()),
                problems: &doc.lsp.problems,
                hover: self
                    .hover
                    .as_ref()
                    .filter(|h| h.path == doc.path)
                    .and_then(|h| Some((h.pos, h.text.as_deref()?))),
            };
            let id = ui.make_persistent_id(&doc.path);
            if std::mem::take(&mut self.focus_editor) {
                ui.memory_mut(|m| m.request_focus(id));
            }
            let styler = doc.syntax.as_mut().map(|s| s as &mut dyn Styler);
            let output = editor_view::show(ui, id, &mut doc.buffer, &mut doc.view, &opts, styler);
            let focused = output.response.has_focus();
            if let Some(tab) = active_tab {
                output.response.labelled_by(tab);
            }
            frame = Some(EditorFrame {
                path: doc.path.clone(),
                focused,
                cursor_rect: output.cursor_rect,
                hover_pos: output.hover_pos,
                ctrl_click: output.ctrl_click,
                typed,
            });
        });
        if let Some(frame) = frame {
            self.after_editor(ui.ctx(), frame);
        }
        if let Some(action) = find_action {
            self.run_find_action(action);
        }
        if reload && let Some(i) = self.active {
            let default_indent = self.settings.default_indent();
            let doc = &mut self.docs[i];
            self.status = match doc.reload(default_indent) {
                Ok(()) => format!("디스크에서 다시 불러왔습니다: {}", doc.name()),
                Err(e) => format!("다시 불러오기 실패: {e}"),
            };
        }
        if let Some(i) = close {
            self.request_close_tab(i);
        }
    }

    /// 창 닫기 요청을 가로채, 저장하지 않은 문서가 있으면 확인 대화상자로 넘긴다.
    fn intercept_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.docs.iter().any(Document::is_dirty)
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.pending_close = Some(PendingClose::App);
        }
    }

    fn pending_close_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_close else {
            return;
        };
        let message = match pending {
            PendingClose::Tab(i) => {
                format!("'{}'의 변경 사항을 저장할까요?", self.docs[*i].title())
            }
            PendingClose::App => format!(
                "저장하지 않은 파일이 {}개 있습니다. 저장할까요?",
                self.docs.iter().filter(|d| d.is_dirty()).count()
            ),
        };
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::unique("pending_close")).show(ctx, |ui| {
            ui.heading("저장하지 않은 변경 사항");
            ui.label(message);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("저장").clicked() {
                    choice = Some(CloseChoice::Save);
                }
                if ui.button("저장 안 함").clicked() {
                    choice = Some(CloseChoice::Discard);
                }
                if ui.button("취소").clicked() {
                    choice = Some(CloseChoice::Cancel);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(CloseChoice::Cancel);
        }
        let Some(choice) = choice else {
            return;
        };

        match (self.pending_close.take(), choice) {
            (_, CloseChoice::Cancel) | (None, _) => {}
            (Some(PendingClose::Tab(i)), CloseChoice::Save) => {
                if self.save(i) {
                    self.close_tab(i);
                }
            }
            (Some(PendingClose::Tab(i)), CloseChoice::Discard) => self.close_tab(i),
            (Some(PendingClose::App), CloseChoice::Save) => {
                // 하나라도 저장에 실패하면 종료하지 않는다(실패 사유는 상태바에 남는다).
                let dirty: Vec<usize> = (0..self.docs.len())
                    .filter(|&i| self.docs[i].is_dirty())
                    .collect();
                let mut all_saved = true;
                for i in dirty {
                    all_saved &= self.save(i);
                }
                if all_saved {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
            (Some(PendingClose::App), CloseChoice::Discard) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }
}

impl eframe::App for FerriteApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_fs_events();
        self.finish_server_jobs();
        self.sync_language_servers();
        if std::mem::take(&mut self.settings_changed) {
            let (settings, error) = load_settings();
            self.status = error.unwrap_or_else(|| "설정을 적용했습니다".into());
            apply_settings(ctx, &settings);
            self.settings = settings;
        }
        self.intercept_close(ctx);
        // 확인 대화상자가 떠 있는 동안에는 단축키로 탭 목록이 바뀌지 않게 한다.
        if !self.modal_open() {
            self.handle_shortcuts(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.menu_bar(ui);
        self.status_bar(ui);
        self.problems_panel(ui);
        self.references_panel(ui);
        self.project_panel(ui);
        self.editor(ui);
        self.pending_close_dialog(ui.ctx());
        self.name_prompt_dialog(ui.ctx());
        self.delete_dialog(ui.ctx());
        let ctx = ui.ctx().clone();
        if let Some(action) = self.servers.ui(&ctx) {
            self.start_server_action(action, &ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn request_window_close(harness: &mut Harness<'_, FerriteApp>) {
        harness
            .input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
    }

    #[test]
    fn 미저장_문서가_있으면_창_닫기_전에_저장을_확인한다() {
        let dir = std::env::temp_dir().join(format!("ferrite-app-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "hello").unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));

        // 트리에서 파일 열기
        harness.get_by_label("a.txt").click();
        harness.run();
        assert_eq!(harness.state().docs.len(), 1);

        // 편집하면 탭에 미저장 표시가 붙는다
        let editor = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
        editor.focus();
        editor.type_text("!");
        harness.run();
        assert!(harness.state().docs[0].is_dirty());
        harness.get_by_label("● a.txt");

        // 창 닫기 요청은 막히고 확인 대화상자가 뜬다
        request_window_close(&mut harness);
        harness.run();
        assert!(!harness.state().allow_close);
        harness.get_by_label("저장하지 않은 변경 사항");

        // 저장을 고르면 파일에 기록되고 종료가 허용된다
        harness.get_by_label("저장").click();
        harness.run();
        assert!(harness.state().allow_close);
        assert!(!harness.state().docs[0].is_dirty());
        let saved = fs::read_to_string(&file).unwrap();
        assert!(
            saved.contains('!') && saved.contains("hello"),
            "저장 내용: {saved}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 찾기로_일치_항목을_오가고_모두_바꾼다() {
        use egui::accesskit::Role;

        let dir = std::env::temp_dir().join(format!("ferrite-find-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "foo bar foo").unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        let selection = |h: &Harness<'_, FerriteApp>| h.state().docs[0].buffer.selection().range();

        // Ctrl/Cmd+F로 열면 검색어 칸에 포커스가 간다
        harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "찾기")
            .type_text("foo");
        harness.run();
        assert_eq!(harness.state().find.as_ref().unwrap().matches().len(), 2);

        // F3은 다음, 끝에서는 처음으로 돌아간다
        harness.key_press(Key::F3);
        harness.run();
        assert_eq!(selection(&harness), 0..3);
        harness.key_press(Key::F3);
        harness.run();
        assert_eq!(selection(&harness), 8..11);
        harness.key_press(Key::F3);
        harness.run();
        assert_eq!(selection(&harness), 0..3);

        // 모두 바꾸기
        harness.key_press_modifiers(Modifiers::COMMAND, Key::H);
        harness.run();
        let field = harness.get_by_role_and_label(Role::TextInput, "바꾸기");
        field.focus();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "바꾸기")
            .type_text("X");
        harness.run();
        harness.get_by_label("모두 바꾸기").click();
        harness.run();
        assert_eq!(harness.state().docs[0].buffer.text(), "X bar X");
        assert!(harness.state().docs[0].is_dirty());

        fs::remove_dir_all(&dir).unwrap();
    }

    /// 파일 감시 알림은 OS 스레드에서 비동기로 오므로 조건이 맞을 때까지 프레임을 돌린다.
    fn run_until(
        harness: &mut Harness<'_, FerriteApp>,
        done: impl Fn(&FerriteApp) -> bool,
    ) -> bool {
        for _ in 0..50 {
            // 기다리는 동안 스피너 등이 계속 다시 그리기를 요청하므로 run()을 쓰면 패닉한다.
            harness.run_ok();
            if done(harness.state()) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    }

    #[test]
    fn 디스크에서_바뀐_파일은_다시_불러오고_미저장_변경이_있으면_충돌로_표시한다() {
        let dir = std::env::temp_dir().join(format!("ferrite-watch-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "v1").unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        harness.run(); // 감시 시작

        // 미저장 변경이 없으면 조용히 다시 불러온다
        fs::write(&file, "v2").unwrap();
        assert!(
            run_until(&mut harness, |app| app.docs[0].buffer.text() == "v2"),
            "외부 변경이 반영되지 않음"
        );
        assert!(!harness.state().docs[0].is_dirty());

        // 미저장 변경이 있으면 덮어쓰지 않고 충돌로 표시한다
        harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        harness.run();
        harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .type_text("!");
        harness.run();
        fs::write(&file, "v3").unwrap();
        assert!(
            run_until(&mut harness, |app| app.docs[0].external_change),
            "충돌이 표시되지 않음"
        );
        assert!(
            harness.state().docs[0].buffer.text().contains('!'),
            "내 편집이 보존되어야 한다"
        );

        // 사용자가 다시 불러오기를 고르면 디스크 내용으로 바뀐다
        harness
            .get_by_label("디스크 내용으로 다시 불러오기 (내 변경 버림)")
            .click();
        harness.run();
        assert_eq!(harness.state().docs[0].buffer.text(), "v3");
        assert!(!harness.state().docs[0].external_change);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 자기_저장은_외부_변경으로_취급하지_않는다() {
        let dir =
            std::env::temp_dir().join(format!("ferrite-selfsave-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        fs::write(&file, "x").unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        harness.run();
        let editor = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
        editor.focus();
        harness.run();
        harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .type_text("y");
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
        harness.run();
        // 저장으로 생긴 알림이 처리될 시간(1초)을 준다
        for _ in 0..10 {
            harness.run();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let doc = &harness.state().docs[0];
        assert!(!doc.external_change);
        assert!(!doc.is_dirty());
        assert_eq!(fs::read_to_string(&file).unwrap(), doc.buffer.text());

        fs::remove_dir_all(&dir).unwrap();
    }

    /// 같은 이름이 탭과 트리에 함께 보일 때 트리(왼쪽) 항목을 고른다.
    fn tree_item<'a>(
        harness: &'a Harness<'_, FerriteApp>,
        label: &'a str,
    ) -> egui_kittest::Node<'a> {
        harness
            .get_all_by_label(label)
            .min_by(|a, b| a.rect().min.x.total_cmp(&b.rect().min.x))
            .expect("트리 항목이 없음")
    }

    #[test]
    fn 트리_우클릭으로_파일을_만들고_이름을_바꾼다() {
        use egui::accesskit::Role;

        let name = format!("ferrite-fileops-ui-{}", std::process::id());
        let dir = std::env::temp_dir().join(&name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.run();

        // 새 파일: 만들고 바로 연다
        harness.get_by_label(&name).click_secondary();
        harness.run();
        harness.get_by_label("새 파일…").click();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "이름")
            .type_text("b.txt");
        harness.key_press(Key::Enter);
        harness.run();
        assert!(dir.join("b.txt").is_file());
        assert!(harness.state().docs[0].path.ends_with("b.txt"));

        // 잘못된 이름은 대화상자에 오류를 보이고 만들지 않는다
        harness.get_by_label(&name).click_secondary();
        harness.run();
        harness.get_by_label("새 폴더…").click();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "이름")
            .type_text("bad:name");
        harness.key_press(Key::Enter);
        harness.run();
        harness.get_by_label_contains("문자는 이름에 쓸 수 없습니다");
        harness.get_by_label("취소").click();
        harness.run();
        assert!(harness.state().name_prompt.is_none());

        // 이름 바꾸기: 열린 탭의 경로도 따라 바뀐다
        tree_item(&harness, "b.txt").click_secondary();
        harness.run();
        harness.get_by_label("이름 바꾸기…").click();
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness
            .get_by_role_and_label(Role::TextInput, "이름")
            .type_text("c.toml");
        harness.key_press(Key::Enter);
        harness.run();
        assert!(dir.join("c.toml").is_file() && !dir.join("b.txt").exists());
        let doc = &harness.state().docs[0];
        assert!(doc.path.ends_with("c.toml"));
        assert_eq!(
            doc.lang,
            Some(Lang::Toml),
            "확장자가 바뀌면 언어도 바뀐다(서버가 없는 언어로 골라 테스트가 서버를 띄우지 않게)"
        );

        // 삭제는 확인을 거친다(취소하면 그대로)
        tree_item(&harness, "c.toml").click_secondary();
        harness.run();
        harness.get_by_label("휴지통으로 삭제…").click();
        harness.run();
        harness.get_by_label("취소").click();
        harness.run();
        assert!(dir.join("c.toml").is_file());

        fs::remove_dir_all(&dir).unwrap();
    }

    // 여기부터 rust-analyzer를 실제로 띄우는 테스트는 `run()` 대신 `run_ok()`를 쓴다. 서버 알림
    // (색인 진행률 등)마다 다시 그리기를 요청해, 알림이 몰리면 `run()`이 max_steps를 넘겨 패닉한다.
    #[test]
    fn 언어_서버_진단이_상태바와_문제_목록에_보이고_누르면_이동한다() {
        if !rust_analyzer_runs() {
            return;
        }
        let (dir, file) = cargo_project("diag", "fn main() {\n    let 값 = ;\n}\n");

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        // 첫 실행은 rust-analyzer가 표준 라이브러리를 읽느라 오래 걸릴 수 있다
        let mut arrived = false;
        for _ in 0..1200 {
            harness.run_ok();
            if !harness.state().docs[0].lsp.problems.is_empty() {
                arrived = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(arrived, "진단이 오지 않음: {}", harness.state().status);

        let problem = harness.state().docs[0].lsp.problems[0].clone();
        assert_eq!(problem.severity, Severity::Error);
        assert_eq!(
            harness.state().docs[0]
                .buffer
                .pos_to_line_col(problem.range.start)
                .0,
            1,
            "둘째 줄의 문법 오류"
        );

        // 상태바의 개수를 눌러 문제 목록을 열고, 항목을 누르면 커서가 그 위치로 간다
        harness.get_by_label_contains(" · 경고").click();
        harness.run_ok();
        let (line, col) = harness.state().docs[0]
            .buffer
            .pos_to_line_col(problem.range.start);
        let entry = format!("{}:{}  [오류]", line + 1, col + 1);
        harness
            .get_all_by_label_contains(&entry)
            .next()
            .expect("문제 목록 항목")
            .click();
        harness.run_ok();
        assert_eq!(
            harness.state().docs[0].buffer.selection().head,
            problem.range.start
        );

        drop(harness);
        remove_project(&dir);
    }

    /// rust-analyzer가 실제로 실행되는지. 없으면 건너뛰되 CI에서는 실패시킨다.
    fn rust_analyzer_runs() -> bool {
        let runs = ferrite_lsp::client::find_program("rust-analyzer").is_some_and(|p| {
            std::process::Command::new(p)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        });
        if !runs {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI에는 rust-analyzer가 있어야 합니다"
            );
            eprintln!("rust-analyzer가 없어 건너뜁니다");
        }
        runs
    }

    /// 임시 Cargo 프로젝트를 만들고 (폴더, src/main.rs)를 돌려준다.
    fn cargo_project(name: &str, main: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("ferrite-lsp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        )
        .unwrap();
        let file = dir.join("src").join("main.rs");
        fs::write(&file, main).unwrap();
        (dir, file)
    }

    /// 서버 프로세스가 작업 폴더를 놓을 때까지 잠시 기다린다(Windows는 쓰는 중인 폴더를 못 지운다).
    fn remove_project(dir: &std::path::Path) {
        for _ in 0..50 {
            if fs::remove_dir_all(dir).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// 조건이 맞을 때까지(최대 2분) 프레임을 돌린다. 첫 실행 rust-analyzer는 준비가 오래 걸린다.
    fn run_until_slow(
        harness: &mut Harness<'_, FerriteApp>,
        done: impl Fn(&FerriteApp) -> bool,
    ) -> bool {
        for _ in 0..1200 {
            harness.run_ok();
            if done(harness.state()) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    }

    #[test]
    fn 자동완성_적용과_정의_이동과_참조_목록() {
        if !rust_analyzer_runs() {
            return;
        }
        let text = "struct Point {\n    x: i32,\n}\n\nimpl Point {\n    fn length(&self) -> i32 {\n        self.x\n    }\n}\n\nfn main() {\n    let p = Point { x: 1 };\n    p.length();\n    p.le\n}\n";
        let (dir, file) = cargo_project("nav", text);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        assert!(
            run_until_slow(&mut harness, |app| {
                matches!(app.hub.status(Lang::Rust), Some((_, ServerState::Ready)))
            }),
            "서버 준비 시간 초과"
        );
        // 이 테스트의 문서는 모두 ASCII라 바이트 위치 = 문자 위치
        let caret = |h: &mut Harness<'_, FerriteApp>, pos: usize| {
            h.state_mut().docs[0]
                .buffer
                .set_selection(Selection::caret(pos));
        };

        // 서버는 색인이 끝나기 전에 빈 결과를 돌려줄 수 있다(느린 CI 러너에서 실제로 발생).
        // 사용자가 다시 누르듯, 원하는 결과가 나올 때까지 같은 키를 다시 누른다(최대 약 60초).
        let retry_key = |h: &mut Harness<'_, FerriteApp>,
                         pos: usize,
                         modifiers: Modifiers,
                         key: Key,
                         done: &dyn Fn(&Harness<'_, FerriteApp>) -> bool| {
            for _ in 0..60 {
                caret(h, pos);
                h.key_press_modifiers(modifiers, key);
                for _ in 0..20 {
                    h.run_ok();
                    if done(h) {
                        return true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            false
        };

        // 자동완성
        let end_of_le = text.find("p.le\n").unwrap() + 4;
        assert!(
            retry_key(&mut harness, end_of_le, Modifiers::CTRL, Key::Space, &|h| {
                h.query_by_label_contains("length").is_some()
            }),
            "자동완성 팝업에 length가 없음"
        );
        harness.key_press(Key::Enter);
        harness.run_ok();
        let after = harness.state().docs[0].buffer.text();
        assert!(
            after.contains("p.length\n") || after.contains("p.length()\n"),
            "{after}"
        );
        assert!(harness.state().completion.is_none());

        // F12: 호출 위치에서 정의(impl 안의 fn length, 6번째 줄)로 이동
        let call = text.find("p.length();").unwrap() + 3;
        assert!(
            retry_key(&mut harness, call, Modifiers::NONE, Key::F12, &|h| {
                let buffer = &h.state().docs[0].buffer;
                buffer.pos_to_line_col(buffer.selection().head).0 == 5
            }),
            "정의로 이동하지 않음: {}",
            harness.state().status
        );

        // Shift+F12: 참조 목록(정의 + 호출들)
        assert!(
            retry_key(&mut harness, call, Modifiers::SHIFT, Key::F12, &|h| {
                h.state()
                    .references
                    .as_ref()
                    .is_some_and(|r| r.items.len() >= 2)
            }),
            "참조 목록이 열리지 않음: {}",
            harness.state().status
        );
        let refs = harness.state().references.as_ref().unwrap();
        assert!(refs.title.contains("length"), "{}", refs.title);
        assert!(refs.items.len() >= 2, "정의와 호출이 모두 있어야 한다");

        drop(harness);
        remove_project(&dir);
    }

    /// `act`를 하고 몇 프레임 돌려 `done`이 될 때까지 되풀이한다(최대 약 60초).
    /// 서버가 준비 중이면 빈 결과나 일시적 오류를 주므로, 사용자가 다시 시도하듯 반복한다.
    fn retry_until(
        harness: &mut Harness<'_, FerriteApp>,
        act: &dyn Fn(&mut Harness<'_, FerriteApp>),
        done: &dyn Fn(&Harness<'_, FerriteApp>) -> bool,
    ) -> bool {
        for _ in 0..60 {
            act(harness);
            for _ in 0..20 {
                harness.run_ok();
                if done(harness) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        false
    }

    #[test]
    fn 기호_이름_바꾸기와_포맷과_코드_액션() {
        use egui::accesskit::Role;

        if !rust_analyzer_runs() {
            return;
        }
        let text = "fn length() -> i32 {\n    1\n}\n\nfn main() {\n    let  n   =  length();\n    let _ = n;\n}\n";
        let (dir, file) = cargo_project("edits", text);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        assert!(
            run_until_slow(&mut harness, |app| {
                matches!(app.hub.status(Lang::Rust), Some((_, ServerState::Ready)))
            }),
            "서버 준비 시간 초과"
        );
        let doc_text = |h: &Harness<'_, FerriteApp>| h.state().docs[0].buffer.text();

        // F2: 정의와 호출을 함께 바꾼다
        let on_name = text.find("fn length").unwrap() + 4;
        let renamed = retry_until(
            &mut harness,
            &|h| {
                h.state_mut().docs[0]
                    .buffer
                    .set_selection(Selection::caret(on_name));
                h.key_press(Key::F2);
                h.run_ok();
                h.key_press_modifiers(Modifiers::COMMAND, Key::A);
                h.get_by_role_and_label(Role::TextInput, "이름")
                    .type_text("size");
                h.key_press(Key::Enter);
            },
            &|h| doc_text(h).contains("fn size()") && doc_text(h).contains("size();"),
        );
        assert!(renamed, "이름 바꾸기 실패: {}", harness.state().status);
        assert!(
            harness.state().docs[0].is_dirty(),
            "바꾼 결과는 저장하지 않은 상태로 둔다"
        );

        // Shift+Alt+F: rustfmt 규칙으로 정리
        let formatted = retry_until(
            &mut harness,
            &|h| h.key_press_modifiers(Modifiers::SHIFT.plus(Modifiers::ALT), Key::F),
            &|h| doc_text(h).contains("    let n = size();"),
        );
        assert!(
            formatted,
            "포맷 실패: {}\n{}",
            harness.state().status,
            doc_text(&harness)
        );

        // Ctrl+.: 코드 액션 메뉴를 열고 첫 항목을 실행하면 문서가 바뀐다
        let on_n = doc_text(&harness).find("let n").unwrap() + 4;
        let opened = retry_until(
            &mut harness,
            &|h| {
                h.state_mut().docs[0]
                    .buffer
                    .set_selection(Selection::caret(on_n));
                h.key_press_modifiers(Modifiers::COMMAND, Key::Period);
            },
            &|h| h.state().actions.is_some(),
        );
        assert!(
            opened,
            "코드 액션 메뉴가 열리지 않음: {}",
            harness.state().status
        );
        let before = harness.state().docs[0].buffer.version();
        harness.key_press(Key::Enter);
        assert!(
            run_until_slow(&mut harness, |app| app.docs[0].buffer.version() != before),
            "코드 액션이 적용되지 않음: {}",
            harness.state().status
        );
        assert!(harness.state().actions.is_none());

        drop(harness);
        remove_project(&dir);
    }

    /// 사용자의 실제 설치 폴더를 건드리지 않도록 언어 서버 설치 위치를 임시 폴더로 바꾼다.
    fn isolate_servers(harness: &mut Harness<'_, FerriteApp>, root: &Path) {
        let ctx = harness.ctx.clone();
        let app = harness.state_mut();
        app.servers = ServerManager::new(root.to_path_buf());
        app.hub = LspHub::new(ctx, root.to_path_buf());
    }

    #[test]
    fn 언어_서버_창은_업데이트를_알리고_설치본을_지운다() {
        let dir = std::env::temp_dir().join(format!("ferrite-servers-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("servers");
        // 예전 버전 lemminx가 설치된 것처럼 꾸민다(실행하지는 않는다).
        let old = root.join("lemminx").join("0.1");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("lemminx"), "").unwrap();
        fs::write(old.join(ferrite_lsp::install::RECORD), "lemminx").unwrap();
        let lemminx = ferrite_lsp::catalog::for_language("xml").unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        isolate_servers(&mut harness, &root);
        assert_eq!(harness.state().servers.outdated(), ["lemminx"]);

        harness.get_by_label("도구").click();
        harness.run();
        harness.get_by_label("언어 서버…").click();
        harness.run();
        harness.get_by_label(&format!("업데이트 있음 (0.1 → {})", lemminx.version));

        harness.get_by_label("삭제").click();
        assert!(
            run_until(&mut harness, |app| app.status.contains("삭제를 마쳤습니다")),
            "삭제가 끝나지 않음: {}",
            harness.state().status
        );
        assert!(!root.join("lemminx").exists());
        harness.run();
        if ferrite_lsp::install::installable(lemminx) {
            harness.get_by_label(&format!("설치 ({})", install_hint(lemminx)));
        }

        drop(harness);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 언어_서버가_없으면_상태바에서_설치를_제안한다() {
        if ferrite_lsp::client::find_program("lemminx").is_some() {
            eprintln!("lemminx가 PATH에 있어 건너뜀");
            return;
        }
        let dir = std::env::temp_dir().join(format!("ferrite-servers-bar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mapper = dir.join("mapper.xml");
        fs::write(&mapper, "<mapper namespace=\"a\"/>\n").unwrap();
        let config = dir.join("config.toml");
        fs::write(&config, "a = 1\n").unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(mapper.clone())));
        isolate_servers(&mut harness, &dir.join("servers"));
        assert!(run_until(&mut harness, |app| app
            .hub
            .status(Lang::Xml)
            .is_some()));
        let lemminx = ferrite_lsp::catalog::for_language("xml").unwrap();
        if ferrite_lsp::install::installable(lemminx) {
            harness.get_by_label("lemminx 설치");
        } else {
            harness.get_by_label("lemminx: 설치 안 됨");
        }

        // 맡을 서버가 없는 언어는 상태바에 서버 항목을 두지 않는다.
        harness.state_mut().open_file(config);
        harness.run();
        assert!(harness.state().hub.status(Lang::Toml).is_none());
        assert!(harness.query_by_label_contains("설치 안 됨").is_none());

        drop(harness);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn jdtls의_편집_적용_명령은_서버에_보내지_않고_직접_적용한다() {
        let dir = std::env::temp_dir().join(format!("ferrite-java-cmd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Hello.java");
        fs::write(&file, "class Hello {}\n").unwrap();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        isolate_servers(&mut harness, &dir.join("servers"));
        harness.run();
        let doc = &harness.state().docs[0];
        let (path, version) = (doc.path.clone(), doc.buffer.version());
        let uri = ferrite_lsp::uri::path_to_uri(&path).unwrap();
        // jdt.ls가 빠른 수정으로 돌려주는 모양: 편집 대신 클라이언트 명령에 WorkspaceEdit을 싣는다.
        let item = ferrite_lsp::edits::CodeActionItem {
            title: "public 붙이기".into(),
            preferred: true,
            edit: Vec::new(),
            unresolved: None,
            command: Some(serde_json::json!({
                "command": "java.apply.workspaceEdit",
                "arguments": [{ "changes": { uri.as_str(): [{
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                    "newText": "public ",
                }] } }],
            })),
        };
        harness.state_mut().actions = Some(ActionMenu::new(
            path,
            vec![item],
            PositionEncoding::Utf16,
            version,
        ));
        harness.run();
        harness.key_press(Key::Enter);
        harness.run();
        assert_eq!(
            harness.state().docs[0].buffer.text(),
            "public class Hello {}\n",
            "{}",
            harness.state().status
        );

        drop(harness);
        fs::remove_dir_all(&dir).unwrap();
    }
}
