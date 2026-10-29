//! Ferrite: Rust 기반 크로스플랫폼 경량 IDE.

// 릴리스 빌드에서 Windows 콘솔 창을 띄우지 않는다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod codegen;
mod commngt_ui;
mod config_gen;
mod db_ui;
mod debug_ui;
mod dock;
mod editor_view;
mod file_tree;
mod find_bar;
mod find_in_files;
mod fonts;
mod frame_ui;
mod git_ui;
mod http_ui;
mod lsp_hub;
mod lsp_ui;
mod mapper_db;
mod new_project;
mod notebook_ui;
mod picker;
mod plugin_ui;
mod quick_open;
mod sarif_ui;
mod schema_ui;
mod servers;
mod sql_console;
mod symbols_ui;
mod terminal_ui;
mod test_ui;
mod theme;
mod update_ui;
mod watcher;

#[cfg(test)]
mod perf;

use eframe::egui;

const WINDOW_ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");

fn main() -> eframe::Result {
    // `ferrite --version`: 판만 찍고 끝낸다(설치 시험이 설치한 실행 파일이 뜨는지 본다). 릴리스 빌드의
    // Windows는 콘솔이 없는 프로그램이라 파이프로 받을 때만 보인다.
    if std::env::args_os().nth(1).as_deref() == Some("--version".as_ref()) {
        println!("ferrite {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // `ferrite --egov-ls`: 창을 띄우지 않고 egov 언어 서버로 동작한다(Ferrite가 스스로를 띄운다).
    if std::env::args_os().nth(1).as_deref() == Some(ferrite_lsp::catalog::EGOV_LS_FLAG.as_ref()) {
        return ferrite_egov::run_stdio().map_err(|e| eframe::Error::AppCreation(Box::new(e)));
    }
    let start = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Ferrite")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([640.0, 400.0]);
    // 설치 파일과 같은 아이콘(`icons/`, cargo-packager 설정은 Cargo.toml).
    if let Ok(icon) = eframe::icon_data::from_png_bytes(WINDOW_ICON) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Ferrite",
        options,
        Box::new(|cc| Ok(Box::new(app::FerriteApp::new(cc, start)))),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn 창_아이콘은_정사각형_png다() {
        let icon = eframe::icon_data::from_png_bytes(super::WINDOW_ICON).unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
    }
}
