//! 기호 화면: 지금 파일의 개요 창(`textDocument/documentSymbol`), 프로젝트 기호 찾기(Ctrl+T,
//! `workspace/symbol`), 엔드포인트 창(`@/` 기호: Spring Boot LS, 없으면 egov 서버가 모은 요청 매핑).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers};
use ferrite_lsp::features::{
    Symbol, WorkspaceSymbol, parse_document_symbols, parse_workspace_symbols, symbol_kind_label,
};
use ferrite_lsp::position::PositionEncoding;
use lsp_types::Range as LspRange;
use serde_json::Value;

/// 문서를 고치는 동안 개요를 다시 묻기 전에 기다리는 시간.
const OUTLINE_DEBOUNCE: Duration = Duration::from_millis(400);
/// 기호 찾기에서 치는 동안 묻기 전에 기다리는 시간.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
/// 기호 찾기에 보이는 결과 수의 상한.
const MAX_RESULTS: usize = 500;
/// Spring Boot LS의 엔드포인트가 있으면 egov 서버의 것은 감춘다(같은 매핑을 둘 다 준다).
const SPRING: &str = "spring-boot";

/// 이동할 곳: (파일, 범위, 그 범위의 위치 단위).
pub type Jump = (PathBuf, LspRange, PositionEncoding);

/// 종류와 덧붙임을 흐리게 붙인 이름.
fn symbol_text(
    ui: &egui::Ui,
    name: &str,
    kind: &str,
    extra: Option<&str>,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let strong = egui::TextFormat::simple(font.clone(), ui.visuals().strong_text_color());
    let weak = egui::TextFormat::simple(font, ui.visuals().weak_text_color());
    job.append(name, 0.0, strong);
    job.append(&format!("  {kind}"), 0.0, weak.clone());
    if let Some(extra) = extra {
        job.append(&format!("  {extra}"), 0.0, weak);
    }
    job
}

#[derive(Default)]
pub struct Outline {
    /// 보이는 기호의 (문서, 판).
    shown: Option<(PathBuf, u64)>,
    /// 물어 둔 (문서, 판).
    asked: Option<(PathBuf, u64)>,
    /// 마지막으로 본 (문서, 판)과 그때. 같은 문서를 고치는 동안은 잠깐 기다렸다 묻는다.
    seen: Option<((PathBuf, u64), Instant)>,
    symbols: Vec<Symbol>,
    encoding: PositionEncoding,
    filter: String,
    /// 다음에 그릴 때 거르기 칸에 커서를 둔다(Ctrl+Shift+O).
    pub focus: bool,
    /// 이 문서를 맡은 서버가 기호를 주지 않거나 아직 준비되지 않았다.
    no_server: bool,
}

impl Outline {
    /// 지금 문서(`doc`: 경로와 판)의 개요를 물어야 하면 true. 문서를 바꾸면 바로, 같은 문서를 고치면
    /// 잠깐 멈춘 뒤에 묻는다.
    pub fn due(&mut self, doc: Option<(PathBuf, u64)>) -> bool {
        let Some(doc) = doc else {
            self.shown = None;
            self.symbols.clear();
            return false;
        };
        if self.shown.as_ref() == Some(&doc) || self.asked.as_ref() == Some(&doc) {
            return false;
        }
        let same_path = self.shown.as_ref().is_some_and(|(p, _)| *p == doc.0);
        match &self.seen {
            Some((key, at)) if *key == doc => !same_path || at.elapsed() >= OUTLINE_DEBOUNCE,
            _ => {
                self.seen = Some((doc, Instant::now()));
                !same_path
            }
        }
    }

    /// 물었다(`sent`가 false면 맡을 서버가 아직 없다).
    pub fn asked(&mut self, doc: (PathBuf, u64), sent: bool) {
        self.no_server = !sent;
        if sent {
            self.asked = Some(doc);
        } else {
            self.shown = None;
            self.symbols.clear();
        }
    }

    pub fn receive(&mut self, path: &Path, result: &Value, encoding: PositionEncoding) {
        let Some(asked) = self.asked.take_if(|(p, _)| p == path) else {
            return;
        };
        self.symbols = parse_document_symbols(result);
        self.encoding = encoding;
        self.shown = Some(asked);
    }

