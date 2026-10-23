//! 대용량 파일 성능 확인(Phase 1 완료 조건: 100MB 파일에서 스크롤·편집 60fps).
//! 파일을 만들어 재므로 평소에는 건너뛴다. 릴리스 빌드로 따로 실행한다:
//! `cargo test --release -p ferrite-app -- --ignored --nocapture 대용량`

use std::fs;
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::app::FerriteApp;

const FRAME_BUDGET: Duration = Duration::from_micros(16_667);
const TARGET_BYTES: usize = 100 * 1024 * 1024;

struct Stats {
    avg: Duration,
    p95: Duration,
    max: Duration,
}

/// 매 프레임 `before`로 입력을 넣고 한 프레임을 돌린 시간을 잰다.
fn measure(
    harness: &mut Harness<'_, FerriteApp>,
    frames: usize,
    before: impl Fn(&Harness<'_, FerriteApp>),
) -> Stats {
    let mut times: Vec<Duration> = (0..frames)
        .map(|_| {
            before(harness);
            let t = Instant::now();
            harness.step();
            t.elapsed()
        })
        .collect();
    times.sort();
    Stats {
        avg: times.iter().sum::<Duration>() / frames as u32,
        p95: times[frames * 95 / 100],
        max: times[frames - 1],
    }
}

#[test]
#[ignore = "100MB 파일을 만들어 측정한다. 릴리스 빌드에서 --ignored로 실행"]
fn 대용량_100mb_파일에서_스크롤과_편집이_60fps를_유지한다() {
    let dir = std::env::temp_dir().join(format!("ferrite-perf-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("big.txt");
    let line = "가나다라 The quick brown fox jumps over the lazy dog 0123456789 마바사\n";
    let text = line.repeat(TARGET_BYTES / line.len() + 1);
    fs::write(&file, &text).unwrap();
    let lines = text.lines().count();
    drop(text);

    let t = Instant::now();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| FerriteApp::new(cc, Some(file.clone())));
    harness.run();
    let open = t.elapsed();
    harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .focus();
    harness.run();

    let scroll = measure(&mut harness, 300, |h| h.key_press(Key::PageDown));
    // 문서 중간으로 크게 건너뛰며 스크롤(끝 → 위로 페이지 이동)
    harness.key_press_modifiers(Modifiers::COMMAND, Key::End);
    harness.run();
    let scroll_far = measure(&mut harness, 300, |h| h.key_press(Key::PageUp));
    let typing = measure(&mut harness, 300, |h| {
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .type_text("a");
    });
    let newline = measure(&mut harness, 100, |h| h.key_press(Key::Enter));

    println!(
        "파일: {} MB, {lines} 줄, 열기 {open:?}",
        TARGET_BYTES / 1024 / 1024
    );
    for (name, s) in [
        ("스크롤(PageDown)", &scroll),
        ("스크롤(문서 끝에서 PageUp)", &scroll_far),
        ("입력", &typing),
        ("줄바꿈", &newline),
    ] {
        println!(
            "{name}: 평균 {:?}, p95 {:?}, 최대 {:?}",
            s.avg, s.p95, s.max
        );
    }
    for (name, s) in [
        ("스크롤", &scroll),
        ("먼 스크롤", &scroll_far),
        ("입력", &typing),
        ("줄바꿈", &newline),
    ] {
        assert!(
            s.p95 < FRAME_BUDGET,
            "{name} p95 {:?}가 16.7ms를 넘음",
            s.p95
        );
    }
    drop(harness);
    fs::remove_dir_all(&dir).unwrap();
}
