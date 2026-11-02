//! 저장 시 작업: 저장한 파일이 `.ferrite/run.toml`의 `[[on_save]]` 패턴에 맞으면 그 명령(포매터·Sass
//! 컴파일 등)을 창 없이 차례로 돌리고 결과를 상태바에 알린다. 편집 포커스를 가져가지 않는다. 포매터가
//! 파일을 고치면 열린 문서는 파일 감시로 다시 읽힌다.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui;
use ferrite_core::run;

pub struct OnSave {
    ctx: egui::Context,
    tx: Sender<String>,
    rx: Receiver<String>,
}

impl OnSave {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        Self {
            ctx: ctx.clone(),
            tx,
            rx,
        }
    }

    /// 저장한 파일에 맞는 작업을 띄운다. 띄웠거나 구성 파일이 잘못됐으면 상태바에 보일 글.
    pub fn saved(&self, root: &Path, path: &Path) -> Option<String> {
        let tasks = match run::load_on_save(root) {
            Ok(tasks) => tasks,
            Err(e) => return Some(e),
        };
        let jobs: Vec<_> = tasks
            .iter()
            .filter(|t| t.matches(root, path))
            .map(|t| {
                (
                    t.name.clone(),
                    t.command_for(root, path),
                    t.working_directory(root),
                    t.env.clone(),
                )
            })
            .collect();
        if jobs.is_empty() {
            return None;
        }
        let names: Vec<String> = jobs.iter().map(|j| j.0.clone()).collect();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        thread::spawn(move || {
            let mut done = Vec::new();
            let mut failed = None;
            // 앞의 작업이 고친 파일을 뒤의 작업이 보도록 차례로 돌린다.
            for (name, line, dir, env) in jobs {
                match crate::debug_ui::run_hidden(&line, &dir, &env) {
                    Ok(output) if output.status.success() => done.push(name),
                    Ok(output) => {
                        let text = output.stdout + &output.stderr;
                        failed.get_or_insert(format!(
                            "{name}({}): {}",
                            output.status,
                            crate::debug_ui::tail(&text, 3)
                        ));
                    }
                    Err(e) => {
                        failed.get_or_insert(format!("{name}: 띄우지 못했습니다: {e}"));
                    }
                }
            }
            let message = match failed {
                Some(failure) => format!("저장 시 작업 실패: {failure}"),
                None => format!("저장 시 작업 완료: {}", done.join(", ")),
            };
            let _ = tx.send(message);
            ctx.request_repaint();
        });
        Some(format!("저장 시 작업 중: {}", names.join(", ")))
    }

    /// 끝난 작업의 결과(상태바에 보일 글).
    pub fn poll(&self) -> Option<String> {
        self.rx.try_iter().last()
    }
}
