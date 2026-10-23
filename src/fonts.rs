//! OS별 한글 폰트 로딩. egui 기본 폰트에는 한글 글리프가 없어 그대로 두면 한글이 깨진다.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

const FONT_NAME: &str = "korean";

pub fn install_korean_font(ctx: &egui::Context) {
    let Some(bytes) = find_korean_font() else {
        eprintln!("경고: 한글 폰트를 찾지 못했습니다. 한글이 깨져 보일 수 있습니다.");
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        FONT_NAME.into(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    // 기본 폰트 뒤에 대체(fallback) 폰트로 추가: 영문은 기존 폰트, 한글만 이 폰트로 그린다.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(FONT_NAME.into());
    }
    ctx.set_fonts(fonts);
}

fn find_korean_font() -> Option<Vec<u8>> {
    candidate_paths()
        .into_iter()
        // egui는 해석할 수 없는 폰트 데이터에서 패닉하므로 아웃라인 폰트 형식만 허용한다.
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
        })
        .find_map(|p| std::fs::read(p).ok())
}

#[cfg(target_os = "windows")]
fn candidate_paths() -> Vec<PathBuf> {
    let windir =
        std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    vec![
        windir.join(r"Fonts\malgun.ttf"),
        windir.join(r"Fonts\gulim.ttc"),
    ]
}

#[cfg(target_os = "macos")]
fn candidate_paths() -> Vec<PathBuf> {
    vec![
        "/System/Library/Fonts/AppleSDGothicNeo.ttc".into(),
        "/System/Library/Fonts/Supplemental/AppleGothic.ttf".into(),
    ]
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn candidate_paths() -> Vec<PathBuf> {
    // 배포판마다 경로가 달라 fontconfig로 찾는다. fc-match는 한글 폰트가 없어도 아무 폰트나
    // 돌려주므로(한글이 깨짐), 한국어를 실제로 지원하는 폰트만 나열하는 fc-list를 쓴다.
    let listed = std::process::Command::new("fc-list")
        .args(["-f", "%{file}\n", ":lang=ko"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let mut paths: Vec<PathBuf> = listed.lines().map(PathBuf::from).collect();
    // UI용으로 기본 굵기를 먼저 고른다(fc-list 출력 순서는 정해져 있지 않다).
    paths.sort_by_key(|p| {
        let name = p.to_string_lossy().to_lowercase();
        ["bold", "light", "thin", "black", "heavy", "medium"]
            .iter()
            .any(|w| name.contains(w))
    });
    paths.extend([
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into(),
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc".into(),
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc".into(),
        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf".into(),
    ]);
    paths
}
