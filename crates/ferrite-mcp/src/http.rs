//! MCP가 쓰는 만큼의 HTTP/1.1: 요청 하나를 읽고 응답 하나를 쓴 뒤 연결을 닫는다. 본문은
//! Content-Length나 chunked로 받는다.

use std::io::{self, BufRead, Read, Write};

/// 머리줄 한 줄과 본문의 한도(에이전트의 요청은 작다).
const MAX_LINE: usize = 16 * 1024;
const MAX_HEADERS: usize = 100;
const MAX_BODY: usize = 4 * 1024 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// 머리의 값(이름은 대소문자를 가리지 않는다).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn read_line(reader: &mut impl BufRead) -> io::Result<String> {
    let mut line = Vec::new();
    reader
        .by_ref()
        .take(MAX_LINE as u64)
        .read_until(b'\n', &mut line)?;
    if !line.ends_with(b"\n") {
        return Err(invalid("머리줄이 너무 길거나 끊겼습니다"));
    }
    String::from_utf8(line)
        .map(|l| l.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|_| invalid("머리줄이 UTF-8이 아닙니다"))
}

/// 요청 하나를 읽는다. 연결이 바로 닫혔으면 None.
pub fn read_request(reader: &mut impl BufRead) -> io::Result<Option<Request>> {
    let first = match read_line(reader) {
        Ok(line) => line,
        Err(e) if e.kind() == io::ErrorKind::InvalidData && reader.fill_buf()?.is_empty() => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    let mut parts = first.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(invalid("요청 줄을 읽지 못했습니다"));
    };
    let path = target.split('?').next().unwrap_or(target).to_owned();
    let mut headers = Vec::new();
    loop {
        let line = read_line(reader)?;
        if line.is_empty() {
            break;
        }
        if headers.len() == MAX_HEADERS {
            return Err(invalid("머리가 너무 많습니다"));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("머리를 읽지 못했습니다"))?;
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
    let mut request = Request {
        method: method.to_owned(),
        path,
        headers,
        body: Vec::new(),
    };
    if request
        .header("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"))
    {
        loop {
            let size = read_line(reader)?;
            let size = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| invalid("chunk 크기를 읽지 못했습니다"))?;
            if size == 0 {
                // 끝의 머리(trailer)와 빈 줄
                while !read_line(reader)?.is_empty() {}
                break;
            }
            if request.body.len() + size > MAX_BODY {
                return Err(invalid("본문이 너무 큽니다"));
            }
            let start = request.body.len();
            request.body.resize(start + size, 0);
            reader.read_exact(&mut request.body[start..])?;
            read_line(reader)?;
        }
    } else if let Some(length) = request.header("content-length") {
        let length: usize = length
            .parse()
            .map_err(|_| invalid("Content-Length를 읽지 못했습니다"))?;
        if length > MAX_BODY {
            return Err(invalid("본문이 너무 큽니다"));
        }
        request.body = vec![0; length];
        reader.read_exact(&mut request.body)?;
    }
    Ok(Some(request))
}

/// 응답을 쓰고 연결을 닫는다고 알린다.
pub fn write_response(
    writer: &mut impl Write,
    status: u16,
    headers: &[(&str, String)],
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    writer.write_all(head.as_bytes())?;
    writer.write_all(body)?;
    writer.flush()
}
