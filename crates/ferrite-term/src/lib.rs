//! 통합 터미널. Alacritty의 터미널 엔진(alacritty_terminal)으로 셸을 PTY(Windows ConPTY, Unix pty)에
//! 띄운다. PTY 출력은 그 읽기 스레드(`EventLoop`)가 화면 상태(`Term`)에 바로 반영하고, UI는
//! [`Terminal::snapshot`]으로 화면을 꺼내 그린다. UI(egui)와 떨어져 있어 단독으로 시험한다.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::tty;
pub use alacritty_terminal::vte::ansi::CursorShape;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// 위로 되돌려 볼 수 있는 줄 수.
const SCROLLBACK: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl Size {
    fn window(self) -> WindowSize {
        WindowSize {
            num_lines: self.lines as u16,
            num_cols: self.columns as u16,
            cell_width: 1,
            cell_height: 1,
        }
    }
}

enum Notice {
    Title(String),
    Exited(Option<i32>),
}

#[derive(Clone)]
struct Listener {
    notices: Sender<Notice>,
    /// 셸의 질의에 답을 보낼 곳. 이벤트 루프를 만든 뒤에 채운다.
    pty: Arc<OnceLock<EventLoopSender>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            // 셸이 물은 것(커서 위치 등)에 대한 터미널의 답. ConPTY는 시작할 때 커서 위치를 묻는다.
            Event::PtyWrite(text) => {
                if let Some(pty) = self.pty.get() {
                    let _ = pty.send(Msg::Input(text.into_bytes().into()));
                }
            }
            Event::Title(title) => {
                let _ = self.notices.send(Notice::Title(title));
            }
            Event::ResetTitle => {
                let _ = self.notices.send(Notice::Title(String::new()));
            }
            Event::ChildExit(status) => {
                let _ = self.notices.send(Notice::Exited(status.code()));
            }
            Event::Exit => {
                let _ = self.notices.send(Notice::Exited(None));
            }
            // 클립보드 읽기·쓰기 요청(OSC 52)은 받지 않는다(셸 출력이 클립보드를 엿보지 못하게).
            _ => {}
        }
        (self.wake)();
    }
}

#[derive(Default)]
pub struct Options {
    pub working_directory: PathBuf,
    pub columns: u16,
    pub lines: u16,
    /// (프로그램, 인자). 없으면 기본 셸(Windows PowerShell, Unix 사용자 셸).
    pub shell: Option<(String, Vec<String>)>,
    /// 있으면 `shell` 대신 이 명령 줄을 플랫폼 셸로 실행한다(실행 구성).
    pub command: Option<String>,
    /// 더할 환경 변수.
    pub env: HashMap<String, String>,
}

/// 명령 줄을 실행할 플랫폼 셸.
/// - Windows: `cmd /d /s /c "줄"`. cmd는 C 런타임과 따옴표 규칙이 달라, 줄을 이스케이프하지 않고
///   그대로 넘긴다(`/s`가 바깥 따옴표만 벗긴다). `.cmd`·`.bat`(mvn, gradle)도 PATH에서 찾는다.
/// - 그 밖: 사용자 셸을 로그인 셸(`-l -c`)로 띄워, 프로필이 정한 PATH(Homebrew·SDKMAN 등)를 쓴다.
///
/// 돌려주는 bool은 Windows에서 인자를 C 런타임 규칙으로 이스케이프할지.
fn command_shell(line: &str) -> (tty::Shell, bool) {
    #[cfg(windows)]
    {
        let args = ["/d", "/s", "/c"].map(String::from);
        let mut args = args.to_vec();
        args.push(format!("\"{line}\""));
        (tty::Shell::new("cmd.exe".to_owned(), args), false)
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
        let args = vec!["-l".to_owned(), "-c".to_owned(), line.to_owned()];
        (tty::Shell::new(shell, args), true)
    }
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    pty: EventLoopSender,
    notices: Receiver<Notice>,
    size: Size,
    title: String,
    /// 셸이 끝났으면 Some(종료 코드, 모르면 None).
    exited: Option<Option<i32>>,
}

