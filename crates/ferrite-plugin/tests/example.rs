//! 예제 플러그인(examples/plugins/text-stats)을 wasm32로 빌드해 불러 본다. wasm32 타깃이 없으면
//! 로컬에서는 건너뛰고 CI에서는 실패한다.

use std::path::{Path, PathBuf};
use std::process::Command;

use ferrite_plugin::{Action, Editor, Event, Plugin, Widget};

/// 빌드한 예제를 plugin.toml과 함께 임시 플러그인 폴더에 둔다.
fn example() -> Option<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/text-stats");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    // 바깥 cargo test가 작업 공간 target을 잠그고 있으므로 예제 폴더의 target에 빌드한다.
    let built = Command::new(cargo)
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
        ])
        .arg(src.join("target"))
        .arg("--manifest-path")
        .arg(src.join("Cargo.toml"))
        .status()
        .is_ok_and(|s| s.success());
    if !built {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에서는 예제 플러그인을 빌드해야 한다"
        );
        eprintln!("wasm32-unknown-unknown 타깃이 없어 건너뜀");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("ferrite-example-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(src.join("plugin.toml"), dir.join("plugin.toml")).unwrap();
    std::fs::copy(
        src.join("target/wasm32-unknown-unknown/release/text_stats.wasm"),
        dir.join("text_stats.wasm"),
    )
    .unwrap();
    Some(dir)
}

#[test]
fn 예제_플러그인의_명령과_패널() {
    let Some(dir) = example() else { return };
    let mut plugin = Plugin::load(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let ids: Vec<_> = plugin
        .manifest
        .commands
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["upper", "count"]);
    assert_eq!(plugin.manifest.panels[0].id, "stats");

    let editor = Editor {
        path: Path::new("/p/메모.txt"),
        language: None,
        text: "가나 abc\nabc 끝",
        selection: "abc",
    };
    let reply = plugin.command("upper", Some(&editor)).unwrap();
    assert_eq!(
        reply.actions,
        [Action::ReplaceSelection { text: "ABC".into() }]
    );
    let reply = plugin.command("count", Some(&editor)).unwrap();
    assert_eq!(
        reply.actions,
        [Action::Message {
            text: "줄 2 · 낱말 4 · 글자 12".into()
        }]
    );
    let reply = plugin.command("count", None).unwrap();
    assert_eq!(
        reply.actions,
        [Action::Message {
            text: "열린 문서가 없습니다".into()
        }]
    );

    let labels = |ui: &[Widget]| -> Vec<String> {
        ui.iter()
            .filter_map(|w| match w {
                Widget::Label { text } | Widget::Code { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    };
    let reply = plugin.panel("stats", None, Some(&editor)).unwrap();
    assert_eq!(labels(&reply.ui), ["메모.txt", "줄 2 · 낱말 4 · 글자 12"]);
    assert!(reply.ui.contains(&Widget::Button {
        id: "upper".into(),
        text: "선택 영역을 대문자로".into()
    }));
    // 입력은 플러그인 상태로 남아 다음에 그릴 때도 쓴다.
    let change = Event::Change {
        id: "find".into(),
        value: "abc".into(),
    };
    plugin.panel("stats", Some(&change), Some(&editor)).unwrap();
    let reply = plugin.panel("stats", None, Some(&editor)).unwrap();
    assert!(labels(&reply.ui).contains(&"'abc': 2곳".to_owned()));
    assert!(reply.ui.contains(&Widget::Input {
        id: "find".into(),
        value: "abc".into(),
        hint: "찾을 글".into()
    }));
    let click = Event::Click { id: "upper".into() };
    let reply = plugin.panel("stats", Some(&click), Some(&editor)).unwrap();
    assert_eq!(
        reply.actions,
        [Action::ReplaceSelection { text: "ABC".into() }]
    );
}
