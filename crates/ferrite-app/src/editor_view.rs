//! 가상화 코드 에디터 위젯. 화면에 보이는 줄만 레이아웃하므로 파일 크기와 관계없이 그리는 비용이 일정하다.

use std::ops::Range;
use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, Event, EventFilter, FontId, ImeEvent, Key, Modifiers, Pos2, Rect, Sense,
    Stroke, TextFormat, Vec2,
    text::{CCursor, Galley, LayoutJob},
};
use ferrite_core::buffer::{Buffer, Indent};
use ferrite_core::git::LineChange;
use ferrite_lsp::problem::{Problem, Severity};
use std::sync::Arc;

/// 한 줄에서 그리는 최대 문자 수.
// ponytail: 압축된 한 줄짜리 파일 대응. 가로 가상화(보이는 열만 레이아웃)가 필요해지면 교체.
const MAX_RENDERED_CHARS: usize = 10_000;
const GUTTER_PAD: f32 = 8.0;
/// 거터 오른쪽 git 변경 막대의 폭.
const CHANGE_BAR: f32 = 3.0;
/// 중단점 점이 들어가는 거터 왼쪽 칸의 폭(줄 높이의 배수).
const MARK_COLUMN: f32 = 1.1;
const CURSOR_WIDTH: f32 = 2.0;

/// 줄마다 (줄 안 바이트 범위, 색) 목록.
pub type LineStyles = Vec<Vec<(Range<usize>, Color32)>>;

/// 구문 강조 제공자. 보이는 줄 범위에 대해서만 호출된다.
pub trait Styler {
    fn styles(&mut self, buffer: &mut Buffer, lines: Range<usize>, dark: bool) -> LineStyles;
}

/// 거터에 그리는 중단점.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakpointMark {
    Set,
    /// 조건식이 있다.
    Conditional,
    /// 어댑터가 걸 수 없다고 했다(실행 중일 때).
    Unverified,
}

/// 문서마다 유지하는 화면 상태.
#[derive(Default)]
pub struct EditorState {
    /// IME로 조합 중인 글자. 확정되기 전까지 버퍼에 넣지 않고 화면에만 그린다.
    preedit: Option<String>,
    /// 지금까지 그린 줄 중 가장 넓은 폭. 전체 줄을 재지 않고 가로 스크롤 범위로 쓴다.
    max_width: f32,
    scroll_to_cursor: bool,
}

impl EditorState {
    /// 다음 프레임에 커서가 보이도록 스크롤한다(찾기 결과로 이동할 때 등).
    pub fn reveal_cursor(&mut self) {
        self.scroll_to_cursor = true;
    }
}

/// 언어 설정이 없을 때 짝을 강조하는 괄호.
pub const DEFAULT_BRACKETS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

pub struct EditorOptions<'a> {
    pub tab_size: usize,
    pub indent: Indent,
    pub comment_token: Option<&'a str>,
    /// 커서 옆 괄호와 그 짝을 강조할 괄호 짝.
    pub brackets: &'a [(char, char)],
    /// 찾기 결과(문자 범위, 시작 순 정렬). 배경을 칠한다.
    pub search_matches: &'a [Range<usize>],
    /// 언어 서버 진단(시작 순 정렬). 물결 밑줄과 말풍선으로 보여 준다.
    pub problems: &'a [Problem],
    /// 언어 서버 호버 정보와 그것을 요청한 위치. 마우스가 그 위치에 있으면 말풍선에 함께 보인다.
    pub hover: Option<(usize, &'a str)>,
    /// git 인덱스 대비 줄 변경(줄 순). 거터 오른쪽에 색 막대로 보인다.
    pub line_changes: &'a [(usize, LineChange)],
    /// 중단점(0부터 센 줄 순). 거터 왼쪽에 점으로 보인다.
    pub breakpoints: &'a [(usize, BreakpointMark)],
    /// 디버거가 멈춘 줄(0부터). 줄 배경과 거터 화살표로 보인다.
    pub debug_line: Option<usize>,
    /// 실행할 수 있는 줄(0부터 순, `.http`의 요청 줄). 거터 왼쪽에 초록 ▶로 보인다.
    pub run_lines: &'a [usize],
    /// 읽기 전용(디버그 어댑터에서 받은 스크립트). 고치는 입력은 버리고 이동·고르기·복사만 받는다.
    pub read_only: bool,
}

