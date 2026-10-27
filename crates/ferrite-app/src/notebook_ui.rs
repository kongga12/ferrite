//! 노트북 보기(.ipynb). 버퍼(노트북 JSON)는 그대로 두고 그 위에 셀을 보인다. 셀을 고치면 노트북을
//! 다시 써서 버퍼를 바꾼다(저장·변경 표시·외부 변경 감지는 다른 파일과 같다). [JSON 보기]로 원래 글을
//! 고칠 수 있다. 출력은 글, 그림(PNG), 오류(색 코드를 뺀 추적), HTML은 태그를 뺀 요약으로 보인다.
//!
//! 코드 셀은 Jupyter 커널로 실행한다(`ferrite_notebook::kernel`). 커널은 처음 실행할 때 노트북 위 폴더의
//! 가상 환경(.venv, venv)이나 PATH의 Python으로 띄우고, 노트북을 닫으면 끈다. 출력은 오는 대로 노트북에
//! 쓴다(Jupyter처럼 저장하면 남는다).

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Instant;

use base64::Engine as _;
use eframe::egui;
use ferrite_core::buffer::Buffer;
use ferrite_notebook::kernel::{Event, Kernel, Status};
use ferrite_notebook::{CellKind, Notebook, Output, strip_ansi};

/// HTML 출력 요약에 보이는 최대 줄 수.
const HTML_LINES: usize = 20;

#[derive(Default)]
enum KernelState {
    #[default]
    Off,
    Starting,
    Ready {
        name: String,
        notice: Option<String>,
    },
    /// 끝났거나 띄우지 못했다(까닭). 다시 실행하면 새로 띄운다.
    Ended(String),
}

#[derive(Default)]
pub struct NotebookView {
    /// 노트북 대신 JSON 글을 보인다.
    pub show_json: bool,
    /// 읽은 버퍼 버전과 그 노트북(또는 읽지 못한 까닭).
    parsed: Option<(u64, Result<Notebook, String>)>,
    /// 셀마다 편집 중인 글.
    sources: Vec<String>,
    selected: usize,
    /// 입력 초점을 옮길 셀(실행하고 다음 셀로 갈 때).
    focus: Option<usize>,
    /// 그림 출력(base64 글의 해시 → 텍스처).
    images: HashMap<u64, Option<egui::TextureHandle>>,
    /// 셀마다 붙인 번호. 셀을 넣고 옮기고 지워도 실행 출력이 제 셀로 간다.
    keys: Vec<u64>,
    next_key: u64,
    kernel: Option<Kernel>,
    kernel_state: KernelState,
    /// 실행 줄: (실행 id, 셀 번호). 맨 앞이 실행 중이다.
    queue: VecDeque<(u64, u64)>,
}

