//! 언어 서버 기능의 화면 요소: 자동완성 팝업, 코드 액션 메뉴, 참조 목록.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use eframe::egui;
use ferrite_core::buffer::Buffer;
use ferrite_lsp::edits::CodeActionItem;
use ferrite_lsp::features::{Completion, Target};
use ferrite_lsp::position::PositionEncoding;

/// 팝업에 한 번에 보여 주는 최대 항목 수.
const MAX_VISIBLE: usize = 50;
const POPUP_WIDTH: f32 = 420.0;

/// 응답을 기다리는 요청. 응답이 오면 무엇을 할지 기억한다.
pub enum Pending {
    Completion { path: PathBuf, anchor: usize },
    Hover { path: PathBuf, pos: usize },
    Definition,
    References { name: String },
    Format { path: PathBuf },
    Rename,
    CodeActions { path: PathBuf },
    Resolve,
    Execute,
}

/// 코드 액션 목록(`Ctrl+.`, `Alt+Enter`). 커서 아래에 띄우고 키보드·마우스로 고른다.
pub struct ActionMenu {
    pub path: PathBuf,
    pub items: Vec<CodeActionItem>,
    pub encoding: PositionEncoding,
    /// 목록을 요청한 시점의 버퍼 버전. 실행할 때 문서가 바뀌었으면 편집을 적용하지 않는다.
    pub version: u64,
    selected: usize,
}

impl ActionMenu {
    pub fn new(
        path: PathBuf,
        items: Vec<CodeActionItem>,
        encoding: PositionEncoding,
        version: u64,
    ) -> Self {
        Self {
            path,
            items,
            encoding,
            version,
            selected: 0,
        }
    }

    pub fn move_selection(&mut self, down: bool) {
        let n = self.items.len();
        if n > 0 {
            self.selected = if down {
                (self.selected + 1) % n
            } else {
                (self.selected + n - 1) % n
            };
        }
    }

    pub fn take_selected(self) -> Option<CodeActionItem> {
        self.items.into_iter().nth(self.selected)
    }

    /// 목록을 그린다. 항목을 클릭하면 true(실행해야 함).
    pub fn ui(&mut self, ctx: &egui::Context, at: egui::Pos2) -> bool {
        let mut clicked = None;
        egui::Area::new(egui::Id::unique("code_action_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(POPUP_WIDTH);
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for (row, item) in self.items.iter().enumerate() {
                                let marker = if item.preferred { "★ " } else { "" };
                                if ui
                                    .selectable_label(
                                        row == self.selected,
                                        format!("{marker}{}", item.title),
                                    )
                                    .clicked()
                                {
                                    clicked = Some(row);
                                }
                            }
                        });
                });
            });
        if let Some(row) = clicked {
            self.selected = row;
        }
        clicked.is_some()
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `pos` 앞쪽으로 이어진 단어의 시작 위치(자동완성으로 바꿀 범위의 시작).
pub fn word_start(buffer: &Buffer, pos: usize) -> usize {
    let mut chars = buffer.rope().chars_at(pos);
    let mut start = pos;
    while chars.prev().is_some_and(is_word) {
        start -= 1;
    }
    start
}

/// `pos`를 포함하는 단어(참조 목록 제목용). 단어가 아니면 빈 문자열.
pub fn word_at(buffer: &Buffer, pos: usize) -> String {
    let start = word_start(buffer, pos);
    let rest: String = buffer
        .rope()
        .chars_at(pos)
        .take_while(|c| is_word(*c))
        .collect();
    format!("{}{rest}", buffer.rope().slice(start..pos))
}

pub fn is_identifier_char(c: char) -> bool {
    is_word(c)
}

/// `needle`의 글자가 `haystack`에 순서대로 들어 있는지(사이에 다른 글자가 있어도 됨).
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut rest = haystack.chars();
    needle.chars().all(|n| rest.any(|h| h == n))
}