    /// 개요 창. 고른 기호의 범위와 그 위치 단위.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<(LspRange, PositionEncoding)> {
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("거르기")
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut self.focus) {
            field.request_focus();
        }
        if self.symbols.is_empty() {
            ui.weak(if self.no_server {
                "기호를 줄 언어 서버가 준비되지 않았습니다"
            } else if self.shown.is_none() {
                "파일을 열면 기호가 보입니다"
            } else {
                "기호가 없습니다"
            });
            return None;
        }
        let filter = self.filter.to_lowercase();
        let mut chosen = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            if filter.is_empty() {
                tree(ui, &self.symbols, "outline", &mut chosen);
            } else {
                flat(ui, &self.symbols, &filter, None, &mut chosen);
            }
        });
        chosen.map(|range| (range, self.encoding))
    }
}

/// 나무로 그린다. 자식이 있는 기호는 접을 수 있다.
fn tree(ui: &mut egui::Ui, symbols: &[Symbol], id: &str, chosen: &mut Option<LspRange>) {
    for (i, s) in symbols.iter().enumerate() {
        let text = symbol_text(ui, &s.name, symbol_kind_label(s.kind), s.detail.as_deref());
        if s.children.is_empty() {
            if ui.selectable_label(false, text).clicked() {
                *chosen = Some(s.range);
            }
            continue;
        }
        let key = format!("{id}/{i}");
        egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id(&key),
            true,
        )
        .show_header(ui, |ui| {
            if ui.selectable_label(false, text).clicked() {
                *chosen = Some(s.range);
            }
        })
        .body(|ui| tree(ui, &s.children, &key, chosen));
    }
}

/// 이름에 `filter`가 든 기호만 평평하게(담은 기호 이름을 붙여) 그린다.
fn flat(
    ui: &mut egui::Ui,
    symbols: &[Symbol],
    filter: &str,
    parent: Option<&str>,
    chosen: &mut Option<LspRange>,
) {
    for s in symbols {
        if s.name.to_lowercase().contains(filter) {
            let text = symbol_text(ui, &s.name, symbol_kind_label(s.kind), parent);
            if ui.selectable_label(false, text).clicked() {
                *chosen = Some(s.range);
            }
        }
        flat(ui, &s.children, filter, Some(&s.name), chosen);
    }
}

/// 프로젝트 기호를 묻는 차례. 서버마다 따로 답한다.
#[derive(Default)]
struct Query {
    generation: u64,
    /// 물은 서버 수와 답한 서버 수.
    asked: usize,
    answered: usize,
}

impl Query {
    fn started(&mut self, generation: u64, asked: usize) {
        *self = Self {
            generation,
            asked,
            answered: 0,
        };
    }

    /// 이 차례의 답이면 true.
    fn answer(&mut self, generation: u64) -> bool {
        let ours = generation == self.generation && self.asked > 0;
        if ours {
            self.answered += 1;
        }
        ours
    }

    fn waiting(&self) -> bool {
        self.answered < self.asked
    }
}

/// 기호 찾기 창에서 할 일.
pub enum SearchAction {
    /// 이 글로 프로젝트 기호를 묻는다.
    Query(String),
    Go(Jump),
}

#[derive(Default)]
pub struct SymbolSearch {
    open: bool,
    focus: bool,
    query: String,
    /// 마지막으로 물은 글.
    sent: String,
    changed_at: Option<Instant>,
    selected: usize,
    pending: Query,
    results: Vec<(WorkspaceSymbol, PositionEncoding)>,
}

impl SymbolSearch {
    pub fn show(&mut self) {
        self.open = true;
        self.focus = true;
        // 같은 글로 다시 물어 새로 연 서버의 기호도 받는다.
        self.sent.clear();
        self.changed_at = Some(Instant::now() - SEARCH_DEBOUNCE);
    }

    pub fn started(&mut self, generation: u64, asked: usize) {
        self.pending.started(generation, asked);
        self.results.clear();
        self.selected = 0;
    }

    pub fn receive(&mut self, generation: u64, result: &Value, encoding: PositionEncoding) {
        if !self.pending.answer(generation) {
            return;
        }
        let room = MAX_RESULTS.saturating_sub(self.results.len());
        self.results.extend(
            parse_workspace_symbols(result)
                .into_iter()
                .take(room)
                .map(|s| (s, encoding)),
        );
    }

