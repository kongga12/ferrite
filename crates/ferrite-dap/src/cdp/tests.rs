use super::*;
use std::net::TcpListener;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

/// 서버가 보내는 틀(가리지 않는다).
fn frame(opcode: u8, fin: bool, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![if fin { 0x80 } else { 0 } | opcode];
    let len = payload.len();
    if len < 126 {
        frame.push(len as u8);
    } else {
        frame.push(126);
        frame.extend((len as u16).to_be_bytes());
    }
    frame.extend(payload);
    frame
}

/// 클라이언트가 보낸 틀 하나: (opcode, 가림을 푼 내용). 클라이언트가 연결을 닫았으면 None.
fn read_frame(reader: &mut impl Read) -> Option<(u8, Vec<u8>)> {
    let mut head = [0u8; 2];
    reader.read_exact(&mut head).ok()?;
    assert_ne!(head[1] & 0x80, 0, "클라이언트 틀은 가린다");
    let mut len = usize::from(head[1] & 0x7f);
    if len == 126 {
        let mut short = [0u8; 2];
        reader.read_exact(&mut short).unwrap();
        len = usize::from(u16::from_be_bytes(short));
    }
    let mut mask = [0u8; 4];
    reader.read_exact(&mut mask).unwrap();
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).unwrap();
    payload
        .iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b ^= mask[i % 4]);
    Some((head[0] & 0x0f, payload))
}

/// 가짜 CDP 프록시: 연결마다(차례로) 손짓에 101로 답하고, 요청마다 `answer`가 준 틀들을 보낸다. 받은
/// (opcode, 내용)을 돌려준다.
fn fake_proxy(
    answer: impl Fn(&Value) -> Vec<Vec<u8>> + Send + 'static,
) -> (Proxy, Receiver<(u8, Vec<u8>)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sent, received) = channel();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("GET /devtools/abc HTTP/1.1"), "{line}");
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
                .unwrap();
            while let Some((opcode, payload)) = read_frame(&mut reader) {
                let _ = sent.send((opcode, payload.clone()));
                if opcode == 0x1 {
                    let request: Value = serde_json::from_slice(&payload).unwrap();
                    for frame in answer(&request) {
                        stream.write_all(&frame).unwrap();
                    }
                }
            }
        }
    });
    let proxy =
        Proxy::from_json(&json!({ "host": "127.0.0.1", "port": port, "path": "/devtools/abc" }))
            .unwrap();
    (proxy, received)
}

#[test]
fn 프록시로_화면을_연_프레임의_원래_html을_받는다() {
    // 한글이 든 긴 HTML(126바이트를 넘어 길이를 두 바이트로 적는다).
    let html = format!(
        "<!doctype html>\n<script>\nvar 제목 = \"게시판\";\n</script>\n{}",
        "<p>목록</p>\n".repeat(20)
    );
    let expected = html.clone();
    let (proxy, received) = fake_proxy(move |request| {
        let id = request["id"].as_u64().unwrap();
        match request["method"].as_str().unwrap() {
            // 다른 메시지(이벤트)와 ping이 먼저 온다.
            "Page.getFrameTree" => vec![
                frame(0x1, true, br#"{"method":"Page.frameNavigated","params":{}}"#),
                frame(0x9, true, b"hi"),
                frame(0x1, true, json!({ "id": id, "result": { "frameTree": {
                    "frame": { "id": "top", "url": "http://127.0.0.1:8080/app/main.do" },
                    "childFrames": [{ "frame": { "id": "child", "url": "http://127.0.0.1:8080/app/list.do#top" } }]
                } } }).to_string().as_bytes()),
            ],
            "Page.getResourceContent" => {
                assert_eq!(request["params"]["frameId"], "child");
                assert_eq!(request["params"]["url"], "http://127.0.0.1:8080/app/list.do#top");
                // 둘로 나눠 보낸다.
                let body = json!({ "id": id, "result": { "content": html.clone(), "base64Encoded": false } })
                    .to_string()
                    .into_bytes();
                let (first, rest) = body.split_at(body.len() / 2);
                vec![frame(0x1, false, first), frame(0x0, true, rest)]
            }
            other => panic!("{other}"),
        }
    });
    assert_eq!(
        page_html(&proxy, "127.0.0.1:8080/app/list.do").unwrap(),
        expected
    );
    // ping에는 같은 내용의 pong으로 답했다.
    let frames: Vec<(u8, Vec<u8>)> = received.try_iter().collect();
    assert!(frames.contains(&(0xa, b"hi".to_vec())), "{frames:?}");
}

/// 받은 글 틀의 CDP 명령 이름들.
fn methods(received: &Receiver<(u8, Vec<u8>)>) -> Vec<String> {
    received
        .try_iter()
        .filter(|(opcode, _)| *opcode == 0x1)
        .map(|(_, payload)| {
            let request: Value = serde_json::from_slice(&payload).unwrap();
            request["method"].as_str().unwrap().to_owned()
        })
        .collect()
}

/// 맨 위 화면이 list.do(자식 프레임 detail.do)이고, `readyState`를 `states` 차례로(끝나면 마지막 것을) 주고,
/// 지금 화면을 연 방식이 `transition`인 가짜 프록시.
fn page_proxy(
    states: &'static [&'static str],
    transition: &'static str,
) -> (Proxy, Receiver<(u8, Vec<u8>)>) {
    let asked = std::sync::atomic::AtomicUsize::new(0);
    fake_proxy(move |request| {
        let id = request["id"].as_u64().unwrap();
        let result = match request["method"].as_str().unwrap() {
            "Page.getFrameTree" => json!({ "frameTree": {
                "frame": { "id": "top", "url": "http://127.0.0.1:8080/app/list.do" },
                "childFrames": [{ "frame": { "id": "child", "url": "http://127.0.0.1:8080/app/detail.do" } }]
            } }),
            "Runtime.evaluate" => {
                assert_eq!(request["params"]["expression"], "document.readyState");
                let n = asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                json!({ "result": { "type": "string", "value": states[n.min(states.len() - 1)] } })
            }
            // 앞 화면에서 링크로 왔다가 지금 화면은 `transition`으로 열었다.
            "Page.getNavigationHistory" => json!({ "currentIndex": 1, "entries": [
                { "id": 1, "url": "http://127.0.0.1:8080/app/main.do", "transitionType": "typed" },
                { "id": 2, "url": "http://127.0.0.1:8080/app/list.do", "transitionType": transition },
            ] }),
            "Page.reload" => json!({}),
            other => panic!("{other}"),
        };
        vec![frame(
            0x1,
            true,
            json!({ "id": id, "result": result }).to_string().as_bytes(),
        )]
    })
}

