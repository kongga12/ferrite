//! 메인 창: 메뉴바, 도킹 창(프로젝트 트리·탭 에디터·도구 창), 상태바, 미저장 변경 확인.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, ViewportCommand};
use ferrite_core::buffer::{Buffer, Indent, Selection, detect_indent};
use ferrite_core::settings::{self, Settings, Theme};
use ferrite_core::syntax::{Lang, MAX_SYNTAX_BYTES, Syntax};
use ferrite_core::text_file::{self, Encoding, LineEnding, TextContent, TextFileError};
use ferrite_core::{fs_ops, search};

use crate::dock::{self, Pane};
use crate::editor_view::{self, BreakpointMark, EditorOptions, EditorState, Styler};
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
use ferrite_lsp::problem::Severity;
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
const QUICK_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::P);
const SYMBOL_SEARCH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::T);
/// `.http` 파일에서 커서가 든 요청을 보낸다.
const RUN_REQUEST: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);
const OUTLINE: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
const SOURCE_CONTROL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
// 터미널은 VS Code처럼 macOS에서도 실제 Ctrl+`다.
const TOGGLE_TERMINAL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Backtick);
// macOS의 Cmd+F5는 VoiceOver를 켜므로 모든 OS에서 Ctrl을 쓴다.
const RUN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::F5);
// 디버그. egui는 덧붙은 Shift를 무시하고 맞추므로 Shift 조합을 먼저 확인한다.
const DEBUG: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
const DEBUG_STOP: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F5);
const DEBUG_PAUSE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F6);
const TOGGLE_BREAKPOINT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F9);
const STEP_OVER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F10);
const STEP_IN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F11);
const STEP_OUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F11);
const FIND_IN_FILES: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);
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

/// jdt.ls에 메인 클래스·클래스패스·디버그 포트를 묻는 중인 java-debug 시작.
struct JavaDebugPending {
    /// 기다리는 jdt.ls 요청 번호.
    id: i64,
    start: ferrite_dap::java::JavaStart,
    config: ferrite_core::debug_config::DebugConfig,
    file: Option<PathBuf>,
    ctx: egui::Context,
}

/// java-debug를 싣는 언어 서버.
const JDTLS: &str = "jdtls";

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

/// 받거나 가져온 egov 템플릿을 두는 곳(언어 서버 옆). 새 프로젝트 템플릿은 `egov-<태그>/`,
/// 공통컴포넌트는 `commngt-<버전>/`에 둔다.
fn templates_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("ferrite")
        .join("templates")
}

fn templates_root() -> PathBuf {
    templates_dir().join(format!("egov-{}", ferrite_codegen::scaffold::RELEASE))
}

fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("ferrite").join("settings.toml"))
}

/// DB 연결 목록 파일. 테스트는 사용자 목록을 읽거나 덮어쓰지 않는다.
fn connections_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(dirs::config_dir()?.join("ferrite").join("connections.toml"))
}

/// DB 비밀번호 저장소. 테스트는 사용자 키체인을 건드리지 않는다.
fn secret_store() -> std::sync::Arc<dyn ferrite_db::secret::SecretStore> {
    if cfg!(test) {
        std::sync::Arc::new(ferrite_db::secret::MemoryStore::default())
    } else {
        std::sync::Arc::new(ferrite_db::secret::Keychain)
    }
}

/// JDBC 사이드카(컴파일한 클래스)와 [받기]로 받은 드라이버를 두는 곳. 테스트는 사용자 폴더에 쓰지 않는다.
fn jdbc_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(dirs::data_local_dir()?.join("ferrite").join("jdbc"))
}

/// 드라이버 jar를 찾는 로컬 Maven 저장소. 테스트는 사용자 저장소를 보지 않는다.
fn maven_repository() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(dirs::home_dir()?.join(".m2").join("repository"))
}

/// 창 배치 파일. 테스트는 사용자 배치를 읽거나 덮어쓰지 않는다.
fn layout_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(dirs::config_dir()?.join("ferrite").join("layout.json"))
}

