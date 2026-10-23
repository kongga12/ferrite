//! 텍스트 파일 입출력: 인코딩(UTF-8/CP949)·줄바꿈 감지와 보존, 원자적 저장.

use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use encoding_rs::EUC_KR;

/// 편집기로 여는 파일의 최대 크기.
// ponytail: 파일 전체를 메모리에 올리는 방식의 상한(읽을 때 원본·디코딩·로프로 순간 3배 가까이 쓴다).
// 더 큰 파일은 메모리 매핑 + 부분 로딩이 필요하다.
pub const MAX_FILE_SIZE: u64 = 256 * 1024 * 1024;

/// 바이너리 판별 시 검사할 앞부분 길이(git과 같은 휴리스틱).
const BINARY_SNIFF_LEN: usize = 8000;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    /// MS949(CP949). encoding_rs의 EUC-KR은 WHATWG 정의상 CP949 확장 한글까지 포함한다.
    Cp949,
}

impl Encoding {
    pub const ALL: [Encoding; 3] = [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Cp949];

    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Cp949 => "CP949",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub const ALL: [LineEnding; 2] = [LineEnding::Lf, LineEnding::CrLf];

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }
}

/// 디코딩된 파일 내용. `text`의 줄바꿈은 항상 `\n`으로 정규화되어 있고,
/// 저장 시 `line_ending`으로 되돌린다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextContent {
    pub text: String,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

#[derive(Debug)]
pub enum TextFileError {
    Io(io::Error),
    TooLarge(u64),
    Binary,
    UnknownEncoding,
    /// 대상 인코딩으로 표현할 수 없는 문자(처음 발견된 것).
    Unmappable {
        encoding: Encoding,
        ch: char,
    },
}

impl fmt::Display for TextFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextFileError::Io(e) => write!(f, "{e}"),
            TextFileError::TooLarge(size) => write!(
                f,
                "파일이 너무 큽니다({}MB, 최대 {}MB)",
                size / 1024 / 1024,
                MAX_FILE_SIZE / 1024 / 1024
            ),
            TextFileError::Binary => write!(f, "바이너리 파일은 열 수 없습니다"),
            TextFileError::UnknownEncoding => {
                write!(f, "지원하지 않는 인코딩입니다(UTF-8, CP949만 지원)")
            }
            TextFileError::Unmappable { encoding, ch } => write!(
                f,
                "'{ch}' 문자는 {}로 저장할 수 없습니다. 상태바에서 인코딩을 UTF-8로 바꾸세요",
                encoding.label()
            ),
        }
    }
}

impl std::error::Error for TextFileError {}

impl From<io::Error> for TextFileError {
    fn from(e: io::Error) -> Self {
        TextFileError::Io(e)
    }
}

/// 크기 제한을 확인하고 파일 내용을 읽는다.
pub fn read(path: &Path) -> Result<Vec<u8>, TextFileError> {
    let size = fs::metadata(path)?.len();
    if size > MAX_FILE_SIZE {
        return Err(TextFileError::TooLarge(size));
    }
    Ok(fs::read(path)?)
}

/// 디스크 내용이 바뀌었는지 비교할 때 쓰는 해시.
pub fn content_hash(bytes: &[u8]) -> u64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// UTF-8(BOM 유무) → CP949 순으로 판별한다. 둘 다 아니면 데이터 손상을 막기 위해 거부한다.
/// 혼합 줄바꿈 파일은 CRLF가 하나라도 있으면 CRLF로 간주한다(저장 시 LF 줄도 CRLF가 된다).
pub fn decode(bytes: &[u8]) -> Result<TextContent, TextFileError> {
    if bytes[..bytes.len().min(BINARY_SNIFF_LEN)].contains(&0) {
        return Err(TextFileError::Binary);
    }
    let (raw, encoding) = if let Some(rest) = bytes.strip_prefix(UTF8_BOM) {
        let text = std::str::from_utf8(rest).map_err(|_| TextFileError::UnknownEncoding)?;
        (text.to_owned(), Encoding::Utf8Bom)
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        (text.to_owned(), Encoding::Utf8)
    } else {
        let text = EUC_KR
            .decode_without_bom_handling_and_without_replacement(bytes)
            .ok_or(TextFileError::UnknownEncoding)?;
        (text.into_owned(), Encoding::Cp949)
    };
    let (text, line_ending) = if raw.contains("\r\n") {
        (raw.replace("\r\n", "\n"), LineEnding::CrLf)
    } else {
        (raw, LineEnding::Lf)
    };
    Ok(TextContent {
        text,
        encoding,
        line_ending,
    })
}

