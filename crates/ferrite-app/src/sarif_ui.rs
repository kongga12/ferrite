//! 문제 창의 "정적 분석"(SARIF): 프로젝트의 SARIF 파일을 골라 읽어 파일별로 보이고, 누르면 그 줄로 간다.
//! 읽은 파일이 바뀌면(실행 구성으로 분석을 다시 돌리면) 다시 읽는다. 열린 파일에는 물결선으로도 보인다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use ferrite_core::sarif::{self, Finding, Level};
use ferrite_lsp::problem::{Problem, Severity};
use ropey::Rope;

/// 읽은 파일이 바뀌었는지 보는 간격.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// 앱이 할 일.
pub enum SarifAction {
    /// 이 파일의 이 줄·열(0부터)로 간다.
    Go(PathBuf, usize, usize),
}

#[derive(Default)]
pub struct SarifPanel {
    /// 읽은 SARIF 파일과 그때의 수정 시각.
    file: Option<(PathBuf, Option<SystemTime>)>,
    findings: Vec<Finding>,
    /// 읽을 때마다 하나씩 는다(열린 문서의 물결선을 다시 만들 때).
    generation: u64,
    status: Option<(String, bool)>,
    /// 프로젝트에서 찾은 SARIF 파일과 고른 것.
    candidates: Vec<PathBuf>,
    chosen: usize,
    /// 후보를 찾은 프로젝트 루트(바뀌면 다시 찾는다).
    searched: Option<PathBuf>,
    checked: Option<Instant>,
}

impl SarifPanel {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    fn load(&mut self, path: &Path, root: &Path) {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let dir = path.parent().unwrap_or(root);
        let result = std::fs::read(path)
            .map_err(|e| format!("{}을(를) 읽지 못했습니다: {e}", path.display()))
            .and_then(|bytes| sarif::parse(&String::from_utf8_lossy(&bytes), root, dir));
        self.generation += 1;
        match result {
            Ok(findings) => {
                let count = |l| findings.iter().filter(|f| f.level == l).count();
                let tools: Vec<&str> =
                    findings
                        .iter()
                        .map(|f| f.tool.as_str())
                        .fold(Vec::new(), |mut v, t| {
                            if !v.contains(&t) {
                                v.push(t);
                            }
                            v
                        });
                let missing = findings.iter().filter(|f| f.path.is_none()).count();
                let mut text = format!(
                    "{} · 오류 {} · 경고 {} · 참고 {} · {}",
                    if tools.is_empty() {
                        "결과 없음".to_owned()
                    } else {
                        tools.join(", ")
                    },
                    count(Level::Error),
                    count(Level::Warning),
                    count(Level::Note),
                    shown(path, root)
                );
                if missing > 0 {
                    text.push_str(&format!(" · 프로젝트에서 찾지 못한 파일 {missing}건"));
                }
                self.status = Some((text, false));
                self.findings = findings;
            }
            Err(e) => {
                self.status = Some((e, true));
                self.findings.clear();
            }
        }
        self.file = Some((path.to_path_buf(), modified));
    }

    /// 읽은 파일이 바뀌었으면 다시 읽는다. 매 프레임 부른다(1초마다 본다).
    pub fn poll(&mut self, ctx: &egui::Context, root: Option<&Path>) {
        let (Some((path, modified)), Some(root)) = (self.file.clone(), root) else {
            return;
        };
        ctx.request_repaint_after(CHECK_EVERY);
        if self.checked.is_some_and(|at| at.elapsed() < CHECK_EVERY) {
            return;
        }
        self.checked = Some(Instant::now());
        let now = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if now.is_some() && now != modified {
            self.load(&path, root);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, root: Option<&Path>) -> Option<SarifAction> {
        let Some(root) = root else {
            ui.weak("폴더를 열면 프로젝트의 SARIF(SpotBugs·PMD·Semgrep 등)를 읽습니다");
            return None;
        };
        if self.searched.as_deref() != Some(root) {
            self.candidates = sarif::candidates(root);
            self.chosen = 0;
            self.searched = Some(root.to_path_buf());
        }
        ui.horizontal_wrapped(|ui| {
            let label = ui.label("SARIF");
            let names: Vec<String> = self.candidates.iter().map(|p| shown(p, root)).collect();
            egui::ComboBox::from_id_salt("sarif_file")
                .selected_text(
                    names
                        .get(self.chosen)
                        .cloned()
                        .unwrap_or_else(|| "(찾은 파일 없음)".into()),
                )
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.chosen, i, name);
                    }
                })
                .response
                .labelled_by(label.id);
            if ui
                .add_enabled(!self.candidates.is_empty(), egui::Button::new("불러오기"))
                .clicked()
                && let Some(path) = self.candidates.get(self.chosen).cloned()
            {
                self.load(&path, root);
            }
            if ui.button("다른 파일…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_directory(root)
                    .add_filter("SARIF", &["sarif", "json"])
                    .pick_file()
            {
                self.load(&path, root);
            }
            if ui
                .small_button("찾기")
                .on_hover_text("프로젝트의 SARIF 파일을 다시 찾는다")
                .clicked()
            {
                self.candidates = sarif::candidates(root);
                self.chosen = 0;
            }
            if ui
                .add_enabled(self.file.is_some(), egui::Button::new("지우기"))
                .clicked()
            {
                self.file = None;
                self.findings.clear();
                self.status = None;
                self.generation += 1;
            }
        });
        if let Some((text, bad)) = &self.status {
            let color = if *bad {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().weak_text_color()
            };
            ui.colored_label(color, text);
        }
        if self.file.is_none() {
            ui.weak("실행 구성으로 정적 분석(예: SpotBugs `-Dspotbugs.sarifOutput=true`)을 돌린 뒤 SARIF를 불러오세요. 파일이 바뀌면 다시 읽습니다.");
            return None;
        }
        ui.separator();
        // 파일별로 묶는다. 못 찾은 파일은 적힌 주소로.
        let mut files: BTreeMap<String, Vec<&Finding>> = BTreeMap::new();
        for finding in &self.findings {
            let key = match &finding.path {
                Some(path) => shown(path, root),
                None => format!("(찾지 못함) {}", finding.uri),
            };
            files.entry(key).or_default().push(finding);
        }
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("sarif_findings")
            .auto_shrink(false)
            .show(ui, |ui| {
                for (file, mut findings) in files {
                    findings.sort_by_key(|f| (f.line, f.column));
                    egui::CollapsingHeader::new(format!("{file} ({})", findings.len()))
                        .id_salt(("sarif_file_group", file.as_str()))
                        .default_open(true)
                        .show(ui, |ui| {
                            for finding in findings {
                                let severity = severity(finding.level);
                                let text = egui::RichText::new(format!(
                                    "{}:{}  [{}] {}: {}",
                                    finding.line.max(1),
                                    finding.column.max(1),
                                    severity.label(),
                                    finding.rule,
                                    finding.message
                                ))
                                .color(crate::editor_view::severity_color(ui.visuals(), severity));
                                let mut row = ui.selectable_label(false, text);
                                if let Some(help) = &finding.help {
                                    row = row.on_hover_text(format!("{} · {help}", finding.tool));
                                }
                                if row.clicked()
                                    && let Some(path) = &finding.path
                                {
                                    action = Some(SarifAction::Go(
                                        crate::app::real_path(path),
                                        finding.line.saturating_sub(1) as usize,
                                        finding.column.saturating_sub(1) as usize,
                                    ));
                                }
                            }
                        });
                }
            });
        action
    }
}