    pub fn ui(&mut self, ctx: &egui::Context, root: Option<&Path>) -> Option<SearchAction> {
        if !self.open {
            return None;
        }
        let (down, up, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        let last = self.results.len().saturating_sub(1);
        if down {
            self.selected = (self.selected + 1).min(last);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let mut chosen = enter.then_some(self.selected);
        let mut open = !escape;
        egui::Window::new("기호로 이동")
            .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
            .collapsible(false)
            .resizable(false)
            .default_width(680.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("기호");
                    let field = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.query)
                                .hint_text("예: BoardController, selectList, @/board")
                                .desired_width(f32::INFINITY),
                        )
                        .labelled_by(label.id);
                    if std::mem::take(&mut self.focus) {
                        field.request_focus();
                    }
                    if field.changed() {
                        self.changed_at = Some(Instant::now());
                    }
                });
                if self.pending.asked == 0 && !self.sent.is_empty() {
                    ui.weak("기호를 줄 언어 서버가 떠 있지 않습니다(그 언어의 파일을 열면 뜹니다)");
                } else if self.pending.waiting() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak(format!("찾는 중… {}개", self.results.len()));
                    });
                } else if !self.sent.is_empty() {
                    ui.weak(format!("{}개", self.results.len()));
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (i, (s, _)) in self.results.iter().enumerate() {
                            let file = s.target.path.strip_prefix(root.unwrap_or(Path::new("")));
                            let place = match (&s.container, file) {
                                (Some(c), Ok(f)) => format!("{c} · {}", f.display()),
                                (Some(c), Err(_)) => c.clone(),
                                (None, Ok(f)) => f.display().to_string(),
                                (None, Err(_)) => s.target.path.display().to_string(),
                            };
                            let text =
                                symbol_text(ui, &s.name, symbol_kind_label(s.kind), Some(&place));
                            let row = ui.selectable_label(i == self.selected, text);
                            if i == self.selected && (down || up) {
                                row.scroll_to_me(None);
                            }
                            if row.clicked() {
                                chosen = Some(i);
                            }
                        }
                    });
            });
        if let Some((s, encoding)) = chosen.and_then(|i| self.results.get(i)) {
            self.open = false;
            return Some(SearchAction::Go((
                s.target.path.clone(),
                s.target.range,
                *encoding,
            )));
        }
        self.open = open;
        // 치기를 멈추면 묻는다. 빈 글은 묻지 않는다(서버마다 모든 기호를 줄 수 있다).
        let query = self.query.trim().to_owned();
        match self.changed_at {
            Some(at) if at.elapsed() >= SEARCH_DEBOUNCE => {
                self.changed_at = None;
                if !query.is_empty() && query != self.sent {
                    self.sent = query.clone();
                    return Some(SearchAction::Query(query));
                }
            }
            Some(at) => ctx.request_repaint_after(SEARCH_DEBOUNCE.saturating_sub(at.elapsed())),
            None => {}
        }
        None
    }
}

/// 엔드포인트 창에서 할 일.
pub enum EndpointAction {
    Refresh,
    Go(Jump),
}

/// 엔드포인트 하나: (`/경로`, HTTP 메서드들).
fn split_label(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix('@')?;
    if !rest.starts_with('/') {
        return None;
    }
    Some(rest.split_once(" -- ").unwrap_or((rest, "")))
}

#[derive(Default)]
pub struct Endpoints {
    /// 한 번이라도 물었다(창을 처음 열 때 묻는다).
    requested: bool,
    pending: Query,
    /// (답한 서버, 기호, 위치 단위).
    items: Vec<(&'static str, WorkspaceSymbol, PositionEncoding)>,
    filter: String,
}

impl Endpoints {
    /// 물어야 하면 true(아직 묻지 않았거나, 물었지만 맡을 서버가 없었다).
    pub fn due(&self) -> bool {
        !self.requested || self.pending.asked == 0
    }

    pub fn refresh(&mut self) {
        self.requested = false;
    }

    pub fn started(&mut self, generation: u64, asked: usize) {
        self.requested = true;
        self.pending.started(generation, asked);
        self.items.clear();
    }

