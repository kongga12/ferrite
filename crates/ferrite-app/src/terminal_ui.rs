//! 통합 터미널 창(Ctrl+`). 셸 탭과 실행 구성 탭을 여러 개 둔다. 화면은 ferrite-term의 스냅샷을 칸
//! 단위로 그리고, 키·글자·IME·붙여넣기를 셸로 보낸다. 앱 단축키(Ctrl+S 등)는 앱이 먼저 받는다.

use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Color32, Event, EventFilter, FontId, ImeEvent, Key, Pos2, Rect, Sense, Stroke, Vec2,
};
use ferrite_core::run::RunConfig;
use ferrite_term::{Cell, Mods, Options, Paint, Snapshot, Terminal};

/// 처음 띄울 때의 크기(그린 뒤 패널 크기에 맞춘다).
const INITIAL: (u16, u16) = (100, 20);
/// 탭 이름의 최대 글자 수.
const TITLE_CHARS: usize = 24;

struct Tab {
    terminal: Terminal,
    number: usize,
    /// 실행 구성 탭이면 그 구성 이름(탭 이름으로 쓰고, 다시 실행할 때 이 탭을 바꾼다).
    run: Option<String>,
}

#[derive(Default)]
pub struct TerminalPanel {
    /// 이번 실행에서 셸을 띄운 적이 있는지. 창을 처음 그릴 때 하나 띄운다.
    started: bool,
    tabs: Vec<Tab>,
    active: usize,
    next_number: usize,
    focus: bool,
    error: Option<String>,
    /// 끌어서 고르기 시작한 칸(줄, 열).
    drag_from: Option<(usize, usize)>,
    /// 줄 단위로 바꾸고 남은 스크롤 양.
    scroll_rest: f32,
}

impl TerminalPanel {
    #[cfg(test)]
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// 시험이 실패했을 때 남길 상태.
    #[cfg(test)]
    pub fn describe(&self) -> String {
        let exited = self.tabs.get(self.active).map(|t| t.terminal.exited());
        format!(
            "탭 {}개, 오류 {:?}, 셸 종료 {exited:?}",
            self.tabs.len(),
            self.error
        )
    }

    /// 다음에 그릴 때 화면에 키보드 포커스를 준다.
    pub fn request_focus(&mut self) {
        self.focus = true;
    }

    fn new_tab(&mut self, cwd: Option<&Path>, ctx: &egui::Context) {
        let working_directory = cwd
            .map(Path::to_path_buf)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let options = Options {
            working_directory,
            ..Default::default()
        };
        self.spawn(options, None, ctx);
    }

    /// 실행 구성을 새 탭에서 돌린다. 같은 구성의 이전 탭은 닫는다(아직 돌고 있으면 끝난다).
    pub fn run(&mut self, config: &RunConfig, root: &Path, ctx: &egui::Context) {
        self.tabs
            .retain(|t| t.run.as_deref() != Some(config.name.as_str()));
        self.drag_from = None;
        // 이 탭이 첫 탭이어도 기본 셸을 따로 띄우지 않는다.
        self.started = true;
        let options = Options {
            working_directory: config.working_directory(root),
            command: Some(config.command.clone()),
            env: config.env.clone().into_iter().collect(),
            ..Default::default()
        };
        self.spawn(options, Some(config.name.clone()), ctx);
    }

    fn spawn(&mut self, options: Options, run: Option<String>, ctx: &egui::Context) {
        let wake = ctx.clone();
        let options = Options {
            columns: INITIAL.0,
            lines: INITIAL.1,
            ..options
        };
        match Terminal::spawn(&options, move || wake.request_repaint()) {
            Ok(terminal) => {
                self.next_number += 1;
                self.tabs.push(Tab {
                    terminal,
                    number: self.next_number,
                    run,
                });
                self.active = self.tabs.len() - 1;
                self.error = None;
                self.focus = true;
            }
            Err(e) => {
                let what = run.as_deref().unwrap_or("셸");
                self.error = Some(format!("띄우지 못했습니다({what}): {e}"));
                self.active = self.active.min(self.tabs.len().saturating_sub(1));
            }
        }
    }

