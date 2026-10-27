//! SQL 콘솔(도구 > SQL 콘솔, DB 연결 창의 [SQL 콘솔]). 연결을 골라 SQL을 실행하고 결과를 그리드로 본다.
//! 고른 글이 있으면 그 안의 문장을 차례로, 없으면 커서가 있는 문장 하나를 실행한다(Ctrl+Enter).
//! 그리드는 보이는 행만 그리고, 끝에 가까워지면 다음 페이지를 받는다. 받은 행이 상한을 넘으면 앞쪽을
//! 버린다(`ferrite_db::result`).

use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use eframe::egui;
use ferrite_db::export::export_csv;
use ferrite_db::result::{ROW_LIMIT, ResultBuffer};
use ferrite_db::{Cell, Connection, ConnectionConfig, Event, split};

use crate::db_ui::DbWindow;

/// 칸 너비 상한(글자 수). 긴 값은 줄이고, 마우스를 올리면 다 보인다.
const MAX_CELL_CHARS: usize = 40;
const MAX_LOG: usize = 200;

struct Live {
    config: ConnectionConfig,
    connection: Connection,
    /// 연결되면 서버 버전.
    version: Option<String>,
}

struct Export {
    stop: Arc<AtomicBool>,
    written: Arc<AtomicU64>,
    done: Receiver<Result<u64, String>>,
    path: PathBuf,
}

pub struct SqlConsole {
    live: Option<Live>,
    /// 연결하지 못했거나 끊긴 이유.
    error: Option<String>,
    pub(crate) text: String,
    /// 실행을 기다리는 문장.
    queue: VecDeque<String>,
    /// 보냈고 아직 끝나지 않은 문장.
    running: Option<String>,
    /// 실행 중인 문장의 결과를 페이지로 읽는 중(다음 문장을 실행하면 남은 행을 닫는다).
    reading: bool,
    autocommit: bool,
    /// 보낸 트랜잭션 명령(커밋·롤백·자동 커밋). 결과가 온 순서대로 기록에 남긴다.
    pending_tx: VecDeque<&'static str>,
    result: Option<ResultBuffer>,
    /// 결과를 낸 마지막 조회. CSV로 내보낼 때 다시 실행한다.
    last_query: Option<String>,
    widths: Vec<f32>,
    /// 앞쪽 행을 버려 그리드가 그만큼 당겨졌다. 다음 그림에서 스크롤 위치를 같은 행으로 되돌린다.
    scroll_fix: usize,
    scroll_offset: f32,
    /// (오류인지, 글)
    log: VecDeque<(bool, String)>,
    export: Option<Export>,
}

impl Default for SqlConsole {
    fn default() -> Self {
        Self {
            live: None,
            error: None,
            text: String::new(),
            queue: VecDeque::new(),
            running: None,
            reading: false,
            autocommit: true,
            pending_tx: VecDeque::new(),
            result: None,
            last_query: None,
            widths: Vec::new(),
            scroll_fix: 0,
            scroll_offset: 0.0,
            log: VecDeque::new(),
            export: None,
        }
    }
}

