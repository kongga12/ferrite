//! 가상화 코드 에디터 위젯. 화면에 보이는 줄만 레이아웃하므로 파일 크기와 관계없이 그리는 비용이 일정하다.

use std::ops::Range;
use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, Event, EventFilter, FontId, ImeEvent, Key, Modifiers, Pos2, Rect, Sense,
    Stroke, TextFormat, Vec2,
    text::{CCursor, Galley, LayoutJob},
};
use ferrite_core::buffer::{Buffer, Indent};
use std::sync::Arc;

/// 한 줄에서 그리는 최대 문자 수.
// ponytail: 압축된 한 줄짜리 파일 대응. 가로 가상화(보이는 열만 레이아웃)가 필요해지면 교체.
const MAX_RENDERED_CHARS: usize = 10_000;
const GUTTER_PAD: f32 = 8.0;
const CURSOR_WIDTH: f32 = 2.0;

/// 줄마다 (줄 안 바이트 범위, 색) 목록.
pub type LineStyles = Vec<Vec<(Range<usize>, Color32)>>;

/// 구문 강조 제공자. 보이는 줄 범위에 대해서만 호출된다.
pub trait Styler {
    fn styles(&mut self, buffer: &Buffer, lines: Range<usize>, dark: bool) -> LineStyles;
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

pub struct EditorOptions<'a> {
    pub tab_size: usize,
    pub indent: Indent,
    pub comment_token: Option<&'a str>,
}

struct Geometry {
    /// 내용 영역의 화면 좌표 원점.
    origin: Pos2,
    gutter: f32,
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
}

pub fn show(
    ui: &mut egui::Ui,
    id: egui::Id,
    buffer: &mut Buffer,
    state: &mut EditorState,
    opts: &EditorOptions<'_>,
    styler: Option<&mut dyn Styler>,
) -> egui::Response {
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let (row_height, char_width) =
        ui.fonts_mut(|f| (f.row_height(&font_id), f.glyph_width(&font_id, ' ')));

    egui::ScrollArea::both()
        .id_salt(id)
        .auto_shrink(false)
        .show_viewport(ui, |ui, viewport| {
            let gutter = gutter_width(buffer.line_count(), char_width);
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
                response.has_focus(),
            );

            if state.scroll_to_cursor {
                state.scroll_to_cursor = false;
                let cursor = cursor_rect(ui, buffer, state, &geom);
                ui.scroll_to_rect(
                    cursor.expand2(Vec2::new(char_width * 4.0, row_height)),
                    None,
                );
            }
            if response.has_focus() {
                let cursor = cursor_rect(ui, buffer, state, &geom);
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
            response
        })
        .inner
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
    let y = (pointer.y - geom.origin.y).max(0.0);
    let line = ((y / geom.row_height) as usize).min(buffer.line_count() - 1);
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

#[expect(clippy::too_many_arguments)]
fn paint(
    ui: &egui::Ui,
    screen_view: Rect,
    buffer: &Buffer,
    state: &mut EditorState,
    geom: &Geometry,
    lines: Range<usize>,
    styles: Option<&LineStyles>,
    focused: bool,
) {
    let visuals = ui.visuals();
    let painter = ui.painter().with_clip_rect(screen_view);
    let text_painter = painter.with_clip_rect(Rect::from_min_max(
        Pos2::new(screen_view.min.x + geom.gutter, screen_view.min.y),
        screen_view.max,
    ));
    let selection = buffer.selection();
    let sel = selection.range();
    let (cursor_line, cursor_col) = buffer.pos_to_line_col(selection.head);

    for (i, line) in lines.clone().enumerate() {
        let y = geom.line_y(line);
        let row = Rect::from_min_size(
            Pos2::new(screen_view.min.x, y),
            Vec2::new(screen_view.width(), geom.row_height),
        );
        if line == cursor_line && selection.is_empty() {
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
        if sel.start <= end && sel.end > start && !sel.is_empty() {
            let x = |pos: usize| galley.pos_from_cursor(CCursor::new(pos - start)).min.x;
            let x0 = x(sel.start.max(start));
            // 줄바꿈까지 선택했으면 줄 끝 뒤를 조금 더 칠한다.
            let x1 = x(sel.end.min(end)) + if sel.end > end { geom.char_width } else { 0.0 };
            text_painter.rect_filled(
                Rect::from_x_y_ranges(
                    geom.text_x() + x0..=geom.text_x() + x1,
                    y..=y + geom.row_height,
                ),
                0.0,
                visuals.selection.bg_fill,
            );
        }
        text_painter.galley(Pos2::new(geom.text_x(), y), galley, visuals.text_color());
    }

    // 줄 번호(가로 스크롤과 무관하게 왼쪽에 고정)
    let gutter = Rect::from_min_size(
        screen_view.min,
        Vec2::new(geom.gutter, screen_view.height()),
    );
    painter.rect_filled(gutter, 0.0, visuals.panel_fill);
    for line in lines {
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
