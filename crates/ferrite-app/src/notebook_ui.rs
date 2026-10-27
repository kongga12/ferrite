//! 노트북 보기(.ipynb). 버퍼(노트북 JSON)는 그대로 두고 그 위에 셀을 보인다. 셀을 고치면 노트북을
//! 다시 써서 버퍼를 바꾼다(저장·변경 표시·외부 변경 감지는 다른 파일과 같다). [JSON 보기]로 원래 글을
//! 고칠 수 있다. 출력은 저장된 것을 보인다: 글, 그림(PNG), 오류(색 코드를 뺀 추적), HTML은 태그를 뺀 요약.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;

use base64::Engine as _;
use eframe::egui;
use ferrite_core::buffer::Buffer;
use ferrite_notebook::{CellKind, Notebook, Output, strip_ansi};

/// HTML 출력 요약에 보이는 최대 줄 수.
const HTML_LINES: usize = 20;

#[derive(Default)]
pub struct NotebookView {
    /// 노트북 대신 JSON 글을 보인다.
    pub show_json: bool,
    /// 읽은 버퍼 버전과 그 노트북(또는 읽지 못한 까닭).
    parsed: Option<(u64, Result<Notebook, String>)>,
    /// 셀마다 편집 중인 글.
    sources: Vec<String>,
    selected: usize,
    /// 그림 출력(base64 글의 해시 → 텍스처).
    images: HashMap<u64, Option<egui::TextureHandle>>,
}

enum Edit {
    Source(usize),
    Insert(CellKind),
    Move(bool),
    Remove,
    Kind(CellKind),
}

impl NotebookView {
    /// 버퍼가 바뀌었으면 다시 읽는다.
    fn refresh(&mut self, buffer: &Buffer) {
        if self
            .parsed
            .as_ref()
            .is_some_and(|(v, _)| *v == buffer.version())
        {
            return;
        }
        let parsed = Notebook::parse(&buffer.text());
        if let Ok(notebook) = &parsed {
            self.sources = (0..notebook.len()).map(|i| notebook.source(i)).collect();
            self.selected = self.selected.min(notebook.len().saturating_sub(1));
        }
        self.parsed = Some((buffer.version(), parsed));
    }

    /// 노트북을 그린다. JSON 보기면 아무것도 그리지 않고 false(에디터가 그린다).
    pub fn ui(&mut self, ui: &mut egui::Ui, buffer: &mut Buffer) -> bool {
        if self.show_json {
            return false;
        }
        self.refresh(buffer);
        let mut edit = None;
        let notebook = match self.parsed.as_ref().map(|(_, parsed)| parsed) {
            Some(Ok(notebook)) => notebook,
            Some(Err(e)) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
                if ui.button("JSON 보기로 고치기").clicked() {
                    self.show_json = true;
                }
                return true;
            }
            None => return true,
        };
        ui.horizontal_wrapped(|ui| {
            if ui.button("JSON 보기").clicked() {
                self.show_json = true;
            }
            ui.separator();
            if ui.button("+ 코드 셀").clicked() {
                edit = Some(Edit::Insert(CellKind::Code));
            }
            if ui.button("+ 마크다운 셀").clicked() {
                edit = Some(Edit::Insert(CellKind::Markdown));
            }
            ui.separator();
            let has = !notebook.is_empty();
            if ui.add_enabled(has, egui::Button::new("위로")).clicked() {
                edit = Some(Edit::Move(true));
            }
            if ui.add_enabled(has, egui::Button::new("아래로")).clicked() {
                edit = Some(Edit::Move(false));
            }
            if ui
                .add_enabled(has, egui::Button::new("셀 지우기"))
                .clicked()
            {
                edit = Some(Edit::Remove);
            }
            if has {
                let current = notebook.kind(self.selected);
                let label = ui.label("종류");
                egui::ComboBox::from_id_salt("notebook_cell_kind")
                    .selected_text(current.label())
                    .show_ui(ui, |ui| {
                        for kind in CellKind::ALL {
                            if ui.selectable_label(kind == current, kind.label()).clicked() {
                                edit = Some(Edit::Kind(kind));
                            }
                        }
                    })
                    .response
                    .labelled_by(label.id);
            }
            if let Some(language) = notebook.language() {
                ui.weak(format!("· {language}"));
            }
        });
        ui.separator();
        let ctx = ui.ctx().clone();
        let Self {
            sources,
            selected,
            images,
            ..
        } = self;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if notebook.is_empty() {
                    ui.weak("셀이 없습니다. [+ 코드 셀]로 더하세요.");
                }
                for (i, source) in sources.iter_mut().enumerate() {
                    let kind = notebook.kind(i);
                    let is_selected = *selected == i;
                    let stroke = if is_selected {
                        ui.visuals().selection.stroke
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke
                    };
                    egui::Frame::group(ui.style())
                        .stroke(stroke)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            // 머리글은 셀 입력칸의 이름이기도 하다(화면 읽기 프로그램).
                            let header = match (kind, notebook.execution_count(i)) {
                                (CellKind::Code, Some(n)) => ui.weak(format!("코드 [{n}]")),
                                (CellKind::Code, None) => ui.weak("코드 [ ]"),
                                _ => ui.weak(kind.label()),
                            };
                            let mut editor = egui::TextEdit::multiline(source)
                                .desired_width(f32::INFINITY)
                                .desired_rows(1)
                                .id_salt(("notebook_cell", i));
                            if kind == CellKind::Code {
                                editor = editor.code_editor();
                            }
                            let response = ui.add(editor).labelled_by(header.id);
                            if response.changed() {
                                edit = Some(Edit::Source(i));
                            }
                            if response.gained_focus() || header.clicked() {
                                *selected = i;
                            }
                            if kind == CellKind::Code {
                                for output in notebook.outputs(i) {
                                    output_ui(ui, &ctx, images, &output);
                                }
                            }
                        });
                }
            });
        if let Some(edit) = edit {
            self.apply(edit, buffer);
        }
        true
    }

    /// 고친 것을 노트북에 넣고 버퍼를 다시 쓴다.
    fn apply(&mut self, edit: Edit, buffer: &mut Buffer) {
        let Some((_, Ok(notebook))) = &mut self.parsed else {
            return;
        };
        let at = self.selected;
        match edit {
            Edit::Source(i) => {
                notebook.set_source(i, &self.sources[i]);
                self.selected = i;
            }
            Edit::Insert(kind) => {
                let to = if notebook.is_empty() { 0 } else { at + 1 };
                notebook.insert(to, kind);
                self.selected = to;
            }
            Edit::Move(up) => self.selected = notebook.move_cell(at, up),
            Edit::Remove => {
                notebook.remove(at);
                self.selected = at.min(notebook.len().saturating_sub(1));
            }
            Edit::Kind(kind) => notebook.set_kind(at, kind),
        }
        let text = notebook.to_json();
        if text != buffer.text() {
            let all = 0..buffer.len_chars();
            buffer.replace_ranges(vec![(all, text)], Instant::now());
        }
        let version = buffer.version();
        let notebook = notebook.clone();
        self.sources = (0..notebook.len()).map(|i| notebook.source(i)).collect();
        self.parsed = Some((version, Ok(notebook)));
    }
}

