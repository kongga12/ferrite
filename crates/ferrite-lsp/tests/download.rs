//! 실제 배포처에서 언어 서버를 받아 설치하는 확인(리디렉션, TLS, 체크섬, 실행).
//! 네트워크를 쓰고 수십 MB를 받으므로 기본으로는 돌지 않는다.
//! `cargo test -p ferrite-lsp --test download -- --ignored --nocapture`

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ferrite_lsp::catalog::{self, Server};
use ferrite_lsp::client::{LanguageServer, ServerEvent, parse_diagnostics};
use ferrite_lsp::install;
use lsp_types::{Diagnostic, DiagnosticSeverity};

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ferrite-download-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    root
}

/// 고정 버전을 실제로 내려받아 설치한다. 걸린 시간과 크기를 출력한다.
fn install_pinned(root: &Path, server: &Server) -> install::Installed {
    let started = Instant::now();
    let mut last = (0, 0);
    install::install(root, server, &mut |done, total| last = (done, total))
        .unwrap_or_else(|e| panic!("설치 실패: {e}"));
    eprintln!(
        "{} {}: {:.1}MB, {:.1}초",
        server.name,
        server.version,
        last.0 as f64 / 1_000_000.0,
        started.elapsed().as_secs_f64()
    );
    assert!(
        last.0 > 0 && last.0 == last.1,
        "진행률이 끝까지 오지 않음: {last:?}"
    );
    let installed = install::installed(root, server).expect("설치 기록이 없음");
    assert!(!installed.is_outdated(server));
    installed
}

fn wait_ready(lsp: &LanguageServer, limit: Duration) {
    let deadline = Instant::now() + limit;
    while !lsp.is_ready() {
        for event in lsp.poll_events() {
            if let ServerEvent::Exited(reason) = event {
                panic!("서버가 끝남: {reason}");
            }
        }
        assert!(Instant::now() < deadline, "{limit:?} 안에 준비되지 않음");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Windows는 서버가 완전히 끝나야 실행 파일을 지울 수 있다.
fn remove(root: &Path) {
    for _ in 0..50 {
        if fs::remove_dir_all(root).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "GitHub에서 lemminx(약 16MB)를 내려받는다"]
fn github에서_lemminx를_받아_설치하고_띄운다() {
    let server = catalog::for_language("xml").unwrap();
    if !install::installable(server) {
        eprintln!("이 OS·CPU용 lemminx 배포본이 없어 건너뜁니다");
        return;
    }
    let root = temp_root("lemminx");
    let installed = install_pinned(&root, server);
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    let command = install::command(&root, server, &project).unwrap().unwrap();
    assert_eq!(command.program, installed.bin);

    let lsp = LanguageServer::start(&command, &project, || {}).unwrap();
    wait_ready(&lsp, Duration::from_secs(60));
    eprintln!("준비됨: {}", installed.bin.display());
    drop(lsp);
    remove(&root);
}

#[test]
#[ignore = "jdt.ls(약 51MB)와 Lombok(2MB)을 내려받는다. Java 21 이상과 Maven 저장소의 lombok이 필요하다"]
fn jdtls를_받아_lombok이_만든_getter를_인식하는지_본다() {
    let server = catalog::for_language("java").unwrap();
    let root = temp_root("jdtls");
    let installed = install_pinned(&root, server);
    assert!(installed.dir.join(ferrite_lsp::java::LOMBOK_JAR).is_file());

    // egov 3.x처럼 Java 8 소스 수준의 Maven 프로젝트.
    let project = root.join("demo");
    let src = project.join("src/main/java/demo");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        project.join("pom.xml"),
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>demo</groupId><artifactId>demo</artifactId><version>1.0</version>
  <properties>
    <maven.compiler.source>1.8</maven.compiler.source>
    <maven.compiler.target>1.8</maven.compiler.target>
  </properties>
  <dependencies>
    <dependency>
      <groupId>org.projectlombok</groupId><artifactId>lombok</artifactId>
      <version>1.18.46</version><scope>provided</scope>
    </dependency>
  </dependencies>
</project>
"#,
    )
    .unwrap();
    fs::write(
        src.join("Point.java"),
        "package demo;\n\n@lombok.Getter\npublic class Point {\n    private int x;\n}\n",
    )
    .unwrap();
    let uses_getter = "package demo;\n\npublic class Uses {\n    int f(Point p) {\n        return p.getX();\n    }\n}\n";
    // 진단이 실제로 오는지 확인하는 대조군.
    let broken = "package demo;\n\npublic class Broken {\n    int y = \"문자열\";\n}\n";
    fs::write(src.join("Uses.java"), uses_getter).unwrap();
    fs::write(src.join("Broken.java"), broken).unwrap();

    let command = install::command(&root, server, &project)
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        command
            .args
            .iter()
            .any(|a| a.starts_with("-javaagent:") && a.ends_with("lombok.jar"))
    );
    let started = Instant::now();
    let lsp = LanguageServer::start(&command, &project, || {}).unwrap();
    wait_ready(&lsp, Duration::from_secs(120));
    eprintln!("준비됨: {:.1}초", started.elapsed().as_secs_f64());
    lsp.did_open(&src.join("Uses.java"), "java", 1, uses_getter);
    lsp.did_open(&src.join("Broken.java"), "java", 1, broken);

    // Maven 가져오기가 끝나기 전에는 잠깐 오류가 보일 수 있어, 조건이 맞을 때까지 최신 진단을 본다.
    let errors = |d: &Vec<Diagnostic>| {
        d.iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    let mut latest: HashMap<String, Vec<Diagnostic>> = HashMap::new();
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        for event in lsp.poll_events() {
            match event {
                ServerEvent::Notification { method, params }
                    if method == "textDocument/publishDiagnostics" =>
                {
                    if let Some((path, diagnostics)) = parse_diagnostics(&params) {
                        let name = path.file_name().unwrap().to_string_lossy().into_owned();
                        latest.insert(name, diagnostics);
                    }
                }
                ServerEvent::Exited(reason) => panic!("서버가 끝남: {reason}"),
                _ => {}
            }
        }
        let broken_errors = latest.get("Broken.java").map(errors).unwrap_or_default();
        let uses = latest.get("Uses.java").map(errors);
        if !broken_errors.is_empty() && uses.as_ref().is_some_and(Vec::is_empty) {
            eprintln!(
                "Lombok getter 인식 확인: {:.1}초, 대조군 오류: {broken_errors:?}",
                started.elapsed().as_secs_f64()
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "5분 안에 확인하지 못함. Uses.java 오류: {uses:?}, Broken.java 오류: {broken_errors:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(lsp);
    remove(&root);
}