    /// 터미널 창을 그린다. 창을 닫아도(숨겨도) 셸은 그대로 둔다.
    pub fn ui(&mut self, ui: &mut egui::Ui, cwd: Option<&Path>) {
        let ctx = ui.ctx().clone();
        // 저장된 배치로 처음부터 열려 있어도 같다. 탭을 모두 닫은 뒤에는 "+"로 띄운다.
        if !std::mem::replace(&mut self.started, true) {
            self.new_tab(cwd, &ctx);
        }
        for tab in &mut self.tabs {
            tab.terminal.poll();
        }
        self.tabs_ui(ui, cwd, &ctx);
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if self.active < self.tabs.len() {
            self.screen_ui(ui);
        }
    }

    fn tabs_ui(&mut self, ui: &mut egui::Ui, cwd: Option<&Path>, ctx: &egui::Context) {
        let mut close = None;
        ui.horizontal(|ui| {
            for (i, tab) in self.tabs.iter().enumerate() {
                let title = tab_title(tab);
                if ui.selectable_label(i == self.active, &title).clicked() {
                    self.active = i;
                    self.focus = true;
                }
                if ui
                    .small_button("×")
                    .on_hover_text(format!("{title} 닫기(셸을 끝낸다)"))
                    .clicked()
                {
                    close = Some(i);
                }
            }
            if ui.small_button("+").on_hover_text("새 터미널").clicked() {
                self.new_tab(cwd, ctx);
            }
        });
        if let Some(i) = close {
            self.tabs.remove(i);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            self.drag_from = None;
        }
    }

    fn screen_ui(&mut self, ui: &mut egui::Ui) {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let (cell_width, cell_height) =
            ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let size = ui.available_size().max(Vec2::splat(cell_height * 2.0));
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let id = response.id;
        let tab = &mut self.tabs[self.active];
        let columns = (rect.width() / cell_width).floor().max(2.0) as u16;
        let lines = (rect.height() / cell_height).floor().max(1.0) as u16;
        tab.terminal.resize(columns, lines);

        if std::mem::take(&mut self.focus) || response.clicked() {
            response.request_focus();
        }
        let cell_at = |pos: Pos2| {
            let offset = pos - rect.min;
            (
                (offset.y / cell_height).max(0.0) as usize,
                (offset.x / cell_width).max(0.0) as usize,
            )
        };
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.drag_from = Some(cell_at(pos));
        }
        if response.dragged()
            && let (Some(from), Some(pos)) = (self.drag_from, response.interact_pointer_pos())
        {
            tab.terminal.select(from, cell_at(pos));
        }
        if response.clicked() {
            tab.terminal.clear_selection();
        }
        if response.hovered() {
            self.scroll_rest += ui.input(|i| i.smooth_scroll_delta.y);
            let lines = (self.scroll_rest / cell_height).trunc();
            if lines != 0.0 {
                self.scroll_rest -= lines * cell_height;
                tab.terminal.scroll(lines as i32);
            }
        }
        let focused = response.has_focus();
        if focused {
            send_input(ui, id, &tab.terminal);
        }

        let snapshot = tab.terminal.snapshot();
        let geometry = Geometry {
            origin: rect.min,
            cell: Vec2::new(cell_width, cell_height),
            font,
        };
        paint(ui, rect, &snapshot, &geometry, focused);
        if let Some(code) = tab.terminal.exited() {
            let code = code.map(|c| format!("(코드 {c})")).unwrap_or_default();
            let note = if tab.run.is_some() {
                format!("[실행이 끝났습니다{code}. Ctrl+F5로 다시 실행하거나 탭을 닫으세요]")
            } else {
                format!("[셸이 끝났습니다{code}. 탭을 닫거나 새 탭을 여세요]")
            };
            ui.painter().text(
                rect.left_bottom() + Vec2::new(4.0, -4.0),
                egui::Align2::LEFT_BOTTOM,
                note,
                geometry.font.clone(),
                ui.visuals().warn_fg_color,
            );
        }
        if focused && let Some((line, column)) = snapshot.cursor {
            let to_global = ui
                .ctx()
                .layer_transform_to_global(ui.layer_id())
                .unwrap_or_default();
            let cursor = geometry.cell_rect(line, column, 1);
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Terminal,
                    rect: to_global * rect,
                    cursor_rect: to_global * cursor,
                    should_interrupt_composition: false,
                });
            });
        }
        // 화면 읽기 프로그램(과 시험)이 화면 글자를 읽도록 터미널 역할과 내용을 알린다.
        let label = tab_title(tab);
        response.widget_info(|| egui::WidgetInfo {
            current_text_value: Some(snapshot.text()),
            ..egui::WidgetInfo::labeled(egui::accesskit::Role::Terminal, true, &label)
        });
    }
}

