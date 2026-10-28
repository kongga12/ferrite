//! DataFrame 보기: 노트북 커널의 pandas DataFrame을 그리드로 본다. 보이는 행 근처만 페이지(200행)로
//! 받고(`Kernel::page`), 열 머리글을 누르면 커널에서 정렬한다. 셀 실행이 끝나면 목록을 다시 묻고 보던
//! DataFrame도 다시 받는다(값이 바뀌었을 수 있다).

use std::collections::HashMap;

use eframe::egui;
use ferrite_notebook::kernel::{Event, FrameInfo, Kernel};
use serde_json::Value;

/// 한 번에 받는 행 수.
const PAGE_ROWS: usize = 200;
/// 칸 너비를 정할 때 보는 최대 글자 수(SQL 결과 그리드와 같다).
const MAX_CELL_CHARS: usize = 40;

/// 행 하나: (색인, 칸 값).
type Row = (String, Vec<Value>);

#[derive(Default)]
pub struct FrameViewer {
    pub open: bool,
    frames: Vec<FrameInfo>,
    /// 목록을 다시 물어야 한다(처음 열 때, 실행이 끝났을 때).
    stale: bool,
    frames_request: Option<u64>,
    selected: Option<String>,
    /// (열 자리, 오름차순).
    sort: Option<(usize, bool)>,
    rows: usize,
    columns: Vec<String>,
    dtypes: Vec<String>,
    widths: Vec<f32>,
    /// 받은 페이지(번호 → 행).
    pages: HashMap<usize, Vec<Row>>,
    /// 받는 중인 페이지(요청 id → 번호). 지우면 늦게 온 답은 버린다.
    pending: HashMap<u64, usize>,
    error: Option<String>,
}

impl FrameViewer {
    /// 커널이 새로 떴거나 끝났다: 이전 커널의 것을 모두 버린다.
    pub fn reset(&mut self) {
        *self = Self {
            open: self.open,
            stale: true,
            ..Self::default()
        };
    }

    /// 셀 실행이 끝났다: 목록과 보던 DataFrame을 다시 받는다.
    pub fn invalidate(&mut self) {
        self.stale = true;
        self.pages.clear();
        self.pending.clear();
    }