impl Terminal {
    /// 셸을 띄운다. `wake`는 출력이 오거나 셸이 끝날 때 PTY 스레드에서 불린다(화면을 다시 그리게).
    pub fn spawn(options: &Options, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let size = Size {
            columns: usize::from(options.columns.max(2)),
            lines: usize::from(options.lines.max(1)),
        };
        let mut env = HashMap::from([
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ]);
        env.extend(options.env.clone());
        let (shell, escape_args) = match &options.command {
            Some(line) => {
                let (shell, escape) = command_shell(line);
                (Some(shell), escape)
            }
            None => (
                options
                    .shell
                    .clone()
                    .map(|(program, args)| tty::Shell::new(program, args)),
                true,
            ),
        };
        #[cfg(not(windows))]
        let _ = escape_args;
        let tty_options = tty::Options {
            shell,
            working_directory: Some(options.working_directory.clone()),
            drain_on_exit: true,
            env,
            #[cfg(windows)]
            escape_args,
        };
        let pty = tty::new(&tty_options, size.window(), 0)?;
        let (notices_tx, notices) = channel();
        let pty_sender = Arc::new(OnceLock::new());
        let listener = Listener {
            notices: notices_tx,
            pty: pty_sender.clone(),
            wake: Arc::new(wake),
        };
        let config = term::Config {
            scrolling_history: SCROLLBACK,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        let _ = pty_sender.set(sender.clone());
        // 스레드는 Shutdown을 받거나 셸이 끝나면 스스로 끝난다.
        drop(event_loop.spawn());
        Ok(Self {
            term,
            pty: sender,
            notices,
            size,
            title: String::new(),
            exited: None,
        })
    }

    /// 셸에 바이트를 보낸다. 위로 스크롤해 둔 화면은 맨 아래로 돌린다.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.term.lock().scroll_display(Scroll::Bottom);
        let _ = self.pty.send(Msg::Input(bytes.into()));
    }

    /// 붙여 넣는다. 줄바꿈은 Enter(`\r`)로 바꾼다. 셸이 bracketed paste를 켰으면 감싸고, 그 안의 ESC는
    /// 뺀다(붙여 넣는 글자가 감싸기를 끝내고 명령을 실행하지 못하게).
    pub fn paste(&self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let bytes = if bracketed {
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
        } else {
            text
        };
        self.write(bytes.into_bytes());
    }

    /// 화면 크기(글자 칸)를 바꾼다. 같으면 아무것도 하지 않는다.
    pub fn resize(&mut self, columns: u16, lines: u16) {
        let size = Size {
            columns: usize::from(columns.max(2)),
            lines: usize::from(lines.max(1)),
        };
        if size == self.size {
            return;
        }
        self.size = size;
        self.term.lock().resize(size);
        let _ = self.pty.send(Msg::Resize(size.window()));
    }

    pub fn size(&self) -> (usize, usize) {
        (self.size.columns, self.size.lines)
    }

    /// PTY 스레드가 보낸 알림(제목, 종료)을 거둔다. 매 프레임 부른다.
    pub fn poll(&mut self) {
        for notice in self.notices.try_iter() {
            match notice {
                Notice::Title(title) => self.title = title,
                // 종료 코드를 아는 알림이 먼저 와도 뒤의 "끝남"이 덮지 않게 한다.
                Notice::Exited(code) => {
                    if code.is_some() || self.exited.is_none() {
                        self.exited = Some(code);
                    }
                }
            }
        }
    }

    /// 셸이 정한 제목(OSC 0·2). 없으면 빈 글자.
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn exited(&self) -> Option<Option<i32>> {
        self.exited
    }

    /// 줄 단위로 스크롤한다(양수는 위로).
    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    /// 방향키를 응용 프로그램 모드(`ESC O A`)로 보낼지.
    pub fn app_cursor(&self) -> bool {
        self.term.lock().mode().contains(TermMode::APP_CURSOR)
    }

    fn point(&self, term: &Term<Listener>, line: usize, column: usize) -> Point {
        let offset = term.grid().display_offset() as i32;
        Point::new(
            Line(line as i32 - offset),
            Column(column.min(self.size.columns - 1)),
        )
    }

    /// 화면 위치(줄, 열)에서 끌어 고른다.
    pub fn select(&self, from: (usize, usize), to: (usize, usize)) {
        let mut term = self.term.lock();
        let start = self.point(&term, from.0, from.1);
        let end = self.point(&term, to.0, to.1);
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        term.selection = Some(selection);
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term
            .lock()
            .selection_to_string()
            .filter(|t| !t.is_empty())
    }