fn shown(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn severity(level: Level) -> Severity {
    match level {
        Level::Error => Severity::Error,
        Level::Warning => Severity::Warning,
        Level::Note => Severity::Info,
    }
}

/// 이 문서에 해당하는 결과를 에디터의 진단(문자 범위, 위치 순)으로 바꾼다. 열이 없으면 그 줄의 글자
/// 부분을, 끝이 없으면 시작 줄 끝까지 칠한다.
pub fn problems_for(findings: &[Finding], path: &Path, rope: &Rope) -> Vec<Problem> {
    let lines = rope.len_lines();
    let line_text_range = |line: usize| {
        let line = line.min(lines.saturating_sub(1));
        let start = rope.line_to_char(line);
        let text = rope.line(line);
        let mut len = text.len_chars();
        while len > 0 && matches!(text.char(len - 1), '\n' | '\r') {
            len -= 1;
        }
        let indent = text
            .chars()
            .take(len)
            .take_while(|c| c.is_whitespace())
            .count();
        (start + indent, start + len.max(indent))
    };
    let position = |line: usize, column: usize| {
        let line = line.min(lines.saturating_sub(1));
        let (_, end) = line_text_range(line);
        (rope.line_to_char(line) + column).min(end)
    };
    let mut problems: Vec<Problem> = findings
        .iter()
        .filter(|f| f.path.as_deref() == Some(path))
        .map(|f| {
            let line = f.line.saturating_sub(1) as usize;
            let (text_start, text_end) = line_text_range(line);
            let start = if f.column > 0 {
                position(line, f.column as usize - 1)
            } else {
                text_start
            };
            let end = match (f.end_line, f.end_column) {
                (end_line, end_column) if end_column > 0 => {
                    position(end_line.saturating_sub(1) as usize, end_column as usize - 1)
                }
                _ => text_end,
            }
            .max(start);
            let severity = severity(f.level);
            let message = format!("{} ({} {})", f.message, f.tool, f.rule);
            Problem {
                range: start..end,
                severity,
                message: message.clone(),
                source: f.tool.clone(),
                raw: lsp_types::Diagnostic {
                    message,
                    ..Default::default()
                },
            }
        })
        .collect();
    problems.sort_by_key(|p| (p.range.start, std::cmp::Reverse(p.severity)));
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 결과를_에디터의_문자_범위로_바꾼다() {
        let rope = Rope::from_str("class A {\n    int x = 1;\n}\n");
        let path = Path::new("/p/A.java");
        let finding = |line, column, end_line, end_column| Finding {
            tool: "SpotBugs".into(),
            rule: "R".into(),
            level: Level::Warning,
            message: "m".into(),
            help: None,
            uri: "A.java".into(),
            path: Some(path.to_path_buf()),
            line,
            column,
            end_line,
            end_column,
        };
        let findings = [
            finding(2, 0, 2, 0),
            finding(1, 7, 1, 8),
            finding(99, 0, 99, 0),
        ];
        let problems = problems_for(&findings, path, &rope);
        let ranges: Vec<_> = problems.iter().map(|p| p.range.clone()).collect();
        // 열이 없으면 들여쓰기를 뺀 줄의 글자, 열이 있으면 그 범위, 줄이 넘치면 마지막 줄.
        assert_eq!(ranges[0], 6..7);
        assert_eq!(ranges[1], 14..24);
        assert_eq!(problems[1].message, "m (SpotBugs R)");
        assert_eq!(problems.len(), 3);
        assert!(problems_for(&findings, Path::new("/p/B.java"), &rope).is_empty());
    }
}