    pub fn receive(
        &mut self,
        generation: u64,
        server: &'static str,
        result: &Value,
        encoding: PositionEncoding,
    ) {
        if !self.pending.answer(generation) {
            return;
        }
        self.items.extend(
            parse_workspace_symbols(result)
                .into_iter()
                .filter(|s| split_label(&s.name).is_some())
                .map(|s| (server, s, encoding)),
        );
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<EndpointAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            if ui.button("새로 고침").clicked() {
                action = Some(EndpointAction::Refresh);
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("URL·컨트롤러 거르기")
                    .desired_width(240.0),
            );
            if self.pending.waiting() {
                ui.spinner();
            }
        });
        if self.pending.asked == 0 {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("엔드포인트를 줄 언어 서버(egov)를 띄우는 중…");
            });
            return action;
        }
        // 같은 매핑을 Spring Boot LS와 egov 서버가 함께 주면 Spring Boot LS의 것만 쓴다.
        let spring = self.items.iter().any(|(server, ..)| *server == SPRING);
        let filter = self.filter.to_lowercase();
        let mut groups: BTreeMap<&str, Vec<(&str, &str, &WorkspaceSymbol, PositionEncoding)>> =
            BTreeMap::new();
        for (server, s, encoding) in &self.items {
            if spring && *server != SPRING {
                continue;
            }
            let Some((path, methods)) = split_label(&s.name) else {
                continue;
            };
            let container = s.container.as_deref().unwrap_or("(이름 없음)");
            if !filter.is_empty()
                && !path.to_lowercase().contains(&filter)
                && !container.to_lowercase().contains(&filter)
            {
                continue;
            }
            groups
                .entry(container)
                .or_default()
                .push((path, methods, s, *encoding));
        }
        let total: usize = groups.values().map(Vec::len).sum();
        ui.weak(format!(
            "엔드포인트 {total}개 · 컨트롤러 {}개",
            groups.len()
        ));
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (container, mut rows) in groups {
                rows.sort_by_key(|(path, ..)| *path);
                egui::CollapsingHeader::new(format!("{container} ({})", rows.len()))
                    .id_salt(("endpoints", container))
                    .default_open(true)
                    .show(ui, |ui| {
                        for (path, methods, s, encoding) in rows {
                            let methods = if methods.is_empty() { "ALL" } else { methods };
                            let mut job = egui::text::LayoutJob::default();
                            let font = egui::TextStyle::Monospace.resolve(ui.style());
                            job.append(
                                &format!("{methods:<7}"),
                                0.0,
                                egui::TextFormat::simple(
                                    font.clone(),
                                    ui.visuals().weak_text_color(),
                                ),
                            );
                            job.append(
                                path,
                                0.0,
                                egui::TextFormat::simple(font, ui.visuals().strong_text_color()),
                            );
                            if ui.selectable_label(false, job).clicked() {
                                action = Some(EndpointAction::Go((
                                    s.target.path.clone(),
                                    s.target.range,
                                    encoding,
                                )));
                            }
                        }
                    });
            }
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 엔드포인트_이름에서_경로와_메서드를_나눈다() {
        assert_eq!(
            split_label("@/board/list.do -- GET,POST"),
            Some(("/board/list.do", "GET,POST"))
        );
        assert_eq!(split_label("@/board"), Some(("/board", "")));
        assert_eq!(split_label("@+ 'boardService'"), None);
        assert_eq!(split_label("list"), None);
    }

    #[test]
    fn 개요는_문서를_바꾸면_바로_고치면_멈춘_뒤에_묻는다() {
        let mut o = Outline::default();
        let a = |v| (PathBuf::from("a.java"), v);
        assert!(o.due(Some(a(1))), "처음 연 문서는 바로");
        o.asked(a(1), true);
        assert!(!o.due(Some(a(1))), "물어 둔 판은 다시 묻지 않는다");
        o.receive(
            Path::new("a.java"),
            &Value::Array(Vec::new()),
            PositionEncoding::Utf16,
        );
        assert!(!o.due(Some(a(2))), "같은 문서를 고치면 잠깐 기다린다");
        std::thread::sleep(OUTLINE_DEBOUNCE);
        assert!(o.due(Some(a(2))));
        assert!(
            o.due(Some((PathBuf::from("b.java"), 1))),
            "다른 문서는 바로"
        );
    }
}
