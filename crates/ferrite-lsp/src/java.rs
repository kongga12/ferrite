//! Java 언어 서버 실행.
//! - jdt.ls: 공식 실행 스크립트(`bin/jdtls.py`)와 같은 JVM 인자로 equinox 런처를 띄우고, 설치 폴더에
//!   lombok.jar가 있으면 에이전트로 붙인다. Spring Boot LS가 있으면 그 jdt.ls 확장 번들을 싣는다.
//! - Spring Boot LS: VS Code 확장(vscode-spring-boot)과 같은 JVM 인자로 띄운다.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::client::{ServerCommand, find_program, hide_console};
use crate::uri::path_to_uri;

/// 목록에 고정한 jdt.ls(1.61.0)를 띄우는 데 필요한 최소 Java 버전. 프로젝트가 쓰는 Java(egov는
/// 8·11도 흔하다)와는 별개로, 서버 프로세스만 이 버전 이상이면 된다. jdt.ls 1.62부터는 25다.
pub const MIN_JAVA: u32 = 21;
pub const LOMBOK_JAR: &str = "lombok.jar";
/// Spring Boot LS가 jdt.ls에 싣는 확장 번들(vscode-spring-boot의 `javaExtensions`). vsix를 푼
/// 폴더 기준. 이 번들이 있어야 jdt.ls가 `sts.java.*` 명령(클래스패스·타입 조회)을 처리한다.
const SPRING_BUNDLES: [&str; 5] = [
    "extension/jars/io.projectreactor.reactor-core.jar",
    "extension/jars/org.reactivestreams.reactive-streams.jar",
    "extension/jars/jdt-ls-commons.jar",
    "extension/jars/jdt-ls-extension.jar",
    "extension/jars/sts-gradle-tooling.jar",
];