/// 출력 하나. 그림 > 글 > HTML 요약 순으로 고른다.
fn output_ui(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    images: &mut HashMap<u64, Option<egui::TextureHandle>>,
    output: &Output,
) {
    match output {
        Output::Stream { name, text } => {
            let text = egui::RichText::new(text.trim_end()).monospace();
            if name == "stderr" {
                ui.label(text.color(ui.visuals().warn_fg_color));
            } else {
                ui.label(text);
            }
        }
        Output::Data { data, .. } => {
            let get = |mime: &str| {
                data.iter()
                    .find(|(m, _)| m == mime)
                    .map(|(_, v)| v.as_str())
            };
            if let Some(png) = get("image/png")
                && let Some(texture) = image(ctx, images, png)
            {
                let size = texture.size_vec2();
                let scale = (ui.available_width() / size.x).min(1.0);
                ui.image((texture.id(), size * scale));
            } else if let Some(text) = get("text/plain") {
                ui.monospace(text.trim_end());
            } else if let Some(html) = get("text/html") {
                ui.weak("HTML 출력(요약)");
                ui.monospace(html_summary(html));
            } else {
                let mimes: Vec<&str> = data.iter().map(|(m, _)| m.as_str()).collect();
                ui.weak(format!("보일 수 없는 출력: {}", mimes.join(", ")));
            }
        }
        Output::Error {
            ename,
            evalue,
            traceback,
        } => {
            let text = if traceback.is_empty() {
                format!("{ename}: {evalue}")
            } else {
                strip_ansi(&traceback.join("\n"))
            };
            ui.label(
                egui::RichText::new(text)
                    .monospace()
                    .color(ui.visuals().error_fg_color),
            );
        }
        Output::Other(kind) => {
            ui.weak(format!("모르는 출력: {kind}"));
        }
    }
}

/// base64 PNG를 텍스처로(한 번만 만든다). 읽지 못하면 None.
fn image(
    ctx: &egui::Context,
    images: &mut HashMap<u64, Option<egui::TextureHandle>>,
    base64: &str,
) -> Option<egui::TextureHandle> {
    let mut hasher = DefaultHasher::new();
    base64.hash(&mut hasher);
    let key = hasher.finish();
    images
        .entry(key)
        .or_insert_with(|| {
            // nbformat은 base64에 줄바꿈을 넣기도 한다.
            let clean: String = base64.chars().filter(|c| !c.is_whitespace()).collect();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(clean)
                .ok()?;
            let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                .ok()?
                .to_rgba8();
            let size = [decoded.width() as usize, decoded.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
            Some(ctx.load_texture(format!("notebook-{key:x}"), pixels, Default::default()))
        })
        .clone()
}

/// HTML 태그를 빼고 빈 줄을 줄인 앞부분.
fn html_summary(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut summary = lines[..lines.len().min(HTML_LINES)].join("\n");
    if lines.len() > HTML_LINES {
        summary.push_str(&format!("\n… ({}줄 더)", lines.len() - HTML_LINES));
    }
    summary
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_요약은_태그를_빼고_앞부분만_보인다() {
        assert_eq!(
            html_summary(
                "<table>\n<tr><td>a &amp; b</td></tr>\n\n<tr><td>&lt;c&gt;</td></tr></table>"
            ),
            "a & b\n<c>"
        );
        let long: String = (0..30).map(|i| format!("<p>{i}</p>\n")).collect();
        assert!(html_summary(&long).ends_with("… (10줄 더)"));
    }
}
