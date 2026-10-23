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

use crate::catalog::{Archive, Asset, Server, Source};
use crate::client::{ServerCommand, find_program, hide_console};

pub const RECORD: &str = ".ferrite-bin";
/// 실패한 명령의 출력에서 보여 줄 마지막 줄 수.
const OUTPUT_TAIL_LINES: usize = 12;

#[derive(Debug)]
pub enum InstallError {
    /// 이 OS·CPU용 배포본이 없다.
    Unsupported,
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
    pub bin: PathBuf,
}

impl Installed {
    pub fn is_outdated(&self, server: &Server) -> bool {
        self.version != server.version
    }
}

/// 이 플랫폼용 배포 파일.
pub fn asset(server: &Server) -> Option<&'static Asset> {
    let Source::Release(assets) = server.source else {
        return None;
    };
    let fits = |want: &str, have: &str| want.is_empty() || want == have;
    assets
        .iter()
        .find(|a| fits(a.os, std::env::consts::OS) && fits(a.arch, std::env::consts::ARCH))
}

/// 이 플랫폼에서 Ferrite가 설치해 줄 수 있는가(go·python 같은 도구 유무는 설치할 때 확인한다).
pub fn installable(server: &Server) -> bool {
    !matches!(server.source, Source::Release(_)) || asset(server).is_some()
}