/// 앱이 언어 서버 기능을 붙이는 데 쓰는 정보.
pub struct EditorOutput {
    pub response: egui::Response,
    /// 커서의 화면 위치(자동완성 팝업을 띄울 자리).
    pub cursor_rect: Rect,
    /// 마우스가 올라가 있는 문자 위치(호버 요청용).
    pub hover_pos: Option<usize>,
    /// Ctrl(macOS는 Cmd)+클릭한 문자 위치(정의로 이동).
    pub ctrl_click: Option<usize>,
    /// 거터를 누른 줄(0부터)과, 오른쪽 단추였는지(중단점 조건 편집).
    pub gutter_click: Option<(usize, bool)>,
}

struct Geometry {
    /// 내용 영역의 화면 좌표 원점.
    origin: Pos2,
    /// 중단점 칸을 포함한 거터 전체 폭.
    gutter: f32,
    /// 거터 왼쪽 중단점 칸의 폭.
    mark: f32,
    row_height: f32,
    char_width: f32,
    font_id: FontId,
    tab_size: usize,
}

impl Geometry {
    fn text_x(&self) -> f32 {
        self.origin.x + self.gutter
    }

    fn line_y(&self, line: usize) -> f32 {
        self.origin.y + line as f32 * self.row_height
    }

    fn line_at(&self, pointer: Pos2, buffer: &Buffer) -> usize {
        let y = (pointer.y - self.origin.y).max(0.0);
        ((y / self.row_height) as usize).min(buffer.line_count() - 1)
    }
}

pub fn show(
    ui: &mut egui::Ui,
    id: egui::Id,
    buffer: &mut Buffer,
    state: &mut EditorState,
    opts: &EditorOptions<'_>,
    styler: Option<&mut dyn Styler>,
) -> EditorOutput {
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let (row_height, char_width) =
        ui.fonts_mut(|f| (f.row_height(&font_id), f.glyph_width(&font_id, ' ')));

    egui::ScrollArea::both()
        .id_salt(id)
        .auto_shrink(false)
        .show_viewport(ui, |ui, viewport| {
            let mark = row_height * MARK_COLUMN;
            let gutter = mark + gutter_width(buffer.line_count(), char_width);
            // 마지막 줄 아래로 반 화면을 더 스크롤할 수 있게 한다.
            let content = Vec2::new(
                gutter + state.max_width + char_width * 4.0,
                buffer.line_count() as f32 * row_height + viewport.height() * 0.5,
            )
            .max(viewport.size());
            let (rect, _) = ui.allocate_exact_size(content, Sense::hover());
            let response = ui
                .interact(rect, id, Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::Text);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, "")
            });

            let geom = Geometry {
                origin: rect.min,
                gutter,
                mark,
                row_height,
                char_width,
                font_id: font_id.clone(),
                tab_size: opts.tab_size,
            };
            handle_pointer(ui, &response, buffer, state, &geom);
            if response.has_focus() {
                let page = (viewport.height() / row_height) as isize - 1;
                if handle_keys(ui, id, buffer, state, opts, page.max(1)) {
                    state.scroll_to_cursor = true;
                }
            }

            let screen_view = viewport.translate(rect.min.to_vec2());
            let first = (viewport.min.y / row_height).floor().max(0.0) as usize;
            let last = ((viewport.max.y / row_height).ceil() as usize).min(buffer.line_count());
            let first = first.min(last);
            let styles = styler.map(|s| s.styles(buffer, first..last, ui.visuals().dark_mode));
            paint(
                ui,
                screen_view,
                buffer,
                state,
                &geom,
                first..last,
                styles.as_ref(),
                opts,
                response.has_focus(),
            );
            let mut response = response;
            let ctrl_click = (response.clicked() && ui.input(|i| i.modifiers.command))
                .then(|| response.interact_pointer_pos())
                .flatten()
                .map(|p| hit_test(ui, buffer, &geom, p));
            let gutter_click = (response.clicked() || response.secondary_clicked())
                .then(|| response.interact_pointer_pos())
                .flatten()
                .filter(|p| p.x < geom.text_x())
                .map(|p| (geom.line_at(p, buffer), response.secondary_clicked()));
            let mut hover_pos = None;
            if response.hovered()
                && let Some(pointer) = response.hover_pos()
            {
                let pos = hit_test(ui, buffer, &geom, pointer);
                hover_pos = Some(pos);
                let mut messages: Vec<String> = opts
                    .problems
                    .iter()
                    .filter(|p| p.range.start <= pos && pos <= p.range.end)
                    .map(|p| {
                        let source = if p.source.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", p.source)
                        };
                        format!("[{}] {}{source}", p.severity.label(), p.message)
                    })
                    .collect();
                if let Some((at, text)) = opts.hover
                    && at == pos
                {
                    messages.push(text.to_owned());
                }
                if !messages.is_empty() {
                    response = response.on_hover_text_at_pointer(messages.join("\n\n"));
                }
            }

            if state.scroll_to_cursor {
                state.scroll_to_cursor = false;
                let cursor = cursor_rect(ui, buffer, state, &geom);
                ui.scroll_to_rect(
                    cursor.expand2(Vec2::new(char_width * 4.0, row_height)),
                    None,
                );
            }
            let cursor = cursor_rect(ui, buffer, state, &geom);
            if response.has_focus() {
                let to_global = ui
                    .ctx()
                    .layer_transform_to_global(ui.layer_id())
                    .unwrap_or_default();
                ui.output_mut(|o| {
                    o.ime = Some(egui::output::IMEOutput {
                        purpose: egui::IMEPurpose::Normal,
                        rect: to_global * screen_view,
                        cursor_rect: to_global * cursor,
                        should_interrupt_composition: false,
                    });
                });
            }
            EditorOutput {
                response,
                cursor_rect: cursor,
                hover_pos,
                ctrl_click,
                gutter_click,
            }
        })
        .inner
}

