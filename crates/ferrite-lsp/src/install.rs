//! 언어 서버 설치 관리. 목록(`catalog`)에 고정한 버전을 `<root>/<이름>/<버전>/`에 둔다.
//! 내려받은 파일은 목록의 크기·SHA-256과 맞을 때만 풀고, 모두 끝나면 마지막에 설치 기록
//! (`RECORD`: 실행 파일의 상대 경로)을 쓴다. 기록이 없는 폴더는 중간에 멈춘 설치로 보고 무시한다.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

use crate::catalog::{self, Archive, Asset, Launch, Server, Source};
use crate::client::{ServerCommand, find_program, hide_console};
use crate::java;

pub const RECORD: &str = ".ferrite-bin";
/// 서버별 프로젝트 작업 폴더를 두는 곳(`<root>/.workspaces/<이름>/`).
const WORKSPACES: &str = ".workspaces";
/// 실패한 명령의 출력에서 보여 줄 마지막 줄 수.
const OUTPUT_TAIL_LINES: usize = 12;

#[derive(Debug)]
pub enum InstallError {
    /// 이 OS·CPU용 배포본이 없다.
    Unsupported,
    /// 다른 곳에서 받아 온 파일로 설치할 때 목록의 파일이 빠졌다(파일 이름들).
    MissingFiles(String),
    /// 설치에 필요한 도구를 찾지 못했다.
    MissingTool(&'static str),
    Network(String),
    Checksum {
        expected: String,
        actual: String,
    },
    /// 설치 명령(go, pip)이 실패했다. 출력의 마지막 부분을 담는다.
    Command(String),
    Io(io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "이 OS·CPU용 배포본이 없습니다"),
            Self::MissingFiles(names) => {
                write!(f, "고른 파일 중 해시가 맞는 것이 없습니다: {names}")
            }
            Self::MissingTool(tool) => write!(f, "{tool} 명령을 찾을 수 없습니다(먼저 설치하세요)"),
            Self::Network(e) => write!(f, "내려받기 실패: {e}"),
            Self::Checksum { expected, actual } => write!(
                f,
                "파일이 목록과 다릅니다(손상 또는 변조). SHA-256 기대값 {expected}, 실제 {actual}"
            ),
            Self::Command(output) => write!(f, "설치 명령 실패: {output}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// 설치되어 있는 버전과 실행 파일.
#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    pub version: String,
    /// 버전 폴더.
    pub dir: PathBuf,
    pub bin: PathBuf,
}

impl Installed {
    pub fn is_outdated(&self, server: &Server) -> bool {
        self.version != server.version
    }
}

/// 이 플랫폼에 설치할 배포 파일들. 첫 파일이 실행 대상이다.
pub fn assets(server: &Server) -> Vec<&'static Asset> {
    let Source::Release(assets) = server.source else {
        return Vec::new();
    };
    let fits = |want: &str, have: &str| want.is_empty() || want == have;
    assets
        .iter()
        .filter(|a| fits(a.os, std::env::consts::OS) && fits(a.arch, std::env::consts::ARCH))
        .collect()
}

/// 이 플랫폼에서 Ferrite가 설치해 줄 수 있는가(go·python 같은 도구 유무는 설치할 때 확인한다).
pub fn installable(server: &Server) -> bool {
    match server.source {
        Source::Release(_) => !assets(server).is_empty(),
        Source::Go(_) | Source::Pip(_) => true,
        Source::Builtin(_) => false,
    }
}

/// 내장 서버를 띄울 Ferrite 실행 파일. 테스트 실행 파일(`ferrite-<해시>`)처럼 서버 플래그를
/// 모르는 실행 파일이면 None(그때는 PATH의 단독 실행 파일을 찾는다).
fn builtin_exe() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .filter(|exe| exe.file_stem().is_some_and(|stem| stem == "ferrite"))
}