impl SqlConsole {
    /// 연결을 고른다. 이미 그 연결이면 그대로 둔다. 비밀번호는 키체인에서 꺼낸다.
    pub fn connect(&mut self, config: &ConnectionConfig, db: &DbWindow, ctx: &egui::Context) {
        if self.live.as_ref().is_some_and(|l| l.config.id == config.id) {
            return;
        }
        // 앞 연결을 버리면 작업 스레드가 닫는다.
        *self = Self {
            text: std::mem::take(&mut self.text),
            log: std::mem::take(&mut self.log),
            export: self.export.take(),
            ..Self::default()
        };
        match db.password(&config.id) {
            Ok(password) => {
                let wake = ctx.clone();
                self.live = Some(Live {
                    config: config.clone(),
                    connection: Connection::open(config, password, move || wake.request_repaint()),
                    version: None,
                });
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// 시험이 실패할 때 보일 기록.
    #[cfg(test)]
    pub fn describe(&self) -> String {
        let log: Vec<&str> = self.log.iter().map(|(_, text)| text.as_str()).collect();
        format!("오류: {:?}\n{}", self.error, log.join("\n"))
    }

    /// 문장이 실행 중이다(결과를 읽는 중이면 아니다: 다음 문장을 실행하면 남은 행을 닫는다).
    fn executing(&self) -> bool {
        self.running.is_some() && !self.reading
    }

    fn note(&mut self, error: bool, text: impl Into<String>) {
        if self.log.len() == MAX_LOG {
            self.log.pop_front();
        }
        self.log.push_back((error, text.into()));
    }

    /// 실행할 글의 바이트 범위: 고른 글, 없으면 커서가 있는 문장(커서가 없으면 마지막 문장).
    fn target(&self, ctx: &egui::Context, editor: egui::Id) -> Option<Range<usize>> {
        let backslash = self
            .live
            .as_ref()
            .is_some_and(|l| l.config.kind.backslash_escapes());
        let text = &self.text;
        let byte = |c: egui::text::CCursor| {
            text.char_indices()
                .nth(c.index.0)
                .map_or(text.len(), |(b, _)| b)
        };
        let cursor = egui::TextEdit::load_state(ctx, editor).and_then(|s| s.cursor.char_range());
        match cursor {
            Some(range) if range.primary.index != range.secondary.index => {
                let [start, end] = range.sorted_cursors();
                Some(byte(start)..byte(end))
            }
            cursor => split::statement_at(
                text,
                cursor.map_or(text.len(), |r| byte(r.primary)),
                backslash,
            ),
        }
    }

    /// 고른 글의 문장들, 없으면 커서가 있는 문장을 실행한다. `explain`이면 첫 문장의 실행 계획을 본다.
    fn run(&mut self, ctx: &egui::Context, editor: egui::Id, explain: bool) {
        let Some(kind) = self.live.as_ref().map(|l| l.config.kind) else {
            self.note(true, "연결을 고르세요");
            return;
        };
        if self.executing() {
            return;
        }
        let Some(range) = self.target(ctx, editor) else {
            self.note(true, "실행할 문장이 없습니다");
            return;
        };
        let text = &self.text[range];
        let mut statements: Vec<String> = split::statements(text, kind.backslash_escapes())
            .into_iter()
            .map(|r| text[r].to_owned())
            .collect();
        if explain {
            statements.truncate(1);
            statements = statements.iter().map(|s| kind.explain(s)).collect();
        }
        if statements.is_empty() {
            self.note(true, "실행할 문장이 없습니다");
            return;
        }
        if self.reading
            && let Some(live) = &self.live
        {
            live.connection.close_result();
        }
        self.queue = statements.into();
        self.next();
    }

    /// 앞 문장이 끝났으면 다음 문장을 보낸다.
    fn next(&mut self) {
        if self.running.is_some() {
            return;
        }
        let (Some(live), Some(sql)) = (&self.live, self.queue.pop_front()) else {
            return;
        };
        live.connection.execute(&sql);
        self.result = None;
        self.widths.clear();
        self.running = Some(sql);
    }

    fn transaction(&mut self, done: &'static str, send: impl FnOnce(&Connection)) {
        if let Some(live) = &self.live {
            send(&live.connection);
            self.pending_tx.push_back(done);
        }
    }

    fn poll(&mut self, ui: &egui::Ui) {
        self.poll_export();
        let Some(live) = &self.live else {
            return;
        };
        for event in live.connection.poll() {
            match event {
                Event::Connected { version } => {
                    if let Some(live) = &mut self.live {
                        live.version = Some(version);
                    }
                }
                Event::Disconnected(e) => {
                    self.live = None;
                    self.running = None;
                    self.queue.clear();
                    self.error = Some(e);
                    return;
                }
                Event::Columns(columns) => {
                    self.result = Some(ResultBuffer::new(columns, ROW_LIMIT));
                    self.last_query = self.running.clone();
                }
                Event::Rows { rows, more } => {
                    self.reading = more;
                    if self.widths.is_empty() {
                        self.measure(ui, &rows);
                    }
                    if let Some(buffer) = &mut self.result {
                        let before = buffer.dropped();
                        buffer.push(rows, more);
                        self.scroll_fix += buffer.dropped() - before;
                    }
                }
                Event::Done { affected, elapsed } => {
                    let elapsed = format!("{} ms", elapsed.as_millis());
                    let text = match (affected, &mut self.result) {
                        (Some(n), _) => format!("{n}행 바뀜 · {elapsed}"),
                        (None, Some(buffer)) => {
                            // 결과가 닫혔다(다음 문장을 실행했거나 끝까지 받았다). 남은 행은 받지 않는다.
                            buffer.push(Vec::new(), false);
                            format!("{}행 받음", buffer.received())
                        }
                        (None, None) => format!("실행함 · {elapsed}"),
                    };
                    let sql = self.running.take().unwrap_or_default();
                    self.reading = false;
                    self.note(false, format!("{} — {text}", first_line(&sql)));
                }
                Event::Error(e) => {
                    if let Some(sql) = self.running.take() {
                        self.reading = false;
                        self.queue.clear();
                        self.note(true, format!("{} — {e}", first_line(&sql)));
                    } else {
                        let what = self.pending_tx.pop_front().unwrap_or("트랜잭션");
                        self.note(true, format!("{what} 실패: {e}"));
                    }
                }
                Event::Transaction { autocommit } => {
                    self.autocommit = autocommit;
                    let done = self
                        .pending_tx
                        .pop_front()
                        .unwrap_or("트랜잭션을 바꿨습니다");
                    self.note(false, done);
                }
            }
        }
        self.next();
    }

    /// 첫 페이지로 칸 너비를 정한다(한글처럼 넓은 글자는 두 칸으로 센다).
    fn measure(&mut self, ui: &egui::Ui, rows: &[ferrite_db::Row]) {
        let Some(buffer) = &self.result else {
            return;
        };
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let glyph = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let chars = |text: &str| {
            text.chars()
                .map(|c| if c.is_ascii() { 1 } else { 2 })
                .sum::<usize>()
        };
        self.widths = buffer
            .columns
            .iter()
            .enumerate()
            .map(|(i, column)| {
                let widest = rows
                    .iter()
                    .filter_map(|row| row.get(i))
                    .map(|cell| chars(&cell.to_string()))
                    .chain([chars(&column.name), 4])
                    .max()
                    .unwrap_or(4)
                    .min(MAX_CELL_CHARS);
                widest as f32 * glyph + 12.0
            })
            .collect();
    }

    /// 조회를 따로 연 연결에서 다시 실행해 `path`에 CSV로 쓴다.
    pub(crate) fn start_export(&mut self, path: PathBuf, db: &DbWindow, ctx: &egui::Context) {
        let (Some(live), Some(sql)) = (&self.live, &self.last_query) else {
            return;
        };
        let (config, sql) = (live.config.clone(), sql.clone());
        let password = match db.password(&config.id) {
            Ok(password) => password,
            Err(e) => {
                self.note(true, e);
                return;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let written = Arc::new(AtomicU64::new(0));
        let (tx, done) = channel();
        let (thread_stop, thread_written, wake, target) =
            (stop.clone(), written.clone(), ctx.clone(), path.clone());
        std::thread::spawn(move || {
            let result = export_csv(&config, password, &sql, &target, &thread_stop, &|n| {
                thread_written.store(n, Ordering::Relaxed);
                wake.request_repaint();
            });
            let _ = tx.send(result);
            wake.request_repaint();
        });
        self.export = Some(Export {
            stop,
            written,
            done,
            path,
        });
    }

    fn poll_export(&mut self) {
        let Some(export) = &self.export else {
            return;
        };
        let result = match export.done.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("내보내기 스레드가 끝났습니다".into()),
        };
        let path = export.path.display().to_string();
        self.export = None;
        match result {
            Ok(n) => self.note(false, format!("CSV로 저장했습니다: {path} ({n}행)")),
            Err(e) => self.note(true, format!("CSV로 저장하지 못했습니다: {e}")),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, db: &DbWindow) {
        self.poll(ui);
        let ctx = ui.ctx().clone();
        let editor = ui.make_persistent_id("sql_console_editor");
        let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter);
        // 에디터가 Enter를 받기 전에 가로챈다.
        if ui.memory(|m| m.has_focus(editor)) && ui.input_mut(|i| i.consume_shortcut(&shortcut)) {
            self.run(&ctx, editor, false);
        }
        self.toolbar(ui, db, editor, &ctx, &ctx.format_shortcut(&shortcut));
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        egui::ScrollArea::vertical()
            .id_salt("sql_editor")
            .max_height(ui.available_height() * 0.35)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.text)
                        .id(editor)
                        .code_editor()
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .hint_text(format!(
                            "SQL을 적고 {}로 실행합니다",
                            ctx.format_shortcut(&shortcut)
                        )),
                );
            });
        self.log_ui(ui);
        ui.separator();
        self.grid(ui);
    }

    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        db: &DbWindow,
        editor: egui::Id,
        ctx: &egui::Context,
        shortcut: &str,
    ) {
        let connected = self.live.as_ref().is_some_and(|l| l.version.is_some());
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            let label = ui.label("연결");
            let current = self
                .live
                .as_ref()
                .map_or("연결 고르기", |l| l.config.name.as_str());
            ui.add_enabled_ui(self.autocommit, |ui| {
                egui::ComboBox::from_id_salt("sql_connection")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for config in db.connections() {
                            let selected =
                                self.live.as_ref().is_some_and(|l| l.config.id == config.id);
                            if ui.selectable_label(selected, &config.name).clicked() {
                                chosen = Some(config.clone());
                            }
                        }
                        if db.connections().is_empty() {
                            ui.weak("도구 > DB 연결…에서 연결을 추가하세요");
                        }
                    })
                    .response
                    .labelled_by(label.id)
                    .on_disabled_hover_text("자동 커밋을 켜거나 커밋·롤백한 뒤 바꾸세요");
            });
            match &self.live {
                Some(Live { version: None, .. }) => {
                    ui.spinner();
                    ui.label("연결하는 중…");
                }
                Some(Live {
                    version: Some(version),
                    ..
                }) => {
                    ui.weak(version);
                }
                None => {}
            }
            ui.separator();
            let can_run = connected && !self.executing();
            if ui
                .add_enabled(can_run, egui::Button::new("실행"))
                .on_hover_text(format!("고른 글, 없으면 커서가 있는 문장 ({shortcut})"))
                .clicked()
            {
                self.run(ctx, editor, false);
            }
            if ui
                .add_enabled(can_run, egui::Button::new("실행 계획"))
                .clicked()
            {
                self.run(ctx, editor, true);
            }
            if ui
                .add_enabled(self.running.is_some(), egui::Button::new("멈추기"))
                .clicked()
                && let Some(live) = &self.live
            {
                self.queue.clear();
                live.connection.cancel();
            }
            ui.separator();
            let mut autocommit = self.autocommit;
            if ui
                .add_enabled(can_run, egui::Checkbox::new(&mut autocommit, "자동 커밋"))
                .changed()
            {
                let done = if autocommit {
                    "자동 커밋을 켰습니다"
                } else {
                    "자동 커밋을 껐습니다. 커밋이나 롤백으로 끝내세요"
                };
                self.transaction(done, |c| c.set_autocommit(autocommit));
            }
            let in_transaction = connected && !self.autocommit;
            if ui
                .add_enabled(in_transaction && can_run, egui::Button::new("커밋"))
                .clicked()
            {
                self.transaction("커밋했습니다", Connection::commit);
            }
            if ui
                .add_enabled(in_transaction && can_run, egui::Button::new("롤백"))
                .clicked()
            {
                self.transaction("롤백했습니다", Connection::rollback);
            }
            if in_transaction {
                ui.colored_label(ui.visuals().warn_fg_color, "트랜잭션 중");
            }
            ui.separator();
            match &self.export {
                Some(export) => {
                    ui.spinner();
                    ui.label(format!(
                        "CSV로 내보내는 중: {}행",
                        export.written.load(Ordering::Relaxed)
                    ));
                    if ui.button("내보내기 멈추기").clicked() {
                        export.stop.store(true, Ordering::Relaxed);
                    }
                }
                None => {
                    if ui
                        .add_enabled(
                            connected && self.last_query.is_some(),
                            egui::Button::new("CSV로 내보내기…"),
                        )
                        .on_hover_text("마지막 조회를 다시 실행해 모든 행을 파일로 씁니다")
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("CSV로 내보내기")
                            .set_file_name("결과.csv")
                            .add_filter("CSV", &["csv"])
                            .save_file()
                    {
                        self.start_export(path, db, ctx);
                    }
                }
            }
        });
        if let Some(config) = chosen {
            self.connect(&config, db, ctx);
        }
    }

    fn log_ui(&self, ui: &mut egui::Ui) {
        if self.log.is_empty() {
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("sql_log")
            .max_height(72.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (error, text) in &self.log {
                    if *error {
                        ui.colored_label(ui.visuals().error_fg_color, text);
                    } else {
                        ui.label(text);
                    }
                }
            });
    }

    fn grid(&mut self, ui: &mut egui::Ui) {
        let Self {
            live,
            result,
            widths,
            scroll_fix,
            scroll_offset,
            ..
        } = self;
        let Some(buffer) = result else {
            return;
        };
        let more = if buffer.has_more() { "+" } else { "" };
        ui.horizontal(|ui| {
            ui.label(format!("{}행{more}", buffer.received()));
            if buffer.dropped() > 0 {
                ui.weak(format!(
                    "· 메모리를 아끼려고 앞쪽 {}행을 버렸습니다(다시 보려면 다시 실행)",
                    buffer.dropped()
                ));
            }
        });
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        let number_width = (buffer.received().max(1).ilog10() as f32 + 2.0)
            * ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let cell = |ui: &mut egui::Ui, width: f32, text: egui::RichText| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, row_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(width);
                    ui.add(egui::Label::new(text.monospace()).truncate());
                },
            );
        };
        let mut fetch = false;
        egui::ScrollArea::horizontal()
            .id_salt("sql_grid_h")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    cell(ui, number_width, egui::RichText::new("#").weak());
                    for (column, &width) in buffer.columns.iter().zip(widths.iter()) {
                        cell(ui, width, egui::RichText::new(&column.name).strong());
                    }
                });
                let spacing = ui.spacing().item_spacing.y;
                let mut area = egui::ScrollArea::vertical()
                    .id_salt("sql_grid_v")
                    .auto_shrink(false);
                if *scroll_fix > 0 {
                    let shift = *scroll_fix as f32 * (row_height + spacing);
                    area = area.vertical_scroll_offset((*scroll_offset - shift).max(0.0));
                    *scroll_fix = 0;
                }
                let output = area.show_rows(ui, row_height, buffer.len(), |ui, range| {
                    for index in range.clone() {
                        let Some(row) = buffer.row(index) else {
                            continue;
                        };
                        ui.horizontal(|ui| {
                            let number = buffer.dropped() + index + 1;
                            cell(
                                ui,
                                number_width,
                                egui::RichText::new(number.to_string()).weak(),
                            );
                            for (value, &width) in row.iter().zip(widths.iter()) {
                                let text = match value {
                                    Cell::Null => egui::RichText::new("NULL").weak().italics(),
                                    other => egui::RichText::new(other.to_string()),
                                };
                                cell(ui, width, text);
                            }
                        });
                    }
                    fetch = buffer.wants_more(range.end);
                });
                *scroll_offset = output.state.clamped_offset().y;
            });
        if fetch && let Some(live) = live {
            live.connection.fetch();
        }
    }
}

fn first_line(sql: &str) -> String {
    let line = sql.lines().next().unwrap_or_default();
    let mut short: String = line.chars().take(60).collect();
    if short.len() < line.len() || sql.lines().nth(1).is_some() {
        short.push('…');
    }
    short
}
