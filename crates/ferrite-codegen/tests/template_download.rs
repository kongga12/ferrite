//! 공식 템플릿을 실제로 GitHub에서 받아 프로젝트를 만들고 `mvn package`까지 돌린다(Phase 2c 완료
//! 조건: Web·Boot Web). 네트워크와 Java 17 이상·Maven이 필요하고, 처음에는 Maven 의존성을 받으므로
//! 기본으로는 돌지 않는다.
//! `cargo test -p ferrite-codegen --test template_download -- --ignored --nocapture`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ferrite_codegen::scaffold::{self, Request, TEMPLATES, Template};
use ferrite_lsp::install;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ferrite-template-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn template(zip: &str) -> &'static Template {
    TEMPLATES.iter().find(|t| t.zip.file == zip).unwrap()
}

/// 고정한 주소에서 받는다(앱의 [내려받아 만들기]와 같은 경로).
fn download(template: &Template, cache: &Path) {
    for (pinned, url) in template.files() {
        let started = Instant::now();
        let mut last = (0, 0);
        install::download(
            &url,
            pinned.size,
            pinned.sha256,
            &cache.join(pinned.file),
            &mut |done, total| last = (done, total),
        )
        .unwrap_or_else(|e| panic!("{url}: {e}"));
        eprintln!(
            "{}: {:.1}MB, {:.1}초",
            pinned.file,
            last.0 as f64 / 1_000_000.0,
            started.elapsed().as_secs_f64()
        );
        assert_eq!(
            last,
            (pinned.size, pinned.size),
            "진행률이 끝까지 오지 않음"
        );
    }
}

fn mvn_package(project: &Path) {
    let mvn = if cfg!(windows) { "mvn.cmd" } else { "mvn" };
    let started = Instant::now();
    let output = Command::new(mvn)
        .args(["-B", "-ntp", "package"])
        .current_dir(project)
        .output()
        .unwrap_or_else(|e| panic!("{mvn} 실행 실패(Maven이 PATH에 있어야 한다): {e}"));
    let log = String::from_utf8_lossy(&output.stdout);
    let tail: Vec<&str> = log.lines().rev().take(60).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    assert!(
        output.status.success(),
        "{}: mvn package 실패\n{}\n{}",
        project.display(),
        tail.join("\n"),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "{}: mvn package {:.0}초",
        project.display(),
        started.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "네트워크·Java 17·Maven이 필요하다"]
fn 웹과_boot_웹_템플릿을_내려받아_만들고_mvn_package를_통과한다() {
    let root = temp_root("mvn");
    let cache = root.join("cache");
    for (zip, artifact) in [
        ("egovframe-web.zip", "war"),
        ("egovframe-boot-web.zip", "jar"),
    ] {
        let template = template(zip);
        download(template, &cache);
        assert!(template.cached(&cache));
        let request = Request::new(template, root.clone());
        let project = scaffold::create(template, &request, &cache)
            .unwrap_or_else(|e| panic!("{}: {e}", template.name));
        mvn_package(&project);
        let built = fs::read_dir(project.join("target"))
            .unwrap()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == artifact));
        assert!(built, "{}: target/*.{artifact}이 없다", template.name);
    }
    fs::remove_dir_all(&root).unwrap();
}