/// 심볼릭 링크를 푼 실제 절대 경로. 파일 감시 알림은 실제 경로로 오므로(macOS의 `/var` →
/// `/private/var` 등) 문서·트리 경로를 이것으로 통일해야 알림과 비교할 수 있다.
/// 아직 없는 경로면 절대 경로만 만든다.
pub(crate) fn real_path(path: &Path) -> PathBuf {
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

/// 업데이트 정보(latest.json) 주소: 설정(사내 미러) 또는 공식 릴리스.
fn update_url(settings: &Settings) -> String {
    match settings.update_url.trim() {
        "" => ferrite_update::MANIFEST_URL.to_owned(),
        url => url.to_owned(),
    }
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
fn move_document(
    hub: &mut LspHub,
    languages: &ferrite_textmate::Registry,
    doc: &mut Document,
    path: PathBuf,
) {
    if let Some(lang) = doc.lang {
        hub.closed(&doc.path, lang);
    }
    doc.lsp = DocSync::default();
    doc.set_path(path);
    doc.set_extension(languages);
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
    /// git 인덱스 대비 줄 변경(거터 표시).
    git_marks: Vec<(usize, ferrite_core::git::LineChange)>,
    /// git_marks를 계산한 (버퍼 버전, 기준 내용 세대). 바뀌면 다시 계산한다.
    git_key: Option<(u64, u64)>,
    /// `.http` 파일이면 (센 버퍼 버전, 요청 줄). 거터에 ▶를 그린다.
    http_runs: Option<(u64, Vec<usize>)>,
    /// 이 문서에 해당하는 정적 분석(SARIF) 결과(에디터 물결선)와 그것을 만든 (SARIF 세대, 버퍼 버전).
    sarif_problems: Vec<ferrite_lsp::problem::Problem>,
    sarif_key: Option<(u64, u64)>,
    /// .ipynb면 노트북 보기(버퍼 위에 셀을 보인다).
    notebook: Option<crate::notebook_ui::NotebookView>,
    /// tree-sitter 언어가 없는 파일의 언어 확장(플러그인 폴더)과 그 TextMate 강조.
    ext: Option<std::sync::Arc<ferrite_textmate::Language>>,
    textmate: Option<ferrite_textmate::TextMateSyntax>,
}

impl Document {
    fn open(path: PathBuf, default_indent: Indent) -> Result<Self, TextFileError> {
        let bytes = text_file::read(&path)?;
        let content = text_file::decode(&bytes)?;
        let lang = Lang::from_path(&path);
        let notebook = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ipynb"))
            .then(crate::notebook_ui::NotebookView::default);
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
            git_marks: Vec::new(),
            git_key: None,
            http_runs: None,
            sarif_problems: Vec::new(),
            sarif_key: None,
            notebook,
            ext: None,
            textmate: None,
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
        self.reset_textmate();
    }

    /// 언어 확장을 붙인다(tree-sitter 언어가 없는 파일만).
    fn set_extension(&mut self, languages: &ferrite_textmate::Registry) {
        self.ext = self
            .lang
            .is_none()
            .then(|| languages.language_for(&self.path))
            .flatten();
        self.reset_textmate();
    }

    /// 언어 확장의 강조를 처음부터 다시 만든다. 큰 파일은 tree-sitter처럼 강조하지 않는다.
    fn reset_textmate(&mut self) {
        self.textmate = self
            .ext
            .as_ref()
            .and_then(|e| e.grammar())
            .filter(|_| self.buffer.rope().len_bytes() <= MAX_SYNTAX_BYTES)
            .map(ferrite_textmate::TextMateSyntax::new);
        self.buffer
            .set_track_syntax_edits(self.syntax.is_some() || self.textmate.is_some());
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
    /// egov CRUD 코드 생성 창.
    codegen: crate::codegen::CodegenWindow,
    /// egov 설정 파일 생성 창.
    config_gen: crate::config_gen::ConfigWindow,
    /// 새 egov 프로젝트 창.
    new_project: crate::new_project::NewProjectWindow,
    /// 공통컴포넌트 추가 창.
    commngt: crate::commngt_ui::CommngtWindow,
    /// DB 연결 창.
    db: crate::db_ui::DbWindow,
    sql: crate::sql_console::SqlConsole,
    schema: crate::schema_ui::SchemaBrowser,
    /// 스키마 브라우저가 맡긴 일. 배치를 바꾸므로 창 밖에서 처리한다.
    schema_action: Option<crate::schema_ui::Action>,
    /// 매퍼 SQL을 스키마와 맞춰 본 문서: 경로 → (문서 버전, 스키마 세대).
    mapper_checked: HashMap<PathBuf, (u64, u64)>,
    /// 창 배치. 그리는 동안에는 창들이 앱을 빌려 쓰므로 잠시 밖으로 꺼낸다(`ui`).
    layout: dock::Layout,
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
    /// 파일로 이동(Ctrl+P).
    quick_open: crate::quick_open::QuickOpen,
    /// 파일에서 찾기(Ctrl+Shift+F) 패널.
    find_in_files: crate::find_in_files::FindInFiles,
    /// 통합 터미널(Ctrl+`) 패널.
    terminal: crate::terminal_ui::TerminalPanel,
    /// 실행 구성 고르기(Ctrl+F5).
    run: crate::picker::Picker<ferrite_core::run::RunConfig>,
    /// 디버그 구성 고르기(F5).
    debug_picker: crate::picker::Picker<ferrite_core::debug_config::DebugConfig>,
    debugger: crate::debug_ui::Debugger,
    /// 파일별 중단점(줄 순, 1부터). 디버그하지 않을 때도 둔다.
    breakpoints: std::collections::BTreeMap<PathBuf, Vec<ferrite_dap::Breakpoint>>,
    /// 조건을 고치고 있는 중단점(파일, 줄(1부터), 조건식).
    breakpoint_condition: Option<(PathBuf, usize, String)>,
    /// jdt.ls에 묻는 중인 java-debug 시작.
    java_debug: Option<JavaDebugPending>,
    /// git 상태·소스 제어 창(Ctrl+Shift+G).
    git: crate::git_ui::GitPanel,
    /// 다음 프레임에 에디터로 키보드 포커스를 돌려준다(찾기 막대를 닫은 뒤 등).
    focus_editor: bool,
    settings: Settings,
    /// 설정 파일을 저장해 다음 프레임에 다시 적용해야 한다.
    settings_changed: bool,
    /// 자동 업데이트(새 판 확인, 받기, 닫을 때 설치).
    updater: crate::update_ui::Updater,
    /// WASM 플러그인(메뉴 명령, 플러그인 창).
    plugins: crate::plugin_ui::Plugins,
    /// 개요 창, 기호 찾기(Ctrl+T), 엔드포인트 창.
    outline: crate::symbols_ui::Outline,
    symbol_search: crate::symbols_ui::SymbolSearch,
    endpoints: crate::symbols_ui::Endpoints,
    /// HTTP 요청 파일 실행과 응답 창. 창을 다 그린 뒤에 할 요청 일.
    http: crate::http_ui::HttpPanel,
    http_task: Option<crate::http_ui::Task>,
    /// 테스트 창(테스트 명령 실행, 결과 트리).
    tests: crate::test_ui::TestPanel,
    /// 문제 창의 정적 분석(SARIF)과, 문제 창이 그것을 보이는지(아니면 현재 문서의 언어 서버 진단).
    sarif: crate::sarif_ui::SarifPanel,
    problems_sarif: bool,
    /// jsconfig.json을 만들자고 할지 본 문서와 그 결과(화면 폴더). 문서가 바뀌면 다시 본다.
    jsconfig: Option<(PathBuf, Option<&'static str>)>,
    /// jsconfig.json 제안을 닫은 프로젝트 루트(앱이 켜진 동안).
    jsconfig_dismissed: std::collections::HashSet<PathBuf>,
    /// 감시를 시작하지 못한 환경(권한 등)에서는 없다. 그래도 편집은 된다.
    watcher: Option<FsWatcher>,
}

impl FerriteApp {
    /// `start`는 명령줄로 받은 시작 경로. 폴더면 프로젝트로, 파일이면 상위 폴더를 프로젝트로 열고 파일을 연다.
    pub fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>) -> Self {
        crate::fonts::install_korean_font(&cc.egui_ctx);
        if let Some(dir) = jdbc_dir() {
            ferrite_db::jdbc::set_home(dir);
        }
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
                // Context::format_shortcut은 macOS에서 ⌘ 기호를 그릴 수 있는지 글꼴로 확인하는데, 첫 프레임 전에는
                // 글꼴이 없어 패닉한다. 그래서 기호 대신 이름(Cmd·Ctrl)으로 직접 만든다.
                format!(
                    "폴더를 열어 시작하세요 ({})",
                    OPEN_FOLDER.format(&egui::ModifierNames::NAMES, cfg!(target_os = "macos"))
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
            quick_open: Default::default(),
            find_in_files: Default::default(),
            terminal: Default::default(),
            run: crate::picker::Picker::new("실행", ferrite_core::run::CUSTOM_FILE),
            debug_picker: crate::picker::Picker::new(
                "디버그",
                ferrite_core::debug_config::CUSTOM_FILE,
            ),
            debugger: Default::default(),
            breakpoints: Default::default(),
            breakpoint_condition: None,
            java_debug: None,
            git: Default::default(),
            focus_editor: false,
            settings,
            settings_changed: false,
            updater: Default::default(),
            plugins: crate::plugin_ui::Plugins::load(crate::plugin_ui::plugins_dir()),
            outline: Default::default(),
            symbol_search: Default::default(),
            endpoints: Default::default(),
            http: Default::default(),
            http_task: None,
            tests: Default::default(),
            sarif: Default::default(),
            problems_sarif: false,
            jsconfig: None,
            jsconfig_dismissed: Default::default(),
            watcher: FsWatcher::new(cc.egui_ctx.clone()).ok(),
            name_prompt: None,
            confirm_delete: None,
            hub: LspHub::new(cc.egui_ctx.clone(), servers_root()),
            servers: ServerManager::new(servers_root()),
            codegen: Default::default(),
            config_gen: Default::default(),
            new_project: crate::new_project::NewProjectWindow::new(templates_root()),
            commngt: crate::commngt_ui::CommngtWindow::new(templates_dir()),
            db: crate::db_ui::DbWindow::new(connections_path(), secret_store())
                .with_driver_dirs(maven_repository(), jdbc_dir().map(|d| d.join("drivers"))),
            sql: Default::default(),
            schema: Default::default(),
            schema_action: None,
            mapper_checked: HashMap::new(),
            layout: dock::load(layout_path().as_deref()),
            pending: std::collections::HashMap::new(),
            completion: None,
            actions: None,
            hover: None,
            references: None,
        };
        let root = app.tree.as_ref().map(|t| t.root().to_path_buf());
        app.git.set_root(root.as_deref());
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
        // 테스트는 밖에 묻지 않는다.
        if !cfg!(test) {
            crate::update_ui::clean_downloads();
        }
        if app.settings.update_check && !cfg!(test) {
            app.updater
                .check(&update_url(&app.settings), false, &cc.egui_ctx);
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
                    move_document(&mut self.hub, self.plugins.languages(), doc, moved);
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
                move_document(
                    &mut self.hub,
                    self.plugins.languages(),
                    doc,
                    real_path(&target),
                );
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
            self.open_folder(&dir);
        }
    }

    fn open_folder(&mut self, dir: &Path) {
        let dir = real_path(dir);
        self.status = format!("프로젝트: {}", dir.display());
        self.git.set_root(Some(&dir));
        self.tree = Some(FileTree::new(dir));
    }

    fn open_file(&mut self, path: PathBuf) {
        // 링크로 연 파일과 실제 파일을 같은 문서로 보고, 감시 알림 경로와도 맞춘다.
        let path = real_path(&path);
        if let Some(i) = self.docs.iter().position(|d| d.path == path) {
            self.active = Some(i);
            return;
        }
        match Document::open(path.clone(), self.settings.default_indent()) {
            Ok(mut doc) => {
                doc.set_extension(self.plugins.languages());
                self.docs.push(doc);
                self.active = Some(self.docs.len() - 1);
                // 거터 변경 표시의 기준(인덱스 내용)을 바로 읽는다.
                self.git.request_refresh();
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
                self.git.request_refresh();
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
        if ctx.input_mut(|i| i.consume_shortcut(&SYMBOL_SEARCH)) {
            self.symbol_search.show();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&OUTLINE)) {
            self.show_outline();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&QUICK_OPEN)) {
            self.open_quick_open(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&TOGGLE_TERMINAL)) {
            self.toggle_terminal();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&RUN)) {
            self.open_run_picker(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&DEBUG_STOP)) {
            self.debugger.stop();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&DEBUG)) {
            self.debug_continue_or_start(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&DEBUG_PAUSE)) {
            self.debugger.pause();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&STEP_OUT)) {
            self.debugger.resume(crate::debug_ui::Step::Out);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&STEP_IN)) {
            self.debugger.resume(crate::debug_ui::Step::In);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&STEP_OVER)) {
            self.debugger.resume(crate::debug_ui::Step::Over);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&TOGGLE_BREAKPOINT))
            && let Some(i) = self.active
        {
            let doc = &self.docs[i];
            let line = doc.buffer.pos_to_line_col(doc.buffer.selection().head).0 + 1;
            let path = doc.path.clone();
            self.toggle_breakpoint(&path, line);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SOURCE_CONTROL)) {
            self.git.toggle();
        }
        // Ctrl+F가 Ctrl+Shift+F에도 맞으므로 먼저 확인한다.
        if ctx.input_mut(|i| i.consume_shortcut(&FIND_IN_FILES)) {
            self.open_find_in_files();
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

    /// 매퍼 XML SQL의 DB 표·열 항목(스키마 브라우저가 읽은 스키마). 범위는 문서 글자 자리다.
    fn mapper_completions(&self, doc: &Document) -> Vec<ferrite_lsp::features::Completion> {
        let tables = self.schema.tables();
        if doc.lang != Some(Lang::Xml) || tables.is_empty() {
            return Vec::new();
        }
        let rope = doc.buffer.rope();
        let cursor = rope.char_to_byte(doc.buffer.selection().head);
        let Some((range, mut items)) =
            crate::mapper_db::completions(&rope.to_string(), cursor, tables)
        else {
            return Vec::new();
        };
        let range = rope.byte_to_char(range.start)..rope.byte_to_char(range.end);
        for item in &mut items {
            item.range = Some(range.clone());
        }
        items
    }

    /// 열린 매퍼 XML의 SQL을 스키마 브라우저가 읽은 스키마와 맞춰 본다. 문서나 스키마가 바뀐 것만.
    fn check_mapper_sql(&mut self) {
        let Self {
            docs,
            schema,
            mapper_checked,
            ..
        } = self;
        let generation = schema.generation();
        for doc in docs.iter_mut().filter(|d| d.lang == Some(Lang::Xml)) {
            let key = (doc.buffer.version(), generation);
            if mapper_checked.get(&doc.path) == Some(&key) {
                continue;
            }
            mapper_checked.insert(doc.path.clone(), key);
            let rope = doc.buffer.rope();
            let encoding = PositionEncoding::Utf16;
            let position = |byte: usize| {
                ferrite_lsp::position::to_lsp(rope, rope.byte_to_char(byte), encoding)
            };
            let diagnostics: Vec<lsp_types::Diagnostic> = crate::mapper_db::problems(
                &rope.to_string(),
                schema.tables(),
                schema.connection_name(),
            )
            .into_iter()
            .map(|problem| lsp_types::Diagnostic {
                range: lsp_types::Range::new(
                    position(problem.range.start),
                    position(problem.range.end),
                ),
                severity: Some(lsp_types::DiagnosticSeverity::WARNING),
                source: Some("ferrite-db".into()),
                message: problem.message,
                ..Default::default()
            })
            .collect();
            doc.lsp
                .set_problems("ferrite-db", false, rope, &diagnostics, encoding);
        }
    }

    fn send_lsp_request(&mut self, kind: LspRequest, pos: usize, attempts: u8) {
        let Some(i) = self.active else {
            return;
        };
        // 매퍼 SQL의 DB 표·열은 언어 서버를 기다리지 않고 바로 보인다(서버 응답이 오면 합친다).
        let mut shown = false;
        if matches!(kind, LspRequest::Completion(_)) {
            let items = self.mapper_completions(&self.docs[i]);
            if !items.is_empty() {
                let doc = &self.docs[i];
                let mut popup = CompletionPopup::new(
                    doc.path.clone(),
                    word_start(&doc.buffer, pos),
                    items,
                    false,
                );
                if popup.refilter(&doc.buffer) {
                    self.completion = Some(popup);
                    shown = true;
                }
            }
        }
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
            LspRequest::Definition => (
                "textDocument/definition",
                Pending::Definition {
                    name: word_at(&doc.buffer, pos),
                },
            ),
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
            None if kind.is_explicit() && !shown => {
                self.status = "언어 서버가 준비되지 않았습니다".into();
            }
            None => {}
        }
    }

    fn on_lsp_response(
        &mut self,
        lang: Lang,
        server: &'static str,
        id: i64,
        result: Result<Value, RpcError>,
        encoding: PositionEncoding,
    ) {
        if server == JDTLS
            && let Some(java) = self.java_debug.as_mut().filter(|j| j.id == id)
        {
            let next = java.start.answer(result.map_err(|e| e.to_string()));
            self.continue_java_debug(next);
            return;
        }
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
                let mut list = parse_completion(&result, doc.buffer.rope(), encoding);
                list.items.extend(self.mapper_completions(doc));
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
            Pending::Definition { name } => {
                // 선언 위에서 부르면 jdt.ls가 선언 자신을 돌려준다. 다른 서버(egov)가 찾은 곳이
                // 있으면 그쪽이 가고 싶은 곳이므로 커서가 있던 자리는 뺀다.
                let mut targets = parse_locations(&result);
                let here = |target: &ferrite_lsp::features::Target| {
                    active.is_some_and(|doc| {
                        doc.path == real_path(&target.path)
                            && char_range(doc.buffer.rope(), target.range, encoding)
                                .contains(&retry.pos)
                    })
                };
                if targets.len() > 1 {
                    targets.retain(|t| !here(t));
                }
                match targets.len() {
                    0 => self.status = "정의를 찾지 못했습니다".into(),
                    1 => {
                        let target = &targets[0];
                        self.navigate_to(&target.path, target.range, encoding);
                    }
                    _ => {
                        let root = self.tree.as_ref().map(FileTree::root);
                        let list = ReferenceList::new("정의", &name, targets, encoding, root);
                        self.show_references(list);
                    }
                }
            }
            Pending::References { name } => {
                let targets = parse_locations(&result);
                if targets.is_empty() {
                    self.status = format!("'{name}'의 참조가 없습니다");
                } else {
                    let root = self.tree.as_ref().map(FileTree::root);
                    let list = ReferenceList::new("참조", &name, targets, encoding, root);
                    self.show_references(list);
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
                        .apply_workspace_edit(server, vec![file], encoding)
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
                match self.apply_workspace_edit(server, edits, encoding) {
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
                    let menu = ActionMenu::new(path, items, encoding, retry.version, server);
                    self.actions = Some(menu);
                }
            }
            Pending::Resolve => {
                if !self.request_still_current(&retry) {
                    return;
                }
                let (edits, command) = parse_resolved_action(&result);
                match (self.apply_workspace_edit(server, edits, encoding), command) {
                    (Some(_), Some(command)) => self.execute_command(server, command, encoding),
                    (Some((_, 0)), None) => {
                        self.status = "코드 액션에 적용할 편집이 없습니다".into();
                    }
                    _ => {}
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
        let (path, encoding, version, server) =
            (menu.path.clone(), menu.encoding, menu.version, menu.server);
        let Some(action) = menu.take_selected() else {
            return;
        };
        let Some(doc) = self.docs.iter().find(|d| d.path == path) else {
            return;
        };
        if doc.buffer.version() != version {
            self.status = "메뉴를 연 뒤 문서가 바뀌었습니다. 코드 액션을 다시 여세요".into();
            return;
        }
        if !action.edit.is_empty()
            && self
                .apply_workspace_edit(server, action.edit, encoding)
                .is_none()
        {
            return;
        }
        if let Some(unresolved) = action.unresolved {
            self.request_at_cursor(LspRequest::Resolve(unresolved));
        } else if let Some(command) = action.command {
            self.execute_command(server, command, encoding);
        }
    }

    /// 코드 액션의 명령을 실행한다. jdt.ls의 `java.apply.workspaceEdit`는 클라이언트가 처리하는
    /// 명령이라(서버로 보내면 처리기가 없다는 오류) 인자로 온 편집을 직접 적용한다.
    fn execute_command(
        &mut self,
        server: &'static str,
        command: Value,
        encoding: PositionEncoding,
    ) {
        if command.get("command").and_then(Value::as_str) != Some("java.apply.workspaceEdit") {
            self.request_at_cursor(LspRequest::Execute(command));
            return;
        }
        let arguments = command.get("arguments").and_then(Value::as_array);
        for edit in arguments.into_iter().flatten() {
            self.apply_workspace_edit(server, parse_workspace_edit(edit), encoding);
        }
    }

    /// 여러 파일 편집을 적용한다. 열려 있지 않은 파일은 탭으로 열어 버퍼에만 반영하고
    /// 저장하지 않는다(사용자가 확인하고 저장하도록). 원래 보던 탭으로 돌아온다.
    /// `server`(편집을 준 서버)가 편집에 붙인 문서 버전이 지금과 다르면 하나도 적용하지 않고 None.
    /// 적용했으면 (바뀐 파일 수, 바뀐 곳 수).
    fn apply_workspace_edit(
        &mut self,
        server: &'static str,
        edits: Vec<FileEdit>,
        encoding: PositionEncoding,
    ) -> Option<(usize, usize)> {
        let stale = edits.iter().any(|f| {
            f.version.is_some_and(|v| {
                self.hub
                    .document_version(server, &real_path(&f.path))
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

    /// 자동완성 팝업을 닫고, 아직 오지 않은 자동완성 응답도 버린다. 그러지 않으면 늦게 온 응답이
    /// 적용하거나 닫은 팝업을 다시 연다(Ctrl+Space를 여러 번 눌렀을 때 등).
    fn close_completion(&mut self) -> Option<CompletionPopup> {
        self.pending
            .retain(|_, (pending, _)| !matches!(pending, Pending::Completion { .. }));
        self.completion.take()
    }

    /// 코드 액션 메뉴를 닫고, 아직 오지 않은 코드 액션 응답도 버린다(자동완성과 같은 이유:
    /// Ctrl+.를 여러 번 누르면 실행하거나 닫은 메뉴를 늦게 온 응답이 다시 연다).
    fn close_actions(&mut self) -> Option<ActionMenu> {
        self.pending
            .retain(|_, (pending, _)| !matches!(pending, Pending::CodeActions { .. }));
        self.actions.take()
    }

    fn accept_completion(&mut self) {
        let (Some(popup), Some(i)) = (self.close_completion(), self.active) else {
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

    fn open_quick_open(&mut self, ctx: &egui::Context) {
        match self.tree.as_ref().map(|t| t.root().to_path_buf()) {
            Some(root) => self.quick_open.show(&root, ctx),
            None => self.status = "먼저 프로젝트 폴더를 여세요".into(),
        }
    }

    fn toggle_terminal(&mut self) {
        dock::toggle(&mut self.layout, Pane::Terminal);
        self.terminal.request_focus();
    }

    fn open_run_picker(&mut self, ctx: &egui::Context) {
        let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) else {
            self.status = "먼저 프로젝트 폴더를 여세요".into();
            return;
        };
        let (custom, error) = match ferrite_core::run::load_custom(&root) {
            Ok(custom) => (custom, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        self.run
            .show(custom, ferrite_core::run::detect(&root), error, ctx);
    }

    /// F5: 멈춰 있으면 계속, 디버그 중이 아니면 구성을 고르게 한다.
    fn debug_continue_or_start(&mut self, ctx: &egui::Context) {
        if self.debugger.is_stopped() {
            self.debugger.resume(crate::debug_ui::Step::Continue);
        } else if !self.debugger.is_active() {
            self.open_debug_picker(ctx);
        }
    }

    fn open_debug_picker(&mut self, ctx: &egui::Context) {
        let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) else {
            self.status = "먼저 프로젝트 폴더를 여세요".into();
            return;
        };
        let (custom, error) = match ferrite_core::debug_config::load_custom(&root) {
            Ok(custom) => (custom, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let file = self.active.map(|i| self.docs[i].path.as_path());
        let detected = ferrite_core::debug_config::detect(file, &root);
        self.debug_picker.show(custom, detected, error, ctx);
    }

    fn start_debug(
        &mut self,
        config: &ferrite_core::debug_config::DebugConfig,
        ctx: &egui::Context,
    ) {
        let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) else {
            return;
        };
        let file = self.active.map(|i| self.docs[i].path.clone());
        if config.adapter == "java-debug" {
            self.begin_java_debug(config, &root, file, ctx);
            return;
        }
        self.launch_debug(config, &root, file.as_deref(), None, ctx);
    }

    fn launch_debug(
        &mut self,
        config: &ferrite_core::debug_config::DebugConfig,
        root: &Path,
        file: Option<&Path>,
        adapter: Option<ferrite_dap::Adapter>,
        ctx: &egui::Context,
    ) {
        // 어댑터 문제(설치 안 됨 등)는 빌드하기 전에 알린다.
        let adapter = match adapter {
            Some(adapter) => Ok(adapter),
            None => ferrite_dap::adapters::resolve(&config.adapter, &servers_root(), root),
        };
        let started = adapter.and_then(|adapter| {
            self.debugger
                .start(config, root, file, self.breakpoints.clone(), adapter, ctx)
        });
        match started {
            Ok(()) => {
                dock::show(&mut self.layout, Pane::DebugConsole);
                dock::show(&mut self.layout, Pane::Debug);
                self.status = format!("디버그 시작: {}", config.name);
            }
            Err(e) => self.status = format!("디버그를 시작하지 못했습니다: {e}"),
        }
    }

    /// java-debug는 jdt.ls에 실린 번들이라 jdt.ls에 차례로 묻고(메인 클래스·클래스패스·Java·빌드) 받은
    /// 디버그 포트로 붙는다. jdt.ls는 Java 파일을 열면 뜬다.
    fn begin_java_debug(
        &mut self,
        config: &ferrite_core::debug_config::DebugConfig,
        root: &Path,
        file: Option<PathBuf>,
        ctx: &egui::Context,
    ) {
        let arguments = match config.resolved_arguments(root, file.as_deref()) {
            Ok(arguments) => arguments,
            Err(e) => {
                self.status = format!("디버그를 시작하지 못했습니다: {e}");
                return;
            }
        };
        let start = ferrite_dap::java::JavaStart::new(arguments, config.attach(), file.as_deref());
        let pending = JavaDebugPending {
            id: 0,
            start,
            config: config.clone(),
            file,
            ctx: ctx.clone(),
        };
        let next = pending.start.command();
        self.java_debug = Some(pending);
        self.status = format!("디버그 준비 중(jdt.ls): {}", config.name);
        self.continue_java_debug(next);
    }

    fn continue_java_debug(&mut self, next: ferrite_dap::java::Next) {
        use ferrite_dap::java::Next;
        match next {
            Next::Command(command, arguments) => {
                match self
                    .hub
                    .execute_command(JDTLS, Lang::Java, command, arguments)
                {
                    Some(id) => {
                        if let Some(pending) = &mut self.java_debug {
                            pending.id = id;
                        }
                    }
                    None => {
                        self.java_debug = None;
                        self.status = "jdt.ls가 준비되지 않았습니다. Java 파일을 열고 jdt.ls가 \
                                       준비되면 다시 시작하세요"
                            .into();
                    }
                }
            }
            Next::Ready { port, arguments } => {
                let Some(mut pending) = self.java_debug.take() else {
                    return;
                };
                let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) else {
                    return;
                };
                pending.config.arguments = arguments;
                let adapter = ferrite_dap::Adapter::Tcp(([127, 0, 0, 1], port).into());
                self.launch_debug(
                    &pending.config,
                    &root,
                    pending.file.as_deref(),
                    Some(adapter),
                    &pending.ctx,
                );
            }
            Next::Failed(message) => {
                self.java_debug = None;
                self.status = format!("디버그를 시작하지 못했습니다: {message}");
            }
        }
    }

    /// 디버거가 알린 일(멈춘 자리 보이기, 상태 글)을 한다.
    fn process_debug_notices(&mut self) {
        for notice in self.debugger.poll() {
            self.on_debug_notice(notice);
        }
    }

    fn on_debug_notice(&mut self, notice: crate::debug_ui::DebugNotice) {
        match notice {
            crate::debug_ui::DebugNotice::Reveal(path, line) => {
                self.open_file(path.clone());
                if let Some(i) = self.active.filter(|&i| self.docs[i].path == path) {
                    let doc = &mut self.docs[i];
                    let line = line.min(doc.buffer.line_count().saturating_sub(1));
                    doc.buffer.move_to(doc.buffer.line_start(line), false);
                    doc.view.reveal_cursor();
                }
            }
            crate::debug_ui::DebugNotice::Status(text) => self.status = text,
        }
    }

    /// 그 줄(1부터)의 중단점을 켜고 끈다.
    fn toggle_breakpoint(&mut self, path: &Path, line: usize) {
        let list = self.breakpoints.entry(path.to_path_buf()).or_default();
        match list.iter().position(|b| b.line == line) {
            Some(i) => {
                list.remove(i);
            }
            None => {
                list.push(ferrite_dap::Breakpoint {
                    line,
                    condition: None,
                });
                list.sort_by_key(|b| b.line);
            }
        }
        self.sync_breakpoints(path);
    }

    /// 바뀐 중단점을 디버그 중인 어댑터에 보낸다. 빈 목록은 지운다.
    fn sync_breakpoints(&mut self, path: &Path) {
        let list = self.breakpoints.get(path).cloned().unwrap_or_default();
        if list.is_empty() {
            self.breakpoints.remove(path);
        }
        self.debugger.set_breakpoints(path, list);
    }

    /// 중단점 조건 창. 빈 조건이면 보통 중단점이 된다.
    fn breakpoint_condition_dialog(&mut self, ctx: &egui::Context) {
        let Some((path, line, condition)) = &mut self.breakpoint_condition else {
            return;
        };
        let (mut apply, mut remove, mut open) = (false, false, true);
        egui::Window::new("중단점 조건")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{}:{line}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                let label = ui.label("조건식(참일 때만 멈춤)");
                let field = ui
                    .add(egui::TextEdit::singleline(condition).desired_width(320.0))
                    .labelled_by(label.id);
                if !field.has_focus() && !field.lost_focus() {
                    field.request_focus();
                }
                apply = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.horizontal(|ui| {
                    apply |= ui.button("확인").clicked();
                    remove = ui.button("중단점 지우기").clicked();
                });
            });
        if !(apply || remove) {
            if !open {
                self.breakpoint_condition = None;
            }
            return;
        }
        let Some((path, line, condition)) = self.breakpoint_condition.take() else {
            return;
        };
        let list = self.breakpoints.entry(path.clone()).or_default();
        list.retain(|b| b.line != line);
        if apply {
            let condition = condition.trim();
            list.push(ferrite_dap::Breakpoint {
                line,
                condition: (!condition.is_empty()).then(|| condition.to_owned()),
            });
            list.sort_by_key(|b| b.line);
        }
        self.sync_breakpoints(&path);
    }

    /// 거터 클릭: 왼쪽 단추는 중단점 켜고 끄기, 오른쪽 단추는 조건 고치기.
    fn on_gutter_click(&mut self, path: PathBuf, line: usize, secondary: bool) {
        if let Some(doc) = self.docs.iter().find(|d| d.path == path)
            && let Some((_, runs)) = &doc.http_runs
        {
            // 요청 파일의 거터는 중단점 대신 ▶(요청 보내기)다.
            if !secondary && runs.contains(&line) {
                self.http_task = Some(crate::http_ui::Task::Run { path, line });
            }
            return;
        }
        let line = line + 1;
        if secondary {
            let condition = self
                .breakpoints
                .get(&path)
                .and_then(|list| list.iter().find(|b| b.line == line))
                .and_then(|b| b.condition.clone())
                .unwrap_or_default();
            self.breakpoint_condition = Some((path, line, condition));
        } else {
            self.toggle_breakpoint(&path, line);
        }
    }

    /// 사용자 디버그 구성 파일을 연다. 없으면 견본(모두 주석)으로 만든다.
    fn edit_debug_configs(&mut self) {
        let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) else {
            return;
        };
        let path = root.join(ferrite_core::debug_config::CUSTOM_FILE);
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| {
                    text_file::write_atomic(
                        &path,
                        ferrite_core::debug_config::CUSTOM_TEMPLATE.as_bytes(),
                    )
                });
            if let Err(e) = created {
                self.status = format!("디버그 구성 파일을 만들지 못했습니다: {e}");
                return;
            }
        }
        self.open_file(path);
        self.focus_editor = true;
    }

    /// 실행 구성을 터미널 탭에서 돌리고 터미널 창을 앞으로 가져온다.
    fn start_run(&mut self, config: &ferrite_core::run::RunConfig, ctx: &egui::Context) {
        let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) else {
            return;
        };
        self.terminal.run(config, &root, ctx);
        dock::show(&mut self.layout, Pane::Terminal);
        self.status = format!("실행: {}", config.name);
    }

    /// 사용자 실행 구성 파일을 연다. 없으면 견본(모두 주석)으로 만든다.
    fn edit_run_configs(&mut self) {
        let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) else {
            return;
        };
        let path = root.join(ferrite_core::run::CUSTOM_FILE);
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| {
                    text_file::write_atomic(&path, ferrite_core::run::CUSTOM_TEMPLATE.as_bytes())
                });
            if let Err(e) = created {
                self.status = format!("실행 구성 파일을 만들지 못했습니다: {e}");
                return;
            }
        }
        self.open_file(path);
        self.focus_editor = true;
    }

    fn show_references(&mut self, list: ReferenceList) {
        self.references = Some(list);
        dock::show(&mut self.layout, Pane::References);
    }

    fn open_find_in_files(&mut self) {
        let selected = self
            .active
            .map(|i| self.docs[i].buffer.selected_text())
            .filter(|s| !s.is_empty());
        self.find_in_files.show(selected);
        dock::show(&mut self.layout, Pane::Search);
    }

    fn find_in_files_panel(&mut self, ui: &mut egui::Ui) {
        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
        if let Some(jump) = self.find_in_files.ui(ui, root.as_deref()) {
            let at = lsp_types::Position::new(jump.line as u32, jump.column as u32);
            // 검색과 버퍼가 같은 방식(UTF-8·CP949, CRLF → LF)으로 읽으므로 바이트 위치가 맞는다.
            self.navigate_to(
                &jump.path,
                lsp_types::Range::new(at, at),
                PositionEncoding::Utf8,
            );
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
                    ui.separator();
                    let project = self.tree.is_some();
                    let item = egui::Button::new("파일로 이동…")
                        .shortcut_text(ctx.format_shortcut(&QUICK_OPEN));
                    if ui.add_enabled(project, item).clicked() {
                        ui.close();
                        self.open_quick_open(&ctx);
                    }
                    let item = egui::Button::new("파일에서 찾기…")
                        .shortcut_text(ctx.format_shortcut(&FIND_IN_FILES));
                    if ui.add_enabled(project, item).clicked() {
                        ui.close();
                        self.open_find_in_files();
                    }
                    ui.separator();
                    let item = egui::Button::new("기호로 이동…")
                        .shortcut_text(ctx.format_shortcut(&SYMBOL_SEARCH));
                    if ui.add(item).clicked() {
                        ui.close();
                        self.symbol_search.show();
                    }
                    let item =
                        egui::Button::new("개요").shortcut_text(ctx.format_shortcut(&OUTLINE));
                    if ui.add(item).clicked() {
                        ui.close();
                        self.show_outline();
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
                ui.menu_button("보기", |ui| {
                    if ui.button("창 배치 초기화").clicked() {
                        ui.close();
                        self.find_in_files.stop();
                        self.references = None;
                        self.layout = dock::default_layout();
                    }
                });
                ui.menu_button("실행", |ui| {
                    let project = self.tree.is_some();
                    let item = egui::Button::new("실행…").shortcut_text(ctx.format_shortcut(&RUN));
                    if ui.add_enabled(project, item).clicked() {
                        ui.close();
                        self.open_run_picker(&ctx);
                    }
                    if ui
                        .add_enabled(project, egui::Button::new("실행 구성 편집…"))
                        .clicked()
                    {
                        ui.close();
                        self.edit_run_configs();
                    }
                    if ui
                        .add_enabled(project, egui::Button::new("테스트"))
                        .clicked()
                    {
                        ui.close();
                        dock::show(&mut self.layout, Pane::Tests);
                    }
                    ui.separator();
                    let debugging = self.debugger.is_active();
                    let stopped = self.debugger.is_stopped();
                    let label = if stopped {
                        "계속"
                    } else {
                        "디버그 시작…"
                    };
                    let item = egui::Button::new(label).shortcut_text(ctx.format_shortcut(&DEBUG));
                    if ui
                        .add_enabled(project && (stopped || !debugging), item)
                        .clicked()
                    {
                        ui.close();
                        self.debug_continue_or_start(&ctx);
                    }
                    let steps = [
                        ("한 줄씩", STEP_OVER, crate::debug_ui::Step::Over),
                        ("들어가기", STEP_IN, crate::debug_ui::Step::In),
                        ("나오기", STEP_OUT, crate::debug_ui::Step::Out),
                    ];
                    for (label, shortcut, step) in steps {
                        let item =
                            egui::Button::new(label).shortcut_text(ctx.format_shortcut(&shortcut));
                        if ui.add_enabled(stopped, item).clicked() {
                            ui.close();
                            self.debugger.resume(step);
                        }
                    }
                    let item = egui::Button::new("디버그 멈춤")
                        .shortcut_text(ctx.format_shortcut(&DEBUG_STOP));
                    if ui.add_enabled(debugging, item).clicked() {
                        ui.close();
                        self.debugger.stop();
                    }
                    let item = egui::Button::new("중단점 켜기/끄기")
                        .shortcut_text(ctx.format_shortcut(&TOGGLE_BREAKPOINT));
                    if ui.add_enabled(self.active.is_some(), item).clicked() {
                        ui.close();
                        if let Some(i) = self.active {
                            let doc = &self.docs[i];
                            let line =
                                doc.buffer.pos_to_line_col(doc.buffer.selection().head).0 + 1;
                            let path = doc.path.clone();
                            self.toggle_breakpoint(&path, line);
                        }
                    }
                    if ui
                        .add_enabled(project, egui::Button::new("디버그 구성 편집…"))
                        .clicked()
                    {
                        ui.close();
                        self.edit_debug_configs();
                    }
                });
                ui.menu_button("도구", |ui| {
                    let item = egui::Button::new("터미널")
                        .shortcut_text(ctx.format_shortcut(&TOGGLE_TERMINAL));
                    if ui.add(item).clicked() {
                        ui.close();
                        self.toggle_terminal();
                    }
                    let item = egui::Button::new("소스 제어")
                        .shortcut_text(ctx.format_shortcut(&SOURCE_CONTROL));
                    if ui.add(item).clicked() {
                        ui.close();
                        self.git.toggle();
                    }
                    if ui.button("언어 서버…").clicked() {
                        ui.close();
                        self.servers.show();
                    }
                    if ui.button("업데이트 확인…").clicked() {
                        ui.close();
                        self.updater.check(&update_url(&self.settings), true, &ctx);
                    }
                    if ui.button("egov CRUD 코드 생성…").clicked() {
                        ui.close();
                        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
                        self.codegen.show(root.as_deref());
                    }
                    if ui.button("egov 설정 파일 생성…").clicked() {
                        ui.close();
                        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
                        self.config_gen.show(root.as_deref());
                    }
                    if ui.button("새 egov 프로젝트…").clicked() {
                        ui.close();
                        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
                        self.new_project.show(root.as_deref());
                    }
                    if ui.button("공통컴포넌트 추가…").clicked() {
                        ui.close();
                        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
                        self.commngt.show(root.as_deref());
                    }
                    ui.separator();
                    if ui.button("DB 연결…").clicked() {
                        ui.close();
                        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
                        self.db.show(root.as_deref());
                    }
                    if ui.button("스키마 브라우저").clicked() {
                        ui.close();
                        dock::show(&mut self.layout, Pane::Schema);
                    }
                    if ui.button("SQL 콘솔").clicked() {
                        ui.close();
                        dock::show(&mut self.layout, Pane::Sql);
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.tree.is_some(), egui::Button::new("엔드포인트"))
                        .clicked()
                    {
                        ui.close();
                        dock::show(&mut self.layout, Pane::Endpoints);
                    }
                    if ui.button("HTTP 응답").clicked() {
                        ui.close();
                        dock::show(&mut self.layout, Pane::Http);
                    }
                });
                ui.menu_button("플러그인", |ui| {
                    if let Some(choice) = self.plugins.menu_ui(ui) {
                        self.on_plugin_menu(choice);
                    }
                });
            });
        });
    }

    fn on_plugin_menu(&mut self, choice: crate::plugin_ui::MenuChoice) {
        use crate::plugin_ui::MenuChoice;
        match choice {
            MenuChoice::Command(plugin, id) => {
                let snapshot = plugin_snapshot(&self.docs, self.active);
                self.plugins.run_command(plugin, &id, snapshot.as_ref());
            }
            MenuChoice::Panel(plugin, id) => {
                self.plugins.select(plugin, &id);
                dock::show(&mut self.layout, Pane::Plugins);
            }
            MenuChoice::Window => dock::show(&mut self.layout, Pane::Plugins),
            MenuChoice::Reload => {
                self.status = self.plugins.reload();
                for doc in &mut self.docs {
                    doc.set_extension(self.plugins.languages());
                }
            }
        }
    }

    /// 프로젝트 루트에 jsconfig.json을 만들고, JS 언어 서버를 내려 새 설정으로 다시 띄우게 한다.
    fn create_jsconfig(&mut self) {
        let (Some(root), Some((_, Some(dir)))) = (
            self.tree.as_ref().map(|t| t.root().to_path_buf()),
            self.jsconfig.take(),
        ) else {
            return;
        };
        let path = root.join("jsconfig.json");
        if path.exists() {
            self.status = format!("이미 있습니다: {}", path.display());
            return;
        }
        match text_file::write_atomic(
            &path,
            ferrite_core::web::jsconfig_text(&root, dir).as_bytes(),
        ) {
            Ok(()) => {
                self.status = format!(
                    "jsconfig.json을 만들었습니다({dir}의 스크립트). JS 언어 서버를 다시 띄웁니다"
                );
                if let Some(server) = ferrite_lsp::catalog::for_language("javascript") {
                    self.hub.stop(server.name);
                }
            }
            Err(e) => self.status = format!("jsconfig.json을 만들지 못했습니다: {e}"),
        }
    }

    /// HTTP 요청 일을 한다. 응답 창을 앞으로 가져오므로 창을 다 그린 뒤에 부른다.
    fn apply_http_task(&mut self, ctx: &egui::Context) {
        match self.http_task.take() {
            Some(crate::http_ui::Task::Run { path, line }) => {
                let Some(doc) = self.docs.iter().find(|d| d.path == path) else {
                    return;
                };
                let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
                self.http
                    .run(ctx, &path, &doc.buffer.text(), line, root.as_deref());
                dock::show(&mut self.layout, Pane::Http);
            }
            Some(crate::http_ui::Task::Make {
                controller,
                path,
                methods,
            }) => self.make_request(&controller, &path, &methods),
            None => {}
        }
    }

    /// 엔드포인트의 요청을 `http/<컨트롤러>.http` 끝에 더하고(이미 있으면 찾기만) 그 줄로 간다. 환경
    /// 파일이 없으면 견본을 만든다. 더한 요청은 사용자가 저장한다.
    fn make_request(&mut self, controller: &str, path: &str, methods: &str) {
        let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) else {
            return;
        };
        let dir = root.join("http");
        let file = dir.join(crate::http_ui::request_file_name(controller));
        let created = std::fs::create_dir_all(&dir).and_then(|()| {
            if ferrite_http::env::load(&dir, Some(&root)).0.dir.is_none() {
                text_file::write_atomic(
                    &dir.join(ferrite_http::env::PUBLIC),
                    crate::http_ui::ENV_TEMPLATE.as_bytes(),
                )?;
            }
            if !file.exists() {
                text_file::write_atomic(&file, b"")?;
            }
            Ok(())
        });
        if let Err(e) = created {
            self.status = format!("요청 파일을 만들지 못했습니다: {e}");
            return;
        }
        self.open_file(file.clone());
        let target = real_path(&file);
        let Some(i) = self.docs.iter().position(|d| d.path == target) else {
            return;
        };
        let (block, request_line) = crate::http_ui::request_block(controller, path, methods);
        let find = |text: &str| text.lines().position(|l| l.trim() == request_line);
        let doc = &mut self.docs[i];
        let line = match find(&doc.buffer.text()) {
            Some(line) => line,
            None => {
                let text = doc.buffer.text();
                let gap = if text.is_empty() || text.ends_with("\n\n") {
                    ""
                } else if text.ends_with('\n') {
                    "\n"
                } else {
                    "\n\n"
                };
                doc.buffer
                    .set_selection(Selection::caret(doc.buffer.len_chars()));
                doc.buffer.insert(&format!("{gap}{block}"), Instant::now());
                find(&doc.buffer.text()).unwrap_or(0)
            }
        };
        let pos = doc.buffer.line_col_to_pos(line, 0);
        doc.buffer.set_selection(Selection::caret(pos));
        doc.view.reveal_cursor();
        self.focus_editor = true;
    }

    /// 플러그인이 시킨 일을 한다. 창 배치를 바꾸므로 창을 다 그린 뒤에 부른다.
    fn apply_plugin_actions(&mut self) {
        use ferrite_plugin::Action;
        for (plugin, action) in self.plugins.take_actions() {
            let name = self.plugins.name(plugin).to_owned();
            match action {
                Action::ReplaceSelection { text } => {
                    let doc = self
                        .active
                        .and_then(|i| self.docs.get_mut(i))
                        .filter(|d| d.notebook.is_none());
                    match doc {
                        Some(doc) => {
                            doc.buffer.insert(&text, Instant::now());
                            doc.view.reveal_cursor();
                            self.focus_editor = true;
                        }
                        None => self.status = format!("{name}: 바꿀 문서가 없습니다"),
                    }
                }
                Action::Message { text } => self.status = format!("{name}: {text}"),
                Action::ShowPanel { panel } => {
                    if self.plugins.select(plugin, &panel) {
                        dock::show(&mut self.layout, Pane::Plugins);
                    } else {
                        self.status = format!("{name}: 없는 패널입니다: {panel}");
                    }
                }
                Action::Unknown => {}
            }
        }
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
            // jdt.ls 번들(java-debug)은 jdt.ls를 다시 띄워야 싣거나 뺀다.
            if done.server.launch == ferrite_lsp::catalog::Launch::JdtlsBundle {
                self.hub.stop(JDTLS);
            }
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
                    server,
                    pulled,
                    diagnostics,
                    encoding,
                } => {
                    if let Some(doc) = self.docs.iter_mut().find(|d| d.path == path) {
                        let rope = doc.buffer.rope();
                        doc.lsp
                            .set_problems(server, pulled, rope, &diagnostics, encoding);
                    }
                }
                HubUpdate::Message(message) => self.status = message,
                HubUpdate::Response {
                    lang,
                    server,
                    id,
                    result,
                    encoding,
                } => self.on_lsp_response(lang, server, id, result, encoding),
                HubUpdate::ApplyEdit {
                    server,
                    id,
                    edits,
                    encoding,
                } => {
                    self.apply_workspace_edit(server, edits, encoding);
                    self.hub
                        .reply(server, id, serde_json::json!({ "applied": true }));
                }
                HubUpdate::Outline {
                    path,
                    result,
                    encoding,
                } => self.outline.receive(&path, &result, encoding),
                HubUpdate::WorkspaceSymbols {
                    generation,
                    server,
                    result,
                    encoding,
                } => {
                    self.symbol_search.receive(generation, &result, encoding);
                    self.endpoints
                        .receive(generation, server, &result, encoding);
                }
            }
        }
        self.ask_symbols();
    }

    fn show_outline(&mut self) {
        dock::show_beside(&mut self.layout, Pane::Outline, Pane::Project);
        self.outline.focus = true;
    }

    /// 보이는 개요·엔드포인트 창에 필요한 기호를 묻는다.
    fn ask_symbols(&mut self) {
        if dock::is_shown(&self.layout, Pane::Outline) {
            let doc = self
                .active
                .and_then(|i| self.docs.get_mut(i))
                .filter(|d| d.notebook.is_none());
            let key = doc.as_ref().map(|d| (d.path.clone(), d.buffer.version()));
            if self.outline.due(key.clone())
                && let (Some(doc), Some(key)) = (doc, key)
            {
                let sent = doc.lang.is_some_and(|lang| {
                    self.hub.outline(&doc.path, lang, &doc.buffer, &mut doc.lsp)
                });
                self.outline.asked(key, sent);
            }
        }
        if dock::is_shown(&self.layout, Pane::Endpoints) && self.endpoints.due() {
            self.hub.ensure_builtin();
            let (generation, asked) = self.hub.workspace_symbols("@/");
            self.endpoints.started(generation, asked);
        }
    }

    /// "문제" 창: 현재 문서의 진단 또는 프로젝트의 정적 분석(SARIF). 항목을 누르면 그 위치로 이동한다.
    fn problems_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.problems_sarif, false, "이 파일");
            ui.selectable_value(
                &mut self.problems_sarif,
                true,
                format!("정적 분석 ({})", self.sarif.findings().len()),
            );
        });
        if self.problems_sarif {
            let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
            if let Some(crate::sarif_ui::SarifAction::Go(path, line, column)) =
                self.sarif.ui(ui, root.as_deref())
            {
                self.open_file(path.clone());
                if let Some(i) = self.active.filter(|&i| self.docs[i].path == path) {
                    let doc = &mut self.docs[i];
                    let pos = doc.buffer.line_col_to_pos(line, column);
                    doc.buffer.move_to(pos, false);
                    doc.view.reveal_cursor();
                    self.focus_editor = true;
                }
            }
            return;
        }
        let Some(index) = self.active else {
            ui.weak("열린 파일이 없습니다");
            return;
        };
        let mut jump = None;
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
                self.updater.status_ui(ui);
                if let Some(doc) = self.active.map(|i| &mut self.docs[i]) {
                    let count = |s| {
                        doc.lsp
                            .problems
                            .iter()
                            .chain(&doc.sarif_problems)
                            .filter(|p| p.severity == s)
                            .count()
                    };
                    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
                    if ui
                        .selectable_label(
                            dock::is_shown(&self.layout, Pane::Problems),
                            format!("오류 {errors} · 경고 {warnings}"),
                        )
                        .on_hover_text("문제 목록 보기/숨기기")
                        .clicked()
                    {
                        dock::toggle(&mut self.layout, Pane::Problems);
                    }
                    if let Some(lang) = doc.lang
                        && let Some((server, state)) = self.hub.status(&doc.path, lang)
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
                            ServerState::Restarting(reason) => {
                                ui.label(format!("{name}: 다시 시작하는 중…"))
                                    .on_hover_text(reason.as_str());
                            }
                            ServerState::Failed(reason) => {
                                let text =
                                    egui::RichText::new(format!("{name}: 오류 (눌러서 다시 시작)"))
                                        .color(ui.visuals().error_fg_color);
                                if ui.link(text).on_hover_text(reason.as_str()).clicked() {
                                    restart = Some((doc.path.clone(), lang));
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
                    ui.label(match (doc.lang, &doc.syntax, &doc.ext) {
                        (Some(lang), Some(_), _) => lang.name().to_owned(),
                        (Some(lang), None, _) => {
                            format!("{} (대용량: 구문 강조 꺼짐)", lang.name())
                        }
                        (None, _, Some(ext))
                            if doc.textmate.is_none() && ext.grammar().is_some() =>
                        {
                            format!("{} (대용량: 구문 강조 꺼짐)", ext.name)
                        }
                        (None, _, Some(ext)) => ext.name.clone(),
                        (None, _, None) => "일반 텍스트".to_owned(),
                    });
                    let (line, col) = doc.buffer.pos_to_line_col(doc.buffer.selection().head);
                    ui.label(format!("줄 {}, 열 {}", line + 1, col + 1));
                }
                self.git.status_bar(ui);
                // 남은 폭에 상태 메시지를 둔다. 길면 말줄임으로 잘라 오른쪽 항목과 겹치지 않게 한다.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(&self.status).truncate());
                });
            });
        });
        if let Some((path, lang)) = restart {
            self.hub.restart(&path, lang);
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
                self.close_actions();
            } else if pressed(Key::Enter)
                && let Some(menu) = self.close_actions()
            {
                self.run_code_action(menu);
            }
            return;
        }
        if self.completion.is_none() {
            return;
        }
        if pressed(Key::Escape) {
            self.close_completion();
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
        // 매퍼 XML은 스키마를 읽었으면 언어 서버 없이도 DB 표·열을 자동 완성한다.
        let server_ready = lang.is_some_and(|l| {
            matches!(
                self.hub.status(&frame.path, l),
                Some((_, ServerState::Ready))
            ) || (l == Lang::Xml && !self.schema.tables().is_empty())
        });
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
            if self
                .hub
                .trigger_characters(&frame.path, lang)
                .contains(&last)
            {
                self.lsp_request(LspRequest::Completion(Some(last)), pos);
            } else if crate::lsp_ui::is_identifier_char(last) && (!popup_open || incomplete) {
                self.lsp_request(LspRequest::Completion(None), pos);
            }
        }
        if let Some(popup) = &mut self.completion {
            let doc = &self.docs[i];
            // 여기서는 요청을 버리지 않는다: 같은 프레임에 막 보낸 새 요청(미완성 목록 갱신)일 수 있다.
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
            && let Some(menu) = self.close_actions()
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

    /// "참조" 창(정의·참조 찾기 결과). 항목을 누르면 그 위치로 이동한다.
    fn references_panel(&mut self, ui: &mut egui::Ui) {
        let Some(list) = &self.references else {
            ui.weak("결과가 없습니다");
            return;
        };
        let mut jump = None;
        ui.strong(&list.title);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (n, item) in list.items.iter().enumerate() {
                    if ui
                        .selectable_label(false, egui::RichText::new(&item.label).monospace())
                        .clicked()
                    {
                        jump = Some(n);
                    }
                }
            });
        if let Some(n) = jump {
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
        let mut gutter_click = None;
        let mut run_request = None;
        let mut jsconfig_action = None;
        ui.scope(|ui| {
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
            // JS 언어 서버가 jQuery 같은 전역 라이브러리를 모르는 웹 프로젝트면 jsconfig.json을 만들자고 한다.
            if let Some(root) = self.tree.as_ref().map(|t| t.root()) {
                if self.jsconfig.as_ref().is_none_or(|(path, _)| *path != doc.path) {
                    self.jsconfig = Some((
                        doc.path.clone(),
                        ferrite_core::web::jsconfig_target(root, &doc.path),
                    ));
                }
                if let Some((_, Some(dir))) = &self.jsconfig
                    && !self.jsconfig_dismissed.contains(root)
                {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "jsconfig.json이 없어 JS 언어 서버가 다른 스크립트의 전역 함수와 jQuery 같은 라이브러리를 모릅니다(자동완성·정의 이동).",
                        );
                        if ui
                            .button("jsconfig.json 만들기")
                            .on_hover_text(format!(
                                "{dir}의 스크립트를 한 프로젝트로 묶는다. 언어 서버가 라이브러리 파일 이름을 보고 타입 정보(@types)를 인터넷에서 받는다"
                            ))
                            .clicked()
                        {
                            jsconfig_action = Some(true);
                        }
                        if ui.button("닫기").clicked() {
                            jsconfig_action = Some(false);
                        }
                    });
                    ui.separator();
                }
            }
            if let Some(notebook) = doc.notebook.as_mut() {
                if notebook.ui(ui, &mut doc.buffer, &doc.path) {
                    return;
                }
                if ui.button("노트북 보기").clicked() {
                    notebook.show_json = false;
                }
            }
            if let Some(find) = &mut self.find {
                find.refresh(&doc.path, &doc.buffer);
                find_action = find.ui(ui, &doc.buffer);
                ui.separator();
            }
            let generation = self.git.generation();
            let key = (doc.buffer.version(), generation);
            if doc.git_key != Some(key) {
                doc.git_marks = self.git.base(&doc.path).map_or_else(Vec::new, |base| {
                    ferrite_core::git::line_changes(base, &doc.buffer.text())
                });
                doc.git_key = Some(key);
            }
            let sarif_key = (self.sarif.generation(), doc.buffer.version());
            if doc.sarif_key != Some(sarif_key) {
                doc.sarif_problems = crate::sarif_ui::problems_for(
                    self.sarif.findings(),
                    &doc.path,
                    doc.buffer.rope(),
                );
                doc.sarif_key = Some(sarif_key);
            }
            // 언어 서버 진단과 정적 분석 결과를 위치 순으로 합친다(정적 분석이 없으면 그대로).
            let merged: Vec<ferrite_lsp::problem::Problem>;
            let problems = if doc.sarif_problems.is_empty() {
                &doc.lsp.problems
            } else {
                let mut all = doc.lsp.problems.clone();
                all.extend(doc.sarif_problems.iter().cloned());
                // 같은 자리면 심각한 것을 나중에 그려 위에 보이게 한다.
                all.sort_by_key(|p| (p.range.start, std::cmp::Reverse(p.severity)));
                merged = all;
                &merged
            };
            if crate::http_ui::is_http_file(&doc.path)
                && doc
                    .http_runs
                    .as_ref()
                    .is_none_or(|(version, _)| *version != doc.buffer.version())
            {
                doc.http_runs = Some((
                    doc.buffer.version(),
                    crate::http_ui::run_lines(&doc.buffer.text()),
                ));
            }
            // 어댑터가 걸 수 없다고 한 중단점은 빈 점으로 보인다(디버그 중일 때).
            let marks: Vec<(usize, BreakpointMark)> = self
                .breakpoints
                .get(&doc.path)
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(i, b)| {
                    let verified = self
                        .debugger
                        .breakpoint_state(&doc.path, i)
                        .is_none_or(|s| s.verified);
                    let mark = match (verified, &b.condition) {
                        (false, _) => BreakpointMark::Unverified,
                        (true, Some(_)) => BreakpointMark::Conditional,
                        (true, None) => BreakpointMark::Set,
                    };
                    (b.line.saturating_sub(1), mark)
                })
                .collect();
            let opts = EditorOptions {
                tab_size: self.settings.tab_size,
                indent: doc.indent,
                comment_token: doc
                    .lang
                    .and_then(Lang::line_comment)
                    .or_else(|| doc.ext.as_ref()?.line_comment.as_deref()),
                brackets: doc
                    .ext
                    .as_ref()
                    .map(|e| e.brackets.as_slice())
                    .filter(|b| !b.is_empty())
                    .unwrap_or(&editor_view::DEFAULT_BRACKETS),
                search_matches: self.find.as_ref().map_or(&[], |f| f.matches()),
                problems,
                hover: self
                    .hover
                    .as_ref()
                    .filter(|h| h.path == doc.path)
                    .and_then(|h| Some((h.pos, h.text.as_deref()?))),
                line_changes: &doc.git_marks,
                breakpoints: &marks,
                debug_line: self
                    .debugger
                    .location()
                    .filter(|(path, _)| *path == doc.path)
                    .map(|(_, line)| line),
                run_lines: doc
                    .http_runs
                    .as_ref()
                    .map_or(&[], |(_, lines)| lines.as_slice()),
                read_only: self.debugger.is_stored(&doc.path),
            };
            let id = ui.make_persistent_id(&doc.path);
            // 에디터가 Enter를 받기 전에 가로챈다.
            if doc.http_runs.is_some()
                && ui.memory(|m| m.has_focus(id))
                && ui.input_mut(|i| i.consume_shortcut(&RUN_REQUEST))
            {
                let line = doc.buffer.pos_to_line_col(doc.buffer.selection().head).0;
                run_request = Some((doc.path.clone(), line));
            }
            if std::mem::take(&mut self.focus_editor) {
                ui.memory_mut(|m| m.request_focus(id));
            }
            let styler: Option<&mut dyn Styler> = match (&mut doc.syntax, &mut doc.textmate) {
                (Some(s), _) => Some(s),
                (None, Some(t)) => Some(t),
                (None, None) => None,
            };
            let output = editor_view::show(ui, id, &mut doc.buffer, &mut doc.view, &opts, styler);
            // 언어 확장 강조가 앞 줄을 다 세지 못했으면 다음 프레임에 이어 센다.
            if doc.textmate.as_ref().is_some_and(|t| t.is_pending()) {
                ui.ctx().request_repaint();
            }
            let focused = output.response.has_focus();
            if let Some(tab) = active_tab {
                output.response.labelled_by(tab);
            }
            if let Some((line, secondary)) = output.gutter_click {
                gutter_click = Some((doc.path.clone(), line, secondary));
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
        if let Some((path, line, secondary)) = gutter_click {
            self.on_gutter_click(path, line, secondary);
        }
        if let Some((path, line)) = run_request {
            self.http_task = Some(crate::http_ui::Task::Run { path, line });
        }
        match jsconfig_action {
            Some(true) => self.create_jsconfig(),
            Some(false) => {
                if let Some(root) = self.tree.as_ref().map(|t| t.root().to_path_buf()) {
                    self.jsconfig_dismissed.insert(root);
                }
            }
            None => {}
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

/// 플러그인에 넘기는 활성 문서. 노트북은 버퍼가 JSON이라 넘기지 않는다.
fn plugin_doc(docs: &[Document], active: Option<usize>) -> Option<&Document> {
    docs.get(active?).filter(|d| d.notebook.is_none())
}

fn plugin_snapshot(docs: &[Document], active: Option<usize>) -> Option<crate::plugin_ui::Snapshot> {
    plugin_doc(docs, active).map(|d| crate::plugin_ui::Snapshot {
        path: d.path.clone(),
        language: d
            .lang
            .map(|l| l.name().to_owned())
            .or_else(|| d.ext.as_ref().map(|e| e.name.clone())),
        text: d.buffer.text(),
        selection: d.buffer.selected_text(),
    })
}

/// 창마다 앱의 해당 화면을 그린다. 그리는 동안 배치는 앱 밖에 있으므로 여기서 `layout`을 바꾸면
/// 사라진다(배치를 바꾸는 일은 단축키·메뉴·응답 처리처럼 창 밖에서 한다).
struct Dock<'a>(&'a mut FerriteApp);

impl egui_tiles::Behavior<Pane> for Dock<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile: egui_tiles::TileId,
        pane: &mut Pane,
    ) -> egui_tiles::UiResponse {
        let app = &mut *self.0;
        egui::CentralPanel::default().show(ui, |ui| match pane {
            Pane::Project => app.project_panel(ui),
            Pane::Editor => app.editor(ui),
            Pane::Terminal => {
                let root = app.tree.as_ref().map(|t| t.root().to_path_buf());
                app.terminal.ui(ui, root.as_deref());
            }
            Pane::Problems => app.problems_panel(ui),
            Pane::Search => app.find_in_files_panel(ui),
            Pane::References => app.references_panel(ui),
            Pane::Debug => {
                if let Some(notice) = app.debugger.panel_ui(ui) {
                    app.on_debug_notice(notice);
                }
            }
            Pane::DebugConsole => app.debugger.console_ui(ui),
            Pane::Sql => app.sql.ui(ui, &app.db),
            Pane::Schema => {
                if let Some(action) = app.schema.ui(ui, &app.db) {
                    app.schema_action = Some(action);
                }
            }
            Pane::Outline => {
                if let Some((range, encoding)) = app.outline.ui(ui)
                    && let Some(path) = app.active.map(|i| app.docs[i].path.clone())
                {
                    app.navigate_to(&path, range, encoding);
                }
            }
            Pane::Endpoints => match app.endpoints.ui(ui) {
                Some(crate::symbols_ui::EndpointAction::Refresh) => app.endpoints.refresh(),
                Some(crate::symbols_ui::EndpointAction::Go((path, range, encoding))) => {
                    app.navigate_to(&path, range, encoding);
                }
                Some(crate::symbols_ui::EndpointAction::MakeRequest {
                    controller,
                    path,
                    methods,
                }) => {
                    app.http_task = Some(crate::http_ui::Task::Make {
                        controller,
                        path,
                        methods,
                    });
                }
                None => {}
            },
            Pane::Http => app.http.ui(ui),
            Pane::Tests => {
                let root = app.tree.as_ref().map(|t| t.root().to_path_buf());
                if let Some(crate::test_ui::TestAction::Go(path, line)) =
                    app.tests.ui(ui, root.as_deref())
                {
                    app.on_debug_notice(crate::debug_ui::DebugNotice::Reveal(path, line));
                }
            }
            Pane::Plugins => {
                let key =
                    plugin_doc(&app.docs, app.active).map(|d| (d.path.clone(), d.buffer.version()));
                app.plugins
                    .panel_ui(ui, key, || plugin_snapshot(&app.docs, app.active));
            }
        });
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        pane.title().into()
    }

    fn is_tab_closable(
        &self,
        tiles: &egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(pane)) if pane.closable())
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        match tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(Pane::Search)) => self.0.find_in_files.stop(),
            Some(egui_tiles::Tile::Pane(Pane::References)) => self.0.references = None,
            _ => {}
        }
        true
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        // 창 하나만 있어도 탭 머리를 남겨, 끌어 옮길 손잡이와 창 이름을 보인다.
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}