    /// 지금 보이는 화면. 색은 RGB로 풀고, 반전은 앞뒤 색을 바꿔 둔다.
    pub fn snapshot(&self) -> Snapshot {
        let term = self.term.lock();
        let content = term.renderable_content();
        let (columns, lines) = (self.size.columns, self.size.lines);
        let offset = content.display_offset as i32;
        let mut cells = vec![Cell::default(); columns * lines];
        for indexed in content.display_iter {
            let line = indexed.point.line.0 + offset;
            let column = indexed.point.column.0;
            if line < 0 || line as usize >= lines || column >= columns {
                continue;
            }
            let cell = indexed.cell;
            let flags = cell.flags;
            let mut fg = resolve(term.colors(), cell.fg);
            let mut bg = resolve(term.colors(), cell.bg);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
                fg = fg.inverted(Paint::Background);
                bg = bg.inverted(Paint::Foreground);
            }
            cells[line as usize * columns + column] = Cell {
                c: if flags.contains(Flags::HIDDEN) {
                    ' '
                } else {
                    cell.c
                },
                fg,
                bg,
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                wide: flags.contains(Flags::WIDE_CHAR),
                spacer: flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
                selected: content.selection.is_some_and(|s| s.contains(indexed.point)),
            };
        }
        let cursor_line = content.cursor.point.line.0 + offset;
        let cursor = (content.cursor.shape != CursorShape::Hidden
            && (0..lines as i32).contains(&cursor_line))
        .then_some((cursor_line as usize, content.cursor.point.column.0));
        Snapshot {
            columns,
            lines,
            cells,
            cursor,
            cursor_shape: content.cursor.shape,
            scrolled: content.display_offset > 0,
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // 이벤트 루프를 끝내면 PTY가 닫혀 셸도 끝난다.
        let _ = self.pty.send(Msg::Shutdown);
    }
}

/// 칸의 색. 기본 앞·뒤 색은 UI 테마를 따르도록 따로 둔다.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Paint {
    #[default]
    Foreground,
    Background,
    Rgb(u8, u8, u8),
}

impl Paint {
    /// 반전한 칸에서 기본 색은 반대쪽 기본 색이 된다.
    fn inverted(self, default: Paint) -> Paint {
        match self {
            Paint::Foreground | Paint::Background => default,
            rgb => rgb,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub fg: Paint,
    pub bg: Paint,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// 두 칸을 차지하는 글자(한글 등)의 앞 칸.
    pub wide: bool,
    /// 넓은 글자의 뒤 칸(그리지 않는다).
    pub spacer: bool,
    pub selected: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: Paint::Foreground,
            bg: Paint::Background,
            bold: false,
            italic: false,
            underline: false,
            wide: false,
            spacer: false,
            selected: false,
        }
    }
}

pub struct Snapshot {
    pub columns: usize,
    pub lines: usize,
    /// 줄 우선(`line * columns + column`).
    pub cells: Vec<Cell>,
    /// 보이는 커서(화면 줄, 열).
    pub cursor: Option<(usize, usize)>,
    pub cursor_shape: CursorShape,
    /// 위로 스크롤해 둔 상태.
    pub scrolled: bool,
}

impl Snapshot {
    pub fn line(&self, line: usize) -> &[Cell] {
        &self.cells[line * self.columns..(line + 1) * self.columns]
    }

    /// 화면 글자(줄 끝 공백은 뺀다). 시험과 화면 읽기 프로그램용.
    pub fn text(&self) -> String {
        (0..self.lines)
            .map(|line| {
                let text: String = self
                    .line(line)
                    .iter()
                    .filter(|c| !c.spacer)
                    .map(|c| c.c)
                    .collect();
                text.trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
    }
}

/// 셸이 바꾼 색(OSC 4·10·11)이 있으면 그것을, 없으면 기본 팔레트를 쓴다.
fn resolve(colors: &term::color::Colors, color: Color) -> Paint {
    let rgb = |c: Rgb| Paint::Rgb(c.r, c.g, c.b);
    match color {
        Color::Spec(c) => rgb(c),
        Color::Indexed(i) => colors[usize::from(i)].map_or_else(|| palette(i), rgb),
        Color::Named(named) => {
            let index = match named {
                NamedColor::Foreground
                | NamedColor::BrightForeground
                | NamedColor::DimForeground => {
                    return colors[NamedColor::Foreground as usize].map_or(Paint::Foreground, rgb);
                }
                NamedColor::Background => {
                    return colors[NamedColor::Background as usize].map_or(Paint::Background, rgb);
                }
                NamedColor::Cursor => return Paint::Foreground,
                NamedColor::DimBlack => 0,
                NamedColor::DimRed => 1,
                NamedColor::DimGreen => 2,
                NamedColor::DimYellow => 3,
                NamedColor::DimBlue => 4,
                NamedColor::DimMagenta => 5,
                NamedColor::DimCyan => 6,
                NamedColor::DimWhite => 7,
                other => other as u8,
            };
            colors[usize::from(index)].map_or_else(|| palette(index), rgb)
        }
    }
}

/// xterm 256색. 0~15는 어두운·밝은 배경 모두에서 읽기 쉬운 값(VS Code 기본과 비슷하게).
fn palette(index: u8) -> Paint {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 49, 49),
        (13, 188, 121),
        (229, 229, 16),
        (36, 114, 200),
        (188, 63, 188),
        (17, 168, 205),
        (229, 229, 229),
        (102, 102, 102),
        (241, 76, 76),
        (35, 209, 139),
        (245, 245, 67),
        (59, 142, 234),
        (214, 112, 214),
        (41, 184, 219),
        (255, 255, 255),
    ];
    let (r, g, b) = match index {
        0..16 => ANSI[usize::from(index)],
        16..232 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    };
    Paint::Rgb(r, g, b)
}

