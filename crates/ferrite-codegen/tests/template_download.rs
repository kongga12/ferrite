//! 공식 템플릿을 실제로 GitHub에서 받아 프로젝트를 만들고 `mvn package`까지 돌린다(Phase 2c 완료
//! 조건: Web·Boot Web). 네트워크와 Java 17 이상·Maven이 필요하고, 처음에는 Maven 의존성을 받으므로
//! 기본으로는 돌지 않는다.
//! 웹 템플릿으로 만든 프로젝트에서 테스트 결과 트리와 의존성 취약점도 확인한다(Phase 9 완료 조건).
//! `cargo test -p ferrite-codegen --test template_download -- --ignored --nocapture`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime};

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

/// 셸로 명령 줄을 돌려 (성공했는지, 표준 출력+오류)를 돌려준다(앱의 테스트 창·취약점 검사와 같은 셸).
fn shell(dir: &Path, line: &str) -> (bool, String) {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd.exe");
        command
            .args(["/d", "/s", "/c"])
            .raw_arg(format!("\"{line}\""));
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", line]);
        command
    };
    let output = command.current_dir(dir).output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let mut tail: Vec<&str> = text.lines().rev().take(30).collect();
    tail.reverse();
    eprintln!(
        "> {line}
{}",
        tail.join(
            "
"
        )
    );
    (output.status.success(), text)
}

#[test]
#[ignore = "네트워크·Java 17·Maven이 필요하다. OSV에 의존성 이름·버전을 묻고 Maven DB 묶음(약 10MB)을 받는다"]
fn egov_웹_프로젝트에서_테스트_결과_트리와_의존성_취약점을_본다() {
    use ferrite_core::test_report::{self, Outcome, Tool};
    use ferrite_core::vuln::{Ecosystem, Finding, manifest, osv};

    let root = temp_root("phase9");
    let cache = root.join("cache");
    let template = template("egovframe-web.zip");
    download(template, &cache);
    let request = Request::new(template, root.clone());
    let project = scaffold::create(template, &request, &cache).unwrap();

    // 테스트 러너: 템플릿의 pom은 테스트를 건너뛰게 해 두었다. 테스트 창의 [pom.xml에서 테스트 켜기]와
    // 같이 그 설정을 지우면 `mvn test`가 돌고 보고서에서 실패한 테스트의 줄을 찾는다.
    let java = project.join("src/test/java/egovframework/example/sample");
    fs::create_dir_all(&java).unwrap();
    fs::write(
        java.join("SampleTest.java"),
        "package egovframework.example.sample;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport org.junit.jupiter.api.Test;\n\nclass SampleTest {\n    @Test\n    void adds() {\n        assertEquals(3, 1 + 2);\n    }\n\n    @Test\n    void addsWrong() {\n        assertEquals(4, 1 + 2, \"더하기\");\n    }\n}\n",
    )
    .unwrap();
    let pom = project.join("pom.xml");
    let text = fs::read_to_string(&pom).unwrap();
    assert!(
        test_report::pom_skips_tests(&text).is_some(),
        "egov 템플릿은 테스트를 건너뛴다"
    );
    fs::write(&pom, test_report::enable_tests(&text).unwrap()).unwrap();
    let report = root.join("reports");
    let since = SystemTime::now();
    let (ok, _) = shell(
        &project,
        &test_report::with_report(Tool::Maven, "mvn -B -q test", &report),
    );
    assert!(!ok, "실패한 테스트가 있으면 mvn이 실패한다");
    let mut cases = Vec::new();
    for file in test_report::report_files(Tool::Maven, &project, &report, since) {
        cases.extend(test_report::parse_junit(&fs::read_to_string(file).unwrap()).unwrap());
    }
    let mut outcomes: Vec<_> = cases.iter().map(|c| (c.name.as_str(), c.outcome)).collect();
    outcomes.sort_by_key(|o| o.0);
    assert_eq!(
        outcomes,
        [("adds", Outcome::Passed), ("addsWrong", Outcome::Failed)]
    );
    let wrong = cases.iter().find(|c| c.name == "addsWrong").unwrap();
    let frames = test_report::frames(wrong);
    let frame = &frames[test_report::test_frame(wrong, &frames).unwrap()];
    assert_eq!(
        (test_report::resolve(&project, &project, frame), frame.line),
        (Some(java.join("SampleTest.java")), 15)
    );

    // 의존성 취약점: egov RTE가 끌어오는 Spring·로깅 라이브러리까지 풀어 egov 의존성 줄에 둔다.
    let manifests = manifest::find(&project);
    assert_eq!(manifests.len(), 1);
    let tree = root.join("tree.txt");
    let (ok, _) = shell(&project, &manifests[0].command(&project, &tree).unwrap());
    assert!(ok, "의존성 나무");
    let deps = manifests[0]
        .read(Some(&fs::read_to_string(&tree).unwrap()))
        .unwrap();
    assert!(
        deps.iter()
            .any(|d| d.name.starts_with("org.springframework:")
                && d.via
                    .as_deref()
                    .is_some_and(|v| v.starts_with("org.egovframe.rte:"))),
        "egov RTE가 끌어온 Spring"
    );
    let online = osv::findings(&osv::query_online(&deps).unwrap(), &deps);
    let zip = root.join("osv").join("all.zip");
    osv::download_database(Ecosystem::Maven, &zip, &mut |_, _| {}).unwrap();
    let offline = osv::findings(&osv::query_offline(&zip, &deps).unwrap(), &deps);
    let key = |findings: &[Finding]| {
        let mut keys: Vec<_> = findings
            .iter()
            .map(|f| {
                (
                    f.dependency.name.clone(),
                    f.dependency.version.clone(),
                    f.title().to_owned(),
                )
            })
            .collect();
        keys.sort();
        keys
    };
    eprintln!(
        "의존성 {}개, 취약 온라인 {}건·오프라인 {}건",
        deps.len(),
        online.len(),
        offline.len()
    );
    assert_eq!(key(&online), key(&offline), "온라인과 오프라인 DB가 같다");
    fs::remove_dir_all(&root).unwrap();
}