enum Edit {
    Source(usize),
    Insert(CellKind),
    Move(bool),
    Remove,
    Kind(CellKind),
    /// 셀들을 차례로 실행한다. `advance`면 마지막 셀의 다음 셀로 간다.
    Run {
        cells: Vec<usize>,
        advance: bool,
    },
    Interrupt,
    Restart,
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
            // 밖에서(JSON 보기, 되돌리기) 셀 수가 바뀌었으면 번호를 새로 붙인다(실행 중인 출력은 버린다).
            if self.keys.len() != notebook.len() {
                self.keys = (0..notebook.len())
                    .map(|_| {
                        self.next_key += 1;
                        self.next_key
                    })
                    .collect();
            }
        }
        self.parsed = Some((buffer.version(), parsed));
    }

    /// 노트북을 그린다. JSON 보기면 아무것도 그리지 않고 false(에디터가 그린다).
    pub fn ui(&mut self, ui: &mut egui::Ui, buffer: &mut Buffer, path: &Path) -> bool {
        if self.show_json {
            return false;
        }
        self.refresh(buffer);
        self.pump(buffer);
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
        let has = !notebook.is_empty();
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
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(has, egui::Button::new("▶ 실행"))
                .on_hover_text("고른 셀을 실행하고 다음 셀로 (Shift+Enter, 제자리는 Ctrl+Enter)")
                .clicked()
            {
                edit = Some(Edit::Run {
                    cells: vec![self.selected],
                    advance: true,
                });
            }
            if ui
                .add_enabled(has, egui::Button::new("모두 실행"))
                .clicked()
            {
                edit = Some(Edit::Run {
                    cells: (0..notebook.len()).collect(),
                    advance: false,
                });
            }
            if ui
                .add_enabled(!self.queue.is_empty(), egui::Button::new("멈춤"))
                .clicked()
            {
                edit = Some(Edit::Interrupt);
            }
            let restartable = !matches!(self.kernel_state, KernelState::Off);
            if ui
                .add_enabled(restartable, egui::Button::new("커널 다시 시작"))
                .clicked()
            {
                edit = Some(Edit::Restart);
            }
            ui.separator();
            match &self.kernel_state {
                KernelState::Off => {
                    ui.weak("커널: 꺼짐(실행하면 띄웁니다)");
                }
                KernelState::Starting => {
                    ui.spinner();
                    ui.label("커널: 띄우는 중");
                }
                KernelState::Ready { name, notice } => {
                    if self.queue.is_empty() {
                        ui.label(format!("커널: {name}"));
                    } else {
                        ui.spinner();
                        ui.label(format!("커널: {name} · 실행 중"));
                    }
                    if let Some(notice) = notice {
                        ui.weak(notice);
                    }
                }
                KernelState::Ended(reason) => {
                    let first = reason.lines().next().unwrap_or_default();
                    ui.colored_label(ui.visuals().error_fg_color, format!("커널: {first}"))
                        .on_hover_text(reason);
                }
            }
        });
        ui.separator();
        let ctx = ui.ctx().clone();
        let Self {
            sources,
            selected,
            focus,
            images,
            keys,
            queue,
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
                    let running = keys
                        .get(i)
                        .is_some_and(|key| queue.iter().any(|(_, k)| k == key));
                    egui::Frame::group(ui.style())
                        .stroke(stroke)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            // 머리글은 셀 입력칸의 이름이기도 하다(화면 읽기 프로그램).
                            let header = match (kind, notebook.execution_count(i)) {
                                (CellKind::Code, _) if running => ui.weak("코드 [*]"),
                                (CellKind::Code, Some(n)) => ui.weak(format!("코드 [{n}]")),
                                (CellKind::Code, None) => ui.weak("코드 [ ]"),
                                _ => ui.weak(kind.label()),
                            };
                            let id = ui.make_persistent_id(("notebook_cell", i));
                            // 입력칸이 Enter를 줄바꿈으로 먹기 전에 실행 키를 가로챈다.
                            if kind == CellKind::Code && ui.memory(|m| m.has_focus(id)) {
                                let (next, stay) = ui.input_mut(|input| {
                                    (
                                        input.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter),
                                        input.consume_key(
                                            egui::Modifiers::COMMAND,
                                            egui::Key::Enter,
                                        ),
                                    )
                                });
                                if next || stay {
                                    edit = Some(Edit::Run {
                                        cells: vec![i],
                                        advance: next,
                                    });
                                }
                            }
                            let mut editor = egui::TextEdit::multiline(source)
                                .desired_width(f32::INFINITY)
                                .desired_rows(1)
                                .id(id);
                            if kind == CellKind::Code {
                                editor = editor.code_editor();
                            }
                            let response = ui.add(editor).labelled_by(header.id);
                            if *focus == Some(i) {
                                response.request_focus();
                                *focus = None;
                            }
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
            self.apply(edit, buffer, path, &ctx);
        }
        true
    }

    /// 고친 것을 노트북에 넣고 버퍼를 다시 쓴다.
    fn apply(&mut self, edit: Edit, buffer: &mut Buffer, path: &Path, ctx: &egui::Context) {
        match edit {
            Edit::Interrupt => {
                if let Some(kernel) = &mut self.kernel {
                    kernel.interrupt();
                }
                return;
            }
            Edit::Restart => {
                // 버리면 사이드카가 이전 커널을 끈다(실행 중이어도). 새 커널은 곧바로 띄운다.
                self.kernel = None;
                self.queue.clear();
                self.start_kernel(path, ctx);
                return;
            }
            Edit::Run { .. } if self.kernel.is_none() => self.start_kernel(path, ctx),
            _ => {}
        }
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
                self.next_key += 1;
                self.keys.insert(to.min(self.keys.len()), self.next_key);
                self.selected = to;
            }
            Edit::Move(up) => {
                let to = notebook.move_cell(at, up);
                if at.max(to) < self.keys.len() {
                    self.keys.swap(at, to);
                }
                self.selected = to;
            }
            Edit::Remove => {
                notebook.remove(at);
                if at < self.keys.len() {
                    self.keys.remove(at);
                }
                self.selected = at.min(notebook.len().saturating_sub(1));
            }
            Edit::Kind(kind) => notebook.set_kind(at, kind),
            Edit::Run { cells, advance } => {
                // 출력은 실행을 줄 세울 때 비운다(Jupyter와 같다).
                if let Some(kernel) = &mut self.kernel {
                    for &i in &cells {
                        if notebook.kind(i) != CellKind::Code {
                            continue;
                        }
                        notebook.clear_outputs(i);
                        notebook.set_execution_count(i, None);
                        let id = kernel.execute(&notebook.source(i));
                        self.queue.push_back((id, self.keys[i]));
                    }
                }
                if advance && let Some(&last) = cells.last() {
                    self.selected = (last + 1).min(notebook.len().saturating_sub(1));
                    self.focus = Some(self.selected);
                }
            }
            Edit::Interrupt | Edit::Restart => {}
        }
        self.write(buffer);
    }

    /// 노트북을 버퍼에 다시 쓴다(같으면 그대로 둔다).
    fn write(&mut self, buffer: &mut Buffer) {
        let Some((version, Ok(notebook))) = &mut self.parsed else {
            return;
        };
        let text = notebook.to_json();
        if text != buffer.text() {
            let all = 0..buffer.len_chars();
            buffer.replace_ranges(vec![(all, text)], Instant::now());
        }
        *version = buffer.version();
        self.sources = (0..notebook.len()).map(|i| notebook.source(i)).collect();
    }

    /// 노트북의 커널을 띄운다(결과는 `pump`가 받는다).
    fn start_kernel(&mut self, path: &Path, ctx: &egui::Context) {
        let name = match &self.parsed {
            Some((_, Ok(notebook))) => notebook.kernel_name().map(str::to_owned),
            _ => None,
        };
        let Some(python) = python_for(path) else {
            self.kernel_state = KernelState::Ended(
                "Python을 찾지 못했습니다(노트북 위 폴더의 .venv·venv, PATH)".into(),
            );
            return;
        };
        let cwd = path.parent().unwrap_or(Path::new("."));
        let ctx = ctx.clone();
        match Kernel::start(&python, name.as_deref(), cwd, move || ctx.request_repaint()) {
            Ok(kernel) => {
                self.kernel = Some(kernel);
                self.kernel_state = KernelState::Starting;
            }
            Err(e) => self.kernel_state = KernelState::Ended(e),
        }
    }

    /// 커널에서 온 것을 노트북에 쓴다.
    fn pump(&mut self, buffer: &mut Buffer) {
        let Some(kernel) = &self.kernel else {
            return;
        };
        let events: Vec<Event> = std::iter::from_fn(|| kernel.try_event()).collect();
        let mut changed = false;
        let mut ended = false;
        for event in events {
            let cell = |id| cell_index(&self.queue, &self.keys, id);
            let notebook = match &mut self.parsed {
                Some((_, Ok(notebook))) => Some(notebook),
                _ => None,
            };
            match event {
                Event::Started {
                    display_name,
                    missing,
                    ..
                } => {
                    self.kernel_state = KernelState::Ready {
                        name: display_name,
                        notice: missing.map(|m| {
                            format!("노트북의 커널({m})이 이 컴퓨터에 없어 대신 띄웠습니다")
                        }),
                    };
                }
                Event::Output { id, output } => {
                    if let (Some(notebook), Some(i)) = (notebook, cell(id)) {
                        notebook.push_output(i, output);
                        changed = true;
                    }
                }
                Event::Clear { id } => {
                    if let (Some(notebook), Some(i)) = (notebook, cell(id)) {
                        notebook.clear_outputs(i);
                        changed = true;
                    }
                }
                Event::Done {
                    id,
                    execution_count,
                    status,
                } => {
                    if let (Some(notebook), Some(i), Some(_)) =
                        (notebook, cell(id), execution_count)
                    {
                        notebook.set_execution_count(i, execution_count);
                        changed = true;
                    }
                    self.queue.retain(|(e, _)| *e != id);
                    if status == Status::Dead {
                        self.kernel_state = KernelState::Ended(
                            "커널이 죽었습니다. 다시 실행하면 새로 띄웁니다".into(),
                        );
                        ended = true;
                    }
                }
                Event::Ended(reason) => {
                    self.kernel_state = KernelState::Ended(reason);
                    ended = true;
                }
            }
        }
        if ended {
            self.kernel = None;
            self.queue.clear();
        }
        if changed {
            self.write(buffer);
        }
    }
}