/// 설치본. 고정 버전을 먼저, 없으면 예전 버전(Ferrite를 올린 뒤 아직 업데이트하지 않은 것).
pub fn installed(root: &Path, server: &Server) -> Option<Installed> {
    let base = root.join(server.name);
    let read = |version: &str| {
        let dir = base.join(version);
        let bin = dir.join(fs::read_to_string(dir.join(RECORD)).ok()?.trim());
        bin.is_file().then(|| Installed {
            version: version.to_owned(),
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
pub fn command(root: &Path, server: &Server) -> Option<ServerCommand> {
    let args = server.args.iter().map(|a| (*a).to_owned());
    let Some(installed) = installed(root, server) else {
        return Some(ServerCommand {
            program: find_program(server.program)?,
            args: args.collect(),
        });
    };
    Some(match server.runtime {
        // 런타임을 못 찾으면 이름 그대로 실행해 "php 실행 실패"로 드러나게 한다.
        Some(runtime) => ServerCommand {
            program: find_program(runtime).unwrap_or_else(|| runtime.into()),
            args: std::iter::once(installed.bin.to_string_lossy().into_owned())
                .chain(args)
                .collect(),
        },
        None => ServerCommand {
            program: installed.bin,
            args: args.collect(),
        },
    })
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
        Source::Release(_) => {
            let asset = asset(server).ok_or(InstallError::Unsupported)?;
            fresh_version_dir(root, server)?;
            // 버전 폴더 옆에 둔다(버전에 점이 있어 with_extension은 못 쓴다).
            let download = root
                .join(server.name)
                .join(format!("{}.download", server.version));
            let result = fetch(agent, asset, &download, progress)
                .and_then(|()| install_release(root, server, asset, &download));
            let _ = fs::remove_file(&download);
            result
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

/// 다른 곳에서 받아 온 배포 파일로 설치한다(인터넷이 막힌 환경). 목록의 해시와 맞아야 한다.
pub fn install_from_file(root: &Path, server: &Server, file: &Path) -> Result<(), InstallError> {
    let asset = asset(server).ok_or(InstallError::Unsupported)?;
    fresh_version_dir(root, server)?;
    install_release(root, server, asset, file)
}

/// 설치본을 모두 지운다. 서버가 실행 중이면(Windows) 실패할 수 있다.
pub fn uninstall(root: &Path, server: &Server) -> io::Result<()> {
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

fn install_release(
    root: &Path,
    server: &Server,
    asset: &Asset,
    file: &Path,
) -> Result<(), InstallError> {
    let dir = version_dir(root, server);
    verify(file, asset)?;
    match asset.archive {
        Archive::Gz => {
            let mut gz = GzDecoder::new(BufReader::new(File::open(file)?));
            io::copy(&mut gz, &mut File::create(dir.join(asset.bin))?)?;
        }
        // 경로는 압축 폴더 밖으로 나가지 못하게 정리된다(zip 크레이트의 extract).
        Archive::Zip => zip::ZipArchive::new(File::open(file)?)
            .and_then(|mut zip| zip.extract(&dir))
            .map_err(io::Error::other)?,
        Archive::Plain => {
            fs::copy(file, dir.join(asset.bin))?;
        }
    }
    finish(root, server, &dir, asset.bin)
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
    let path = dir.join(bin);
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("설치했지만 실행 파일이 없습니다: {}", path.display()),
        )
        .into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    fs::write(dir.join(RECORD), bin)?;
    // 실행 중인 예전 버전은 Windows에서 지워지지 않는다. 다음 설치 때 다시 치운다.
    for entry in fs::read_dir(root.join(server.name))?.flatten() {
        if entry.file_name() != server.version && entry.path().is_dir() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
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

    /// 테스트용 서버 하나(모든 플랫폼 공통 배포본). 목록 항목은 'static이어야 해서 누수시킨다.
    fn test_server(url: String, bytes: &[u8], archive: Archive, bin: &'static str) -> Server {
        let asset = Asset {
            os: "",
            arch: "",
            url: url.leak(),
            sha256: sha256(bytes).leak(),
            size: bytes.len() as u64,
            archive,
            bin,
        };
        Server {
            name: "test-ls",
            languages: &["test"],
            version: "2.0",
            program: "test-ls-not-on-path",
            args: &["--stdio"],
            runtime: None,
            source: Source::Release(Vec::leak(vec![asset])),
        }
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
                for a in assets {
                    assert!(a.url.starts_with("https://github.com/"), "{}", a.url);
                    assert!(a.url.contains(server.version), "URL에 고정 버전: {}", a.url);
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
        assert!(for_language("toml").is_none());
    }

    #[test]
    fn zip은_해시가_맞을_때만_풀고_설치_기록을_남긴다() {
        let root = temp_root("zip");
        let bytes = zip_bytes(&[("pkg/bin/test-ls", b"#!bin"), ("pkg/lib/data.txt", b"x")]);
        let server = test_server(String::new(), &bytes, Archive::Zip, "pkg/bin/test-ls");
        let archive = root.join("download.zip");

        fs::write(&archive, b"tampered").unwrap();
        let err = install_from_file(&root, &server, &archive).unwrap_err();
        assert!(matches!(err, InstallError::Checksum { .. }), "{err}");
        assert_eq!(
            installed(&root, &server),
            None,
            "검증 실패면 설치로 치지 않는다"
        );

        fs::write(&archive, &bytes).unwrap();
        install_from_file(&root, &server, &archive).unwrap();
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
        let cmd = command(&root, &server).unwrap();
        assert_eq!(cmd.program, found.bin, "PATH보다 설치본을 먼저 쓴다");
        assert_eq!(cmd.args, ["--stdio"]);

        uninstall(&root, &server).unwrap();
        assert_eq!(installed(&root, &server), None);
        assert!(command(&root, &server).is_none(), "PATH에도 없으면 None");
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
        install_from_file(&root, &server, &archive).unwrap();
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
    fn 런타임이_필요한_서버는_런타임으로_실행한다() {
        let root = temp_root("runtime");
        let mut server = test_server(String::new(), b"<?php", Archive::Plain, "tool.phar");
        server.runtime = Some("ferrite-test-runtime");
        let archive = root.join("tool.phar");
        fs::write(&archive, b"<?php").unwrap();
        install_from_file(&root, &server, &archive).unwrap();
        let cmd = command(&root, &server).unwrap();
        assert_eq!(cmd.program, PathBuf::from("ferrite-test-runtime"));
        assert!(cmd.args[0].ends_with("tool.phar"));
        assert_eq!(cmd.args[1], "--stdio");
        fs::remove_dir_all(&root).unwrap();
    }
}