pub struct CompletionPopup {
    pub path: PathBuf,
    /// 거르기 기준 시작 위치(요청 시점 커서 앞 단어의 시작).
    pub anchor: usize,
    items: Vec<Completion>,
    /// 서버가 목록을 다 보내지 않았다. 입력이 바뀌면 다시 요청해야 한다.
    pub incomplete: bool,
    /// 거른 결과(items의 인덱스), 보여 줄 순서.
    visible: Vec<usize>,
    selected: usize,
}

impl CompletionPopup {
    pub fn new(path: PathBuf, anchor: usize, items: Vec<Completion>, incomplete: bool) -> Self {
        Self {
            path,
            anchor,
            items,
            incomplete,
            visible: Vec::new(),
            selected: 0,
        }
    }

    /// 지금까지 친 글자(anchor..커서)로 다시 거른다. 보여 줄 항목이 없거나
    /// 커서가 단어 밖(앞 또는 다른 줄)으로 나가면 false: 팝업을 닫아야 한다.
    pub fn refilter(&mut self, buffer: &Buffer) -> bool {
        let cursor = buffer.selection().head;
        let line = |p: usize| buffer.pos_to_line_col(p).0;
        if cursor < self.anchor || line(cursor) != line(self.anchor) {
            return false;
        }
        let typed = buffer
            .rope()
            .slice(self.anchor..cursor)
            .to_string()
            .to_lowercase();
        // 접두사 일치를 먼저, 그다음 글자 순서만 맞는 것. 같은 묶음 안에서는 서버 정렬 순서.
        let mut ranked: Vec<(u8, &str, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let filter = c.filter.to_lowercase();
                let rank = if filter.starts_with(&typed) {
                    0
                } else if is_subsequence(&typed, &filter) {
                    1
                } else {
                    return None;
                };
                Some((rank, c.sort.as_str(), i))
            })
            .collect();
        ranked.sort();
        self.visible = ranked
            .into_iter()
            .take(MAX_VISIBLE)
            .map(|(_, _, i)| i)
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
        !self.visible.is_empty()
    }

    pub fn move_selection(&mut self, down: bool) {
        let n = self.visible.len();
        if n > 0 {
            self.selected = if down {
                (self.selected + 1) % n
            } else {
                (self.selected + n - 1) % n
            };
        }
    }

    pub fn selected(&self) -> Option<&Completion> {
        self.visible.get(self.selected).map(|&i| &self.items[i])
    }

    /// 고른 항목을 적용할 편집 목록. 주 편집은 지금 커서까지 늘린다
    /// (응답을 기다리는 동안 더 친 글자까지 바꾸도록). 주 편집과 겹치는 추가 편집은 뺀다.
    pub fn edits(&self, item: &Completion, buffer: &Buffer) -> Vec<(Range<usize>, String)> {
        let cursor = buffer.selection().head;
        let main = match &item.range {
            Some(r) => r.start..r.end.max(cursor),
            None => self.anchor..cursor,
        };
        let mut edits = vec![(main.clone(), item.text.clone())];
        edits.extend(
            item.additional
                .iter()
                .filter(|(r, _)| r.end <= main.start || r.start >= main.end)
                .cloned(),
        );
        edits
    }

    /// 커서 아래에 목록을 그린다. 항목을 클릭하면 true(적용해야 함).
    pub fn ui(&mut self, ctx: &egui::Context, at: egui::Pos2) -> bool {
        let mut clicked = None;
        egui::Area::new(egui::Id::unique("completion_popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(POPUP_WIDTH);
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for (row, &i) in self.visible.iter().enumerate() {
                                let item = &self.items[i];
                                let selected = row == self.selected;
                                let text = format!("{}  {}", item.label, item.detail);
                                let response = ui
                                    .selectable_label(
                                        selected,
                                        egui::RichText::new(text).monospace(),
                                    )
                                    .on_hover_text(item.kind);
                                if selected {
                                    response.scroll_to_me(None);
                                }
                                if response.clicked() {
                                    clicked = Some(row);
                                }
                            }
                        });
                });
            });
        if let Some(row) = clicked {
            self.selected = row;
        }
        clicked.is_some()
    }
}