    /// 커널의 답을 받는다.
    pub fn receive(&mut self, event: Event) {
        match event {
            Event::Frames { id, result } if self.frames_request == Some(id) => {
                self.frames_request = None;
                match result {
                    Ok(frames) => {
                        // 보던 DataFrame이 사라졌으면 닫는다.
                        if let Some(name) = &self.selected
                            && !frames.iter().any(|f| &f.name == name)
                        {
                            self.select(None);
                        }
                        self.frames = frames;
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Event::Page { id, result } => {
                let Some(page_number) = self.pending.remove(&id) else {
                    return;
                };
                match result {
                    Ok(page) => {
                        self.rows = page.rows;
                        if self.columns != page.columns {
                            self.widths.clear();
                        }
                        self.columns = page.columns;
                        self.dtypes = page.dtypes;
                        self.pages
                            .insert(page_number, page.index.into_iter().zip(page.data).collect());
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            _ => {}
        }
    }

    fn select(&mut self, name: Option<String>) {
        self.rows = name
            .as_ref()
            .and_then(|name| self.frames.iter().find(|f| &f.name == name))
            .map_or(0, |f| f.rows);
        self.selected = name;
        self.sort = None;
        self.columns.clear();
        self.dtypes.clear();
        self.widths.clear();
        self.pages.clear();
        self.pending.clear();
        self.error = None;
    }

    /// `kernel`: 쓸 수 있는 Python 커널(꺼졌거나 다른 언어면 None과 그 까닭).
    pub fn ui(&mut self, ui: &mut egui::Ui, kernel: Result<&mut Kernel, &str>) {
        let kernel = match kernel {
            Ok(kernel) => kernel,
            Err(why) => {
                ui.weak(why);
                return;
            }
        };
        if self.stale && self.frames_request.is_none() {
            self.stale = false;
            self.frames_request = Some(kernel.frames());
        }
        let mut chosen = None;
        ui.horizontal(|ui| {
            let label = ui.label("DataFrame");
            let text = self.selected.as_deref().unwrap_or("고르세요");
            egui::ComboBox::from_id_salt("notebook_frame")
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for frame in &self.frames {
                        let shown =
                            format!("{} · {}행 × {}열", frame.name, frame.rows, frame.columns);
                        let on = self.selected.as_deref() == Some(frame.name.as_str());
                        if ui.selectable_label(on, shown).clicked() {
                            chosen = Some(frame.name.clone());
                        }
                    }
                })
                .response
                .labelled_by(label.id);
            if ui.button("새로 고침").clicked() {
                self.invalidate();
            }
            if self.frames_request.is_some() || !self.pending.is_empty() {
                ui.spinner();
            }
            if self.selected.is_some() && !self.columns.is_empty() {
                ui.weak(format!("{}행 × {}열", self.rows, self.columns.len()));
            }
        });
        if let Some(name) = chosen {
            self.select(Some(name));
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let Some(name) = self.selected.clone() else {
            if self.frames.is_empty() && self.frames_request.is_none() {
                ui.weak("커널에 pandas DataFrame이 없습니다.");
            }
            return;
        };
        let wanted = self.grid(ui);
        for page_number in wanted {
            if !self.pending.values().any(|&p| p == page_number) {
                let start = page_number * PAGE_ROWS;
                let id = kernel.page(&name, start..start + PAGE_ROWS, self.sort);
                self.pending.insert(id, page_number);
            }
        }
    }

    /// 첫 페이지로 칸 너비를 정한다(한글처럼 넓은 글자는 두 칸으로 센다).
    fn measure(&mut self, ui: &egui::Ui) {
        let Some(first) = self.pages.get(&0) else {
            return;
        };
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let glyph = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let chars = |text: &str| {
            text.chars()
                .map(|c| if c.is_ascii() { 1 } else { 2 })
                .sum::<usize>()
        };
        let width = |widest: usize| widest.min(MAX_CELL_CHARS) as f32 * glyph + 12.0;
        let index = first
            .iter()
            .map(|(index, _)| chars(index))
            .chain([2])
            .max()
            .unwrap_or(2);
        self.widths = std::iter::once(width(index))
            .chain(self.columns.iter().enumerate().map(|(i, column)| {
                let widest = first
                    .iter()
                    .filter_map(|(_, row)| row.get(i))
                    .map(|value| chars(&cell_text(value)))
                    .chain([chars(column) + 2, 4])
                    .max()
                    .unwrap_or(4);
                width(widest)
            }))
            .collect();
    }

    /// 그리드를 그리고, 보이는데 아직 받지 않은 페이지 번호를 돌려준다.
    fn grid(&mut self, ui: &mut egui::Ui) -> Vec<usize> {
        if self.widths.is_empty() {
            self.measure(ui);
        }
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        let fallback = 8.0 * ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let cell = |ui: &mut egui::Ui, width: f32, text: egui::RichText| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, row_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(width);
                    ui.add(
                        egui::Label::new(text.monospace())
                            .truncate()
                            .sense(egui::Sense::click()),
                    )
                },
            )
            .inner
        };
        let Self {
            columns,
            dtypes,
            widths,
            pages,
            sort,
            rows,
            ..
        } = self;
        let width = |i: usize| widths.get(i).copied().unwrap_or(fallback);
        let mut wanted = Vec::new();
        let mut resort = None;
        egui::ScrollArea::horizontal()
            .id_salt("frame_grid_h")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let _ = cell(ui, width(0), egui::RichText::new("").weak());
                    for (i, column) in columns.iter().enumerate() {
                        let arrow = match *sort {
                            Some((c, true)) if c == i => " ▲",
                            Some((c, false)) if c == i => " ▼",
                            _ => "",
                        };
                        let header = cell(
                            ui,
                            width(i + 1),
                            egui::RichText::new(format!("{column}{arrow}")).strong(),
                        );
                        let dtype = dtypes.get(i).map(String::as_str).unwrap_or_default();
                        if header
                            .on_hover_text(format!("{dtype} · 눌러서 정렬"))
                            .clicked()
                        {
                            // 오름차순 → 내림차순 → 정렬 없음.
                            resort = Some(match *sort {
                                Some((c, true)) if c == i => Some((i, false)),
                                Some((c, false)) if c == i => None,
                                _ => Some((i, true)),
                            });
                        }
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("frame_grid_v")
                    .auto_shrink(false)
                    .show_rows(ui, row_height, *rows, |ui, range| {
                        for index in range {
                            let page_number = index / PAGE_ROWS;
                            let row = pages
                                .get(&page_number)
                                .and_then(|page| page.get(index % PAGE_ROWS));
                            ui.horizontal(|ui| match row {
                                Some((label, values)) => {
                                    let _ = cell(ui, width(0), egui::RichText::new(label).weak());
                                    for (i, value) in values.iter().enumerate() {
                                        let text = match value {
                                            Value::Null => {
                                                egui::RichText::new("NaN").weak().italics()
                                            }
                                            other => egui::RichText::new(cell_text(other)),
                                        };
                                        let _ = cell(ui, width(i + 1), text);
                                    }
                                }
                                None => {
                                    let _ = cell(ui, width(0), egui::RichText::new("…").weak());
                                    if !wanted.contains(&page_number)
                                        && !pages.contains_key(&page_number)
                                    {
                                        wanted.push(page_number);
                                    }
                                }
                            });
                        }
                    });
            });
        // 아직 열을 모르면(처음) 첫 페이지를 받는다.
        if columns.is_empty() && !pages.contains_key(&0) && !wanted.contains(&0) {
            wanted.push(0);
        }
        if let Some(new_sort) = resort {
            *sort = new_sort;
            pages.clear();
            self.pending.clear();
            return vec![0];
        }
        wanted
    }
}

/// 칸 값을 글로(pandas가 보이는 것처럼 참·거짓은 True·False).
fn cell_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "NaN".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_notebook::kernel::Page;
    use serde_json::json;