/// 실행 id의 셀 자리.
fn cell_index(queue: &VecDeque<(u64, u64)>, keys: &[u64], id: u64) -> Option<usize> {
    let (_, key) = queue.iter().find(|(e, _)| *e == id)?;
    keys.iter().position(|k| k == key)
}

/// 노트북 위 폴더의 가상 환경(.venv, venv)의 Python, 없으면 PATH의 Python.
fn python_for(notebook: &Path) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    };
    let names: &[&str] = if cfg!(windows) {
        &["python", "python3"]
    } else {
        &["python3", "python"]
    };
    notebook
        .ancestors()
        .skip(1)
        .flat_map(|dir| [dir.join(".venv"), dir.join("venv")])
        .map(|env| env.join(exe))
        .find(|python| python.is_file())
        .or_else(|| {
            names
                .iter()
                .find_map(|name| ferrite_lsp::client::find_program(name))
        })
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

    #[test]
    fn 노트북_위_폴더의_가상_환경을_먼저_쓴다() {
        let dir = std::env::temp_dir().join(format!("ferrite-venv-{}", std::process::id()));
        let exe = if cfg!(windows) {
            "Scripts/python.exe"
        } else {
            "bin/python"
        };
        let python = dir.join(".venv").join(exe);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        let notebook = dir.join("분석").join("a.ipynb");
        assert_eq!(python_for(&notebook), Some(python));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
