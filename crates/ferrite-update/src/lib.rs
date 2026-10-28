//! 자동 업데이트. 릴리스의 매니페스트(`latest.json`)로 새 판을 알아보고, 받은 설치 파일은 minisign
//! 공개 키로 서명을 검증한 뒤에만 쓴다. 매니페스트·서명 형식은 cargo-packager 업데이터와 같다:
//! 공개 키는 `cargo packager signer generate`가, 서명(`.sig`)은 `cargo packager --private-key`가 만든다.
//!
//! 매니페스트 자체에는 서명이 없다. 설치 파일만 검증하므로, 매니페스트를 바꾼 사람은 서명된 다른 판을
//! 가리키게 할 수 있을 뿐이다.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

/// 공식 릴리스의 매니페스트(가장 최근 릴리스에 올린 `latest.json`).
pub const MANIFEST_URL: &str =
    "https://github.com/kongga12/ferrite/releases/latest/download/latest.json";
/// 이 플랫폼의 설치 파일이 매니페스트에 없을 때 여는 내려받기 페이지.
pub const RELEASES_PAGE: &str = "https://github.com/kongga12/ferrite/releases/latest";
/// 받는 파일의 최대 크기. 잘못된 서버가 끝없이 보내는 것을 막는다.
const MAX_INSTALLER: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;

/// 설치 파일 형식(매니페스트의 `format`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Nsis,
    AppImage,
    /// 앱이 직접 설치하지 않는 형식(msi, dmg, deb 등). 내려받기 링크만 보인다.
    Other(String),
}

/// 이 플랫폼의 설치 파일.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub url: String,
    /// `.sig` 파일 내용(minisign 서명 글을 base64로 감싼 것).
    pub signature: String,
    pub format: Format,
}

/// 지금 판보다 새 판.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    /// 이 플랫폼의 설치 파일. 없으면 내려받기 페이지만 보인다.
    pub asset: Option<Asset>,
}

/// 이 실행 파일을 새 판으로 바꾸는 방법.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installer {
    /// NSIS로 설치한 Windows(실행 파일 옆에 `uninstall.exe`). 앱이 끝난 뒤 설치 파일을 돌린다.
    Nsis,
    /// AppImage(환경 변수 `APPIMAGE`가 가리키는 파일). 그 파일을 바꾼다.
    AppImage(PathBuf),
}

/// 매니페스트의 플랫폼 키(`OS-아키텍처`): windows-x86_64, linux-x86_64, macos-aarch64 등.
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// `candidate`가 `current`보다 새 판인지(`x.y.z`, 앞의 `v`와 `-beta` 같은 꼬리는 무시한다).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(version: &str) -> [u64; 3] {
        let mut parts = [0; 3];
        for (slot, part) in parts.iter_mut().zip(
            version
                .trim()
                .trim_start_matches('v')
                .split(['-', '+'])
                .next()
                .unwrap_or_default()
                .split('.'),
        ) {
            *slot = part.parse().unwrap_or(0);
        }
        parts
    }
    parts(candidate) > parts(current)
}

/// 매니페스트에서 `platform`의 새 판을 읽는다. `current`보다 새롭지 않으면 None.
pub fn parse_manifest(
    text: &str,
    platform: &str,
    current: &str,
) -> Result<Option<Release>, String> {
    let manifest: Value =
        serde_json::from_str(text).map_err(|e| format!("업데이트 정보를 읽지 못했습니다: {e}"))?;
    let version = manifest["version"]
        .as_str()
        .ok_or("업데이트 정보에 판(version)이 없습니다")?;
    if !is_newer(version, current) {
        return Ok(None);
    }
    let entry = &manifest["platforms"][platform];
    let asset = match (entry["url"].as_str(), entry["signature"].as_str()) {
        (Some(url), Some(signature)) => Some(Asset {
            url: url.to_owned(),
            signature: signature.to_owned(),
            format: match entry["format"].as_str().unwrap_or_default() {
                "nsis" => Format::Nsis,
                "appimage" => Format::AppImage,
                other => Format::Other(other.to_owned()),
            },
        }),
        _ => None,
    };
    Ok(Some(Release {
        version: version.trim().trim_start_matches('v').to_owned(),
        notes: manifest["notes"].as_str().unwrap_or_default().to_owned(),
        asset,
    }))
}