const BREAKPOINT_RED: Color32 = Color32::from_rgb(220, 60, 60);
const RUN_GREEN: Color32 = Color32::from_rgb(60, 170, 90);

/// 디버거가 멈춘 줄의 배경(노랑, 글자가 읽히게 옅게).
fn debug_line_color(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgba_unmultiplied(200, 160, 0, 60)
    } else {
        Color32::from_rgba_unmultiplied(255, 210, 0, 90)
    }
}

fn gutter_width(line_count: usize, char_width: f32) -> f32 {
    let digits = line_count.max(1).ilog10() + 1;
    digits as f32 * char_width + GUTTER_PAD * 2.0
}

/// 줄 하나를 레이아웃한다. 탭은 다음 탭 위치까지의 폭을 "다음 구간의 앞 여백"으로 만들어,
/// 문자 인덱스와 화면 글자 인덱스가 1:1로 유지되게 한다.
fn layout_line(
    ui: &egui::Ui,
    text: &str,
    spans: &[(Range<usize>, Color32)],
    preedit: Option<(usize, &str)>,
    geom: &Geometry,
) -> Arc<Galley> {
    let default = ui.visuals().text_color();
    let format = |color: Color32| TextFormat {
        font_id: geom.font_id.clone(),
        color,
        ..Default::default()
    };
    let mut job = JobBuilder::default();
    let mut run = String::new();
    let mut run_color = default;
    let mut display_col = 0;
    let mut spans = spans.iter().peekable();
    let mut chars = 0;
    for (col, (byte, ch)) in text.char_indices().enumerate() {
        if let Some((at, pre)) = preedit
            && at == col
        {
            job.push(&run, format(run_color));
            run.clear();
            let mut underlined = format(default);
            underlined.underline = Stroke::new(1.0, default);
            job.push(pre, underlined);
        }
        while spans.peek().is_some_and(|(r, _)| r.end <= byte) {
            spans.next();
        }
        let color = spans
            .peek()
            .filter(|(r, _)| r.start <= byte)
            .map_or(default, |(_, c)| *c);
        if ch == '\t' {
            job.push(&run, format(run_color));
            run.clear();
            let width = geom.tab_size - display_col % geom.tab_size;
            job.push(" ", format(color));
            job.pending_space = (width - 1) as f32 * geom.char_width;
            display_col += width;
        } else {
            if color != run_color {
                job.push(&run, format(run_color));
                run.clear();
                run_color = color;
            }
            run.push(ch);
            display_col += 1;
        }
        chars = col + 1;
    }
    job.push(&run, format(run_color));
    if let Some((at, pre)) = preedit
        && at >= chars
    {
        let mut underlined = format(default);
        underlined.underline = Stroke::new(1.0, default);
        job.push(pre, underlined);
    }
    ui.fonts_mut(|f| f.layout_job(job.job))
}