/// 터미널에 보낼 수 있는 키(UI의 키에서 바꿔 넣는다).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
    Char(char),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// 키를 셸에 보낼 바이트(xterm 방식)로 바꾼다. 수식 키 없이 친 글자는 None이다(글자 입력으로 보낸다).
pub fn key_bytes(key: Key, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    let modifier = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl);
    let cursor = |end: char| {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{end}")
        } else if app_cursor {
            format!("\x1bO{end}")
        } else {
            format!("\x1b[{end}")
        }
    };
    let tilde = |n: u8| {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~")
        } else {
            format!("\x1b[{n}~")
        }
    };
    let text = match key {
        Key::Enter if mods.alt => "\x1b\r".to_owned(),
        Key::Enter => "\r".to_owned(),
        Key::Backspace if mods.ctrl => "\x08".to_owned(),
        Key::Backspace if mods.alt => "\x1b\x7f".to_owned(),
        Key::Backspace => "\x7f".to_owned(),
        Key::Tab if mods.shift => "\x1b[Z".to_owned(),
        Key::Tab => "\t".to_owned(),
        Key::Escape => "\x1b".to_owned(),
        Key::Up => cursor('A'),
        Key::Down => cursor('B'),
        Key::Right => cursor('C'),
        Key::Left => cursor('D'),
        Key::Home => cursor('H'),
        Key::End => cursor('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => {
            let end = char::from(b'P' + n - 1);
            if modifier > 1 {
                format!("\x1b[1;{modifier}{end}")
            } else {
                format!("\x1bO{end}")
            }
        }
        Key::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        Key::F(_) => return None,
        Key::Char(c) if mods.ctrl => {
            let code = match c.to_ascii_lowercase() {
                c @ 'a'..='z' => c as u8 - b'a' + 1,
                '@' | ' ' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '?' | '8' => 0x7f,
                _ => return None,
            };
            let prefix = if mods.alt { "\x1b" } else { "" };
            format!("{prefix}{}", char::from(code))
        }
        Key::Char(c) if mods.alt => format!("\x1b{c}"),
        Key::Char(_) => return None,
    };
    Some(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(key: Key, mods: Mods) -> Vec<u8> {
        key_bytes(key, mods, false).unwrap()
    }

    #[test]
    fn 키는_xterm_방식으로_보낸다() {
        let none = Mods::default();
        let ctrl = Mods { ctrl: true, ..none };
        let shift = Mods {
            shift: true,
            ..none
        };
        assert_eq!(bytes(Key::Enter, none), b"\r");
        assert_eq!(bytes(Key::Backspace, none), b"\x7f");
        assert_eq!(bytes(Key::Up, none), b"\x1b[A");
        assert_eq!(
            key_bytes(Key::Up, none, true).unwrap(),
            b"\x1bOA",
            "응용 프로그램 모드"
        );
        assert_eq!(bytes(Key::Right, ctrl), b"\x1b[1;5C");
        assert_eq!(bytes(Key::Tab, shift), b"\x1b[Z");
        assert_eq!(bytes(Key::Delete, none), b"\x1b[3~");
        assert_eq!(bytes(Key::F(1), none), b"\x1bOP");
        assert_eq!(bytes(Key::F(12), none), b"\x1b[24~");
        assert_eq!(bytes(Key::Char('c'), ctrl), [3], "Ctrl+C는 인터럽트");
        assert_eq!(bytes(Key::Char('D'), ctrl), [4]);
        assert_eq!(bytes(Key::Char('['), ctrl), [0x1b]);
        assert_eq!(bytes(Key::Char('b'), Mods { alt: true, ..none }), b"\x1bb");
        assert_eq!(
            key_bytes(Key::Char('a'), none, false),
            None,
            "글자는 글자 입력으로"
        );
    }

    #[test]
    fn 팔레트는_xterm_256색이다() {
        assert_eq!(palette(16), Paint::Rgb(0, 0, 0));
        assert_eq!(palette(21), Paint::Rgb(0, 0, 255));
        assert_eq!(palette(231), Paint::Rgb(255, 255, 255));
        assert_eq!(palette(232), Paint::Rgb(8, 8, 8));
        assert_eq!(palette(255), Paint::Rgb(238, 238, 238));
    }
}