/// 매니페스트를 받아 새 판이 있는지 본다. UI 스레드가 아닌 곳에서 부른다.
pub fn check(manifest_url: &str, current: &str) -> Result<Option<Release>, String> {
    let data = get(manifest_url, MAX_MANIFEST, &mut |_, _| {})?;
    let text =
        String::from_utf8(data).map_err(|e| format!("업데이트 정보를 읽지 못했습니다: {e}"))?;
    parse_manifest(&text, &platform(), current)
}

/// 설치 파일을 받아 `public_key`로 서명을 검증하고 `dest`에 쓴다. 검증에 실패하면 쓰지 않는다.
/// `progress`는 (받은 바이트, 전체 바이트)로 불린다. UI 스레드가 아닌 곳에서 부른다.
pub fn download(
    asset: &Asset,
    public_key: &str,
    dest: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), String> {
    // ponytail: 메모리에 받아 검증한다. 설치 파일은 수십 MB라 충분하다.
    let data = get(&asset.url, MAX_INSTALLER, progress)?;
    verify(&data, &asset.signature, public_key)?;
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(dest, &data).map_err(|e| format!("{}: {e}", dest.display()))
}

/// cargo-packager 형식의 서명을 검증한다. 공개 키와 서명은 minisign 글을 base64로 감싼 것이다.
pub fn verify(data: &[u8], signature: &str, public_key: &str) -> Result<(), String> {
    let unwrap = |text: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    };
    let key = unwrap(public_key)
        .and_then(|text| PublicKey::decode(&text).ok())
        .ok_or("업데이트 공개 키가 잘못되었습니다")?;
    let signature = unwrap(signature)
        .and_then(|text| Signature::decode(&text).ok())
        .ok_or("설치 파일의 서명을 읽지 못했습니다")?;
    // 옛 minisign의 해시하지 않은 서명도 받는다(cargo-packager 업데이터와 같다).
    key.verify(data, &signature, true)
        .map_err(|e| format!("설치 파일의 서명이 맞지 않습니다({e}). 설치하지 않습니다"))
}

/// 받은 형식으로 이 실행 파일(`exe`)을 바꿀 수 있으면 그 방법. MSI·deb·dmg 설치나 개발 빌드는 None.
/// `appimage`는 환경 변수 `APPIMAGE`의 값이다.
pub fn installer(format: &Format, exe: &Path, appimage: Option<PathBuf>) -> Option<Installer> {
    match format {
        Format::Nsis if exe.parent()?.join("uninstall.exe").is_file() => Some(Installer::Nsis),
        Format::AppImage => appimage.filter(|p| p.is_file()).map(Installer::AppImage),
        _ => None,
    }
}

/// NSIS 설치 파일을 수동 모드로 띄운다: 묻지 않고 진행 창만 보이며, 끝나면 앱을 다시 띄운다.
/// 앱이 끝날 때 부른다(설치 파일은 아직 도는 Ferrite가 있으면 끝내고 설치한다).
pub fn run_nsis(installer: &Path) -> io::Result<()> {
    Command::new(installer).args(["/P", "/R"]).spawn().map(drop)
}

/// AppImage 파일을 새 파일로 바꾼다. 같은 폴더에 쓴 뒤 이름을 바꾸므로 도는 중인 앱은 영향받지 않는다.
pub fn replace_appimage(new: &Path, target: &Path) -> io::Result<()> {
    let staged = target.with_extension("update");
    fs::copy(new, &staged)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&staged, target)
}

