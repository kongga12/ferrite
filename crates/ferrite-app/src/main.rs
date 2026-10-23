//! Ferrite: Rust 기반 크로스플랫폼 경량 IDE.

// 릴리스 빌드에서 Windows 콘솔 창을 띄우지 않는다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor_view;
mod file_tree;
mod find_bar;
mod fonts;
mod lsp_hub;
mod lsp_ui;
mod theme;
mod watcher;

#[cfg(test)]
mod perf;

use eframe::egui;

fn main() -> eframe::Result {
    let start = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ferrite")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Ferrite",
        options,
        Box::new(|cc| Ok(Box::new(app::FerriteApp::new(cc, start)))),
    )
}
