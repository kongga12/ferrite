//! 로컬 서버로 보내기: 로그인 302의 쿠키를 담아 이어 보내고, EUC-KR·JSON·404를 받는다.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use ferrite_http::{Client, CookieJar, Environments, parse, prepare};

/// 요청 줄, 헤더(소문자 이름), 본문.
fn read_request(stream: &TcpStream) -> (String, Vec<(String, String)>, Vec<u8>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (k, v) = header.split_once(':').unwrap();
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .map_or(0, |(_, v)| v.parse().unwrap());
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (line.trim_end().to_owned(), headers, body)
}

fn respond(mut stream: TcpStream, status: &str, headers: &[&str], body: &[u8]) {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for h in headers {
        head.push_str(h);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
}

fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = stream.unwrap();
            let (line, headers, body) = read_request(&stream);
            let cookie = headers
                .iter()
                .find(|(k, _)| k == "cookie")
                .map(|(_, v)| v.as_str());
            match line.split(' ').take(2).collect::<Vec<_>>()[..] {
                ["POST", "/app/login.do"] if body == b"id=admin" => respond(
                    stream,
                    "302 Found",
                    &[
                        "Location: main.do",
                        "Set-Cookie: JSESSIONID=abc; Path=/app; HttpOnly",
                    ],
                    b"",
                ),
                ["GET", "/app/main.do"] if cookie == Some("JSESSIONID=abc") => {
                    let (euc, _, _) = encoding_rs::EUC_KR.encode("환영합니다");
                    respond(
                        stream,
                        "200 OK",
                        &["Content-Type: text/html; charset=EUC-KR"],
                        &euc,
                    )
                }
                ["GET", "/app/main.do"] => respond(stream, "401 Unauthorized", &[], b""),
                ["GET", "/api/board"] => respond(
                    stream,
                    "200 OK",
                    &["Content-Type: application/json;charset=UTF-8"],
                    r#"{"title":"공지","ids":[1,2]}"#.as_bytes(),
                ),
                _ => respond(
                    stream,
                    "404 Not Found",
                    &["Content-Type: text/plain"],
                    b"none",
                ),
            }
        }
    });
    port
}

#[test]
fn 로그인_쿠키를_담아_이어_보내고_문자_집합과_json을_푼다() {
    let port = serve();
    let text = "### 로그인\nPOST {{base}}/app/login.do\nContent-Type: application/x-www-form-urlencoded\n\nid={{user}}\n\n\
         ### 첫 화면\nGET {{base}}/app/main.do\n\n\
         ### 쿠키 없이\n# @no-cookie-jar\nGET {{base}}/app/main.do\n\n\
         ### JSON\nGET {{base}}/api/board\n\n\
         ### 리다이렉트 그대로\n# @no-redirect\nPOST {{base}}/app/login.do\n\nid=admin\n\n\
         ### 없음\nGET {{base}}/nothing\n";
    let file = parse(text);
    let mut envs = Environments::default();
    let dev = envs.values.entry("dev".into()).or_default();
    dev.insert("base".into(), format!("http://127.0.0.1:{port}"));
    dev.insert("user".into(), "admin".into());
    let client = Client::default();
    let mut jar = CookieJar::default();
    let mut send = |i: usize| {
        let prepared = prepare(&file, &file.requests[i], Some((&envs, "dev"))).unwrap();
        client.send(&prepared, &mut jar).unwrap()
    };

    let login = send(0);
    assert_eq!(
        login.status, 200,
        "302를 따라가 첫 화면까지: {}",
        login.text
    );
    assert_eq!(
        login.redirects,
        [(302, format!("http://127.0.0.1:{port}/app/main.do"))]
    );
    assert_eq!(
        (login.text.as_str(), login.charset),
        ("환영합니다", "EUC-KR")
    );

    assert_eq!(send(1).status, 200, "담은 쿠키로 이어 보낸다");
    assert_eq!(send(2).status, 401, "@no-cookie-jar");

    let json = send(3);
    assert_eq!(
        json.text,
        "{\n  \"title\": \"공지\",\n  \"ids\": [\n    1,\n    2\n  ]\n}"
    );
    assert_eq!(
        json.header("content-type"),
        Some("application/json;charset=UTF-8")
    );

    let raw = send(4);
    assert_eq!((raw.status, raw.header("location")), (302, Some("main.do")));

    let missing = send(5);
    assert_eq!(
        (
            missing.status,
            missing.reason.as_str(),
            missing.text.as_str()
        ),
        (404, "Not Found", "none")
    );
}

#[test]
fn 연결하지_못하면_오류를_돌려준다() {
    // 바로 닫은 포트: 연결이 거절된다.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let file = parse(&format!("GET http://127.0.0.1:{port}/\n"));
    let prepared = prepare(&file, &file.requests[0], None).unwrap();
    assert!(
        Client::default()
            .send(&prepared, &mut CookieJar::default())
            .is_err()
    );
}
