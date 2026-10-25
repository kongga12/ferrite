//! 생성한 Java 코드를 실제 egov·Spring jar로 컴파일해 본다(로컬 Maven 저장소와 JDK가 필요하다).
//! `cargo test -p ferrite-codegen --test compile -- --ignored --nocapture`
//!
//! egov 3.x(3.10, Spring 4.3, Java 8 대상)와 4.x(4.3, Spring 5.3, Java 11 대상)를 각각 컴파일한다.

use std::path::{Path, PathBuf};
use std::process::Command;

use ferrite_codegen::crud::{self, Db, JUnit, Layout, Part, Rte, Spec};
use ferrite_codegen::ddl;

const DDL: &str = "CREATE TABLE TB_NOTICE (
    NOTICE_ID VARCHAR2(20) NOT NULL PRIMARY KEY,
    TITLE VARCHAR2(200) NOT NULL,
    HIT_CNT NUMBER(9),
    AMOUNT NUMBER(12,2),
    REG_DT DATE,
    UPD_TS TIMESTAMP,
    USE_AT CHAR(1),
    FILE_DATA BLOB
);
COMMENT ON COLUMN TB_NOTICE.TITLE IS '제목';";

/// (그룹 경로, artifactId, 버전)의 jar.
fn jar(repo: &Path, group: &str, artifact: &str, version: &str) -> PathBuf {
    let path = repo
        .join(group)
        .join(artifact)
        .join(version)
        .join(format!("{artifact}-{version}.jar"));
    assert!(
        path.is_file(),
        "로컬 Maven 저장소에 없음: {}",
        path.display()
    );
    path
}

/// 컴파일하고, `run`이면 생성한 JUnit 4 테스트도 실행한다.
fn compile(rte: Rte, junit: JUnit, release: &str, jars: &[PathBuf], run: bool) {
    let root = std::env::temp_dir().join(format!(
        "ferrite-codegen-compile-{rte:?}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let table = ddl::parse(DDL).unwrap();
    let spec = Spec::from_table(
        &table,
        "egovframework.example.notice",
        rte,
        Db::Oracle,
        junit,
    );
    let files = crud::generate(&spec, &Layout::default(), &Part::ALL, None).unwrap();
    let mut sources = Vec::new();
    for file in &files {
        let path = root.join(&file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &file.content).unwrap();
        if path.extension().is_some_and(|e| e == "java") {
            sources.push(path);
        }
    }
    let separator = if cfg!(windows) { ";" } else { ":" };
    let classpath = jars
        .iter()
        .map(|j| j.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(separator);
    let out = root.join("classes");
    let output = Command::new("javac")
        .args([
            "-encoding",
            "UTF-8",
            "--release",
            release,
            "-Xlint:none",
            "-d",
        ])
        .arg(&out)
        .arg("-cp")
        .arg(&classpath)
        .args(&sources)
        .output()
        .expect("javac를 실행할 수 없다(JDK 필요)");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{rte:?} 컴파일 실패:\n{stderr}");
    eprintln!(
        "{rte:?}: Java 파일 {}개 컴파일 성공(--release {release})",
        sources.len()
    );
    if run {
        let output = Command::new("java")
            .arg("-cp")
            .arg(format!("{}{separator}{classpath}", out.display()))
            .args([
                "org.junit.runner.JUnitCore",
                "egovframework.example.notice.service.impl.NoticeServiceImplTest",
            ])
            .output()
            .expect("java를 실행할 수 없다");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{rte:?} 테스트 실패:\n{stdout}");
        eprintln!("{rte:?}: 생성한 테스트 통과");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "로컬 Maven 저장소(egov·Spring jar)와 JDK가 필요하다"]
fn 생성한_코드가_egov_3과_4에서_컴파일된다() {
    let repo = ferrite_egov::taglib::maven_repository().expect("로컬 Maven 저장소");
    let spring =
        |artifact: &str, version: &str| jar(&repo, "org/springframework", artifact, version);
    let v3 = vec![
        jar(
            &repo,
            "egovframework/rte",
            "egovframework.rte.fdl.cmmn",
            "3.10.0",
        ),
        jar(
            &repo,
            "egovframework/rte",
            "egovframework.rte.psl.dataaccess",
            "3.10.0",
        ),
        jar(
            &repo,
            "egovframework/rte",
            "egovframework.rte.ptl.mvc",
            "3.10.0",
        ),
        spring("spring-context", "4.3.25.RELEASE"),
        spring("spring-beans", "4.3.25.RELEASE"),
        spring("spring-core", "4.3.25.RELEASE"),
        spring("spring-web", "4.3.25.RELEASE"),
        spring("spring-webmvc", "4.3.25.RELEASE"),
        spring("spring-test", "4.2.4.RELEASE"),
        jar(&repo, "junit", "junit", "4.13.2"),
        jar(&repo, "org/hamcrest", "hamcrest-core", "1.3"),
        jar(&repo, "org/slf4j", "slf4j-api", "1.7.36"),
        // 테스트 실행용(Spring 4의 로깅).
        jar(&repo, "commons-logging", "commons-logging", "1.2"),
    ];
    compile(Rte::V3, JUnit::V4, "8", &v3, true);
    let v4 = vec![
        jar(
            &repo,
            "org/egovframe/rte",
            "org.egovframe.rte.fdl.cmmn",
            "4.3.0",
        ),
        jar(
            &repo,
            "org/egovframe/rte",
            "org.egovframe.rte.psl.dataaccess",
            "4.3.0",
        ),
        jar(
            &repo,
            "org/egovframe/rte",
            "org.egovframe.rte.ptl.mvc",
            "4.3.0",
        ),
        spring("spring-context", "5.3.39"),
        spring("spring-beans", "5.3.39"),
        spring("spring-core", "5.3.39"),
        spring("spring-web", "5.3.39"),
        spring("spring-webmvc", "5.3.39"),
        spring("spring-test", "5.3.39"),
        jar(&repo, "javax/annotation", "javax.annotation-api", "1.3.2"),
        jar(&repo, "org/junit/jupiter", "junit-jupiter-api", "5.12.2"),
        jar(&repo, "org/apiguardian", "apiguardian-api", "1.1.2"),
        jar(&repo, "org/opentest4j", "opentest4j", "1.3.0"),
        jar(
            &repo,
            "org/junit/platform",
            "junit-platform-commons",
            "1.12.2",
        ),
        jar(&repo, "org/slf4j", "slf4j-api", "2.0.17"),
    ];
    compile(Rte::V4, JUnit::V5, "11", &v4, false);
}