impl eframe::App for FerriteApp {
    fn on_exit(&mut self) {
        self.updater.on_exit();
        // 디버그 중이면 대상을 끝낸다(어댑터가 띄운 브라우저 등). 그 뒤 어댑터에서 받아 담아 둔 화면
        // 스크립트와 어댑터 임시 폴더를 지운다. 브라우저가 늦게 끝나 폴더를 쥐고 있으면 잠깐 기다린다.
        self.debugger.shutdown();
        self.tests.stop();
        let _ = std::fs::remove_dir_all(crate::test_ui::reports_dir());
        let _ = std::fs::remove_dir_all(crate::debug_ui::sources_dir());
        let scratch = ferrite_dap::adapters::scratch_dir();
        for _ in 0..20 {
            if !scratch.exists() || std::fs::remove_dir_all(&scratch).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        if let Some(path) = layout_path()
            && let Err(e) = dock::save(&path, &self.layout)
        {
            eprintln!("창 배치를 저장하지 못했습니다({}): {e}", path.display());
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.git
            .tick(ctx, self.docs.iter().map(|d| d.path.clone()).collect());
        self.process_fs_events();
        self.process_debug_notices();
        self.finish_server_jobs();
        self.sync_language_servers();
        self.check_mapper_sql();
        self.updater.poll();
        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
        self.sarif.poll(ctx, root.as_deref());
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
        // 고르기 창은 위에 뜨지만, 고르는 키(Enter·화살표)를 창들보다 먼저 받도록 먼저 그린다.
        let ctx = ui.ctx().clone();
        match self.run.ui(&ctx) {
            Some(crate::picker::PickerAction::Pick(config)) => self.start_run(&config, &ctx),
            Some(crate::picker::PickerAction::Edit) => self.edit_run_configs(),
            None => {}
        }
        match self.debug_picker.ui(&ctx) {
            Some(crate::picker::PickerAction::Pick(config)) => self.start_debug(&config, &ctx),
            Some(crate::picker::PickerAction::Edit) => self.edit_debug_configs(),
            None => {}
        }
        let mut layout = std::mem::replace(
            &mut self.layout,
            egui_tiles::Tree::empty(egui::Id::unique("dock")),
        );
        layout.ui(&mut Dock(self), ui);
        self.layout = layout;
        self.apply_plugin_actions();
        self.apply_http_task(&ctx);
        let root = self.tree.as_ref().map(|t| t.root().to_path_buf());
        match self.symbol_search.ui(&ctx, root.as_deref()) {
            Some(crate::symbols_ui::SearchAction::Query(query)) => {
                let (generation, asked) = self.hub.workspace_symbols(&query);
                self.symbol_search.started(generation, asked);
            }
            Some(crate::symbols_ui::SearchAction::Go((path, range, encoding))) => {
                self.navigate_to(&path, range, encoding);
            }
            None => {}
        }
        if let Some(path) = self.quick_open.ui(ui.ctx()) {
            self.open_file(path);
            self.focus_editor = true;
        }
        self.breakpoint_condition_dialog(ui.ctx());
        self.pending_close_dialog(ui.ctx());
        if self.updater.window(&ctx) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        self.name_prompt_dialog(ui.ctx());
        self.delete_dialog(ui.ctx());
        if let Some(action) = self.servers.ui(&ctx) {
            self.start_server_action(action, &ctx);
        }
        if let Some(dir) = self.new_project.ui(&ctx) {
            self.open_folder(&dir);
        }
        self.commngt.ui(&ctx);
        if let Some(config) = self.db.ui(&ctx) {
            self.sql.connect(&config, &self.db, &ctx);
            dock::show(&mut self.layout, Pane::Sql);
        }
        if let Some(crate::schema_ui::Action::Query(config, sql)) = self.schema_action.take() {
            self.sql.connect(&config, &self.db, &ctx);
            self.sql.query(sql);
            dock::show(&mut self.layout, Pane::Sql);
        }
        self.git.ui(&ctx);
        if let Some(path) = self.config_gen.ui(&ctx) {
            self.status = format!("설정 파일을 만들었습니다: {}", path.display());
            self.open_file(path);
        }
        if let Some(written) = self.codegen.ui(&ctx) {
            self.status = format!("코드를 만들었습니다: 파일 {}개", written.len());
            // 만든 Controller를 연다(흐름의 시작점). 트리는 파일 감시로 갱신된다.
            if let Some(controller) = written
                .iter()
                .find(|p| p.to_string_lossy().ends_with("Controller.java"))
                .or(written.first())
            {
                self.open_file(controller.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;

    /// 앱 하네스. 파일 감시 알림이 다른 스레드에서 와 다시 그리는 프레임이 늘 수 있어(CI에서 기본 상한
    /// 4프레임을 넘김) 여유를 둔다.
    fn harness_builder() -> egui_kittest::HarnessBuilder<FerriteApp> {
        Harness::builder().with_max_steps(32)
    }

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

        let mut harness = harness_builder()
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

        let mut harness = harness_builder()
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

        let mut harness = harness_builder()
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

        let mut harness = harness_builder()
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

        let mut harness = harness_builder()
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

        let mut harness = harness_builder()
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
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        assert!(
            run_until_slow(&mut harness, |app| {
                matches!(
                    app.hub.status(&app.docs[0].path, Lang::Rust),
                    Some((_, ServerState::Ready))
                )
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
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        assert!(
            run_until_slow(&mut harness, |app| {
                matches!(
                    app.hub.status(&app.docs[0].path, Lang::Rust),
                    Some((_, ServerState::Ready))
                )
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

        // Ctrl+.: 코드 액션 메뉴를 열고 첫 항목을 실행하면 문서가 바뀐다. 포맷 직후에는
        // rust-analyzer가 resolve에 빈 편집을 돌려줄 때가 있어(앱은 "적용할 편집이 없습니다"로
        // 알린다, CI에서 가끔 생김) 그때는 메뉴부터 다시 연다.
        let on_n = doc_text(&harness).find("let n").unwrap() + 4;
        let before = harness.state().docs[0].buffer.version();
        let changed = |app: &FerriteApp| app.docs[0].buffer.version() != before;
        let mut applied = false;
        for _ in 0..10 {
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
            harness.state_mut().status.clear();
            harness.key_press(Key::Enter);
            run_until_slow(&mut harness, |app| {
                changed(app) || app.status.contains("적용할 편집이 없습니다")
            });
            applied = changed(harness.state());
            if applied {
                break;
            }
        }
        assert!(
            applied,
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

        let mut harness = harness_builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        isolate_servers(&mut harness, &root);
        assert_eq!(harness.state().servers.outdated(), ["lemminx"]);

        harness.get_by_label("도구").click();
        harness.run();
        harness.get_by_label("언어 서버…").click();
        harness.run();
        harness.get_by_label(&format!("업데이트 있음 (0.1 → {})", lemminx.version));
        // 디버그 어댑터도 같은 창에서 설치한다.
        harness.get_by_label("디버그 어댑터");
        for adapter in ferrite_lsp::catalog::ADAPTERS {
            harness.get_by_label(adapter.name);
        }

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

        let mut harness = harness_builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(mapper.clone())));
        isolate_servers(&mut harness, &dir.join("servers"));
        assert!(run_until(&mut harness, |app| app
            .hub
            .status(&app.docs[0].path, Lang::Xml)
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
        let config = harness.state().docs[1].path.clone();
        assert!(harness.state().hub.status(&config, Lang::Toml).is_none());
        assert!(harness.query_by_label_contains("설치 안 됨").is_none());

        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 실행_중에_죽은_서버는_기다렸다가_다시_띄운다() {
        let dir = std::env::temp_dir().join(format!("ferrite-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("servers");
        // lemminx가 설치된 것처럼 꾸미되, 실행 파일은 잠깐 떠 있다가 끝나는 스크립트다
        // (초기화 요청을 받을 틈을 둬야 "실행 중에 죽음"이 된다).
        let lemminx = ferrite_lsp::catalog::for_language("xml").unwrap();
        let installed = root.join(lemminx.name).join(lemminx.version);
        fs::create_dir_all(&installed).unwrap();
        let (script, body) = if cfg!(windows) {
            ("crash.cmd", "@ping -n 2 127.0.0.1 >nul\r\n@exit /b 3\r\n")
        } else {
            ("crash.sh", "#!/bin/sh\nsleep 1\nexit 3\n")
        };
        fs::write(installed.join(script), body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(installed.join(script), fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(installed.join(ferrite_lsp::install::RECORD), script).unwrap();
        let mapper = dir.join("mapper.xml");
        fs::write(&mapper, "<mapper/>\n").unwrap();

        let mut harness = harness_builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        isolate_servers(&mut harness, &root);
        harness.state_mut().open_file(mapper);

        // 처음 실행이 죽고, 1초 뒤 다시 띄운 것도 죽으면 연달아 죽은 횟수가 2가 된다.
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while harness.state().hub.crash_count("lemminx") < 2 {
            harness.run_ok();
            assert!(
                Instant::now() < deadline,
                "30초 안에 다시 띄우지 않음: {}",
                harness.state().status
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        harness.run_ok();
        assert!(
            harness.state().status.contains("2초 뒤 다시 시작"),
            "두 번째는 더 오래 기다린다: {}",
            harness.state().status
        );
        harness.get_by_label("lemminx: 다시 시작하는 중…");

        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn jdtls의_편집_적용_명령은_서버에_보내지_않고_직접_적용한다() {
        let dir = std::env::temp_dir().join(format!("ferrite-java-cmd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Hello.java");
        fs::write(&file, "class Hello {}\n").unwrap();

        let mut harness = harness_builder()
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
            "jdtls",
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
        remove_project(&dir);
    }

    /// 실제 jdt.ls·Spring Boot LS를 받아 중계를 끝까지 확인한다. Spring Boot LS가 jdt.ls에게서
    /// 클래스패스를 받아야 `server.port` 같은 키를 알므로, 이 결과가 중계가 된다는 증거다.
    /// Java 21 이상과 로컬 Maven 저장소의 Spring Boot 3.5.6(없으면 내려받음)이 필요하다.
    /// `cargo test -p ferrite-app -- --ignored spring_boot --nocapture`
    #[test]
    #[ignore = "jdt.ls(53MB)와 Spring Boot LS(83MB)를 내려받는다"]
    fn spring_boot_ls가_jdtls_중계로_설정_키를_검사한다() {
        let dir = std::env::temp_dir().join(format!("ferrite-spring-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("servers");
        let started = Instant::now();
        for name in ["jdtls", "spring-boot"] {
            let server = ferrite_lsp::catalog::SERVERS
                .iter()
                .find(|s| s.name == name)
                .unwrap();
            ferrite_lsp::install::install(&root, server, &mut |_, _| {})
                .unwrap_or_else(|e| panic!("{name} 설치 실패: {e}"));
        }
        eprintln!("설치: {:.1}초", started.elapsed().as_secs_f64());

        let project = dir.join("demo");
        let java = project.join("src/main/java/demo");
        let resources = project.join("src/main/resources");
        fs::create_dir_all(&java).unwrap();
        fs::create_dir_all(&resources).unwrap();
        fs::write(
            project.join("pom.xml"),
            r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <parent>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-parent</artifactId>
    <version>3.5.6</version>
  </parent>
  <groupId>demo</groupId><artifactId>demo</artifactId><version>1.0</version>
  <dependencies>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-web</artifactId>
    </dependency>
  </dependencies>
</project>
"#,
        )
        .unwrap();
        fs::write(
            java.join("DemoApplication.java"),
            "package demo;\n\n@org.springframework.boot.autoconfigure.SpringBootApplication\npublic class DemoApplication {\n}\n",
        )
        .unwrap();
        let props = resources.join("application.properties");
        fs::write(&props, "server.port=8080\nferrite.unknown.key=1\n").unwrap();

        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(project.clone())));
        isolate_servers(&mut harness, &root);
        // 1) Java 파일만 열면 jdt.ls만 뜨고, jdt.ls 확장이 Spring 프로젝트를 알아본 뒤에 Spring Boot
        //    LS가 뜬다(vscode-spring-boot.ls.start).
        harness
            .state_mut()
            .open_file(java.join("DemoApplication.java"));
        let started = Instant::now();
        let deadline = started + std::time::Duration::from_secs(180);
        while !harness.state().hub.is_running("spring-boot") {
            harness.run_ok();
            assert!(
                Instant::now() < deadline,
                "3분 안에 jdt.ls가 Spring 프로젝트를 알리지 않음. 상태: {}",
                harness.state().status
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        eprintln!(
            "jdt.ls가 Spring 프로젝트를 알려 Spring Boot LS가 뜸: {:.1}초",
            started.elapsed().as_secs_f64()
        );

        // 2) 설정 파일은 Spring Boot LS가 검사한다.
        harness.state_mut().open_file(props.clone());
        let props_doc = harness.state().docs.len() - 1;

        let started = Instant::now();
        let deadline = started + std::time::Duration::from_secs(300);
        let lines = |app: &FerriteApp| {
            let doc = &app.docs[props_doc];
            doc.lsp
                .problems
                .iter()
                .map(|p| {
                    (
                        doc.buffer.pos_to_line_col(p.range.start).0,
                        p.message.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        loop {
            harness.run_ok();
            let problems = lines(harness.state());
            if problems
                .iter()
                .any(|(line, m)| *line == 1 && m.contains("ferrite.unknown.key"))
            {
                assert!(
                    problems.iter().all(|(line, _)| *line != 0),
                    "server.port는 알려진 키여야 한다(클래스패스 중계 실패): {problems:?}"
                );
                eprintln!(
                    "Spring 설정 검사 확인: {:.1}초, 진단: {problems:?}",
                    started.elapsed().as_secs_f64()
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "5분 안에 확인하지 못함. 진단: {problems:?}, 상태: {}",
                harness.state().status
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn egov_crud_코드를_만들고_controller를_연다() {
        use egui::accesskit::Role;

        let dir = std::env::temp_dir().join(format!("ferrite-codegen-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let write = |path: &str, text: &str| {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        };
        // egov 3.x 프로젝트 모양: 매퍼 폴더·이름 모양과 뷰 리졸버 접두사를 그대로 따라야 한다.
        write(
            "pom.xml",
            "<project><dependency><groupId>egovframework.rte</groupId></dependency></project>",
        );
        write(
            "src/main/java/egovframework/example/sample/service/SampleService.java",
            "package egovframework.example.sample.service;\n",
        );
        write(
            "src/main/resources/egovframework/sqlmap/example/mappers/EgovSample_Sample_SQL.xml",
            "<mapper namespace=\"x\"></mapper>",
        );
        write(
            "src/main/webapp/WEB-INF/config/springmvc/dispatcher-servlet.xml",
            "<beans><bean class=\"org.springframework.web.servlet.view.UrlBasedViewResolver\" p:prefix=\"/WEB-INF/jsp/egovframework/example/\" p:suffix=\".jsp\"/></beans>",
        );

        let mut harness = harness_builder()
            .with_size(egui::vec2(1400.0, 1300.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.get_by_label("도구").click();
        harness.run();
        harness.get_by_label("egov CRUD 코드 생성…").click();
        harness.run();

        let ddl = harness.get_by_role(Role::MultilineTextInput);
        ddl.focus();
        ddl.type_text("CREATE TABLE TB_NOTICE (NOTICE_ID VARCHAR2(20) PRIMARY KEY, TITLE VARCHAR2(100) NOT NULL)");
        harness.run();
        harness.get_by_label("DDL 읽기").click();
        harness.run();
        harness.get_by_label("열 2개를 읽었습니다");
        harness.get_by_label("생성").click();
        harness.run();

        let java = dir.join("src/main/java/egovframework/example/notice");
        assert!(java.join("service/NoticeVO.java").is_file());
        assert!(java.join("service/impl/NoticeServiceImpl.java").is_file());
        let mapper =
            dir.join("src/main/resources/egovframework/sqlmap/example/mappers/Notice_SQL.xml");
        assert!(
            mapper.is_file(),
            "기존 매퍼 폴더·이름 모양(_SQL.xml)을 따른다"
        );
        assert!(
            dir.join("src/main/webapp/WEB-INF/jsp/egovframework/example/notice/noticeList.jsp")
                .is_file(),
            "뷰 리졸버 접두사 아래"
        );
        let vo = fs::read_to_string(java.join("service/NoticeVO.java")).unwrap();
        assert!(vo.contains("private String noticeId;"));
        let app = harness.state();
        assert!(app.status.contains("파일 10개"), "{}", app.status);
        assert!(
            app.docs
                .iter()
                .any(|d| d.path.ends_with("NoticeController.java")),
            "만든 Controller를 연다"
        );

        // 다시 만들면 이미 있다고 막는다(덮어쓰기를 고르지 않았으므로).
        harness.get_by_label("생성").click();
        harness.run();
        harness.get_by_label_contains("이미 있는 파일:");
        harness.get_by_label("이미 있는 파일 덮어쓰기");
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn egov_설정_파일을_만들고_연다() {
        let dir = std::env::temp_dir().join(format!("ferrite-config-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let write = |path: &str, text: &str| {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        };
        // 프로젝트의 Spring 설정 폴더와 데이터소스 프로퍼티 키를 따라야 한다.
        write(
            "pom.xml",
            "<project><dependency><groupId>org.egovframe.rte</groupId></dependency></project>",
        );
        write(
            "src/main/java/egovframework/example/sample/service/SampleService.java",
            "package egovframework.example.sample.service;\n",
        );
        write(
            "src/main/resources/egovframework/spring/com/context-common.xml",
            "<beans></beans>",
        );
        write(
            "src/main/resources/egovframework/egovProps/globals.properties",
            "Globals.DriverClassName=org.h2.Driver\nGlobals.Url=jdbc:h2:mem:x\nGlobals.UserName=sa\nGlobals.Password=\n",
        );

        let mut harness = harness_builder()
            .with_size(egui::vec2(1400.0, 1300.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.get_by_label("도구").click();
        harness.run();
        harness.get_by_label("egov 설정 파일 생성…").click();
        harness.run();

        // 기본 종류(DBCP 데이터소스, XML)
        harness.get_by_label("생성").click();
        harness.run();
        let xml = dir.join("src/main/resources/egovframework/spring/com/context-datasource.xml");
        let text = fs::read_to_string(&xml).unwrap();
        assert!(
            text.contains("org.apache.commons.dbcp2.BasicDataSource"),
            "{text}"
        );
        assert!(
            text.contains(r#"value="${Globals.Url}""#),
            "프로젝트의 프로퍼티 키를 플레이스홀더로: {text}"
        );
        assert!(
            harness
                .state()
                .docs
                .iter()
                .any(|d| d.path.ends_with("context-datasource.xml"))
        );

        // 같은 종류를 Java Config로
        harness.get_by_label("Java Config").click();
        harness.run();
        harness.get_by_label("생성").click();
        harness.run();
        let java = dir.join("src/main/java/egovframework/example/config/EgovConfigDatasource.java");
        let text = fs::read_to_string(&java).unwrap();
        assert!(
            text.contains("package egovframework.example.config;"),
            "{text}"
        );
        assert!(text.contains(r#"@Value("${Globals.Url}")"#), "{text}");

        // 다른 종류(로그) 고르기
        harness
            .get_by_role_and_label(egui::accesskit::Role::ComboBox, "설정 종류")
            .click();
        harness.run();
        harness.get_by_label("Logging > 콘솔 로그").click();
        harness.run();
        harness.get_by_label("생성").click();
        harness.run();
        let log = fs::read_to_string(dir.join("src/main/resources/log4j2.xml")).unwrap();
        assert!(log.contains("<Console name=\"console\""), "{log}");
        drop(harness);
        remove_project(&dir);
    }

    fn open_new_project(harness: &mut Harness<'_, FerriteApp>, cache: PathBuf) {
        harness.state_mut().new_project = crate::new_project::NewProjectWindow::new(cache);
        harness.get_by_label("도구").click();
        harness.run();
        harness.get_by_label("새 egov 프로젝트…").click();
        harness.run();
    }

    #[test]
    fn 새_egov_프로젝트_창은_입력을_검사하고_받을_크기를_알린다() {
        use egui::accesskit::Role;
        let dir = std::env::temp_dir().join(format!("ferrite-new-project-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_eframe(|cc| FerriteApp::new(cc, None));
        open_new_project(&mut harness, dir.join("cache"));

        // 기본 템플릿(웹 프로젝트): zip 15,547,491 + pom 5,794바이트를 받아야 한다.
        assert!(
            !harness
                .get_by_label("내려받아 만들기 (15.6MB)")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("groupId");

        harness
            .get_by_role_and_label(Role::TextInput, "프로젝트 이름")
            .focus();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "프로젝트 이름")
            .type_text("/x");
        harness.run();
        harness.get_by_label_contains("프로젝트 이름: 영문");
        harness.get_by_label_contains("artifactId: ");
        assert!(
            harness
                .get_by_label("내려받아 만들기 (15.6MB)")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .get_by_label("파일에서…")
                .accesskit_node()
                .is_disabled()
        );

        // pom 틀이 없는 템플릿은 groupId 칸이 없다.
        harness
            .get_by_role_and_label(Role::ComboBox, "템플릿")
            .click();
        harness.run();
        harness
            .get_by_label("Boot > 심플 홈페이지(프런트엔드)")
            .click();
        harness.run();
        assert!(harness.query_by_label("groupId").is_none());
        harness.get_by_label("내려받아 만들기 (3.7MB)");
        drop(harness);
        let _ = fs::remove_dir_all(&dir);
    }

    /// `FERRITE_EGOV_VSIX=<vsix 절대 경로> cargo test -p ferrite-app 공식_템플릿 -- --ignored`
    #[test]
    #[ignore = "공식 확장 vsix가 필요하다(FERRITE_EGOV_VSIX)"]
    fn 공식_템플릿으로_새_프로젝트를_만들고_연다() {
        use egui::accesskit::Role;
        use ferrite_codegen::scaffold::{self, TEMPLATES};
        let Some(vsix) = std::env::var_os("FERRITE_EGOV_VSIX") else {
            eprintln!("FERRITE_EGOV_VSIX가 없어 건너뜁니다");
            return;
        };
        let dir =
            std::env::temp_dir().join(format!("ferrite-new-project-e2e-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cache = dir.join("cache");
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        // 작은 템플릿(DeviceAPI 웹, 0.2MB)을 받아 둔 상태로 시작한다.
        let template = TEMPLATES
            .iter()
            .find(|t| t.zip.file == "egovframe-mobile-deviceapi.zip")
            .unwrap();
        scaffold::import(template, &[PathBuf::from(vsix)], &cache).unwrap();

        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_eframe(|cc| FerriteApp::new(cc, None));
        open_new_project(&mut harness, cache);
        harness
            .get_by_role_and_label(Role::ComboBox, "템플릿")
            .click();
        harness.run();
        // 목록이 길어 아래쪽 항목은 스크롤해야 보인다.
        harness
            .get_by_label("모바일 > DeviceAPI 웹 프로젝트")
            .scroll_to_me();
        harness.run();
        harness
            .get_by_label("모바일 > DeviceAPI 웹 프로젝트")
            .click();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "만들 위치")
            .set_text_value_accesskit(&out.to_string_lossy());
        harness.run();
        harness.get_by_label("만들기").click();
        // 작업 중에는 스피너가 계속 다시 그리므로 run_ok로 돌린다.
        assert!(run_until_slow(&mut harness, |_| out
            .join("egov-mobile-deviceapi/pom.xml")
            .is_file()));
        for _ in 0..50 {
            harness.run_ok();
            if harness.query_by_label("열기").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        harness.get_by_label("열기").click();
        harness.run();
        let project = real_path(&out.join("egov-mobile-deviceapi"));
        assert_eq!(
            harness
                .state()
                .tree
                .as_ref()
                .map(|t| t.root().to_path_buf()),
            Some(project.clone())
        );
        let pom = fs::read_to_string(project.join("pom.xml")).unwrap();
        assert!(
            pom.contains("<groupId>egovframework.example</groupId>"),
            "{pom}"
        );
        assert!(
            pom.contains("<artifactId>egov-mobile-deviceapi</artifactId>"),
            "{pom}"
        );
        drop(harness);
        remove_project(&dir);
    }

    /// 공식 플러그인 jar 모양의 가짜(기본 묶음 cmm과 FAQ).
    fn fake_commngt_jar() -> Vec<u8> {
        use std::io::Write;
        let zip_of = |entries: &[(&str, &[u8])]| {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for (name, data) in entries {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap().into_inner()
        };
        let catalog = r##"<components>
            <category name="#c1nam#" description="#c1des#"><component name="#c1p1nam#">
              <packageName>#c1p1pac#</packageName></component></category>
            <category name="#c2nam#" description="#c2des#"><component name="#c2p1nam#">
              <packageName>#c2p1pac#</packageName><dependencyPackage>#c2p1de1#</dependencyPackage>
            </component></category></components>"##;
        let labels = "c1nam=공통\nc1des=기본\nc1p1nam=공통\nc1p1pac=egovframework.com.cmm\n\
            c2nam=온라인도움말\nc2des=도움말\nc2p1nam=FAQ관리\nc2p1pac=egovframework.com.uss.olh.faq\n\
            c2p1de1=egovframework.com.cmm\n";
        let cmm = zip_of(&[
            ("src/main/java/egovframework/com/cmm/Base.java", b"class Base {}"),
            ("src/script/ddl/oracle/cmm_create_oracle.sql", b"CREATE TABLE A;"),
            ("pom.xml", b"<project><dependency><groupId>g</groupId><artifactId>a</artifactId></dependency></project>"),
        ]);
        let faq = zip_of(&[(
            "src/main/java/egovframework/com/uss/olh/faq/Faq.java",
            b"class Faq {}",
        )]);
        zip_of(&[
            ("examples/components.xml", catalog.as_bytes()),
            (
                "egovframework/dev/imp/commngt/common/components_ko.properties",
                labels.as_bytes(),
            ),
            ("examples/egovframework.com.cmm.Wizard.4.2.0.zip", &cmm),
            (
                "examples/egovframework.com.uss.olh.faq.Wizard.4.2.0.zip",
                &faq,
            ),
        ])
    }

    #[test]
    fn 공통컴포넌트를_골라_미리_보고_넣는다() {
        let dir = std::env::temp_dir().join(format!("ferrite-commngt-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let project = dir.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("pom.xml"),
            "<project><groupId>org.egovframe.rte</groupId></project>",
        )
        .unwrap();
        let jar = dir.join("egovframework.dev.imp.commngt_4.2.0.jar");
        fs::write(&jar, fake_commngt_jar()).unwrap();
        let cache = dir.join("templates");
        ferrite_codegen::commngt::import(&jar, &cache).unwrap();

        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 1000.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(project.clone())));
        harness.state_mut().commngt = crate::commngt_ui::CommngtWindow::new(cache);
        harness.get_by_label("도구").click();
        harness.run();
        harness.get_by_label("공통컴포넌트 추가…").click();
        harness.run();
        harness.get_by_label("온라인도움말").click();
        harness.run();
        harness.get_by_label("FAQ관리").click();
        harness.run();
        harness.get_by_label("고른 컴포넌트 1개, 의존을 더해 패키지 2개");
        assert!(
            harness.get_by_label("추가").accesskit_node().is_disabled(),
            "미리 보기 전에는 쓰지 않는다"
        );

        let wait_for = |harness: &mut Harness<'_, FerriteApp>, text: &str| {
            for _ in 0..100 {
                harness.run_ok();
                if harness.query_by_label_contains(text).is_some() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            panic!("{text}가 나오지 않음");
        };
        harness.get_by_label("미리 보기").click();
        wait_for(&mut harness, "새 파일 3개, 같은 파일 0개, 다른 파일 1개");
        harness.get_by_label("다른 파일은 건너뜁니다:");
        harness.get_by_label("추가").click();
        wait_for(&mut harness, "넣었습니다");
        harness.get_by_label_contains("새 파일 3개");
        harness.get_by_label("g:a");
        harness.get_by_label("src/script/ddl/oracle/cmm_create_oracle.sql");
        harness.get_by_label("설치됨");
        assert!(
            project
                .join("src/main/java/egovframework/com/uss/olh/faq/Faq.java")
                .is_file()
        );
        assert_eq!(
            fs::read_to_string(project.join("pom.xml")).unwrap(),
            "<project><groupId>org.egovframe.rte</groupId></project>",
            "pom.xml은 덮어쓰지 않는다"
        );
        drop(harness);
        remove_project(&dir);
    }

    /// 검색 시험용 프로젝트: Java 두 개, CP949 JSP 하나, 무시되는 빌드 결과물.
    fn search_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferrite-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let write = |path: &str, bytes: &[u8]| {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, bytes).unwrap();
        };
        write(
            "src/main/java/egov/web/SampleController.java",
            "class SampleController {\n    // 사용자 조회\n    void list() {}\n}\n".as_bytes(),
        );
        write(
            "src/main/java/egov/service/SampleService.java",
            b"interface SampleService {}\n",
        );
        let (jsp, _, _) = encoding_rs::EUC_KR.encode("<p>사용자 목록</p>\r\n");
        write("src/main/webapp/list.jsp", &jsp);
        write("target/classes/SampleController.class", b"\0\0");
        dir
    }

    /// 별도 스레드 작업이 끝나 `text`가 보일 때까지 돌린다(스피너가 계속 다시 그리므로 run_ok).
    fn wait_for_label(harness: &mut Harness<'_, FerriteApp>, text: &str) {
        for _ in 0..200 {
            harness.run_ok();
            if harness.query_by_label_contains(text).is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("{text}가 나오지 않음");
    }

    #[test]
    fn 파일로_이동은_퍼지_검색해_연다() {
        use egui::accesskit::Role;
        let dir = search_project("quick-open");
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
        wait_for_label(&mut harness, "파일 3개");
        harness
            .get_by_role_and_label(Role::TextInput, "파일 이름")
            .type_text("samplecon");
        harness.run();
        harness.get_by_label_contains("SampleController.java  src/main/java/egov/web");
        assert!(
            harness
                .query_by_label_contains("SampleController.class")
                .is_none(),
            "target은 뺀다"
        );
        harness.key_press(Key::Enter);
        harness.run();
        let app = harness.state();
        assert!(
            app.docs
                .iter()
                .any(|d| d.path.ends_with("SampleController.java"))
        );
        assert!(
            harness.query_by_label("파일로 이동").is_none(),
            "열면 닫힌다"
        );

        // Esc로 닫는다.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
        harness.run_ok();
        harness.get_by_label("파일로 이동");
        harness.key_press(Key::Escape);
        harness.run_ok();
        assert!(harness.query_by_label("파일로 이동").is_none());
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 파일에서_찾기는_결과를_눌러_그_줄로_간다() {
        use egui::accesskit::Role;
        let dir = search_project("find-in-files");
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);
        harness.run();
        assert!(harness.state().find.is_none(), "Ctrl+F 찾기 바가 아니다");
        harness
            .get_by_role_and_label(Role::TextInput, "파일에서 찾기")
            .type_text("사용자");
        harness.run();
        harness.key_press(Key::Enter);
        wait_for_label(&mut harness, "파일 2개에서 2개 찾음");
        harness.get_by_label_contains("<p>사용자 목록</p>");
        harness.get_by_label_contains("2: // 사용자 조회").click();
        harness.run();
        let app = harness.state();
        let i = app.active.unwrap();
        assert!(app.docs[i].path.ends_with("SampleController.java"));
        let rope = app.docs[i].buffer.rope();
        let head = app.docs[i].buffer.selection().head;
        assert_eq!(rope.char_to_line(head), 1);
        assert_eq!(head - rope.line_to_char(1), 7, "일치한 글자 앞");

        // 파일 패턴으로 좁힌다.
        harness
            .get_by_role_and_label(Role::TextInput, "파일")
            .focus();
        harness.run();
        harness
            .get_by_role_and_label(Role::TextInput, "파일")
            .type_text("*.jsp");
        harness.run();
        harness.get_by_label("찾기").click();
        wait_for_label(&mut harness, "파일 1개에서 1개 찾음");
        drop(harness);
        remove_project(&dir);
    }

    /// 화면 읽기용으로 알린 터미널 글자.
    fn terminal_text(harness: &Harness<'_, FerriteApp>) -> String {
        harness
            .query_by_role(egui::accesskit::Role::Terminal)
            .and_then(|node| node.value())
            .unwrap_or_default()
    }

    /// 터미널 글자가 조건에 맞을 때까지(최대 60초) 기다린다. 시험이 함께 도는 Windows CI 러너에서는
    /// PowerShell이 20초 넘게 걸리기도 했다(셸은 살아 있었다).
    fn wait_for_terminal(harness: &mut Harness<'_, FerriteApp>, done: impl Fn(&str) -> bool) {
        for _ in 0..1200 {
            harness.run_ok();
            if done(&terminal_text(harness)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let app = harness.state();
        panic!(
            "터미널 시간 초과({}, 창 보임 {}). 화면:\n{}",
            app.terminal.describe(),
            dock::is_shown(&app.layout, Pane::Terminal),
            terminal_text(harness)
        );
    }

    #[test]
    fn 도구_창은_탭으로_모이고_닫거나_배치를_초기화한다() {
        let dir = search_project("dock");
        let file = dir.join("src/main/java/egov/web/SampleController.java");
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        harness.run_ok();
        harness.get_by_label("프로젝트");
        harness.get_by_label("에디터");
        let shown = |h: &Harness<'_, FerriteApp>, pane| dock::is_shown(&h.state().layout, pane);
        let exists =
            |h: &Harness<'_, FerriteApp>, pane| h.state().layout.tiles.find_pane(&pane).is_some();

        // 검색 창을 열고, 상태바로 문제 창을 열면 같은 묶음의 앞 탭이 된다.
        harness.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);
        harness.run_ok();
        assert!(shown(&harness, Pane::Search));
        harness.get_by_label_contains(" · 경고").click();
        harness.run_ok();
        assert!(shown(&harness, Pane::Problems) && !shown(&harness, Pane::Search));

        // 탭을 누르면 앞으로 온다. 탭의 닫기 단추로 닫는다(검색이 먼저 열려 첫 탭).
        harness.get_by_label("검색").click();
        harness.run_ok();
        assert!(shown(&harness, Pane::Search) && !shown(&harness, Pane::Problems));
        harness.get_all_by_label("Close").next().unwrap().click();
        harness.run_ok();
        assert!(!exists(&harness, Pane::Search));
        assert!(shown(&harness, Pane::Problems));

        // 보기 > 창 배치 초기화는 도구 창을 모두 걷는다.
        harness.get_by_label("보기").click();
        harness.run_ok();
        harness.get_by_label("창 배치 초기화").click();
        harness.run_ok();
        assert!(!exists(&harness, Pane::Problems));
        harness.get_by_label("에디터");
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 실행_구성을_골라_터미널_탭에서_돌리고_다시_실행하면_그_탭을_바꾼다() {
        let dir = std::env::temp_dir().join(format!("ferrite-run-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".ferrite")).unwrap();
        // ConPTY는 곧바로 끝나는 프로세스의 마지막 화면을 놓칠 수 있어 Windows에서는 1초 쉰다.
        let command = if cfg!(windows) {
            "echo run-%FERRITE_RUN%& ping -n 2 127.0.0.1 >nul"
        } else {
            "echo run-$FERRITE_RUN"
        };
        fs::write(
            dir.join(ferrite_core::run::CUSTOM_FILE),
            format!(
                "[[run]]\nname = \"인사\"\ncommand = '{command}'\nenv = {{ FERRITE_RUN = \"ok\" }}\n"
            ),
        )
        .unwrap();
        fs::write(
            dir.join("pom.xml"),
            "<project><artifactId>tomcat7-maven-plugin</artifactId></project>",
        )
        .unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.run_ok();

        // 사용자 구성이 먼저, 그다음 자동 감지 구성. Enter로 고른 구성을 돌린다.
        harness.key_press_modifiers(Modifiers::CTRL, Key::F5);
        harness.run_ok();
        harness.get_by_label("사용자 구성");
        harness.get_by_label("자동 감지");
        harness.get_by_label("mvn tomcat7:run (디버그 5005)");
        harness.key_press(Key::Enter);
        wait_for_terminal(&mut harness, |text| {
            text.lines().any(|line| line.trim() == "run-ok")
        });
        assert!(
            harness.query_by_label("사용자 구성").is_none(),
            "고르면 닫힌다"
        );
        assert!(dock::is_shown(&harness.state().layout, Pane::Terminal));
        assert_eq!(
            harness.state().terminal.tab_count(),
            1,
            "기본 셸은 따로 띄우지 않는다"
        );
        harness.get_by_role_and_label(egui::accesskit::Role::Terminal, "인사");

        // 다시 열면 마지막 구성이 골라져 있고, 다시 돌리면 같은 탭을 바꾼다.
        harness.key_press_modifiers(Modifiers::CTRL, Key::F5);
        harness.run_ok();
        harness.key_press(Key::Enter);
        harness.run_ok();
        assert_eq!(harness.state().terminal.tab_count(), 1);
        wait_for_terminal(&mut harness, |text| {
            text.lines().any(|line| line.trim() == "run-ok")
        });

        // 구성 편집은 구성 파일을 에디터로 연다.
        harness.key_press_modifiers(Modifiers::CTRL, Key::F5);
        harness.run_ok();
        harness.get_by_label("구성 편집…").click();
        harness.run_ok();
        let app = harness.state();
        let doc = &app.docs[app.active.unwrap()];
        assert!(doc.path.ends_with("run.toml"), "{}", doc.path.display());
        drop(harness);
        remove_project(&dir);
    }

    /// 거터의 줄(0부터) 가운데를 누른다. 오른쪽 단추면 조건 창을 연다.
    fn click_gutter(harness: &mut Harness<'_, FerriteApp>, line: usize, secondary: bool) {
        let rect = harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .rect();
        // 글꼴 잠금 안에서 스타일을 읽으면 같은 잠금을 다시 잡아 멈춘다. 먼저 읽는다.
        let font = egui::TextStyle::Monospace.resolve(&harness.ctx.global_style());
        let row = harness.ctx.fonts_mut(|f| f.row_height(&font));
        let pos = egui::pos2(rect.min.x + 4.0, rect.min.y + row * (line as f32 + 0.5));
        let button = if secondary {
            egui::PointerButton::Secondary
        } else {
            egui::PointerButton::Primary
        };
        harness.hover_at(pos);
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            });
            harness.run_ok();
        }
    }

    #[test]
    fn 거터를_눌러_중단점을_켜고_조건을_단다() {
        let dir = std::env::temp_dir().join(format!("ferrite-gutter-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.py");
        fs::write(&file, "a = 1\nb = 2\nc = a + b\n").unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        harness.run_ok();
        let path = harness.state().docs[0].path.clone();
        let lines = |h: &Harness<'_, FerriteApp>| -> Vec<(usize, Option<String>)> {
            h.state()
                .breakpoints
                .get(&path)
                .into_iter()
                .flatten()
                .map(|b| (b.line, b.condition.clone()))
                .collect()
        };

        // 거터를 누르면 켜지고, 커서는 그대로다. 다시 누르면 꺼진다.
        click_gutter(&mut harness, 1, false);
        assert_eq!(lines(&harness), [(2, None)]);
        assert_eq!(
            harness.state().docs[0].buffer.selection().head,
            0,
            "커서는 그대로"
        );
        click_gutter(&mut harness, 1, false);
        assert!(lines(&harness).is_empty());
        assert!(
            harness.state().breakpoints.is_empty(),
            "빈 파일 항목은 지운다"
        );

        // 오른쪽 단추로 조건을 단다.
        click_gutter(&mut harness, 2, true);
        harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, "조건식(참일 때만 멈춤)")
            .type_text("a > 0");
        harness.key_press(Key::Enter);
        harness.run_ok();
        assert_eq!(lines(&harness), [(3, Some("a > 0".to_owned()))]);
        assert!(
            harness.query_by_label("중단점 조건").is_none(),
            "확인하면 닫힌다"
        );

        // F9는 커서 줄을 켜고 끈다.
        harness.key_press(Key::F9);
        harness.run_ok();
        assert_eq!(lines(&harness), [(1, None), (3, Some("a > 0".to_owned()))]);
        drop(harness);
        remove_project(&dir);
    }

    /// debugpy를 쓸 수 있는지. 없으면 건너뛰되 CI에서는 실패시킨다.
    fn debugpy_available() -> bool {
        let python = if cfg!(windows) { "python" } else { "python3" };
        let mut command = std::process::Command::new(python);
        command.args(["-c", "import debugpy"]);
        if let Some(path) = std::env::var_os("FERRITE_DEBUGPY_PATH") {
            command.env("PYTHONPATH", path);
        }
        let found = command.output().is_ok_and(|o| o.status.success());
        if !found {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI에는 debugpy가 설치되어 있어야 합니다"
            );
            eprintln!("debugpy가 없어 건너뜁니다");
        }
        found
    }

    /// 디버거 상태가 `done`이 될 때까지 돌린다(최대 60초, 처음 띄울 때 느리다).
    fn wait_debugger(
        harness: &mut Harness<'_, FerriteApp>,
        what: &str,
        done: impl Fn(&Harness<'_, FerriteApp>) -> bool,
    ) {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(60) {
            harness.run_ok();
            if done(harness) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!(
            "{what} 시간 초과. 상태: {}\n출력:\n{}",
            harness.state().status,
            harness.state().debugger.console()
        );
    }

    #[test]
    fn 디버그로_중단점에서_멈추고_변수를_보고_이어_간다() {
        if !debugpy_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("ferrite-debug-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.py");
        fs::write(
            &file,
            "def add(a, b):\n    total = a + b\n    return total\n\n\nfor x in (1, 41):\n    y = add(x, 1)\nprint(\"결과\", y)\n",
        )
        .unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        harness.run_ok();

        // `return total` 줄에 F9로 중단점을 건다.
        {
            let doc = &mut harness.state_mut().docs[0];
            let start = doc.buffer.line_start(2);
            doc.buffer.move_to(start, false);
        }
        harness.key_press(Key::F9);
        harness.run_ok();

        // F5 → 고르기 창의 자동 구성(Python: 현재 파일) → Enter
        harness.key_press(Key::F5);
        harness.run_ok();
        harness.get_by_label("Python: 현재 파일");
        harness.key_press(Key::Enter);
        wait_debugger(&mut harness, "첫 멈춤", |h| {
            h.query_by_label("total = 2").is_some()
        });
        let path = harness.state().docs[0].path.clone();
        assert_eq!(
            harness.state().debugger.location(),
            Some((path.as_path(), 2)),
            "멈춘 줄을 에디터에 보인다"
        );
        assert!(dock::is_shown(&harness.state().layout, Pane::Debug));
        harness.get_by_label_contains("add  main.py:3");

        // 한 줄씩(F10): 함수에서 돌아와 호출한 줄에 멈춘다.
        harness.key_press(Key::F10);
        wait_debugger(&mut harness, "한 줄씩", |h| {
            h.state().debugger.is_stopped()
                && h.state()
                    .debugger
                    .location()
                    .is_some_and(|(_, line)| line == 6)
        });

        // 계속(F5): 두 번째 호출에서 다시 멈춘다.
        harness.key_press(Key::F5);
        wait_debugger(&mut harness, "두 번째 멈춤", |h| {
            h.query_by_label("total = 42").is_some()
        });

        // 멈춤(Shift+F5): 어댑터가 대상을 끝내고 세션이 닫힌다.
        harness.key_press_modifiers(Modifiers::SHIFT, Key::F5);
        wait_debugger(&mut harness, "멈춤", |h| !h.state().debugger.is_active());
        assert!(harness.state().debugger.location().is_none());
        assert!(harness.state().debugger.console().contains("[멈추는 중…]"));
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 빌드가_실패하면_디버그를_시작하지_않고_출력을_보인다() {
        let dir = std::env::temp_dir().join(format!("ferrite-debug-build-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".ferrite")).unwrap();
        let file = dir.join("main.py");
        fs::write(&file, "print(1)\n").unwrap();
        let build = if cfg!(windows) {
            "echo 빌드-출력& exit 3"
        } else {
            "echo 빌드-출력; exit 3"
        };
        fs::write(
            dir.join(ferrite_core::debug_config::CUSTOM_FILE),
            format!(
                "[[debug]]\nname = \"빌드 실패\"\nadapter = \"debugpy\"\nbuild = '{build}'\narguments = {{ program = \"${{file}}\" }}\n"
            ),
        )
        .unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        harness.run_ok();
        harness.key_press(Key::F5);
        harness.run_ok();
        harness.get_by_label("사용자 구성");
        harness.key_press(Key::Enter);
        // debugpy를 찾을 Python이 없으면(빌드 전에 알림) 이 시험의 대상이 아니다.
        if harness.state().status.contains("없습니다") {
            assert!(
                std::env::var_os("CI").is_none(),
                "{}",
                harness.state().status
            );
            return;
        }
        wait_debugger(&mut harness, "빌드 실패", |h| {
            !h.state().debugger.is_active() && h.state().status.contains("빌드에 실패")
        });
        let console = harness.state().debugger.console().to_owned();
        assert!(console.contains("[빌드: "), "{console}");
        assert!(
            console.contains("빌드-출력"),
            "빌드 출력(한글)을 보인다: {console}"
        );
        assert!(console.contains("[빌드 실패"), "{console}");
        drop(harness);
        remove_project(&dir);
    }

    /// 글자 칸(비밀번호 칸 포함)에 포커스를 주고 적는다.
    fn type_into(harness: &mut Harness<'_, FerriteApp>, label: &str, text: &str) {
        use egui::accesskit::Role;
        let role = if harness
            .query_by_role_and_label(Role::PasswordInput, label)
            .is_some()
        {
            Role::PasswordInput
        } else {
            Role::TextInput
        };
        harness.get_by_role_and_label(role, label).focus();
        harness.run_ok();
        harness.get_by_role_and_label(role, label).type_text(text);
        harness.run_ok();
    }

    #[test]
    fn db_연결을_추가하고_시험하고_비밀번호는_키체인에_둔다() {
        use egui::accesskit::Role;
        let dir = std::env::temp_dir().join(format!("ferrite-db-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.run_ok();
        harness.get_by_label("도구").click();
        harness.run_ok();
        harness.get_by_label("DB 연결…").click();
        harness.run_ok();
        harness.get_by_label("연결이 없습니다. [새 연결]로 추가하세요.");

        // SQLite 연결: 이름·파일을 적고 저장한 뒤 시험한다.
        harness
            .get_by_role_and_label(Role::Button, "SQLite")
            .click();
        harness.run_ok();
        type_into(&mut harness, "이름", "로컬 파일");
        harness.run_ok();
        let file = dir.join("시험.db");
        type_into(&mut harness, "파일", &file.to_string_lossy());
        harness.run_ok();
        harness.get_by_label("저장").click();
        harness.run_ok();
        assert_eq!(harness.state().db.connections().len(), 1);
        harness.get_by_label("로컬 파일");
        harness.get_by_label("연결 시험").click();
        wait_debugger(&mut harness, "SQLite 연결 시험", |h| {
            h.query_by_label_contains("연결됨: SQLite").is_some()
        });

        // PostgreSQL 연결: 빠진 값을 알리고, 비밀번호는 목록이 아니라 키체인에 둔다.
        harness
            .get_by_role_and_label(Role::Button, "PostgreSQL")
            .click();
        harness.run_ok();
        harness.get_by_label("저장").click();
        harness.run_ok();
        harness.get_by_label_contains("이름을 적으세요");
        type_into(&mut harness, "이름", "운영");
        type_into(&mut harness, "사용자", "egov");
        type_into(&mut harness, "비밀번호", "비밀!");
        harness.run_ok();
        harness.get_by_label("저장").click();
        harness.run_ok();
        let app = harness.state();
        let pg = app
            .db
            .connections()
            .iter()
            .find(|c| c.name == "운영")
            .unwrap();
        assert_eq!(pg.port, 5432);
        assert_eq!(app.db.password(&pg.id).unwrap().as_deref(), Some("비밀!"));

        // Oracle 연결: 드라이버 jar가 없으면 알리고, 프로젝트의 WEB-INF/lib에서 찾은 jar를 넣는다.
        // 시험은 받을 곳이 없어 [받기]가 꺼져 있다.
        harness
            .get_by_role_and_label(Role::Button, "Oracle")
            .click();
        harness.run_ok();
        type_into(&mut harness, "이름", "오라클");
        type_into(&mut harness, "서비스 이름", "XE");
        type_into(&mut harness, "사용자", "egov");
        harness.get_by_label("저장").click();
        harness.run_ok();
        harness.get_by_label_contains("드라이버 jar를 고르세요");
        assert!(
            harness
                .get_by_label_contains("받기(")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("취소").click();
        harness.run_ok();
        let lib = dir.join("src/main/webapp/WEB-INF/lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("ojdbc6.jar"), b"").unwrap();
        harness.get_by_label("도구").click();
        harness.run_ok();
        harness.get_by_label("DB 연결…").click();
        harness.run_ok();
        harness
            .get_by_role_and_label(Role::Button, "Oracle")
            .click();
        harness.run_ok();
        type_into(&mut harness, "이름", "오라클");
        type_into(&mut harness, "서비스 이름", "XE");
        type_into(&mut harness, "사용자", "egov");
        harness.get_by_label("ojdbc6.jar");
        harness.get_by_label("추가").click();
        harness.run_ok();
        harness.get_by_label("빼기");
        harness.get_by_label("저장").click();
        harness.run_ok();
        let oracle = harness
            .state()
            .db
            .connections()
            .iter()
            .find(|c| c.name == "오라클")
            .cloned()
            .unwrap();
        // 앱은 프로젝트 경로를 실제 경로로 연다(macOS 임시 폴더 `/var` → `/private/var`).
        assert_eq!(oracle.drivers, [real_path(&lib.join("ojdbc6.jar"))]);
        assert_eq!(oracle.jdbc_url(), "jdbc:oracle:thin:@//localhost:1521/XE");
        drop(harness);
        remove_project(&dir);
    }

    /// SQL 콘솔이 `done`이 될 때까지 그린다. 늦으면 콘솔 기록과 함께 실패한다.
    fn wait_sql(
        harness: &mut Harness<'_, FerriteApp>,
        what: &str,
        done: impl Fn(&Harness<'_, FerriteApp>) -> bool,
    ) {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(60) {
            harness.run_ok();
            if done(harness) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!(
            "{what} 시간 초과. 기록:\n{}",
            harness.state().sql.describe()
        );
    }

    /// SQL을 콘솔에 넣고 모두 골라 Ctrl+Enter로 실행한다.
    fn run_sql(harness: &mut Harness<'_, FerriteApp>, sql: &str) {
        use egui::accesskit::Role;
        harness.state_mut().sql.text = sql.to_owned();
        harness.run_ok();
        harness.get_by_role(Role::MultilineTextInput).focus();
        harness.run_ok();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run_ok();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
        harness.run_ok();
    }

    #[test]
    fn 스키마_브라우저로_표와_열을_보고_콘솔에서_조회한다() {
        use egui::accesskit::Role;
        let dir = std::env::temp_dir().join(format!("ferrite-schema-ui-{}", std::process::id()));
        let mut harness = sql_console_harness(&dir);
        run_sql(
            &mut harness,
            "CREATE TABLE 주문 (번호 INTEGER PRIMARY KEY, 메모 TEXT);\nCREATE TABLE item (id INTEGER, price REAL);\nCREATE VIEW v_item AS SELECT id FROM item;\nINSERT INTO 주문 VALUES (1, '첫'), (2, '둘')",
        );
        wait_sql(&mut harness, "표 만들기", |h| {
            h.query_by_label_contains("— 2행 바뀜").is_some()
        });

        harness.get_by_label("도구").click();
        harness.run_ok();
        harness.get_by_label("스키마 브라우저").click();
        harness.run_ok();
        assert!(dock::is_shown(&harness.state().layout, Pane::Schema));
        harness
            .get_by_role_and_label(Role::ComboBox, "연결")
            .click();
        harness.run_ok();
        // 연결 창 목록의 "로컬"은 글자이고, 고르기 목록의 것은 버튼이다.
        harness.get_by_role_and_label(Role::Button, "로컬").click();
        wait_sql(&mut harness, "스키마 읽기", |h| {
            h.query_by_label_contains("표 2개 · 뷰 1개").is_some()
        });

        // 열 이름으로 찾으면 그 표만 펼쳐 보인다.
        type_into(&mut harness, "찾기", "price");
        harness.run_ok();
        harness.get_by_label_contains("찾은 것 1개");
        harness.get_by_label("price");
        assert!(harness.query_by_label("주문").is_none());

        // 표를 펼쳐 SQL 콘솔에서 조회한다(한글 이름은 따옴표로 감싼다).
        harness
            .get_by_role_and_label(Role::TextInput, "찾기")
            .focus();
        harness.run_ok();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run_ok();
        harness
            .get_by_role_and_label(Role::TextInput, "찾기")
            .type_text("주문");
        harness.run_ok();
        harness.get_by_label("주문").click();
        harness.run_ok();
        harness.get_by_label("메모");
        harness.get_by_label("SQL 콘솔에서 조회").click();
        wait_sql(&mut harness, "조회", |h| {
            h.query_by_label("SELECT * FROM \"주문\" — 2행 받음")
                .is_some()
        });
        assert_eq!(harness.state().sql.text, "SELECT * FROM \"주문\"");
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 매퍼_sql의_표를_자동_완성하고_스키마에_없는_표와_열을_알린다() {
        use egui::accesskit::Role;
        let dir = std::env::temp_dir().join(format!("ferrite-mapper-db-{}", std::process::id()));
        let mut harness = sql_console_harness(&dir);
        run_sql(
            &mut harness,
            "CREATE TABLE emp (emp_id INTEGER PRIMARY KEY, emp_nm TEXT)",
        );
        wait_sql(&mut harness, "표 만들기", |h| {
            h.query_by_label_contains("CREATE TABLE emp").is_some()
        });
        harness.get_by_label("도구").click();
        harness.run_ok();
        harness.get_by_label("스키마 브라우저").click();
        harness.run_ok();
        harness
            .get_by_role_and_label(Role::ComboBox, "연결")
            .click();
        harness.run_ok();
        harness.get_by_role_and_label(Role::Button, "로컬").click();
        wait_sql(&mut harness, "스키마 읽기", |h| {
            h.query_by_label_contains("표 1개 · 뷰 0개").is_some()
        });

        // 매퍼를 열면 스키마에 없는 표를 알린다(별칭이 없는 표를 가리키면 열은 보지 않는다).
        let mapper = dir.join("EmpMapper.xml");
        let text = "<mapper namespace=\"egov.Emp\">\n  <select id=\"list\">SELECT e.emp_nm, e.nope FROM em e, missing m</select>\n</mapper>\n";
        fs::write(&mapper, text).unwrap();
        harness.state_mut().open_file(mapper);
        harness.run_ok();
        let messages = |h: &Harness<'_, FerriteApp>| -> Vec<String> {
            let app = h.state();
            app.docs[app.active.unwrap()]
                .lsp
                .problems
                .iter()
                .map(|p| p.message.clone())
                .collect()
        };
        assert_eq!(
            messages(&harness),
            [
                "표 em이(가) 연결 '로컬'의 스키마에 없습니다",
                "표 missing이(가) 연결 '로컬'의 스키마에 없습니다"
            ]
        );

        // FROM 뒤에서 자동 완성하면 스키마의 표가 나온다(언어 서버 없이). 고치면 다시 맞춰 본다.
        let pos = text.find("FROM em").unwrap() + "FROM em".len();
        {
            let app = harness.state_mut();
            let i = app.active.unwrap();
            app.docs[i].buffer.move_to(pos, false);
            app.lsp_request(LspRequest::Completion(None), pos);
        }
        harness.run_ok();
        let selected = harness
            .state()
            .completion
            .as_ref()
            .and_then(|p| p.selected())
            .map(|c| (c.label.clone(), c.detail.clone()));
        assert_eq!(selected, Some(("emp".into(), "표 · 열 2개".into())));
        harness.state_mut().accept_completion();
        harness.run_ok();
        let app = harness.state();
        assert!(
            app.docs[app.active.unwrap()]
                .buffer
                .text()
                .contains("FROM emp e, missing m")
        );
        assert_eq!(
            messages(&harness),
            [
                "열 nope이(가) 표 emp에 없습니다",
                "표 missing이(가) 연결 '로컬'의 스키마에 없습니다"
            ]
        );
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 노트북을_셀로_보고_고쳐_저장하면_그_셀만_바뀐다() {
        use egui::accesskit::Role;
        // nbformat(Jupyter)이 쓴 모양으로 만든 노트북과, 둘째 셀만 고친 기대 결과.
        const SAMPLE: &str = include_str!("../../ferrite-notebook/tests/fixtures/sample.ipynb");
        const EDITED: &str =
            include_str!("../../ferrite-notebook/tests/fixtures/sample_edited.ipynb");
        let dir = std::env::temp_dir().join(format!("ferrite-notebook-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("분석.ipynb");
        fs::write(&path, SAMPLE).unwrap();
        let root = dir.clone();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 1400.0))
            .build_eframe(move |cc| FerriteApp::new(cc, Some(root.clone())));
        harness.run_ok();
        harness.state_mut().open_file(path.clone());
        harness.run_ok();

        // 셀과 저장된 출력: 글, 실행 번호, 오류(색 코드를 뺀 추적), 원본 셀.
        harness.get_by_label("JSON 보기");
        harness.get_by_label("코드 [1]");
        harness.get_by_label("안녕\n둘째 줄");
        harness.get_by_label("'결과'");
        harness.get_by_label_contains("ZeroDivisionError: division by zero");
        assert_eq!(
            harness.query_all_by_role(Role::MultilineTextInput).count(),
            5
        );

        // 둘째 셀(코드)의 글을 바꾸고 저장하면 그 셀만 달라진다(나머지는 바이트까지 그대로).
        let cell = harness
            .query_all_by_role(Role::MultilineTextInput)
            .nth(1)
            .unwrap();
        cell.focus();
        harness.run_ok();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run_ok();
        harness
            .query_all_by_role(Role::MultilineTextInput)
            .nth(1)
            .unwrap()
            .type_text("print('바뀜')");
        harness.key_press(egui::Key::Enter);
        harness.run_ok();
        let index = harness.state().active.unwrap();
        assert!(harness.state().docs[index].is_dirty());
        assert!(harness.state_mut().save(index));
        assert_eq!(fs::read_to_string(&path).unwrap(), EDITED);

        // 셀을 더하고 JSON 보기로 바꾸면 에디터가 노트북 글을 보인다.
        harness.get_by_label("+ 마크다운 셀").click();
        harness.run_ok();
        assert_eq!(
            harness.query_all_by_role(Role::MultilineTextInput).count(),
            6
        );
        harness.get_by_label("JSON 보기").click();
        harness.run_ok();
        harness.get_by_label("노트북 보기");
        let text = harness.state().docs[index].buffer.text();
        let notebook = ferrite_notebook::Notebook::parse(&text).unwrap();
        assert_eq!(notebook.len(), 6);
        assert_eq!(notebook.kind(2), ferrite_notebook::CellKind::Markdown);
        drop(harness);
        remove_project(&dir);
    }

    /// PATH의 Python으로 `modules`를 가져올 수 있는지. 없으면 건너뛰되 CI에서는 실패시킨다.
    fn python_has(modules: &str) -> bool {
        let python = if cfg!(windows) { "python" } else { "python3" };
        let found = std::process::Command::new(python)
            .args(["-c", &format!("import {modules}")])
            .output()
            .is_ok_and(|o| o.status.success());
        if !found {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI에는 {modules}이(가) 설치되어 있어야 합니다"
            );
            eprintln!("{modules}이(가) 없어 건너뜁니다");
        }
        found
    }

    fn ipykernel_available() -> bool {
        python_has("ipykernel, jupyter_client")
    }

    /// `done`이 될 때까지 프레임을 돌린다(최대 120초, 커널을 처음 띄울 때 느리다).
    fn wait_until(
        harness: &mut Harness<'_, FerriteApp>,
        what: &str,
        mut done: impl FnMut(&mut Harness<'_, FerriteApp>) -> bool,
    ) {
        let started = std::time::Instant::now();
        while !done(harness) {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(120),
                "{what} 시간 초과"
            );
            harness.run_ok();
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        harness.run_ok();
    }

    /// 경로마다 정한 본문을 돌려주는 로컬 HTTP 서버. 주소(`http://127.0.0.1:포트`)를 돌려준다.
    fn http_server(routes: Vec<(&'static str, String)>) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                let _ = reader.read_line(&mut request);
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) <= 2 {
                        break;
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (status, body) = routes
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map_or(("404 Not Found", ""), |(_, b)| ("200 OK", b.as_str()));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        address
    }

    #[test]
    fn 업데이트_확인은_새_판과_최신_판과_오류를_알린다() {
        let platform = ferrite_update::platform();
        // MSI는 Ferrite가 직접 바꾸지 않으므로 내려받기 페이지만 보인다.
        let newer = format!(
            r#"{{"version": "99.0.0", "notes": "고친 것들", "platforms": {{"{platform}": {{
                "url": "https://example.invalid/Ferrite.msi", "signature": "c2ln", "format": "msi"}}}}}}"#
        );
        let same = format!(r#"{{"version": "{}"}}"#, env!("CARGO_PKG_VERSION"));
        let base = http_server(vec![("/newer.json", newer), ("/same.json", same)]);
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, None));
        harness.run_ok();
        harness.get_by_label("파일");

        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .updater
            .check(&format!("{base}/newer.json"), true, &ctx);
        wait_until(&mut harness, "새 판 알림", |h| {
            h.query_by_label_contains("새 판 99.0.0이 있습니다")
                .is_some()
        });
        harness.get_by_label("고친 것들");
        harness.get_by_label("내려받기 페이지 열기");
        harness.get_by_label("새 판 99.0.0"); // 상태 표시줄

        harness
            .state_mut()
            .updater
            .check(&format!("{base}/same.json"), true, &ctx);
        wait_until(&mut harness, "최신 판", |h| {
            h.query_by_label_contains("최신 판입니다").is_some()
        });
        assert!(harness.query_by_label("새 판 99.0.0").is_none());

        harness
            .state_mut()
            .updater
            .check(&format!("{base}/none.json"), true, &ctx);
        wait_until(&mut harness, "오류", |h| {
            h.query_by_label_contains("404").is_some()
        });
    }

    /// 예제 플러그인을 wasm32로 빌드해 `root/text-stats/`에 둔다. wasm32 타깃이 없으면 로컬에서는
    /// false(건너뜀)이고 CI에서는 실패한다.
    fn install_example_plugin(root: &Path) -> bool {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/text-stats");
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        // 바깥 cargo test가 작업 공간 target을 잠그고 있으므로 예제 폴더의 target에 빌드한다.
        let built = std::process::Command::new(cargo)
            .args([
                "build",
                "--release",
                "--target",
                "wasm32-unknown-unknown",
                "--target-dir",
            ])
            .arg(src.join("target"))
            .arg("--manifest-path")
            .arg(src.join("Cargo.toml"))
            .status()
            .is_ok_and(|s| s.success());
        if !built {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI에서는 예제 플러그인을 빌드해야 한다"
            );
            eprintln!("wasm32-unknown-unknown 타깃이 없어 건너뜀");
            return false;
        }
        let dir = root.join("text-stats");
        fs::create_dir_all(&dir).unwrap();
        fs::copy(src.join("plugin.toml"), dir.join("plugin.toml")).unwrap();
        fs::copy(
            src.join("target/wasm32-unknown-unknown/release/text_stats.wasm"),
            dir.join("text_stats.wasm"),
        )
        .unwrap();
        true
    }

    #[test]
    fn 예제_플러그인의_명령과_패널을_쓴다() {
        use egui::accesskit::Role;
        let dir = std::env::temp_dir().join(format!("ferrite-plugin-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (project, plugins) = (dir.join("project"), dir.join("plugins"));
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("a.txt"), "hello world\nhello").unwrap();
        if !install_example_plugin(&plugins) {
            return;
        }
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| {
                let mut app = FerriteApp::new(cc, Some(project.clone()));
                app.plugins = crate::plugin_ui::Plugins::load(Some(plugins.clone()));
                app
            });
        assert_eq!(harness.state().plugins.name(0), "문서 통계");
        harness.get_by_label("a.txt").click();
        harness.run();
        harness.state_mut().docs[0]
            .buffer
            .set_selection(Selection { anchor: 0, head: 5 });
        let menu = |h: &mut Harness<'_, FerriteApp>, item: &str| {
            h.get_by_label("플러그인").click();
            h.run();
            h.get_by_label("문서 통계").click();
            h.run();
            h.get_by_label(item).click();
            h.run();
        };

        // 명령: 선택 영역 바꾸기와 상태 표시줄 알림
        menu(&mut harness, "선택 영역을 대문자로");
        assert_eq!(harness.state().docs[0].buffer.text(), "HELLO world\nhello");
        menu(&mut harness, "글자 수 알리기");
        assert_eq!(harness.state().status, "문서 통계: 줄 2 · 낱말 3 · 글자 17");

        // 패널: 플러그인 창에 그리고, 입력과 버튼을 플러그인에 보낸다
        menu(&mut harness, "문서 통계 창");
        harness.get_by_label("줄 2 · 낱말 3 · 글자 17");
        let input = harness.get_by_role(Role::TextInput);
        input.focus();
        input.type_text("hello");
        harness.run();
        harness.get_by_label("'hello': 1곳");
        harness.state_mut().docs[0].buffer.set_selection(Selection {
            anchor: 6,
            head: 11,
        });
        harness.get_by_label("선택 영역을 대문자로").click();
        harness.run();
        assert_eq!(harness.state().docs[0].buffer.text(), "HELLO WORLD\nhello");
        // 문서가 바뀌면 패널을 다시 그린다(찾을 글은 플러그인에 남는다)
        harness.get_by_label("'hello': 1곳");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 언어_확장으로_강조하고_주석_기호를_쓴다() {
        let dir = std::env::temp_dir().join(format!("ferrite-lang-ext-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (project, plugins) = (dir.join("project"), dir.join("plugins"));
        let ext = plugins.join("foo-lang");
        fs::create_dir_all(ext.join("syntaxes")).unwrap();
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("a.foo"), "foo (bar)\n").unwrap();
        fs::write(
            ext.join("package.json"),
            r#"{"contributes": {
                "languages": [{"id": "foo", "aliases": ["Foo 언어"], "extensions": [".foo"],
                               "configuration": "./language-configuration.json"}],
                "grammars": [{"language": "foo", "scopeName": "source.foo",
                              "path": "./syntaxes/foo.tmLanguage.json"}]}}"#,
        )
        .unwrap();
        fs::write(
            ext.join("language-configuration.json"),
            "{ // 주석\n \"comments\": { \"lineComment\": \"#\" }, \"brackets\": [[\"(\", \")\"],] }",
        )
        .unwrap();
        fs::write(
            ext.join("syntaxes/foo.tmLanguage.json"),
            r#"{"scopeName": "source.foo", "patterns": [{"match": "\\bfoo\\b", "name": "keyword.foo"}]}"#,
        )
        .unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| {
                let mut app = FerriteApp::new(cc, Some(project.clone()));
                app.plugins = crate::plugin_ui::Plugins::load(Some(plugins.clone()));
                app
            });
        harness.get_by_label("a.foo").click();
        harness.run();
        harness.get_by_label("Foo 언어"); // 상태 표시줄
        let doc = &mut harness.state_mut().docs[0];
        let rope = doc.buffer.rope().clone();
        let lines = doc.textmate.as_mut().unwrap().highlight_lines(&rope, 0..1);
        assert_eq!(lines[0], [(0..3, ferrite_core::syntax::Highlight::Keyword)]);

        // 줄 주석은 언어 설정의 기호로
        let editor = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
        editor.focus();
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Slash);
        harness.run();
        assert_eq!(harness.state().docs[0].buffer.text(), "# foo (bar)\n");

        // 확장을 지우고 다시 읽으면 일반 텍스트로 돌아간다
        fs::remove_dir_all(&ext).unwrap();
        harness
            .state_mut()
            .on_plugin_menu(crate::plugin_ui::MenuChoice::Reload);
        harness.run();
        assert!(harness.state().docs[0].textmate.is_none());
        harness.get_by_label("일반 텍스트");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 커널의_dataframe을_그리드로_보고_정렬한다() {
        use egui::accesskit::Role;
        if !python_has("ipykernel, jupyter_client, pandas") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("ferrite-frame-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("표.ipynb");
        let mut notebook = ferrite_notebook::Notebook::parse(
            r#"{"cells": [], "metadata": {}, "nbformat": 4, "nbformat_minor": 5}"#,
        )
        .unwrap();
        notebook.insert(0, ferrite_notebook::CellKind::Code);
        notebook.set_source(
            0,
            "import pandas as pd\ndf = pd.DataFrame({'이름': [f'사람{i}' for i in range(500)], 'n': range(500)})",
        );
        fs::write(&path, notebook.to_json()).unwrap();
        let root = dir.clone();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 1000.0))
            .build_eframe(move |cc| FerriteApp::new(cc, Some(root.clone())));
        harness.run_ok();
        harness.state_mut().open_file(path.clone());
        harness.run_ok();

        harness.get_by_label("DataFrame 보기").click();
        harness.run_ok();
        harness.get_by_label("커널이 꺼져 있습니다. 셀을 실행하면 DataFrame을 볼 수 있습니다");
        harness.get_by_label("모두 실행").click();
        wait_until(&mut harness, "셀 실행", |h| {
            h.query_by_label("코드 [1]").is_some()
        });

        // 실행이 끝나면 목록을 다시 묻는다. 답이 올 때까지 목록을 열어 본다.
        wait_until(&mut harness, "DataFrame 목록", |h| {
            h.get_by_role_and_label(Role::ComboBox, "DataFrame").click();
            h.run_ok();
            let found = h.query_by_label("df · 500행 × 2열").is_some();
            if found {
                h.get_by_label("df · 500행 × 2열").click();
            } else {
                h.key_press(egui::Key::Escape);
            }
            found
        });
        wait_until(&mut harness, "첫 페이지", |h| {
            h.query_by_label("사람0").is_some()
        });
        harness.get_by_label("500행 × 2열");

        // 머리글을 누르면 오름차순, 한 번 더 누르면 내림차순으로 커널에서 정렬한다.
        harness.get_by_label("n").click();
        harness.run_ok();
        harness.get_by_label("n ▲").click();
        wait_until(&mut harness, "내림차순 정렬", |h| {
            h.query_by_label("사람499").is_some()
        });
        assert!(
            harness.query_by_label("사람0").is_none(),
            "맨 위는 큰 수부터"
        );
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 노트북_셀을_커널로_실행해_출력을_노트북에_쓴다() {
        use egui::accesskit::Role;
        if !ipykernel_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("ferrite-notebook-run-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("실행.ipynb");
        let mut notebook = ferrite_notebook::Notebook::parse(
            r#"{"cells": [], "metadata": {}, "nbformat": 4, "nbformat_minor": 5}"#,
        )
        .unwrap();
        for (i, source) in ["print('커널')\n6*7", "1/0", "print('안 돈다')"]
            .iter()
            .enumerate()
        {
            notebook.insert(i, ferrite_notebook::CellKind::Code);
            notebook.set_source(i, source);
        }
        fs::write(&path, notebook.to_json()).unwrap();
        let root = dir.clone();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 1000.0))
            .build_eframe(move |cc| FerriteApp::new(cc, Some(root.clone())));
        harness.run_ok();
        harness.state_mut().open_file(path.clone());
        harness.run_ok();
        harness.get_by_label("커널: 꺼짐(실행하면 띄웁니다)");

        // 모두 실행: 둘째 셀의 오류에서 멈추고 셋째 셀은 돌지 않는다.
        harness.get_by_label("모두 실행").click();
        let index = harness.state().active.unwrap();
        let parsed = |harness: &Harness<'_, FerriteApp>| {
            ferrite_notebook::Notebook::parse(&harness.state().docs[index].buffer.text()).unwrap()
        };
        let started = std::time::Instant::now();
        while harness.query_by_label("코드 [2]").is_none() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(120),
                "실행이 끝나지 않았습니다"
            );
            harness.run_ok();
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        harness.run_ok();
        harness.get_by_label("코드 [1]");
        harness.get_by_label("코드 [ ]");
        harness.get_by_label("커널: Python 3 (ipykernel)");
        harness.get_by_label("커널");
        harness.get_by_label("42");
        harness.get_by_label_contains("ZeroDivisionError: division by zero");
        let ran = parsed(&harness);
        assert_eq!(
            ran.outputs(0),
            [
                ferrite_notebook::Output::Stream {
                    name: "stdout".into(),
                    text: "커널\n".into()
                },
                ferrite_notebook::Output::Data {
                    data: vec![("text/plain".into(), "42".into())],
                    execution_count: Some(1)
                }
            ]
        );
        assert!(ran.outputs(2).is_empty(), "오류 뒤의 셀은 돌지 않는다");
        assert!(harness.state().docs[index].is_dirty());

        // 첫 셀에서 Shift+Enter: 그 셀을 다시 실행하고 다음 셀로 간다.
        harness
            .query_all_by_role(Role::MultilineTextInput)
            .next()
            .unwrap()
            .focus();
        harness.run_ok();
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
        let started = std::time::Instant::now();
        while harness.query_by_label("코드 [3]").is_none() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(60),
                "Shift+Enter로 실행되지 않았습니다"
            );
            harness.run_ok();
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert_eq!(
            parsed(&harness).source(0),
            "print('커널')\n6*7",
            "줄바꿈이 들어가지 않는다"
        );
        assert!(harness.state_mut().save(index));
        let saved = ferrite_notebook::Notebook::parse(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.execution_count(0), Some(3));

        // 커널을 다시 시작하면 새 커널이라 실행 번호가 1부터 다시 센다.
        harness.get_by_label("커널 다시 시작").click();
        harness.run_ok();
        harness.get_by_label("커널: 띄우는 중");
        harness.get_by_label("▶ 실행").click();
        let started = std::time::Instant::now();
        while parsed(&harness).execution_count(1) != Some(1) {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(120),
                "다시 시작한 커널로 실행되지 않았습니다"
            );
            harness.run_ok();
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        drop(harness);
        remove_project(&dir);
    }

    /// SQLite 연결을 추가하고 연결 창의 [SQL 콘솔]로 SQL 창을 열어 붙인다.
    fn sql_console_harness(dir: &std::path::Path) -> Harness<'static, FerriteApp> {
        use egui::accesskit::Role;
        let _ = fs::remove_dir_all(dir);
        fs::create_dir_all(dir).unwrap();
        let root = dir.to_path_buf();
        // 도구 창이 에디터 아래 30%에 붙으므로 결과 행이 보이도록 높게 띄운다. 한 프레임을 1/60초로 해
        // 두 번 누르기가 egui의 더블 클릭 간격(0.3초) 안에 들어가게 한다(기본은 프레임마다 0.25초).
        let mut harness = harness_builder()
            .with_size(egui::vec2(1400.0, 1300.0))
            .with_step_dt(1.0 / 60.0)
            .build_eframe(move |cc| FerriteApp::new(cc, Some(root.clone())));
        harness.run_ok();
        harness.get_by_label("도구").click();
        harness.run_ok();
        harness.get_by_label("DB 연결…").click();
        harness.run_ok();
        harness
            .get_by_role_and_label(Role::Button, "SQLite")
            .click();
        harness.run_ok();
        type_into(&mut harness, "이름", "로컬");
        type_into(&mut harness, "파일", &dir.join("콘솔.db").to_string_lossy());
        harness.get_by_label("저장").click();
        harness.run_ok();

        // 연결 창의 [SQL 콘솔]이 SQL 창을 열고 그 연결로 붙는다.
        harness.get_by_label("SQL 콘솔").click();
        wait_sql(&mut harness, "연결", |h| {
            h.query_by_label_contains("SQLite 3").is_some()
        });
        assert!(dock::is_shown(&harness.state().layout, Pane::Sql));
        harness
    }

    #[test]
    fn sql_콘솔에서_실행하고_롤백하고_csv로_내보낸다() {
        use egui::accesskit::Role;
        let dir = std::env::temp_dir().join(format!("ferrite-sql-ui-{}", std::process::id()));
        let mut harness = sql_console_harness(&dir);

        // 고른 글의 세 문장을 차례로 실행하고 마지막 조회를 그리드에 보인다.
        run_sql(
            &mut harness,
            "CREATE TABLE t (a INTEGER, b TEXT);\nINSERT INTO t VALUES (1, '가나'), (2, NULL);\nSELECT a, b FROM t ORDER BY a",
        );
        wait_sql(&mut harness, "조회", |h| {
            h.query_by_label_contains("ORDER BY a — 2행 받음").is_some()
        });
        harness.get_by_label_contains("INSERT INTO t VALUES");
        harness.get_by_label("가나");
        harness.get_by_label("NULL");

        // 마지막 조회를 다른 연결로 다시 실행해 CSV로 쓴다.
        let csv = dir.join("결과.csv");
        let ctx = harness.ctx.clone();
        let app = harness.state_mut();
        app.sql.start_export(csv.clone(), &app.db, &ctx);
        wait_sql(&mut harness, "CSV", |h| {
            h.query_by_label_contains("CSV로 저장했습니다").is_some()
        });
        assert_eq!(
            fs::read_to_string(&csv).unwrap(),
            "\u{feff}a,b\n1,가나\n2,\n"
        );

        // 자동 커밋을 끄고 지운 뒤 롤백하면 행이 돌아온다.
        harness.get_by_label("자동 커밋").click();
        wait_sql(&mut harness, "자동 커밋 끄기", |h| {
            h.query_by_label("트랜잭션 중").is_some()
        });
        run_sql(&mut harness, "DELETE FROM t");
        wait_sql(&mut harness, "지우기", |h| {
            h.query_by_label_contains("DELETE FROM t — 2행 바뀜")
                .is_some()
        });
        harness.get_by_label("롤백").click();
        wait_sql(&mut harness, "롤백", |h| {
            h.query_by_label("롤백했습니다").is_some()
        });
        run_sql(&mut harness, "SELECT count(*) AS n FROM t");
        wait_sql(&mut harness, "세기", |h| {
            h.query_by_label_contains("FROM t — 1행 받음").is_some()
        });
        harness.get_by_label("2");

        // 실행 계획은 종류마다 맞는 문장으로 본다(SQLite: EXPLAIN QUERY PLAN).
        harness.state_mut().sql.text = "SELECT * FROM t WHERE a = 1".into();
        harness.run_ok();
        harness.get_by_role(Role::MultilineTextInput).focus();
        harness.run_ok();
        harness.get_by_label("실행 계획").click();
        wait_sql(&mut harness, "실행 계획", |h| {
            h.query_by_label_contains("EXPLAIN QUERY PLAN SELECT")
                .is_some()
        });
        harness.get_by_label_contains("SCAN t");
        drop(harness);
        remove_project(&dir);
    }

    /// 그리드 칸을 두 번 눌러 고친다. 앞서 누른 단추와 0.3초 안이면 egui가 세 번 누르기로 세므로 먼저
    /// 시간을 흘려 보낸다.
    fn edit_cell(harness: &mut Harness<'_, FerriteApp>, label: &str) {
        harness.run_steps(20);
        harness.get_by_label(label).click();
        harness.step();
        harness.get_by_label(label).click();
        harness.run_ok();
        harness.get_by_label("새 값");
    }

    #[test]
    fn sql_콘솔_그리드에서_값을_고친다() {
        use egui::accesskit::Role;
        let dir = std::env::temp_dir().join(format!("ferrite-sql-edit-{}", std::process::id()));
        let mut harness = sql_console_harness(&dir);
        run_sql(
            &mut harness,
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, note TEXT);\nINSERT INTO t VALUES (1, '가나', NULL), (2, '다라', 'x');\nSELECT id, name, note FROM t ORDER BY id",
        );
        wait_sql(&mut harness, "조회", |h| {
            h.query_by_label_contains("ORDER BY id — 2행 받음")
                .is_some()
        });
        harness.get_by_label_contains("t · 칸을 두 번 눌러 고칩니다");

        // 글자 칸의 값을 모두 골라 바꾸고 Enter로 저장한다(따옴표·세미콜론도 값 그대로).
        edit_cell(&mut harness, "다라");
        harness
            .get_by_role_and_label(Role::TextInput, "새 값")
            .focus();
        harness.run_ok();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run_ok();
        harness
            .get_by_role_and_label(Role::TextInput, "새 값")
            .type_text("it's; 바뀜");
        harness.run_ok();
        harness.key_press(egui::Key::Enter);
        wait_sql(&mut harness, "이름 고치기", |h| {
            h.query_by_label("2행 값을 고쳤습니다").is_some()
        });
        harness.get_by_label("it's; 바뀜");

        // NULL 칸은 NULL을 풀고 적어 [저장]한다.
        edit_cell(&mut harness, "NULL");
        harness
            .get_by_role_and_label(Role::CheckBox, "NULL")
            .click();
        harness.run_ok();
        type_into(&mut harness, "새 값", "메모");
        harness.get_by_label("저장").click();
        wait_sql(&mut harness, "NULL 고치기", |h| {
            h.query_by_label("1행 값을 고쳤습니다").is_some()
        });

        // 트랜잭션 안에서 NULL로 고친 뒤 롤백하면, 다시 실행했을 때 앞 값이 보인다.
        harness.get_by_label("자동 커밋").click();
        wait_sql(&mut harness, "자동 커밋 끄기", |h| {
            h.query_by_label("트랜잭션 중").is_some()
        });
        edit_cell(&mut harness, "가나");
        harness
            .get_by_role_and_label(Role::CheckBox, "NULL")
            .click();
        harness.run_ok();
        harness.get_by_label("저장").click();
        wait_sql(&mut harness, "트랜잭션 안에서 고치기", |h| {
            h.query_by_label("1행 값을 고쳤습니다 (커밋해야 저장됩니다)")
                .is_some()
        });
        harness.get_by_label("롤백").click();
        wait_sql(&mut harness, "롤백", |h| {
            h.query_by_label("롤백했습니다").is_some()
        });
        run_sql(&mut harness, "SELECT id, name, note FROM t ORDER BY id");
        // 처음 실행한 같은 조회의 기록과 둘이 된다.
        wait_sql(&mut harness, "다시 조회", |h| {
            h.query_all_by_label("SELECT id, name, note FROM t ORDER BY id — 2행 받음")
                .count()
                == 2
        });
        harness.get_by_label("가나");
        harness.get_by_label("메모");
        harness.get_by_label("it's; 바뀜");

        // 기본 키가 빠진 조회는 읽기 전용이다.
        run_sql(&mut harness, "SELECT name FROM t");
        wait_sql(&mut harness, "읽기 전용", |h| {
            h.query_by_label_contains("읽기 전용").is_some()
        });
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 터미널에서_명령을_실행하고_탭을_닫는다() {
        let dir = std::env::temp_dir().join(format!("ferrite-terminal-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.run();
        harness.key_press_modifiers(Modifiers::CTRL, Key::Backtick);
        // 셸(Windows PowerShell, 그 밖에는 사용자 셸)이 프롬프트를 띄울 때까지 기다린다.
        wait_for_terminal(&mut harness, |text| !text.trim().is_empty());
        harness
            .query_by_role(egui::accesskit::Role::Terminal)
            .unwrap()
            .type_text("echo kittest-ferrite");
        harness.key_press(Key::Enter);
        // 출력이 한 줄로 따로 찍혔는지 본다. 입력한 명령 줄은 프롬프트가 길면(macOS CI의 호스트 이름)
        // 터미널 폭에서 접혀 낱말이 끊기므로 세지 않는다.
        wait_for_terminal(&mut harness, |text| {
            text.lines().any(|line| line.trim() == "kittest-ferrite")
        });

        // 두 번째 탭을 열고 첫 탭을 닫는다(셸을 끝낸다).
        harness.get_by_label("+").click();
        harness.run_ok();
        assert_eq!(harness.state().terminal.tab_count(), 2);
        harness.get_all_by_label("×").next().unwrap().click();
        harness.run_ok();
        assert_eq!(harness.state().terminal.tab_count(), 1);

        // 다시 누르면 숨긴다.
        harness.key_press_modifiers(Modifiers::CTRL, Key::Backtick);
        harness.run_ok();
        assert!(
            harness
                .query_by_role(egui::accesskit::Role::Terminal)
                .is_none()
        );
        drop(harness);
        remove_project(&dir);
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn 소스_제어에서_거터를_보고_스테이지해_커밋한다() {
        use egui::accesskit::Role;
        let dir = std::env::temp_dir().join(format!("ferrite-git-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.name", "Ferrite Test"]);
        git(&dir, &["config", "user.email", "test@example.invalid"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "first"]);

        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.run_ok();
        harness.state_mut().open_file(dir.join("a.txt"));
        wait_for_label(&mut harness, "Git: main");
        let file = harness.state().docs[0].path.clone();
        for _ in 0..200 {
            harness.run_ok();
            if harness.state().git.base(&file).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert_eq!(harness.state().git.base(&file), Some("one\ntwo\n"));

        // 저장하지 않은 편집도 거터에 바로 보인다(첫 줄 수정).
        harness.get_by_role(Role::MultilineTextInput).focus();
        harness.run_ok();
        harness.get_by_role(Role::MultilineTextInput).type_text("X");
        harness.run_ok();
        assert_eq!(
            harness.state().docs[0].git_marks,
            [(0, ferrite_core::git::LineChange::Modified)]
        );
        harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
        harness.run_ok();

        harness.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
        wait_for_label(&mut harness, "변경 사항 1");
        harness.get_by_label_contains("M a.txt").click();
        harness.run_ok();
        harness.get_by_label_contains("+Xone");
        harness.get_by_label("+").click();
        wait_for_label(&mut harness, "스테이지됨 1");
        harness
            .get_by_role_and_label(Role::MultilineTextInput, "커밋 메시지")
            .focus();
        harness.run_ok();
        harness
            .get_by_role_and_label(Role::MultilineTextInput, "커밋 메시지")
            .type_text("두 번째 커밋");
        harness.run_ok();
        harness.get_by_label("커밋").click();
        wait_for_label(&mut harness, "커밋했습니다");
        wait_for_label(&mut harness, "변경 사항 0");
        let log = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["log", "--format=%s"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&log.stdout),
            "두 번째 커밋\nfirst\n"
        );
        // 커밋하면 인덱스가 바뀌어 거터 표시가 사라진다.
        for _ in 0..100 {
            harness.run_ok();
            if harness.state().docs[0].git_marks.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(harness.state().docs[0].git_marks.is_empty());
        drop(harness);
        remove_project(&dir);
    }

    /// 로그인(302 + 세션 쿠키) 뒤 쿠키가 있어야 답하는 로컬 서버.
    fn http_login_server() -> u16 {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(&stream);
                let mut head = Vec::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    head.push(line.trim().to_owned());
                }
                let first = head.first().cloned().unwrap_or_default();
                let cookie = head.iter().any(|h| h.eq_ignore_ascii_case("cookie: sid=1"));
                let (status, extra, body) = if first.starts_with("GET /login ") {
                    (
                        "302 Found",
                        "Location: /me\r\nSet-Cookie: sid=1\r\n",
                        String::new(),
                    )
                } else if first.starts_with("GET /me ") && cookie {
                    (
                        "200 OK",
                        "Content-Type: application/json; charset=UTF-8\r\n",
                        r#"{"user":"관리자"}"#.to_owned(),
                    )
                } else {
                    ("401 Unauthorized", "", String::new())
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        port
    }

    #[test]
    fn 요청_파일을_보내고_응답_창에_보이며_엔드포인트로_요청을_만든다() {
        use egui::accesskit::Role;

        let port = http_login_server();
        let dir = std::env::temp_dir().join(format!("ferrite-http-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join("http")).unwrap();
        fs::write(
            dir.join("http").join(ferrite_http::env::PUBLIC),
            format!(r#"{{"dev": {{"base": "http://127.0.0.1:{port}"}}}}"#),
        )
        .unwrap();
        fs::write(
            dir.join("http").join(ferrite_http::env::PRIVATE),
            r#"{"dev": {"path": "/me"}}"#,
        )
        .unwrap();
        let file = dir.join("http").join("login.http");
        fs::write(
            &file,
            "### 로그인\nGET {{base}}/login\n\n### 내 정보\nGET {{base}}{{path}}\n",
        )
        .unwrap();

        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.state_mut().open_file(file.clone());
        harness.run_ok();
        assert_eq!(
            harness.state().docs[0]
                .http_runs
                .as_ref()
                .map(|(_, l)| l.clone()),
            Some(vec![1, 4]),
            "요청 줄에 ▶"
        );

        // 쿠키가 없으면 401, 거터 ▶로 로그인하면 302를 따라가 쿠키와 함께 200을 받는다.
        let path = harness.state().docs[0].path.clone();
        harness.state_mut().on_gutter_click(path.clone(), 4, false);
        assert!(run_until(&mut harness, |app| app.http.done()));
        assert!(dock::is_shown(&harness.state().layout, Pane::Http));
        harness.get_by_label("401 Unauthorized");
        harness.state_mut().on_gutter_click(path.clone(), 2, false);
        harness.run_ok();
        assert!(
            !harness.state().http.running(),
            "▶가 아닌 줄은 보내지 않는다"
        );
        harness.state_mut().on_gutter_click(path, 1, false);
        assert!(run_until(&mut harness, |app| app.http.done()));
        harness.get_by_label("200 OK");
        harness.get_by_label("쿠키 1개 지우기");

        // Ctrl+Enter: 커서가 든 요청(둘째)을 보낸다. 개인 환경 파일의 값도 채운다.
        harness
            .get_by_role_and_label(Role::MultilineTextInput, "login.http")
            .focus();
        harness.run_ok();
        {
            let doc = &mut harness.state_mut().docs[0];
            let pos = doc.buffer.line_col_to_pos(4, 3);
            doc.buffer.set_selection(Selection::caret(pos));
        }
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        assert!(run_until(&mut harness, |app| app.http.done()));
        assert!(
            !harness.state().docs[0].is_dirty(),
            "Ctrl+Enter는 줄을 나누지 않는다"
        );
        harness.get_by_label("200 OK");
        assert_eq!(
            harness.state().http.body(),
            Some("{\n  \"user\": \"관리자\"\n}")
        );

        // 개인 환경 파일이 git에서 빠지지 않았으면 경고하고, 누르면 .gitignore에 더한다.
        harness.get_by_label(".gitignore에 더하기").click();
        harness.run_ok();
        assert!(ferrite_http::env::ignored_by_git(&dir));
        assert!(harness.query_by_label(".gitignore에 더하기").is_none());

        // 엔드포인트로 요청 만들기: 컨트롤러 파일 끝에 더하고, 다시 하면 더하지 않고 찾아간다.
        for _ in 0..2 {
            harness.state_mut().http_task = Some(crate::http_ui::Task::Make {
                controller: "BoardController".into(),
                path: "/board/save.do".into(),
                methods: "POST".into(),
            });
            harness.run_ok();
        }
        let app = harness.state();
        let doc = &app.docs[app.active.unwrap()];
        assert!(doc.path.ends_with("http/BoardController.http"));
        assert_eq!(
            doc.buffer.text(),
            "### BoardController /board/save.do\nPOST {{baseUrl}}/board/save.do\nContent-Type: application/x-www-form-urlencoded\n\n"
        );
        assert_eq!(
            doc.buffer.pos_to_line_col(doc.buffer.selection().head),
            (1, 0)
        );
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 어댑터에서_받은_화면_스크립트는_읽기_전용으로_연다() {
        use egui::accesskit::Role;

        let dir =
            std::env::temp_dir().join(format!("ferrite-readonly-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("list.do_5_9.html");
        let text = "<script>\ntick();\n</script>\n";
        fs::write(&file, text).unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
        harness.run_ok();
        let path = harness.state().docs[0].path.clone();
        harness.state_mut().debugger.mark_stored(path);
        harness.get_by_role(Role::MultilineTextInput).focus();
        harness.run_ok();
        // 글자·지우기·줄 나누기는 버리고, 움직임은 받는다.
        harness.get_by_role(Role::MultilineTextInput).type_text("x");
        harness.key_press(Key::Backspace);
        harness.key_press(Key::Enter);
        harness.run_ok();
        harness.key_press(Key::ArrowDown);
        harness.run_ok();
        let doc = &harness.state().docs[0];
        assert_eq!(doc.buffer.text(), text);
        assert!(!doc.is_dirty());
        assert_eq!(doc.buffer.pos_to_line_col(doc.buffer.selection().head).0, 1);
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 웹_프로젝트의_스크립트에_jsconfig가_없으면_만들자고_한다() {
        let dir =
            std::env::temp_dir().join(format!("ferrite-jsconfig-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let js = dir.join("src/main/webapp/js");
        fs::create_dir_all(&js).unwrap();
        fs::write(js.join("board.js"), "$.ajax({});\n").unwrap();
        fs::write(js.join("list.js"), "$('#a').val();\n").unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 700.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.state_mut().open_file(js.join("board.js"));
        harness.run_ok();
        // 닫으면 그 프로젝트에서는 다시 묻지 않는다(다른 문서도).
        harness.get_by_label("닫기").click();
        harness.run_ok();
        assert!(harness.query_by_label("jsconfig.json 만들기").is_none());
        harness.state_mut().open_file(js.join("list.js"));
        harness.run_ok();
        assert!(harness.query_by_label("jsconfig.json 만들기").is_none());

        harness.state_mut().jsconfig_dismissed.clear();
        harness.run_ok();
        harness.get_by_label("jsconfig.json 만들기").click();
        harness.run_ok();
        let text = fs::read_to_string(dir.join("jsconfig.json")).unwrap();
        assert_eq!(
            text,
            ferrite_core::web::jsconfig_text(&dir, "src/main/webapp")
        );
        assert!(harness.query_by_label("jsconfig.json 만들기").is_none());
        assert!(
            harness
                .state()
                .status
                .contains("jsconfig.json을 만들었습니다")
        );
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 테스트_결과를_트리로_보이고_실패_스택의_테스트_줄로_간다() {
        let dir = std::env::temp_dir().join(format!("ferrite-tests-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let java = dir.join("src/test/java/com/example/board");
        fs::create_dir_all(&java).unwrap();
        fs::write(dir.join("pom.xml"), "<project/>").unwrap();
        let source: String = (1..=30).map(|i| format!("// {i}\n")).collect();
        fs::write(java.join("CalcTest.java"), &source).unwrap();
        let report = r#"<testsuite name="com.example.board.CalcTest">
  <testcase name="adds" classname="com.example.board.CalcTest" time="0.022"/>
  <testcase name="addsWrong" classname="com.example.board.CalcTest" time="0.004">
    <failure message="더하기 ==&gt; expected: &lt;4&gt; but was: &lt;3&gt;"><![CDATA[org.opentest4j.AssertionFailedError: 더하기
	at org.junit.jupiter.api.Assertions.assertEquals(Assertions.java:563)
	at com.example.board.CalcTest.addsWrong(CalcTest.java:16)
]]></failure>
  </testcase>
</testsuite>"#;
        let cases = ferrite_core::test_report::parse_junit(report).unwrap();
        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 1000.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        harness.run_ok();
        let config = ferrite_core::run::RunConfig {
            name: "mvn test".into(),
            command: "mvn test".into(),
            cwd: None,
            env: Default::default(),
        };
        {
            let app = harness.state_mut();
            app.tests.show_results(config, dir.clone(), cases);
            dock::show(&mut app.layout, Pane::Tests);
        }
        harness.run_ok();
        // 실패가 있는 묶음은 펼쳐 두고, 실패를 골라 사유를 보인다.
        harness.get_by_label("com.example.board.CalcTest (실패 1/2)");
        harness.get_by_label("adds  0.022초");
        harness.get_by_label("더하기 ==> expected: <4> but was: <3>");
        assert!(harness.query_by_label("실패만 다시").is_some());
        // 스택의 테스트 줄을 누르면 그 파일의 그 줄로 간다(라이브러리 프레임이 아니라).
        harness
            .get_by_label("at com.example.board.CalcTest.addsWrong(CalcTest.java:16)")
            .click();
        harness.run_ok();
        let app = harness.state();
        let doc = &app.docs[app.active.unwrap()];
        assert!(
            doc.path.ends_with("CalcTest.java"),
            "{}",
            doc.path.display()
        );
        assert_eq!(
            doc.buffer.pos_to_line_col(doc.buffer.selection().head).0,
            15
        );
        drop(harness);
        remove_project(&dir);
    }

    #[test]
    fn 정적_분석_sarif를_불러와_파일별로_보이고_줄로_가며_바뀌면_다시_읽는다() {
        let dir = std::env::temp_dir().join(format!("ferrite-sarif-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let java = dir.join("src/main/java/com/example/board");
        fs::create_dir_all(&java).unwrap();
        fs::create_dir_all(dir.join("target")).unwrap();
        let source: String = (1..=16).map(|i| format!("    line{i}();\n")).collect();
        fs::write(java.join("Board.java"), &source).unwrap();
        // SpotBugs가 쓰는 꼴: 원본 폴더 기준 경로, 규칙의 메시지 틀과 인자.
        let sarif = |results: &str| {
            format!(
                r#"{{"version": "2.1.0", "runs": [{{"tool": {{"driver": {{"name": "SpotBugs", "rules": [
                  {{"id": "NP_ALWAYS_NULL", "messageStrings": {{"default": {{"text": "Null pointer dereference of {{0}} in {{1}}."}}}}}},
                  {{"id": "ES_COMPARING_PARAMETER_STRING_WITH_EQ", "messageStrings": {{"default": {{"text": "Comparison of String parameter using == or != in {{0}}."}}}}}}
                ]}}}}, "results": [{results}]}}]}}"#
            )
        };
        let np = r#"{"ruleId": "NP_ALWAYS_NULL", "ruleIndex": 0, "level": "error", "message": {"id": "default", "text": "Null pointer dereference", "arguments": ["s", "Board.length(String)"]}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "com/example/board/Board.java"}, "region": {"startLine": 12}}}]}"#;
        let es = r#"{"ruleId": "ES_COMPARING_PARAMETER_STRING_WITH_EQ", "ruleIndex": 1, "level": "warning", "message": {"id": "default", "arguments": ["Board.same(String)"]}, "locations": [{"physicalLocation": {"artifactLocation": {"uri": "com/example/board/Board.java"}, "region": {"startLine": 7}}}]}"#;
        let report = dir.join("target/spotbugsSarif.json");
        fs::write(&report, sarif(&format!("{np}, {es}"))).unwrap();

        let mut harness = harness_builder()
            .with_size(egui::vec2(1200.0, 1000.0))
            .build_eframe(|cc| FerriteApp::new(cc, Some(dir.clone())));
        {
            let app = harness.state_mut();
            app.problems_sarif = true;
            dock::show(&mut app.layout, Pane::Problems);
        }
        harness.run_ok();
        harness.get_by_label("불러오기").click();
        harness.run_ok();
        harness.get_by_label("src/main/java/com/example/board/Board.java (2)");
        let row =
            "12:1  [오류] NP_ALWAYS_NULL: Null pointer dereference of s in Board.length(String).";
        harness.get_by_label(row).click();
        harness.run_ok();
        {
            let app = harness.state();
            let doc = &app.docs[app.active.unwrap()];
            assert!(doc.path.ends_with("Board.java"), "{}", doc.path.display());
            assert_eq!(
                doc.buffer.pos_to_line_col(doc.buffer.selection().head),
                (11, 0)
            );
            // 열린 파일에는 물결선(들여쓰기를 뺀 그 줄의 글자)으로도 보이고, 상태바 개수에 들어간다.
            let ranges: Vec<_> = doc.sarif_problems.iter().map(|p| p.range.clone()).collect();
            let line7 = doc.buffer.line_col_to_pos(6, 4);
            assert_eq!(ranges[0], line7..line7 + 8);
            assert_eq!(doc.sarif_problems.len(), 2);
        }
        harness.get_by_label("오류 1 · 경고 1");

        // 분석을 다시 돌려 SARIF가 바뀌면 다시 읽는다(1초마다 본다).
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(&report, sarif(es)).unwrap();
        assert!(
            run_until(&mut harness, |app| app.sarif.findings().len() == 1),
            "바뀐 SARIF를 다시 읽지 않음"
        );
        harness.run_ok();
        let app = harness.state();
        assert_eq!(app.docs[app.active.unwrap()].sarif_problems.len(), 1);
        drop(harness);
        remove_project(&dir);
    }
}