fn tab_title(tab: &Tab) -> String {
    let title = tab.run.as_deref().unwrap_or(tab.terminal.title()).trim();
    if title.is_empty() {
        format!("터미널 {}", tab.number)
    } else if title.chars().count() > TITLE_CHARS {
        format!("{}…", title.chars().take(TITLE_CHARS).collect::<String>())
    } else {
        title.to_owned()
    }
}

/// 포커스가 있을 때 키·글자를 셸로 보낸다. Tab·방향키·Esc도 터미널이 받는다.
fn send_input(ui: &egui::Ui, id: egui::Id, terminal: &Terminal) {
    let filter = EventFilter {
        tab: true,
        horizontal_arrows: true,
        vertical_arrows: true,
        escape: true,
    };
    ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
    for event in ui.input(|i| i.filtered_events(&filter)) {
        match event {
            Event::Text(text) | Event::Ime(ImeEvent::Commit(text)) if !text.is_empty() => {
                terminal.write(text.into_bytes());
            }
            Event::Paste(text) => terminal.paste(&text),
            // Windows·Linux의 Ctrl+C는 복사 이벤트로 온다: 고른 글자가 있으면 복사, 없으면 인터럽트.
            Event::Copy => match terminal.selection_text() {
                Some(text) => {
                    ui.copy_text(text);
                    terminal.clear_selection();
                }
                None => terminal.write(&b"\x03"[..]),
            },
            Event::Cut => terminal.write(&b"\x18"[..]),
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                // 복사·붙여넣기·잘라내기 키는 위의 이벤트로 따로 온다.
                if modifiers.command && matches!(key, Key::C | Key::V | Key::X) {
                    continue;
                }
                let mods = Mods {
                    ctrl: modifiers.ctrl,
                    alt: modifiers.alt,
                    shift: modifiers.shift,
                };
                if let Some(key) = terminal_key(key)
                    && let Some(bytes) = ferrite_term::key_bytes(key, mods, terminal.app_cursor())
                {
                    terminal.write(bytes);
                }
            }
            _ => {}
        }
    }
}

fn terminal_key(key: Key) -> Option<ferrite_term::Key> {
    use ferrite_term::Key as T;
    Some(match key {
        Key::Enter => T::Enter,
        Key::Backspace => T::Backspace,
        Key::Tab => T::Tab,
        Key::Escape => T::Escape,
        Key::ArrowUp => T::Up,
        Key::ArrowDown => T::Down,
        Key::ArrowLeft => T::Left,
        Key::ArrowRight => T::Right,
        Key::Home => T::Home,
        Key::End => T::End,
        Key::PageUp => T::PageUp,
        Key::PageDown => T::PageDown,
        Key::Insert => T::Insert,
        Key::Delete => T::Delete,
        Key::Space => T::Char(' '),
        Key::F1 => T::F(1),
        Key::F2 => T::F(2),
        Key::F3 => T::F(3),
        Key::F4 => T::F(4),
        Key::F5 => T::F(5),
        Key::F6 => T::F(6),
        Key::F7 => T::F(7),
        Key::F8 => T::F(8),
        Key::F9 => T::F(9),
        Key::F10 => T::F(10),
        Key::F11 => T::F(11),
        Key::F12 => T::F(12),
        // 글자 키(A, 1, [ 등)는 이름이 한 글자다.
        other => {
            let mut chars = other.symbol_or_name().chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            T::Char(c.to_ascii_lowercase())
        }
    })
}

struct Geometry {
    origin: Pos2,
    cell: Vec2,
    font: FontId,
}