/// 설치본. 고정 버전을 먼저, 없으면 예전 버전(Ferrite를 올린 뒤 아직 업데이트하지 않은 것).
pub fn installed(root: &Path, server: &Server) -> Option<Installed> {
    let base = root.join(server.name);
    let read = |version: &str| {
        let dir = base.join(version);
        let bin = dir.join(fs::read_to_string(dir.join(RECORD)).ok()?.trim());
        bin.is_file().then(|| Installed {
            version: version.to_owned(),
            dir,
            bin,
        })
    };
    read(server.version).or_else(|| {
        fs::read_dir(&base)
            .ok()?
            .flatten()
            .find_map(|entry| read(&entry.file_name().to_string_lossy()))
    })
}

/// 서버 실행 명령. Ferrite가 설치한 것이 있으면 그것을, 없으면 PATH에서 찾는다.
/// 어디에도 없으면 None, 있지만 띄울 수 없으면(런타임이 없거나 낮음) 그 이유.
/// `project`는 프로젝트 루트로, 프로젝트별 작업 폴더가 필요한 서버(jdt.ls)가 쓴다.
pub fn command(
    root: &Path,
    server: &Server,
    project: &Path,
) -> Option<Result<ServerCommand, String>> {
    let mut args: Vec<String> = server.args.iter().map(|a| (*a).to_owned()).collect();
    if let Source::Builtin(flag) = server.source
        && let Some(program) = builtin_exe()
    {
        args.insert(0, flag.to_owned());
        return Some(Ok(ServerCommand {
            program,
            args,
            ..Default::default()
        }));
    }
    let Some(installed) = installed(root, server) else {
        let program = find_program(server.program)?;
        if server.launch == Launch::Jdtls {
            // 직접 설치한 jdtls 스크립트도 작업 폴더는 Ferrite가 정한다(Java·Lombok은 스크립트 몫).
            args.push("-data".into());
            args.push(
                workspace_dir(root, server, project)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        return Some(Ok(ServerCommand {
            program,
            args,
            ..Default::default()
        }));
    };
    Some(match server.launch {
        Launch::Binary => Ok(ServerCommand {
            program: installed.bin,
            args,
            ..Default::default()
        }),
        Launch::Runtime(runtime) => find_program(runtime)
            .map(|program| ServerCommand {
                program,
                args: std::iter::once(installed.bin.to_string_lossy().into_owned())
                    .chain(args)
                    .collect(),
                ..Default::default()
            })
            .ok_or_else(|| {
                format!(
                    "{}을(를) 실행하려면 {runtime}이(가) 필요합니다",
                    server.name
                )
            }),
        Launch::Jdtls => java::find_java().map(|(java, major)| {
            let data = workspace_dir(root, server, project);
            // Spring Boot LS가 설치되어 있으면 그 확장 번들을 실어야 중계가 된다.
            let bundles = catalog::by_launch(Launch::SpringBoot)
                .and_then(|spring| crate::install::installed(root, spring))
                .map(|spring| java::spring_bundles(&spring.dir))
                .unwrap_or_default();
            java::jdtls_command(java, major, &installed.dir, &installed.bin, &data, &bundles)
        }),
        Launch::SpringBoot => java::find_java().map(|(java, _)| {
            let log = workspace_dir(root, server, project).join("server.log");
            java::spring_boot_command(java, &installed.bin, &log, project)
        }),
    })
}

/// 프로젝트별 작업 폴더(jdt.ls의 색인 등). 설치 폴더와 따로 두어 업데이트해도 남는다.
fn workspace_dir(root: &Path, server: &Server, project: &Path) -> PathBuf {
    let hash: String = Sha256::digest(project.to_string_lossy().as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let name = project
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    root.join(WORKSPACES)
        .join(server.name)
        .join(format!("{name}-{hash}"))
}

/// 고정 버전을 설치한다. 오래 걸리므로 UI 스레드가 아닌 곳에서 부른다.
/// `progress(받은 바이트, 전체 바이트)`는 내려받는 동안만 불린다.
pub fn install(
    root: &Path,
    server: &Server,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), InstallError> {
    install_with(&agent(), root, server, progress)
}

fn install_with(
    agent: &Agent,
    root: &Path,
    server: &Server,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), InstallError> {
    match server.source {
        Source::Builtin(_) => Err(InstallError::Unsupported),
        Source::Release(_) => {
            let assets = assets(server);
            let main = assets.first().ok_or(InstallError::Unsupported)?;
            let dir = fresh_version_dir(root, server)?;
            // 버전 폴더 옆에 둔다(버전에 점이 있어 with_extension은 못 쓴다).
            let download = root
                .join(server.name)
                .join(format!("{}.download", server.version));
            let total: u64 = assets.iter().map(|a| a.size).sum();
            let mut done = 0;
            for asset in &assets {
                let result = fetch(agent, asset, &download, &mut |received, _| {
                    progress(done + received, total)
                })
                .and_then(|()| verify(&download, asset))
                .and_then(|()| unpack(asset, &download, &dir));
                let _ = fs::remove_file(&download);
                result?;
                done += asset.size;
            }
            finish(root, server, &dir, main.bin)
        }
        Source::Go(module) => {
            let go = find_program("go").ok_or(InstallError::MissingTool("go"))?;
            let dir = fresh_version_dir(root, server)?;
            run(Command::new(go)
                .args(["install", &format!("{module}@{}", server.version)])
                .env("GOBIN", &dir))?;
            finish(
                root,
                server,
                &dir,
                &format!("{}{}", server.program, std::env::consts::EXE_SUFFIX),
            )
        }
        Source::Pip(package) => {
            let python = python().ok_or(InstallError::MissingTool("python"))?;
            // 가상환경은 옮기면 깨지므로 처음부터 최종 위치에 만든다.
            let dir = fresh_version_dir(root, server)?;
            run(Command::new(python).args(["-m", "venv"]).arg(&dir))?;
            let scripts = if cfg!(windows) { "Scripts" } else { "bin" };
            let exe = std::env::consts::EXE_SUFFIX;
            run(Command::new(dir.join(scripts).join(format!("python{exe}")))
                .args([
                    "-m",
                    "pip",
                    "install",
                    "--disable-pip-version-check",
                    "--no-input",
                ])
                .arg(format!("{package}=={}", server.version)))?;
            finish(
                root,
                server,
                &dir,
                &format!("{scripts}/{}{exe}", server.program),
            )
        }
    }
}

/// 다른 곳에서 받아 온 배포 파일들로 설치한다(인터넷이 막힌 환경). 목록의 파일마다 크기와
/// 해시가 같은 파일이 하나씩 있어야 한다. 파일 이름은 보지 않는다.
pub fn install_from_files(
    root: &Path,
    server: &Server,
    files: &[PathBuf],
) -> Result<(), InstallError> {
    let assets = assets(server);
    let main = assets.first().ok_or(InstallError::Unsupported)?;
    let mut matched = Vec::new();
    let mut missing = Vec::new();
    for asset in &assets {
        match files.iter().find(|f| verify(f, asset).is_ok()) {
            Some(file) => matched.push((*asset, file)),
            None => missing.push(asset.url.rsplit('/').next().unwrap_or(asset.url)),
        }
    }
    if !missing.is_empty() {
        return Err(InstallError::MissingFiles(missing.join(", ")));
    }
    let dir = fresh_version_dir(root, server)?;
    for (asset, file) in matched {
        unpack(asset, file, &dir)?;
    }
    finish(root, server, &dir, main.bin)
}

/// 설치본을 모두 지운다. 서버가 실행 중이면(Windows) 실패할 수 있다.
pub fn uninstall(root: &Path, server: &Server) -> io::Result<()> {
    // 작업 폴더는 캐시라 지우는 데 실패해도 상관없다.
    let _ = fs::remove_dir_all(root.join(WORKSPACES).join(server.name));
    match fs::remove_dir_all(root.join(server.name)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn agent() -> Agent {
    Agent::config_builder()
        .https_only(true)
        // 회사망의 TLS 검사 프록시처럼 OS에 추가된 루트 인증서도 믿도록 OS 검증기를 쓴다.
        // 프록시 자체는 HTTPS_PROXY 등 환경 변수에서 읽는다(ureq 기본).
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .user_agent(concat!("ferrite/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// 내려받는다. 목록의 크기보다 많이 오면 거기서 멈춘다(검증에서 걸린다).
fn fetch(
    agent: &Agent,
    asset: &Asset,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), InstallError> {
    let network = |e: &dyn fmt::Display| InstallError::Network(e.to_string());
    let mut response = agent.get(asset.url).call().map_err(|e| network(&e))?;
    let mut body = response.body_mut().as_reader().take(asset.size + 1);
    let mut file = File::create(dest)?;
    let mut buf = vec![0; 64 * 1024];
    let mut received = 0;
    loop {
        let n = body.read(&mut buf).map_err(|e| network(&e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        received += n as u64;
        progress(received, asset.size);
    }
    file.sync_all()?;
    Ok(())
}

/// 검증한 배포 파일을 버전 폴더에 푼다. 경로는 폴더 밖으로 나가지 못하게 정리된다
/// (zip·tar 크레이트의 extract·unpack).
fn unpack(asset: &Asset, file: &Path, dir: &Path) -> Result<(), InstallError> {
    let open = || File::open(file).map(BufReader::new);
    match asset.archive {
        Archive::Gz => {
            io::copy(
                &mut GzDecoder::new(open()?),
                &mut File::create(dir.join(asset.bin))?,
            )?;
        }
        Archive::Zip => zip::ZipArchive::new(File::open(file)?)
            .and_then(|mut zip| zip.extract(dir))
            .map_err(io::Error::other)?,
        Archive::TarGz => tar::Archive::new(GzDecoder::new(open()?)).unpack(dir)?,
        Archive::Plain => {
            fs::copy(file, dir.join(asset.bin))?;
        }
    }
    Ok(())
}

fn verify(file: &Path, asset: &Asset) -> Result<(), InstallError> {
    let mut reader = File::open(file)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let mut size = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if size == asset.size && actual == asset.sha256 {
        Ok(())
    } else {
        Err(InstallError::Checksum {
            expected: asset.sha256.to_owned(),
            actual,
        })
    }
}

fn version_dir(root: &Path, server: &Server) -> PathBuf {
    root.join(server.name).join(server.version)
}

/// 같은 버전의 이전(실패한) 설치 흔적을 지우고 빈 폴더를 만든다.
fn fresh_version_dir(root: &Path, server: &Server) -> io::Result<PathBuf> {
    let dir = version_dir(root, server);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 실행 파일을 확인하고 설치 기록을 쓴 뒤 예전 버전을 치운다.
fn finish(root: &Path, server: &Server, dir: &Path, bin: &str) -> Result<(), InstallError> {
    let Some(bin) = resolve_bin(dir, bin) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "설치했지만 실행 파일이 없습니다: {}",
                dir.join(bin).display()
            ),
        )
        .into());
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.join(&bin), fs::Permissions::from_mode(0o755))?;
    }
    fs::write(dir.join(RECORD), &bin)?;
    // 실행 중인 예전 버전은 Windows에서 지워지지 않는다. 다음 설치 때 다시 치운다.
    for entry in fs::read_dir(root.join(server.name))?.flatten() {
        if entry.file_name() != server.version && entry.path().is_dir() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

/// 설치 폴더 기준 실행 파일 경로. 파일 이름의 `*` 하나는 실제 파일 이름으로 바꾼다.
fn resolve_bin(dir: &Path, bin: &str) -> Option<String> {
    let Some((head, suffix)) = bin.split_once('*') else {
        return dir.join(bin).is_file().then(|| bin.to_owned());
    };
    let (parent, prefix) = head.rsplit_once('/').unwrap_or(("", head));
    fs::read_dir(dir.join(parent))
        .ok()?
        .flatten()
        .find_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let fits = name.starts_with(prefix) && name.ends_with(suffix) && entry.path().is_file();
            fits.then(|| match parent {
                "" => name,
                _ => format!("{parent}/{name}"),
            })
        })
}

fn python() -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) {
        &["py", "python", "python3"]
    } else {
        &["python3", "python"]
    };
    candidates.iter().find_map(|name| find_program(name))
}

fn run(command: &mut Command) -> Result<(), InstallError> {
    hide_console(command);
    let output = command.stdin(Stdio::null()).output()?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr) + String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(OUTPUT_TAIL_LINES)..].join("\n");
    Err(InstallError::Command(format!(
        "{} ({})",
        tail, output.status
    )))
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Write};
    use std::net::TcpListener;
    use std::thread;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;
    use crate::catalog::{SERVERS, for_language};

    fn temp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ferrite-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// 모든 플랫폼 공통 배포 파일 하나. 목록 항목은 'static이어야 해서 누수시킨다.
    fn test_asset(url: String, bytes: &[u8], archive: Archive, bin: &'static str) -> Asset {
        Asset {
            os: "",
            arch: "",
            url: url.leak(),
            sha256: sha256(bytes).leak(),
            size: bytes.len() as u64,
            archive,
            bin,
        }
    }

    fn server_with(assets: Vec<Asset>) -> Server {
        Server {
            name: "test-ls",
            languages: &["test"],
            version: "2.0",
            program: "test-ls-not-on-path",
            args: &["--stdio"],
            launch: Launch::Binary,
            source: Source::Release(Vec::leak(assets)),
        }
    }

    fn test_server(url: String, bytes: &[u8], archive: Archive, bin: &'static str) -> Server {
        server_with(vec![test_asset(url, bytes, archive, bin)])
    }

    fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        for (name, data) in files {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn 목록은_https_고정_해시와_언어를_갖춘다() {
        for server in SERVERS {
            assert!(!server.languages.is_empty(), "{}", server.name);
            if let Source::Release(assets) = server.source {
                // 첫 파일(서버 본체)의 주소에는 고정 버전이 들어 있어야 한다.
                assert!(assets[0].url.contains(server.version), "{}", assets[0].url);
                for a in assets {
                    let hosts = [
                        "https://github.com/",
                        "https://download.eclipse.org/",
                        "https://repo1.maven.org/",
                        "https://open-vsx.org/",
                    ];
                    assert!(hosts.iter().any(|h| a.url.starts_with(h)), "{}", a.url);
                    assert_eq!(a.sha256.len(), 64, "{}", a.url);
                    assert!(
                        a.sha256
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    );
                    assert!(a.size > 0 && !a.bin.is_empty());
                }
            }
        }
        let names: std::collections::HashSet<_> = SERVERS.iter().map(|s| s.name).collect();
        assert_eq!(
            names.len(),
            SERVERS.len(),
            "이름(설치 폴더)이 겹치면 안 된다"
        );
        assert_eq!(for_language("cpp").map(|s| s.name), Some("clangd"));
        assert_eq!(for_language("xml").map(|s| s.name), Some("lemminx"));
        assert_eq!(
            for_language("java").map(|s| s.name),
            Some("jdtls"),
            "주 서버"
        );
        let java: Vec<_> = crate::catalog::servers_for("java")
            .map(|s| s.name)
            .collect();
        assert_eq!(java, ["jdtls", "spring-boot"]);
        assert_eq!(
            for_language("spring-boot-properties-yaml").map(|s| s.name),
            Some("spring-boot")
        );
        assert!(for_language("toml").is_none());
    }

    #[test]
    fn zip은_해시가_맞을_때만_풀고_설치_기록을_남긴다() {
        let root = temp_root("zip");
        let bytes = zip_bytes(&[("pkg/bin/test-ls", b"#!bin"), ("pkg/lib/data.txt", b"x")]);
        let server = test_server(String::new(), &bytes, Archive::Zip, "pkg/bin/test-ls");
        let archive = root.join("download.zip");

        fs::write(&archive, b"tampered").unwrap();
        let err = install_from_files(&root, &server, std::slice::from_ref(&archive)).unwrap_err();
        assert!(matches!(err, InstallError::MissingFiles(_)), "{err}");
        assert_eq!(
            installed(&root, &server),
            None,
            "해시가 맞지 않으면 설치로 치지 않는다"
        );

        fs::write(&archive, &bytes).unwrap();
        install_from_files(&root, &server, std::slice::from_ref(&archive)).unwrap();
        let found = installed(&root, &server).unwrap();
        assert_eq!(found.version, "2.0");
        assert!(found.bin.ends_with("pkg/bin/test-ls"));
        assert!(
            found
                .bin
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("lib/data.txt")
                .is_file()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&found.bin).unwrap().permissions().mode() & 0o111,
                0o111
            );
        }
        let cmd = command(&root, &server, &root).unwrap().unwrap();
        assert_eq!(cmd.program, found.bin, "PATH보다 설치본을 먼저 쓴다");
        assert_eq!(cmd.args, ["--stdio"]);

        uninstall(&root, &server).unwrap();
        assert_eq!(installed(&root, &server), None);
        assert!(
            command(&root, &server, &root).is_none(),
            "PATH에도 없으면 None"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn 예전_버전은_업데이트로_보이고_새로_설치하면_치운다() {
        let root = temp_root("update");
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(b"server binary").unwrap();
        let bytes = gz.finish().unwrap();
        let server = test_server(String::new(), &bytes, Archive::Gz, "test-ls");

        let old = root.join("test-ls/1.0");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("old-bin"), b"").unwrap();
        fs::write(old.join(RECORD), "old-bin").unwrap();
        let found = installed(&root, &server).unwrap();
        assert!(found.is_outdated(&server));

        let archive = root.join("download.gz");
        fs::write(&archive, &bytes).unwrap();
        install_from_files(&root, &server, std::slice::from_ref(&archive)).unwrap();
        let found = installed(&root, &server).unwrap();
        assert!(!found.is_outdated(&server));
        assert_eq!(fs::read(&found.bin).unwrap(), b"server binary");
        assert!(!old.exists(), "새 버전을 설치하면 예전 버전을 지운다");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn 내려받기는_진행률을_알리고_크기를_넘으면_검증에서_막는다() {
        let payload = b"plain script".to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/test-ls.phar", listener.local_addr().unwrap());
        // 두 번 응답한다: 정상 본문, 그다음 목록보다 긴 본문.
        let served = payload.clone();
        let server_thread = thread::spawn(move || {
            for body in [served.clone(), [served, b"-extra".to_vec()].concat()] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 2 {
                    line.clear();
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let root = temp_root("fetch");
        let server = test_server(url, &payload, Archive::Plain, "test-ls.phar");
        let agent: Agent = Agent::config_builder().build().into();

        let mut seen = Vec::new();
        install_with(&agent, &root, &server, &mut |done, total| {
            seen.push((done, total))
        })
        .unwrap();
        assert_eq!(
            seen.last(),
            Some(&(payload.len() as u64, payload.len() as u64))
        );
        assert_eq!(
            fs::read(installed(&root, &server).unwrap().bin).unwrap(),
            payload
        );
        assert!(
            !root.join("test-ls/2.0.download").exists(),
            "내려받은 임시 파일은 지운다"
        );

        let err = install_with(&agent, &root, &server, &mut |_, _| {}).unwrap_err();
        assert!(matches!(err, InstallError::Checksum { .. }), "{err}");
        server_thread.join().unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn 런타임이_필요한_서버는_런타임으로_실행하고_없으면_이유를_알린다() {
        let root = temp_root("runtime");
        let mut server = test_server(String::new(), b"<?php", Archive::Plain, "tool.phar");
        let archive = root.join("tool.phar");
        fs::write(&archive, b"<?php").unwrap();
        install_from_files(&root, &server, std::slice::from_ref(&archive)).unwrap();

        // 어느 PC에나 있는 프로그램을 런타임 삼아 확인한다.
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        server.launch = Launch::Runtime(shell);
        let cmd = command(&root, &server, &root).unwrap().unwrap();
        assert_eq!(cmd.program, find_program(shell).unwrap());
        assert!(cmd.args[0].ends_with("tool.phar"));
        assert_eq!(cmd.args[1], "--stdio");

        server.launch = Launch::Runtime("ferrite-test-runtime");
        let err = command(&root, &server, &root).unwrap().unwrap_err();
        assert!(err.contains("ferrite-test-runtime"), "{err}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn 여러_파일을_모두_풀고_별표_이름을_실제_jar로_기록한다() {
        let root = temp_root("multi");
        let launcher = b"launcher";
        let mut header = tar::Header::new_gnu();
        header.set_size(launcher.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        builder
            .append_data(
                &mut header,
                "plugins/org.eclipse.equinox.launcher_1.7.0.v2026.jar",
                &launcher[..],
            )
            .unwrap();
        let tar_gz = builder.into_inner().unwrap().finish().unwrap();
        let agent = b"lombok agent";
        let server = server_with(vec![
            test_asset(
                "https://example.invalid/server.tar.gz".into(),
                &tar_gz,
                Archive::TarGz,
                "plugins/org.eclipse.equinox.launcher_*.jar",
            ),
            test_asset(
                "https://example.invalid/lombok-1.0.jar".into(),
                agent,
                Archive::Plain,
                "lombok.jar",
            ),
        ]);
        let (tar_file, jar_file) = (root.join("a.bin"), root.join("b.bin"));
        fs::write(&tar_file, &tar_gz).unwrap();
        fs::write(&jar_file, agent).unwrap();

        let err = install_from_files(&root, &server, std::slice::from_ref(&tar_file)).unwrap_err();
        assert!(
            matches!(&err, InstallError::MissingFiles(names) if names == "lombok-1.0.jar"),
            "{err}"
        );
        // 파일 이름·순서와 상관없이 해시로 짝을 찾는다.
        install_from_files(&root, &server, &[jar_file, tar_file]).unwrap();
        let found = installed(&root, &server).unwrap();
        assert!(
            found
                .bin
                .ends_with("plugins/org.eclipse.equinox.launcher_1.7.0.v2026.jar")
        );
        assert_eq!(fs::read(found.dir.join("lombok.jar")).unwrap(), agent);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn jdtls는_프로젝트마다_다른_작업_폴더로_띄운다() {
        let root = temp_root("jdtls");
        let mut server = test_server(String::new(), b"jar", Archive::Plain, "launcher.jar");
        server.launch = Launch::Jdtls;
        let file = root.join("launcher.jar");
        fs::write(&file, b"jar").unwrap();
        install_from_files(&root, &server, std::slice::from_ref(&file)).unwrap();

        let data_of = |project: &str| {
            command(&root, &server, Path::new(project))
                .unwrap()
                .map(|cmd| {
                    let at = cmd.args.iter().position(|a| a == "-data").unwrap();
                    PathBuf::from(&cmd.args[at + 1])
                })
        };
        match (data_of("/work/egov-portal"), data_of("/work/other")) {
            (Ok(a), Ok(b)) => {
                assert!(a.starts_with(root.join(WORKSPACES).join("test-ls")));
                assert!(
                    a.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("egov-portal-")
                );
                assert_ne!(a, b);
            }
            // 이 PC에 Java 21 이상이 없으면 이유를 알려야 한다.
            (Err(e), _) | (_, Err(e)) => assert!(e.contains("21"), "{e}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
