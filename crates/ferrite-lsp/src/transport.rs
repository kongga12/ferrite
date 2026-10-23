//! LSP 메시지 틀: `Content-Length` 헤더 + 빈 줄 + JSON 본문.

use std::io::{self, BufRead, Write};

/// 메시지 하나를 읽는다. 스트림이 끝났으면 `None`.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        // 헤더 이름은 대소문자를 구분하지 않는다. Content-Type 등 다른 헤더는 무시한다.
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            let n = value.trim().parse::<usize>().map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Content-Length 오류: {e}"),
                )
            })?;
            length = Some(n);
        }
    }
    let length = length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Content-Length 헤더가 없습니다")
    })?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn write_message(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn 쓴_메시지를_그대로_읽는다_한글_바이트_길이() {
        let mut out = Vec::new();
        write_message(&mut out, "{\"a\":\"한글\"}".as_bytes()).unwrap();
        write_message(&mut out, b"{}").unwrap();
        let mut reader = Cursor::new(out);
        assert_eq!(
            read_message(&mut reader).unwrap().unwrap(),
            "{\"a\":\"한글\"}".as_bytes()
        );
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), b"{}");
        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn 다른_헤더와_대소문자를_허용한다() {
        let raw = b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}";
        assert_eq!(
            read_message(&mut Cursor::new(&raw[..])).unwrap().unwrap(),
            b"{}"
        );
    }

    #[test]
    fn 길이_헤더가_없으면_오류() {
        assert!(read_message(&mut Cursor::new(&b"X: 1\r\n\r\n{}"[..])).is_err());
    }
}