fn agent() -> Agent {
    Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .user_agent(concat!("ferrite/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// `limit` 바이트까지 받는다. 넘으면 오류.
fn get(
    url: &str,
    limit: u64,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, String> {
    let network = |e: &dyn std::fmt::Display| format!("{url}: {e}");
    let mut response = agent().get(url).call().map_err(|e| network(&e))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse().ok());
    let mut body = response.body_mut().as_reader().take(limit + 1);
    let mut data = Vec::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = body.read(&mut buf).map_err(|e| network(&e))?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        progress(data.len() as u64, total);
    }
    if data.len() as u64 > limit {
        return Err(format!("{url}: 파일이 너무 큽니다"));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = include_str!("../tests/fixtures/test.pub");
    const PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/payload.bin");
    const SIGNATURE: &str = include_str!("../tests/fixtures/payload.sig");

    #[test]
    fn 판을_비교한다() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn 매니페스트에서_이_플랫폼의_새_판을_읽는다() {
        let manifest = r#"{"version": "v0.2.0", "notes": "고친 것", "pub_date": "2026-11-01T00:00:00Z",
            "platforms": {
                "windows-x86_64": {"url": "https://x/setup.exe", "signature": "c2ln", "format": "nsis"},
                "macos-aarch64": {"url": "https://x/a.dmg", "signature": "c2ln", "format": "dmg"}
            }}"#;
        let release = parse_manifest(manifest, "windows-x86_64", "0.1.0")
            .unwrap()
            .unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.notes, "고친 것");
        assert_eq!(release.asset.unwrap().format, Format::Nsis);
        let mac = parse_manifest(manifest, "macos-aarch64", "0.1.0")
            .unwrap()
            .unwrap();
        assert_eq!(mac.asset.unwrap().format, Format::Other("dmg".into()));
        let linux = parse_manifest(manifest, "linux-x86_64", "0.1.0")
            .unwrap()
            .unwrap();
        assert_eq!(
            linux.asset, None,
            "이 플랫폼 파일이 없으면 내려받기 페이지만"
        );
        assert_eq!(
            parse_manifest(manifest, "windows-x86_64", "0.2.0").unwrap(),
            None
        );
        assert!(parse_manifest("{}", "windows-x86_64", "0.1.0").is_err());
    }

    #[test]
    fn 서명이_맞는_파일만_받아들인다() {
        verify(PAYLOAD, SIGNATURE, KEY).unwrap();
        let mut tampered = PAYLOAD.to_vec();
        tampered[0] ^= 1;
        assert!(
            verify(&tampered, SIGNATURE, KEY)
                .unwrap_err()
                .contains("맞지 않습니다")
        );
        assert!(
            verify(PAYLOAD, "Zm9v", KEY)
                .unwrap_err()
                .contains("서명을 읽지")
        );
        assert!(
            verify(PAYLOAD, SIGNATURE, "Zm9v")
                .unwrap_err()
                .contains("공개 키")
        );
    }

    #[test]
    fn 바꿀_수_있는_설치만_고른다() {
        let dir = std::env::temp_dir().join(format!("ferrite-update-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("ferrite.exe");
        assert_eq!(installer(&Format::Nsis, &exe, None), None, "개발 빌드");
        fs::write(dir.join("uninstall.exe"), "").unwrap();
        assert_eq!(installer(&Format::Nsis, &exe, None), Some(Installer::Nsis));
        assert_eq!(installer(&Format::Other("msi".into()), &exe, None), None);
        let image = dir.join("Ferrite.AppImage");
        assert_eq!(
            installer(&Format::AppImage, &exe, Some(image.clone())),
            None
        );
        fs::write(&image, "old").unwrap();
        assert_eq!(
            installer(&Format::AppImage, &exe, Some(image.clone())),
            Some(Installer::AppImage(image.clone()))
        );
        let new = dir.join("new.AppImage");
        fs::write(&new, "new").unwrap();
        replace_appimage(&new, &image).unwrap();
        assert_eq!(fs::read_to_string(&image).unwrap(), "new");
        let _ = fs::remove_dir_all(&dir);
    }
}