#[derive(Default)]
struct JobBuilder {
    job: LayoutJob,
    /// 직전 탭의 남은 폭. 다음 구간의 앞 여백으로 붙인다.
    pending_space: f32,
}

impl JobBuilder {
    fn push(&mut self, text: &str, format: TextFormat) {
        if !text.is_empty() {
            self.job.append(text, self.pending_space, format);
            self.pending_space = 0.0;
        }
    }
}

/// 그릴 줄 내용(너무 긴 줄은 앞부분만).
fn rendered_text(buffer: &Buffer, line: usize) -> String {
    let start = buffer.line_start(line);
    let len = buffer.line_len(line).min(MAX_RENDERED_CHARS);
    buffer.rope().slice(start..start + len).to_string()
}

fn handle_pointer(
    ui: &egui::Ui,
    response: &egui::Response,
    buffer: &mut Buffer,
    state: &mut EditorState,
    geom: &Geometry,
) {
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let Some(pointer) = response.interact_pointer_pos() else {
        return;
    };
    // 거터에서 누른 것은 중단점용이다. 커서를 옮기거나 단어를 고르지 않는다.
    let pressed_in_gutter = ui
        .input(|i| i.pointer.press_origin())
        .is_some_and(|p| p.x < geom.text_x());
    if pressed_in_gutter || (response.double_clicked() && pointer.x < geom.text_x()) {
        return;
    }
    let pos = hit_test(ui, buffer, geom, pointer);
    if response.double_clicked() {
        buffer.select_word_at(pos);
    } else if response.is_pointer_button_down_on() {
        let (pressed, shift) = ui.input(|i| (i.pointer.any_pressed(), i.modifiers.shift));
        buffer.move_to(pos, !pressed || shift);
        // 드래그로 화면 밖까지 선택하면 따라 스크롤한다.
        state.scroll_to_cursor = !pressed;
    }
}

fn hit_test(ui: &egui::Ui, buffer: &Buffer, geom: &Geometry, pointer: Pos2) -> usize {
    let line = geom.line_at(pointer, buffer);
    let galley = layout_line(ui, &rendered_text(buffer, line), &[], None, geom);
    let x = (pointer.x - geom.text_x()).max(0.0);
    let col = galley
        .cursor_from_pos(Vec2::new(x, geom.row_height / 2.0))
        .index
        .0
        .min(buffer.line_len(line));
    buffer.line_start(line) + col
}