#[test]
fn 화면을_다_읽기를_기다려_폼을_보내_열었는지_알리고_다시_부른다() {
    // 읽는 중이던 화면이 다 읽히면 탐색 기록을 본다. 링크로 연 화면은 다시 불러도 된다.
    let (proxy, received) = page_proxy(&["loading", "loading", "interactive"], "link");
    let loaded = page_loaded(&proxy, "127.0.0.1:8080/app/list.do", Duration::from_secs(5)).unwrap();
    assert_eq!(
        loaded,
        Loaded {
            top: true,
            form_submit: false
        }
    );
    assert_eq!(
        methods(&received),
        [
            "Page.getFrameTree",
            "Runtime.evaluate",
            "Runtime.evaluate",
            "Runtime.evaluate",
            "Page.getNavigationHistory"
        ]
    );
    reload(&proxy).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    while !seen.iter().any(|m| m == "Page.reload") {
        assert!(Instant::now() < deadline, "{seen:?}");
        seen.extend(methods(&received));
        std::thread::sleep(Duration::from_millis(10));
    }

    // 폼을 보내 연 화면.
    let (proxy, _) = page_proxy(&["complete"], "form_submit");
    assert!(
        page_loaded(&proxy, "127.0.0.1:8080/app/list.do", Duration::from_secs(5))
            .unwrap()
            .form_submit
    );
    // 프레임 안 화면은 맨 위가 아니다(읽기를 기다리지 않는다).
    let (proxy, received) = page_proxy(&["complete"], "link");
    assert!(
        !page_loaded(
            &proxy,
            "127.0.0.1:8080/app/detail.do",
            Duration::from_secs(5)
        )
        .unwrap()
        .top
    );
    assert_eq!(methods(&received), ["Page.getFrameTree"]);
    // 다 읽지 못하면(읽는 중에 멈춰 있으면) 기다리다 그만둔다.
    let (proxy, _) = page_proxy(&["loading"], "link");
    assert!(!wait_loaded(&proxy, Duration::from_millis(250)).unwrap());
    assert!(
        page_loaded(
            &proxy,
            "127.0.0.1:8080/app/list.do",
            Duration::from_millis(250)
        )
        .unwrap_err()
        .contains("다 읽지 못했습니다")
    );
}

#[test]
fn 프레임이_없으면_맨_위_프레임이고_cdp_오류는_알린다() {
    let (proxy, _) = fake_proxy(|request| {
        let id = request["id"].as_u64().unwrap();
        let body = match request["method"].as_str().unwrap() {
            "Page.getFrameTree" => json!({ "id": id, "result": { "frameTree": {
                "frame": { "id": "top", "url": "http://127.0.0.1:8080/app/main.do" } } } }),
            _ => {
                assert_eq!(request["params"]["frameId"], "top");
                json!({ "id": id, "error": { "code": -32000, "message": "No resource with given URL found" } })
            }
        };
        vec![frame(0x1, true, body.to_string().as_bytes())]
    });
    let error = page_html(&proxy, "127.0.0.1:8080/app/other.do").unwrap_err();
    assert!(
        error.contains("No resource with given URL found"),
        "{error}"
    );
    // 웹소켓을 열지 않는 곳.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut stream, _) = closed.accept().unwrap();
        // 요청을 다 읽고 답한다(읽지 않은 글이 남은 채 닫으면 Windows는 답 대신 연결을 끊는다).
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
            line.clear();
        }
        stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n").unwrap();
    });
    let proxy = Proxy {
        host: "127.0.0.1".into(),
        port,
        path: "/".into(),
    };
    assert!(page_html(&proxy, "x").unwrap_err().contains("404"));
    assert_eq!(Proxy::from_json(&json!({ "port": 1 })), None);
}