pub fn encode(content: &TextContent) -> Result<Vec<u8>, TextFileError> {
    let text: Cow<str> = match content.line_ending {
        LineEnding::Lf => Cow::Borrowed(&content.text),
        LineEnding::CrLf => Cow::Owned(content.text.replace('\n', "\r\n")),
    };
    match content.encoding {
        Encoding::Utf8 => Ok(text.into_owned().into_bytes()),
        Encoding::Utf8Bom => Ok([UTF8_BOM, text.as_bytes()].concat()),
        Encoding::Cp949 => {
            let (bytes, _, had_unmappable) = EUC_KR.encode(&text);
            if had_unmappable {
                // encoding_rs는 표현 불가 문자를 "&#...;"로 치환하므로, 그대로 저장하면 내용이 바뀐다.
                let ch = text
                    .chars()
                    .find(|c| EUC_KR.encode(c.encode_utf8(&mut [0; 4])).2)
                    .unwrap_or(char::REPLACEMENT_CHARACTER);
                return Err(TextFileError::Unmappable {
                    encoding: content.encoding,
                    ch,
                });
            }
            Ok(bytes.into_owned())
        }
    }
}

/// 같은 디렉터리의 임시 파일에 쓴 뒤 rename으로 교체한다.
/// 쓰는 도중 실패하거나 프로세스가 죽어도 원본 파일은 손상되지 않는다.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // 심볼릭 링크는 링크 자체가 아니라 실제 대상 파일을 교체해야 한다.
    let target = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(e) => return Err(e),
    };
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "저장 경로가 올바르지 않습니다",
        ));
    };
    let tmp = dir.join(format!(".{}.ferrite-tmp", name.to_string_lossy()));

    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Ok(meta) = fs::metadata(&target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(bytes: &[u8]) -> TextContent {
        let content = decode(bytes).expect("디코딩 실패");
        assert_eq!(encode(&content).expect("인코딩 실패"), bytes);
        content
    }

    #[test]
    fn utf8_bom_crlf_보존() {
        let content = roundtrip(b"\xEF\xBB\xBFfn main() {}\r\n// \xED\x95\x9C\xEA\xB8\x80\r\n");
        assert_eq!(content.encoding, Encoding::Utf8Bom);
        assert_eq!(content.line_ending, LineEnding::CrLf);
        assert_eq!(content.text, "fn main() {}\n// 한글\n");
    }

    #[test]
    fn cp949_확장한글_보존() {
        // '똠'은 KS X 1001(EUC-KR)에는 없고 CP949 확장 영역에만 있다.
        let (bytes, _, _) = EUC_KR.encode("가나다\r\n똠방각하\r\n");
        let content = roundtrip(&bytes);
        assert_eq!(content.encoding, Encoding::Cp949);
        assert_eq!(content.line_ending, LineEnding::CrLf);
        assert_eq!(content.text, "가나다\n똠방각하\n");
    }

    #[test]
    fn 바이너리_거부() {
        assert!(matches!(
            decode(b"\x7FELF\x00\x01"),
            Err(TextFileError::Binary)
        ));
    }

    #[test]
    fn cp949_표현불가_문자는_저장_거부() {
        let content = TextContent {
            text: "한글 😀".into(),
            encoding: Encoding::Cp949,
            line_ending: LineEnding::Lf,
        };
        assert!(matches!(
            encode(&content),
            Err(TextFileError::Unmappable { ch: '😀', .. })
        ));
    }

    #[test]
    fn 원자적_저장_후_임시파일_없음() {
        let dir = std::env::temp_dir().join(format!("ferrite-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        fs::write(&path, "old").unwrap();

        write_atomic(&path, b"new").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert!(!dir.join(".a.txt.ferrite-tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