impl Geometry {
    fn cell_rect(&self, line: usize, column: usize, width: usize) -> Rect {
        Rect::from_min_size(
            self.origin + Vec2::new(column as f32 * self.cell.x, line as f32 * self.cell.y),
            Vec2::new(width as f32 * self.cell.x, self.cell.y),
        )
    }
}

fn color(paint: Paint, foreground: Color32, background: Color32) -> Color32 {
    match paint {
        Paint::Foreground => foreground,
        Paint::Background => background,
        Paint::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

/// 배경, 글자, 밑줄, 커서 순으로 그린다. ASCII 글자는 같은 색끼리 이어 한 번에 그리고, 한글 같은
/// 넓은 글자는 칸 위치가 어긋나지 않게 하나씩 그린다.
fn paint(ui: &egui::Ui, rect: Rect, snapshot: &Snapshot, geometry: &Geometry, focused: bool) {
    let visuals = ui.visuals();
    let (foreground, background) = (visuals.text_color(), visuals.extreme_bg_color);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, background);
    for line in 0..snapshot.lines {
        let cells = snapshot.line(line);
        for (column, cell) in cells.iter().enumerate() {
            if cell.spacer {
                continue;
            }
            let width = if cell.wide { 2 } else { 1 };
            let fill = if cell.selected {
                visuals.selection.bg_fill
            } else {
                color(cell.bg, foreground, background)
            };
            if fill != background {
                painter.rect_filled(geometry.cell_rect(line, column, width), 0.0, fill);
            }
            if cell.underline {
                let r = geometry.cell_rect(line, column, width);
                painter.line_segment(
                    [r.left_bottom() - Vec2::Y, r.right_bottom() - Vec2::Y],
                    Stroke::new(1.0, color(cell.fg, foreground, background)),
                );
            }
        }
        let mut run = String::new();
        let mut run_start = 0;
        let mut run_color = foreground;
        let flush = |run: &mut String, start: usize, color: Color32| {
            if !run.trim().is_empty() {
                painter.text(
                    geometry.cell_rect(line, start, 1).min,
                    egui::Align2::LEFT_TOP,
                    run.as_str(),
                    geometry.font.clone(),
                    color,
                );
            }
            run.clear();
        };
        for (column, cell) in cells.iter().enumerate() {
            if cell.spacer {
                continue;
            }
            let fg = text_color(cell, foreground, background);
            if cell.c.is_ascii() && !cell.wide {
                if run.is_empty() {
                    run_start = column;
                    run_color = fg;
                } else if fg != run_color {
                    flush(&mut run, run_start, run_color);
                    run_start = column;
                    run_color = fg;
                }
                run.push(cell.c);
            } else {
                flush(&mut run, run_start, run_color);
                // 한글 글리프는 두 칸보다 좁아 왼쪽에 붙이면 글자 사이가 벌어져 보이므로 가운데에 둔다.
                let width = if cell.wide { 2 } else { 1 };
                painter.text(
                    geometry.cell_rect(line, column, width).center_top(),
                    egui::Align2::CENTER_TOP,
                    cell.c,
                    geometry.font.clone(),
                    fg,
                );
            }
        }
        flush(&mut run, run_start, run_color);
    }
    if let Some((line, column)) = snapshot.cursor {
        let wide = snapshot.line(line).get(column).is_some_and(|c| c.wide);
        let r = geometry.cell_rect(line, column, if wide { 2 } else { 1 });
        let stroke = Stroke::new(1.0, foreground);
        use ferrite_term::CursorShape;
        match snapshot.cursor_shape {
            CursorShape::Beam => {
                painter.line_segment(
                    [r.left_top(), r.left_bottom()],
                    Stroke::new(2.0, foreground),
                );
            }
            CursorShape::Underline => {
                painter.line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(2.0, foreground),
                );
            }
            _ if focused => {
                painter.rect_filled(r, 0.0, foreground.gamma_multiply(0.5));
            }
            _ => {
                painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
            }
        }
    }
}

fn text_color(cell: &Cell, foreground: Color32, background: Color32) -> Color32 {
    color(cell.fg, foreground, background)
}