    #[test]
    fn 늦게_온_페이지는_버리고_실행이_끝나면_다시_받는다() {
        let mut viewer = FrameViewer {
            frames: vec![FrameInfo {
                name: "df".into(),
                rows: 3,
                columns: 1,
            }],
            ..FrameViewer::default()
        };
        viewer.select(Some("df".into()));
        viewer.pending.insert(7, 0);
        let page = |rows| Page {
            rows,
            columns: vec!["a".into()],
            dtypes: vec!["int64".into()],
            index: vec!["0".into()],
            data: vec![vec![json!(1)]],
        };
        viewer.receive(Event::Page {
            id: 7,
            result: Ok(page(3)),
        });
        assert_eq!(viewer.pages[&0].len(), 1);
        // 실행이 끝나면 받은 페이지를 버리고, 그 전에 보낸 요청의 답도 버린다.
        viewer.pending.insert(8, 0);
        viewer.invalidate();
        viewer.receive(Event::Page {
            id: 8,
            result: Ok(page(5)),
        });
        assert!(viewer.pages.is_empty() && viewer.stale);
        assert_eq!(viewer.rows, 3);
        // 보던 DataFrame이 목록에서 사라지면 닫는다.
        viewer.frames_request = Some(9);
        viewer.receive(Event::Frames {
            id: 9,
            result: Ok(Vec::new()),
        });
        assert_eq!(viewer.selected, None);
        assert_eq!(cell_text(&json!(true)), "True");
        assert_eq!(cell_text(&json!(1.5)), "1.5");
    }
}