pub struct ReferenceItem {
    pub target: Target,
    pub label: String,
}

/// 참조 찾기 결과. 항목을 누르면 그 위치로 이동한다.
pub struct ReferenceList {
    pub title: String,
    pub items: Vec<ReferenceItem>,
    pub encoding: PositionEncoding,
}

impl ReferenceList {
    /// 각 위치에 "파일:줄  내용" 라벨을 붙인다. 열리지 않은 파일은 디스크에서 그 줄을 읽는다.
    pub fn new(
        name: &str,
        targets: Vec<Target>,
        encoding: PositionEncoding,
        root: Option<&Path>,
    ) -> Self {
        let mut files: HashMap<PathBuf, Vec<String>> = HashMap::new();
        let items = targets
            .into_iter()
            .map(|target| {
                let lines = files.entry(target.path.clone()).or_insert_with(|| {
                    std::fs::read_to_string(&target.path)
                        .map(|s| s.lines().map(str::to_owned).collect())
                        .unwrap_or_default()
                });
                let line = target.range.start.line as usize;
                let preview = lines.get(line).map_or("", |l| l.trim());
                let shown = root
                    .and_then(|r| target.path.strip_prefix(r).ok())
                    .unwrap_or(&target.path);
                let label = format!("{}:{}  {preview}", shown.display(), line + 1);
                ReferenceItem { target, label }
            })
            .collect::<Vec<_>>();
        Self {
            title: format!("참조 — {name} ({}개)", items.len()),
            items,
            encoding,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ferrite_core::buffer::Selection;

    use super::*;

    fn completion(label: &str) -> Completion {
        Completion {
            label: label.into(),
            detail: String::new(),
            filter: label.into(),
            sort: label.into(),
            kind: "",
            range: None,
            text: label.into(),
            additional: Vec::new(),
        }
    }

    #[test]
    fn 접두사_일치가_먼저이고_입력에_따라_다시_거른다() {
        let mut buffer = Buffer::new("x.le");
        buffer.set_selection(Selection::caret(4));
        let anchor = word_start(&buffer, 4);
        assert_eq!(anchor, 2);
        let items = ["length", "is_empty", "len", "clear"]
            .map(completion)
            .to_vec();
        let mut popup = CompletionPopup::new(PathBuf::new(), anchor, items, false);
        assert!(popup.refilter(&buffer));
        let shown: Vec<&str> = popup
            .visible
            .iter()
            .map(|&i| popup.items[i].label.as_str())
            .collect();
        assert_eq!(
            shown,
            ["len", "length", "clear"],
            "접두사 일치(len, length) 뒤에 순서만 맞는 것(clear)"
        );

        // 한 글자 더 치면 그에 맞게 줄어든다
        buffer.insert("n", Instant::now());
        assert!(popup.refilter(&buffer));
        assert_eq!(popup.visible.len(), 2);
        // 단어 앞으로 나가면 닫는다
        buffer.set_selection(Selection::caret(1));
        assert!(!popup.refilter(&buffer));
    }

    #[test]
    fn 적용_범위는_지금_커서까지_늘리고_겹치는_추가_편집은_뺀다() {
        let mut buffer = Buffer::new("let s = Str");
        buffer.set_selection(Selection::caret(11));
        let mut item = completion("String");
        item.range = Some(8..10); // 요청 뒤에 'r'을 더 친 상황
        item.additional = vec![(0..0, "use x;\n".into()), (9..10, "겹침".into())];
        let popup = CompletionPopup::new(PathBuf::new(), 8, vec![], false);
        assert_eq!(
            popup.edits(&item, &buffer),
            vec![(8..11, "String".to_owned()), (0..0, "use x;\n".to_owned())]
        );
    }

    #[test]
    fn 커서_위치의_단어() {
        let buffer = Buffer::new("let 한글_이름 = 1;");
        assert_eq!(word_at(&buffer, 6), "한글_이름");
    }
}