/// `java -version` 출력에서 주 버전. `1.8.0_402` → 8, `21.0.11` → 21.
pub fn parse_java_version(output: &str) -> Option<u32> {
    let start = output.find("version \"")? + "version \"".len();
    let mut parts = output[start..].split(|c: char| !c.is_ascii_digit());
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

/// jdt.ls를 띄울 Java와 그 주 버전. JAVA_HOME, PATH 순으로 보고 21 이상인 첫 것을 고른다.
/// `java -version`을 실행해 확인하므로 서버를 띄울 때 한 번만 부른다.
pub fn find_java() -> Result<(PathBuf, u32), String> {
    let exe = format!("java{}", std::env::consts::EXE_SUFFIX);
    let from_home = std::env::var_os("JAVA_HOME")
        .map(|home| PathBuf::from(home).join("bin").join(&exe))
        .filter(|p| p.is_file());
    let mut too_old = Vec::new();
    for java in from_home.into_iter().chain(find_program("java")) {
        match java_version(&java) {
            Some(major) if major >= MIN_JAVA => return Ok((java, major)),
            Some(major) => too_old.push(format!("{} (Java {major})", java.display())),
            None => {}
        }
    }
    Err(if too_old.is_empty() {
        format!("jdt.ls를 실행할 Java {MIN_JAVA} 이상을 찾지 못했습니다(JAVA_HOME, PATH)")
    } else {
        format!(
            "jdt.ls는 Java {MIN_JAVA} 이상으로 실행해야 합니다. 찾은 것: {}. \
             JAVA_HOME을 Java {MIN_JAVA} 이상으로 지정하세요(프로젝트는 예전 Java여도 됩니다)",
            too_old.join(", ")
        )
    })
}

fn java_version(java: &Path) -> Option<u32> {
    let mut command = Command::new(java);
    command.arg("-version").stdin(Stdio::null());
    hide_console(&mut command);
    let output = command.output().ok()?;
    // `-version`은 stderr로 출력한다.
    parse_java_version(&String::from_utf8_lossy(&output.stderr))
}

/// Spring Boot LS 설치 폴더에서 jdt.ls에 실을 번들. 하나라도 없으면 빈 목록(반쪽만 실으면
/// jdt.ls가 번들을 못 풀어 오류를 낸다).
pub fn spring_bundles(spring_dir: &Path) -> Vec<PathBuf> {
    let bundles: Vec<PathBuf> = SPRING_BUNDLES.iter().map(|b| spring_dir.join(b)).collect();
    if bundles.iter().all(|b| b.is_file()) {
        bundles
    } else {
        Vec::new()
    }
}

/// 설치 폴더 `dir`의 jdt.ls를 띄우는 명령. `data`는 프로젝트별 작업 폴더(색인, 빌드 상태).
/// `bundles`는 함께 실을 jdt.ls 확장(Spring Boot LS 중계용).
pub fn jdtls_command(
    java: PathBuf,
    java_major: u32,
    dir: &Path,
    launcher: &Path,
    data: &Path,
    bundles: &[PathBuf],
) -> ServerCommand {
    let config = match std::env::consts::OS {
        "windows" => "config_win",
        "macos" => "config_mac",
        _ => "config_linux",
    };
    let mut args: Vec<String> = Vec::new();
    if java_major >= 24 {
        // JDK 24부터 XML 엔티티 크기 제한이 생겨 일부 Maven POM을 읽지 못한다(공식 스크립트와 같게).
        args.extend(
            [
                "-Djdk.xml.maxGeneralEntitySizeLimit=0",
                "-Djdk.xml.totalEntitySizeLimit=0",
            ]
            .map(String::from),
        );
    }
    args.extend(
        [
            "-Declipse.application=org.eclipse.jdt.ls.core.id1",
            "-Dosgi.bundles.defaultStartLevel=4",
            "-Declipse.product=org.eclipse.jdt.ls.core.product",
            "-Dosgi.checkConfiguration=true",
        ]
        .map(String::from),
    );
    // 설치 폴더의 설정은 읽기 전용으로 여러 프로젝트가 함께 쓴다.
    args.push(format!(
        "-Dosgi.sharedConfiguration.area={}",
        dir.join(config).display()
    ));
    args.extend(
        [
            "-Dosgi.sharedConfiguration.area.readOnly=true",
            "-Dosgi.configuration.cascaded=true",
            // ponytail: 힙은 vscode-java 기본값. 큰 프로젝트용 조정은 필요해지면 settings.toml로.
            "-Xms100m",
            "-Xmx2G",
            "--add-modules=ALL-SYSTEM",
            "--add-opens",
            "java.base/java.util=ALL-UNNAMED",
            "--add-opens",
            "java.base/java.lang=ALL-UNNAMED",
        ]
        .map(String::from),
    );
    let lombok = dir.join(LOMBOK_JAR);
    if lombok.is_file() {
        args.push(format!("-javaagent:{}", lombok.display()));
    }
    args.push("-jar".into());
    args.push(launcher.to_string_lossy().into_owned());
    args.push("-data".into());
    args.push(data.to_string_lossy().into_owned());
    let init_options = if bundles.is_empty() {
        Value::Null
    } else {
        json!({ "bundles": bundles })
    };
    ServerCommand {
        program: java,
        args,
        init_options,
    }
}

/// Spring Boot LS를 띄우는 명령. 로그는 `log`로 보낸다(표준 출력은 LSP 통신에 쓰므로).
/// `project`는 작업 폴더로 알린다. jdt.ls 클래스패스 연동은 시작한 뒤 클라이언트가 켠다.
pub fn spring_boot_command(java: PathBuf, jar: &Path, log: &Path, project: &Path) -> ServerCommand {
    let args = [
        // ponytail: 힙은 VS Code 확장 기본값. 큰 프로젝트용 조정은 필요해지면 settings.toml로.
        "-Xmx1024m".to_owned(),
        "-Dsts.lsp.client=vscode".into(),
        "-Dspring.config.location=classpath:/application.properties".into(),
        "-Djdk.util.zip.disableZip64ExtraFieldValidation=true".into(),
        "-Dspring.main.web-application-type=NONE".into(),
        "-Dspring.profiles.active=file-logging".into(),
        format!("-Dlogging.file.name={}", log.display()),
        "-Dlogging.level.root=warn".into(),
        "-Xlog:jni+resolve=off".into(),
        "-jar".into(),
        jar.to_string_lossy().into_owned(),
    ];
    let folders: Vec<_> = path_to_uri(project).into_iter().collect();
    ServerCommand {
        program: java,
        args: args.to_vec(),
        init_options: json!({ "workspaceFolders": folders, "enableJdtClasspath": false }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn java_버전_출력에서_주_버전을_읽는다() {
        let jdk8 = "openjdk version \"1.8.0_402\"\nOpenJDK Runtime Environment (Temurin)(build 1.8.0_402-b06)";
        let jdk21 = "openjdk version \"21.0.11\" 2026-04-21 LTS\nOpenJDK Runtime Environment";
        let jdk25 = "java version \"25\" 2025-09-16\n";
        assert_eq!(parse_java_version(jdk8), Some(8));
        assert_eq!(parse_java_version(jdk21), Some(21));
        assert_eq!(parse_java_version(jdk25), Some(25));
        assert_eq!(
            parse_java_version("Picked up JAVA_TOOL_OPTIONS: -Dx=1\nopenjdk version \"17.0.9\""),
            Some(17)
        );
        assert_eq!(parse_java_version("명령을 찾을 수 없음"), None);
    }

    #[test]
    fn lombok이_있으면_에이전트로_붙이고_런처와_작업_폴더를_넘긴다() {
        let dir = std::env::temp_dir().join(format!("ferrite-jdtls-args-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let launcher = dir.join("plugins/org.eclipse.equinox.launcher_1.7.0.jar");
        let data = dir.join("data");

        let without = jdtls_command("java".into(), 21, &dir, &launcher, &data, &[]);
        assert!(!without.args.iter().any(|a| a.starts_with("-javaagent:")));
        assert!(
            !without
                .args
                .iter()
                .any(|a| a.contains("maxGeneralEntitySizeLimit"))
        );
        let config = without
            .args
            .iter()
            .find(|a| a.starts_with("-Dosgi.sharedConfiguration.area="))
            .unwrap();
        assert!(
            config.ends_with("config_win")
                || config.ends_with("config_mac")
                || config.ends_with("config_linux")
        );
        let tail = &without.args[without.args.len() - 4..];
        assert_eq!(tail[0], "-jar");
        assert_eq!(tail[1], launcher.to_string_lossy());
        assert_eq!(tail[2], "-data");
        assert_eq!(tail[3], data.to_string_lossy());

        fs::write(dir.join(LOMBOK_JAR), b"").unwrap();
        let with = jdtls_command(
            "java".into(),
            25,
            &dir,
            &launcher,
            &data,
            &[dir.join("b.jar")],
        );
        let agent = with
            .args
            .iter()
            .position(|a| a.starts_with("-javaagent:"))
            .unwrap();
        assert!(with.args[agent].ends_with(LOMBOK_JAR));
        assert!(
            agent < with.args.iter().position(|a| a == "-jar").unwrap(),
            "에이전트는 JVM 인자라 -jar 앞"
        );
        assert!(
            with.args
                .iter()
                .any(|a| a.contains("maxGeneralEntitySizeLimit")),
            "JDK 24 이상"
        );
        assert!(
            without.init_options.is_null(),
            "번들이 없으면 초기화 옵션도 없다"
        );
        let bundles = with.init_options["bundles"].as_array().unwrap();
        assert!(bundles[0].as_str().unwrap().ends_with("b.jar"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn spring_번들은_다섯_개가_모두_있을_때만_싣는다() {
        let dir =
            std::env::temp_dir().join(format!("ferrite-spring-bundles-{}", std::process::id()));
        fs::create_dir_all(dir.join("extension/jars")).unwrap();
        for bundle in &SPRING_BUNDLES[..4] {
            fs::write(dir.join(bundle), b"").unwrap();
        }
        assert!(
            spring_bundles(&dir).is_empty(),
            "하나라도 빠지면 싣지 않는다"
        );
        fs::write(dir.join(SPRING_BUNDLES[4]), b"").unwrap();
        assert_eq!(spring_bundles(&dir).len(), 5);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn spring_boot_ls는_로그를_파일로_보내고_작업_폴더를_알린다() {
        let project = std::env::temp_dir().join("demo");
        let cmd = spring_boot_command(
            "java".into(),
            Path::new("ls/spring-boot-language-server.jar"),
            Path::new("logs/server.log"),
            &project,
        );
        let tail = &cmd.args[cmd.args.len() - 2..];
        assert_eq!(tail[0], "-jar");
        assert!(tail[1].ends_with("spring-boot-language-server.jar"));
        assert!(
            cmd.args
                .iter()
                .any(|a| a.starts_with("-Dlogging.file.name=") && a.ends_with("server.log"))
        );
        assert_eq!(cmd.init_options["enableJdtClasspath"], false);
        let folder = cmd.init_options["workspaceFolders"][0].as_str().unwrap();
        assert!(
            folder.starts_with("file://") && folder.ends_with("/demo"),
            "{folder}"
        );
    }
}