fn handle_keys(
    ui: &egui::Ui,
    id: egui::Id,
    buffer: &mut Buffer,
    state: &mut EditorState,
    opts: &EditorOptions<'_>,
    page: isize,
) -> bool {
    let filter = EventFilter {
        tab: true,
        horizontal_arrows: true,
        vertical_arrows: true,
        escape: false,
    };
    ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
    let events = ui.input(|i| i.filtered_events(&filter));
    let now = Instant::now();
    let mut acted = false;
    for event in events {
        let edits = match &event {
            Event::Text(_) | Event::Paste(_) | Event::Cut | Event::Ime(_) => true,
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => edits_key(*key, *modifiers),
            _ => false,
        };
        if opts.read_only && edits {
            continue;
        }
        acted |= match event {
            Event::Text(text) => {
                buffer.insert(&text, now);
                true
            }
            Event::Paste(text) => {
                buffer.insert(&text.replace("\r\n", "\n"), now);
                true
            }
            Event::Copy => {
                if !buffer.selection().is_empty() {
                    ui.copy_text(buffer.selected_text());
                }
                false
            }
            Event::Cut => {
                let text = buffer.cut(now);
                if !text.is_empty() {
                    ui.copy_text(text);
                }
                true
            }
            Event::Ime(ImeEvent::Preedit { text, .. }) => {
                state.preedit = (!text.is_empty()).then_some(text);
                true
            }
            Event::Ime(ImeEvent::Commit(text)) => {
                state.preedit = None;
                if !text.is_empty() {
                    buffer.insert(&text, now);
                }
                true
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => handle_key(buffer, key, modifiers, opts, page, now),
            _ => false,
        };
    }
    acted
}

/// 버퍼를 고치는 키인지(읽기 전용이면 버린다). [`handle_key`]의 고치는 키와 맞춘다.
fn edits_key(key: Key, m: Modifiers) -> bool {
    matches!(key, Key::Backspace | Key::Delete | Key::Enter | Key::Tab)
        || (m.command && matches!(key, Key::Z | Key::Y | Key::Slash))
        || (m.alt && matches!(key, Key::ArrowUp | Key::ArrowDown))
}

/// 키 하나를 처리했으면 true. macOS는 단어 이동이 Option, 줄 처음·끝이 Cmd다.
fn handle_key(
    buffer: &mut Buffer,
    key: Key,
    m: Modifiers,
    opts: &EditorOptions<'_>,
    page: isize,
    now: Instant,
) -> bool {
    let word = if cfg!(target_os = "macos") {
        m.alt
    } else {
        m.ctrl
    };
    let shift = m.shift;
    match key {
        Key::ArrowLeft if m.mac_cmd => buffer.move_line_start(shift),
        Key::ArrowRight if m.mac_cmd => buffer.move_line_end(shift),
        Key::ArrowLeft if word => buffer.move_word(false, shift),
        Key::ArrowRight if word => buffer.move_word(true, shift),
        Key::ArrowLeft => buffer.move_left(shift),
        Key::ArrowRight => buffer.move_right(shift),
        Key::ArrowUp if m.alt => buffer.move_lines(true, now),
        Key::ArrowDown if m.alt => buffer.move_lines(false, now),
        Key::ArrowUp if m.mac_cmd => buffer.move_to(0, shift),
        Key::ArrowDown if m.mac_cmd => buffer.move_to(buffer.len_chars(), shift),
        Key::ArrowUp => buffer.move_vertical(-1, shift),
        Key::ArrowDown => buffer.move_vertical(1, shift),
        Key::PageUp => buffer.move_vertical(-page, shift),
        Key::PageDown => buffer.move_vertical(page, shift),
        Key::Home if m.command => buffer.move_to(0, shift),
        Key::End if m.command => buffer.move_to(buffer.len_chars(), shift),
        Key::Home => buffer.move_line_start(shift),
        Key::End => buffer.move_line_end(shift),
        Key::Backspace if word => buffer.delete_word_back(now),
        Key::Backspace => buffer.backspace(now),
        Key::Delete => buffer.delete_forward(now),
        Key::Enter => buffer.newline(now),
        Key::Tab if shift => buffer.dedent(opts.tab_size, now),
        Key::Tab => buffer.indent(&opts.indent.unit(), now),
        Key::A if m.command => buffer.select_all(),
        Key::Z if m.command && shift => {
            buffer.redo();
        }
        Key::Z if m.command => {
            buffer.undo();
        }
        Key::Y if m.command => {
            buffer.redo();
        }
        Key::Slash if m.command => {
            if let Some(token) = opts.comment_token {
                buffer.toggle_line_comment(token, now);
            }
        }
        Key::Escape => {
            let head = buffer.selection().head;
            buffer.move_to(head, false);
        }
        _ => return false,
    }
    true
}

/// 커서의 화면 좌표 사각형(얇은 세로 막대).
fn cursor_rect(ui: &egui::Ui, buffer: &Buffer, state: &EditorState, geom: &Geometry) -> Rect {
    let (line, col) = buffer.pos_to_line_col(buffer.selection().head);
    let preedit = state.preedit.as_deref().map(|p| (col, p));
    let galley = layout_line(ui, &rendered_text(buffer, line), &[], preedit, geom);
    let preedit_len = state.preedit.as_ref().map_or(0, |p| p.chars().count());
    let x = galley
        .pos_from_cursor(CCursor::new(col + preedit_len))
        .min
        .x;
    Rect::from_min_size(
        Pos2::new(geom.text_x() + x, geom.line_y(line)),
        Vec2::new(CURSOR_WIDTH, geom.row_height),
    )
}

pub fn severity_color(visuals: &egui::Visuals, severity: Severity) -> Color32 {
    match severity {
        Severity::Error => visuals.error_fg_color,
        Severity::Warning => visuals.warn_fg_color,
        Severity::Info => Color32::from_rgb(0x37, 0x94, 0xFF),
        Severity::Hint => visuals.weak_text_color(),
    }
}

/// 진단 위치 밑의 물결선.
fn squiggle(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    const AMPLITUDE: f32 = 1.5;
    const STEP: f32 = 2.0;
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        points.push(Pos2::new(x, y + if up { -AMPLITUDE } else { AMPLITUDE }));
        x += STEP;
        up = !up;
    }
    if points.len() >= 2 {
        painter.add(egui::Shape::line(points, Stroke::new(1.0, color)));
    }
}

