//! 파일 경로 ↔ `file://` URI. 서버가 돌려준 URI를 열린 문서와 비교할 수 있도록 양방향으로 맞춘다.

use std::path::{Path, PathBuf};

use lsp_types::Uri;

/// 퍼센트 인코딩하지 않는 문자: RFC 3986 unreserved + 경로 구분자 + 드라이브 콜론.
fn keep(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':')
}

pub fn path_to_uri(path: &Path) -> Result<Uri, String> {
    let raw = path.to_string_lossy().replace('\\', "/");
    // Windows UNC(//server/share)는 file://server/share, 드라이브(C:/)는 file:///C:/ 형태다.
    let (authority, rest) = match raw.strip_prefix("//") {
        Some(unc) => unc.split_once('/').map_or((unc, ""), |(host, p)| (host, p)),
        None => ("", raw.as_str()),
    };
    let mut out = format!("file://{authority}");
    if !rest.starts_with('/') {
        out.push('/');
    }
    for b in rest.bytes() {
        if keep(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.parse::<Uri>()
        .map_err(|e| format!("URI로 바꿀 수 없는 경로입니다: {}: {e:?}", path.display()))
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `file://` URI가 아니면 None. Windows에서는 드라이브 문자를 대문자로 맞춘다
/// (서버마다 `c:`/`C:`, `%3A`처럼 표기가 달라 열린 문서 경로와 비교가 어긋나지 않게).
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let rest = uri.as_str().strip_prefix("file://")?;
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path = percent_decode(path)?;
    if !authority.is_empty() && authority != "localhost" {
        return Some(PathBuf::from(format!(
            r"\\{authority}{}",
            path.replace('/', "\\")
        )));
    }
    let bytes = path.as_bytes();
    let is_drive =
        bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    if is_drive {
        let mut p = path[1..].replace('/', "\\");
        p[..1].make_ascii_uppercase();
        return Some(PathBuf::from(p));
    }
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(s: &str) -> Uri {
        s.parse().unwrap()
    }

    #[test]
    fn 윈도우_드라이브_경로_왕복과_표기_차이_흡수() {
        let u = path_to_uri(Path::new(r"D:\works\내 파일 #1.rs")).unwrap();
        assert_eq!(
            u.as_str(),
            "file:///D:/works/%EB%82%B4%20%ED%8C%8C%EC%9D%BC%20%231.rs"
        );
        assert_eq!(
            uri_to_path(&u).unwrap(),
            PathBuf::from(r"D:\works\내 파일 #1.rs")
        );
        // 서버가 소문자 드라이브·인코딩된 콜론으로 돌려줘도 같은 경로가 된다
        assert_eq!(
            uri_to_path(&uri("file:///d%3A/works/a.rs")).unwrap(),
            PathBuf::from(r"D:\works\a.rs")
        );
    }

    #[test]
    fn 유닉스_경로와_unc_경로() {
        let u = path_to_uri(Path::new("/home/user/a b.rs")).unwrap();
        assert_eq!(u.as_str(), "file:///home/user/a%20b.rs");
        assert_eq!(uri_to_path(&u).unwrap(), PathBuf::from("/home/user/a b.rs"));
        let unc = path_to_uri(Path::new(r"\\server\share\x.rs")).unwrap();
        assert_eq!(unc.as_str(), "file://server/share/x.rs");
        assert_eq!(
            uri_to_path(&unc).unwrap(),
            PathBuf::from(r"\\server\share\x.rs")
        );
    }

    #[test]
    fn 파일_uri가_아니면_none() {
        assert!(uri_to_path(&uri("untitled:Untitled-1")).is_none());
    }
}