#[expect(clippy::too_many_arguments)]
fn paint(
    ui: &egui::Ui,
    screen_view: Rect,
    buffer: &Buffer,
    state: &mut EditorState,
    geom: &Geometry,
    lines: Range<usize>,
    styles: Option<&LineStyles>,
    opts: &EditorOptions<'_>,
    focused: bool,
) {
    let visuals = ui.visuals();
    let match_fill = Color32::from_rgba_unmultiplied(230, 180, 0, 70);
    let matches = opts.search_matches;
    // 진단은 여러 줄에 걸칠 수 있어 끝 위치로 이분 탐색할 수 없다. 보이는 범위에 걸친 것만 미리 고른다.
    let visible_end = if lines.end >= buffer.line_count() {
        buffer.len_chars()
    } else {
        buffer.line_start(lines.end)
    };
    let visible = buffer.line_start(lines.start)..visible_end;
    let problems: Vec<&Problem> = opts
        .problems
        .iter()
        .filter(|p| p.range.start <= visible.end && p.range.end >= visible.start)
        .collect();
    let painter = ui.painter().with_clip_rect(screen_view);
    let text_painter = painter.with_clip_rect(Rect::from_min_max(
        Pos2::new(screen_view.min.x + geom.gutter, screen_view.min.y),
        screen_view.max,
    ));
    let selection = buffer.selection();
    let sel = selection.range();
    let (cursor_line, cursor_col) = buffer.pos_to_line_col(selection.head);
    let bracket_fill = Color32::from_rgba_unmultiplied(128, 128, 128, 80);
    let brackets: Vec<Range<usize>> = buffer
        .bracket_pair(opts.brackets)
        .map(|(a, b)| vec![a..a + 1, b..b + 1])
        .unwrap_or_default();

    for (i, line) in lines.clone().enumerate() {
        let y = geom.line_y(line);
        let row = Rect::from_min_size(
            Pos2::new(screen_view.min.x, y),
            Vec2::new(screen_view.width(), geom.row_height),
        );
        if opts.debug_line == Some(line) {
            painter.rect_filled(row, 0.0, debug_line_color(visuals.dark_mode));
        } else if line == cursor_line && selection.is_empty() {
            painter.rect_filled(row, 0.0, visuals.faint_bg_color);
        }

        let text = rendered_text(buffer, line);
        let spans = styles.and_then(|s| s.get(i)).map_or(&[][..], Vec::as_slice);
        let preedit = (line == cursor_line)
            .then_some(state.preedit.as_deref())
            .flatten()
            .map(|p| (cursor_col, p));
        let galley = layout_line(ui, &text, spans, preedit, geom);
        state.max_width = state.max_width.max(galley.size().x);

        let start = buffer.line_start(line);
        let end = start + buffer.line_len(line);
        let fill = |range: &Range<usize>, color: Color32| {
            if range.is_empty() || range.start > end || range.end <= start {
                return;
            }
            let x = |pos: usize| galley.pos_from_cursor(CCursor::new(pos - start)).min.x;
            let x0 = x(range.start.max(start));
            // 줄바꿈까지 걸쳐 있으면 줄 끝 뒤를 조금 더 칠한다.
            let tail = if range.end > end {
                geom.char_width
            } else {
                0.0
            };
            let x1 = x(range.end.min(end)) + tail;
            text_painter.rect_filled(
                Rect::from_x_y_ranges(
                    geom.text_x() + x0..=geom.text_x() + x1,
                    y..=y + geom.row_height,
                ),
                0.0,
                color,
            );
        };
        let first_match = matches.partition_point(|m| m.end <= start);
        for m in matches[first_match..].iter().take_while(|m| m.start <= end) {
            fill(m, match_fill);
        }
        for b in &brackets {
            fill(b, bracket_fill);
        }
        fill(&sel, visuals.selection.bg_fill);
        let x = |pos: usize| {
            galley
                .pos_from_cursor(CCursor::new(pos.clamp(start, end) - start))
                .min
                .x
        };
        let squiggles: Vec<(f32, f32, Color32)> = problems
            .iter()
            .filter(|p| p.range.start <= end && p.range.end >= start)
            .map(|p| {
                let x0 = x(p.range.start);
                // 빈 범위(위치 하나)도 보이도록 최소 한 글자 폭을 칠한다.
                let x1 = x(p.range.end).max(x0 + geom.char_width);
                (x0, x1, severity_color(visuals, p.severity))
            })
            .collect();
        text_painter.galley(Pos2::new(geom.text_x(), y), galley, visuals.text_color());
        for (x0, x1, color) in squiggles {
            squiggle(
                &text_painter,
                geom.text_x() + x0,
                geom.text_x() + x1,
                y + geom.row_height - 2.0,
                color,
            );
        }
    }

    // 줄 번호(가로 스크롤과 무관하게 왼쪽에 고정)
    let gutter = Rect::from_min_size(
        screen_view.min,
        Vec2::new(geom.gutter, screen_view.height()),
    );
    painter.rect_filled(gutter, 0.0, visuals.panel_fill);
    for line in lines {
        if let Ok(i) = opts
            .line_changes
            .binary_search_by_key(&line, |(changed, _)| *changed)
        {
            let y = geom.line_y(line);
            let (color, rect) = match opts.line_changes[i].1 {
                LineChange::Added => (
                    Color32::from_rgb(70, 170, 90),
                    Rect::from_min_max(
                        Pos2::new(gutter.max.x - CHANGE_BAR, y),
                        Pos2::new(gutter.max.x, y + geom.row_height),
                    ),
                ),
                LineChange::Modified => (
                    Color32::from_rgb(70, 130, 210),
                    Rect::from_min_max(
                        Pos2::new(gutter.max.x - CHANGE_BAR, y),
                        Pos2::new(gutter.max.x, y + geom.row_height),
                    ),
                ),
                // 지운 자리는 줄 경계에 짧은 막대로 보인다.
                LineChange::DeletedAbove => (
                    Color32::from_rgb(210, 80, 80),
                    Rect::from_min_max(
                        Pos2::new(gutter.max.x - CHANGE_BAR * 2.0, y - 1.5),
                        Pos2::new(gutter.max.x, y + 1.5),
                    ),
                ),
            };
            painter.rect_filled(rect, 0.0, color);
        }
        let center = Pos2::new(
            gutter.min.x + geom.mark / 2.0,
            geom.line_y(line) + geom.row_height / 2.0,
        );
        let radius = geom.row_height * 0.3;
        if opts.run_lines.binary_search(&line).is_ok() {
            let r = radius * 1.2;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + Vec2::new(-r * 0.7, -r),
                    center + Vec2::new(r, 0.0),
                    center + Vec2::new(-r * 0.7, r),
                ],
                RUN_GREEN,
                Stroke::NONE,
            ));
        }
        if let Ok(i) = opts
            .breakpoints
            .binary_search_by_key(&line, |(marked, _)| *marked)
        {
            match opts.breakpoints[i].1 {
                BreakpointMark::Set => {
                    painter.circle_filled(center, radius, BREAKPOINT_RED);
                }
                BreakpointMark::Conditional => {
                    painter.circle_filled(center, radius, BREAKPOINT_RED);
                    // 조건이 있다는 표시: 가운데 흰 가로줄 두 개(=)
                    for dy in [-radius * 0.3, radius * 0.3] {
                        painter.hline(
                            (center.x - radius * 0.5)..=(center.x + radius * 0.5),
                            center.y + dy,
                            Stroke::new(1.5, Color32::WHITE),
                        );
                    }
                }
                BreakpointMark::Unverified => {
                    painter.circle_stroke(
                        center,
                        radius,
                        Stroke::new(1.5, visuals.weak_text_color()),
                    );
                }
            }
        }
        if opts.debug_line == Some(line) {
            let r = radius * 1.1;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + Vec2::new(-r * 0.8, -r),
                    center + Vec2::new(r, 0.0),
                    center + Vec2::new(-r * 0.8, r),
                ],
                Color32::from_rgb(240, 190, 30),
                Stroke::new(1.0, Color32::from_rgb(150, 110, 0)),
            ));
        }
        let color = if line == cursor_line {
            visuals.strong_text_color()
        } else {
            visuals.weak_text_color()
        };
        painter.text(
            Pos2::new(gutter.max.x - GUTTER_PAD, geom.line_y(line)),
            Align2::RIGHT_TOP,
            (line + 1).to_string(),
            geom.font_id.clone(),
            color,
        );
    }

    if focused {
        let cursor = cursor_rect(ui, buffer, state, geom);
        text_painter.rect_filled(cursor, 0.0, visuals.text_cursor.stroke.color);
    }
}
