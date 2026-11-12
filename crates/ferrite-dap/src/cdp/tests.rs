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

/// 받은 글 틀의 CDP 명령들(이름, 인자).
fn requests(received: &Receiver<(u8, Vec<u8>)>) -> Vec<(String, Value)> {
    received
        .try_iter()
        .filter(|(opcode, _)| *opcode == 0x1)
        .map(|(_, payload)| {
            let request: Value = serde_json::from_slice(&payload).unwrap();
            (
                request["method"].as_str().unwrap().to_owned(),
                request["params"].clone(),
            )
        })
        .collect()
}

/// 맨 위 화면이 list.do(자식 프레임 detail.do, 그 안의 프레임 inner.do)인 가짜 프록시. `readyState`를 `states` 차례로(끝나면 마지막
/// 것을) 주고, 지금 화면을 연 방식은 `transition`, 자식 프레임이 처음 주소(src)를 보이는지는 `from_src`다.
/// 자식 프레임의 스크립트는 3줄(9칸)과 10줄에서 시작하고(97줄은 스크립트가 없고 98줄은 둘이 겹친다),
/// 그 글은 `블록 {줄}`이다.
fn page_proxy(
    states: &'static [&'static str],
    transition: &'static str,
    from_src: bool,
) -> (Proxy, Receiver<(u8, Vec<u8>)>) {
    let asked = std::sync::atomic::AtomicUsize::new(0);
    fake_proxy(move |request| {
        let id = request["id"].as_u64().unwrap();
        let params = &request["params"];
        let result = match request["method"].as_str().unwrap() {
            "Page.getFrameTree" => json!({ "frameTree": {
                "frame": { "id": "top", "url": "http://127.0.0.1:8080/app/list.do" },
                "childFrames": [{
                    "frame": { "id": "child", "url": "http://127.0.0.1:8080/app/detail.do" },
                    "childFrames": [{ "frame": { "id": "grand", "url": "http://127.0.0.1:8080/app/inner.do" } }]
                }]
            } }),
            "Page.createIsolatedWorld" => {
                assert!(params["frameId"] == "child" || params["frameId"] == "grand");
                json!({ "executionContextId": 7 })
            }
            "Runtime.evaluate" if params["expression"] == "document.readyState" => {
                let n = asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                json!({ "result": { "type": "string", "value": states[n.min(states.len() - 1)] } })
            }
            "Runtime.evaluate" => {
                // 프레임이 처음 주소를 보이는지는 그 프레임의 문서에서 묻는다.
                assert_eq!(params["contextId"], 7);
                assert!(
                    params["expression"]
                        .as_str()
                        .unwrap()
                        .contains("frameElement.src")
                );
                json!({ "result": { "type": "boolean", "value": from_src } })
            }
            // 앞 화면에서 링크로 왔다가 지금 화면은 `transition`으로 열었다.
            "Page.getNavigationHistory" => json!({ "currentIndex": 1, "entries": [
                { "id": 1, "url": "http://127.0.0.1:8080/app/main.do", "transitionType": "typed" },
                { "id": 2, "url": "http://127.0.0.1:8080/app/list.do", "transitionType": transition },
            ] }),
            "Page.reload" => json!({}),
            "Page.navigate" => json!({ "frameId": params["frameId"], "loaderId": "L2" }),
            "Debugger.setBreakpointByUrl" => {
                assert_eq!(params["url"], "http://127.0.0.1:8080/app/detail.do");
                assert_eq!(params["condition"], "false", "멈추지 않는 중단점");
                let line = params["lineNumber"].as_u64().unwrap() + 1;
                let locations: Vec<Value> = match line {
                    97 => Vec::new(),
                    98 => vec![
                        json!({ "scriptId": "s3", "lineNumber": 97, "columnNumber": 0 }),
                        json!({ "scriptId": "s4", "lineNumber": 98, "columnNumber": 0 }),
                    ],
                    _ => vec![
                        json!({ "scriptId": format!("s{line}"), "lineNumber": line, "columnNumber": 2 }),
                    ],
                };
                json!({ "breakpointId": format!("bp{line}"), "locations": locations })
            }
            "Debugger.getScriptSource" => {
                let script = params["scriptId"].as_str().unwrap();
                json!({ "scriptSource": format!("블록 {}", &script[1..]) })
            }
            "Debugger.removeBreakpoint" => json!({}),
            other => panic!("{other}"),
        };
        vec![frame(
            0x1,
            true,
            json!({ "id": id, "result": result }).to_string().as_bytes(),
        )]
    })
}

/// 받은 명령 이름이 `method`인 것이 올 때까지 기다린다.
fn wait_method(received: &Receiver<(u8, Vec<u8>)>, method: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    loop {
        seen.extend(requests(received));
        if let Some((_, params)) = seen.iter().find(|(m, _)| m == method) {
            return params.clone();
        }
        assert!(Instant::now() < deadline, "{seen:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn 화면을_다_읽기를_기다려_다시_불러도_되는지_알리고_다시_부른다() {
    const LIST: &str = "127.0.0.1:8080/app/list.do";
    const DETAIL: &str = "127.0.0.1:8080/app/detail.do";
    let wait = Duration::from_secs(5);
    // 맨 위 화면: 읽는 중이던 화면이 다 읽히면 탐색 기록을 본다. 링크로 연 화면은 다시 불러도 된다.
    let (proxy, received) = page_proxy(&["loading", "loading", "interactive"], "link", true);
    assert_eq!(
        page_loaded(&proxy, LIST, wait, &[]).unwrap(),
        Loaded::Reload(Target::Top)
    );
    let names: Vec<String> = requests(&received).into_iter().map(|(m, _)| m).collect();
    assert_eq!(
        names,
        [
            "Page.getFrameTree",
            "Runtime.evaluate",
            "Runtime.evaluate",
            "Runtime.evaluate",
            "Page.getNavigationHistory"
        ]
    );
    reload(&proxy, &Target::Top).unwrap();
    wait_method(&received, "Page.reload");
    // 폼을 보내 연 맨 위 화면.
    let (proxy, _) = page_proxy(&["complete"], "form_submit", true);
    assert_eq!(
        page_loaded(&proxy, LIST, wait, &[]).unwrap(),
        Loaded::FormSubmit
    );

    // 프레임 안 화면: 그 프레임의 문서(격리된 세계)에서 다 읽기를 기다리고, 처음 주소를 보이면 그 프레임만
    // 같은 주소로 다시 연다.
    let (proxy, received) = page_proxy(&["loading", "complete"], "form_submit", true);
    let target = Target::Frame {
        id: "child".into(),
        url: "http://127.0.0.1:8080/app/detail.do".into(),
        ancestors: vec![LIST.into()],
    };
    assert_eq!(
        page_loaded(&proxy, DETAIL, wait, &[]).unwrap(),
        Loaded::Reload(target.clone()),
        "맨 위 화면을 연 방식과는 상관없다"
    );
    let seen = requests(&received);
    assert!(
        seen.iter()
            .filter(|(m, _)| m == "Runtime.evaluate")
            .all(|(_, params)| params["contextId"] == 7),
        "{seen:?}"
    );
    reload(&proxy, &target).unwrap();
    let navigate = wait_method(&received, "Page.navigate");
    assert_eq!(
        (&navigate["frameId"], &navigate["url"]),
        (
            &json!("child"),
            &json!("http://127.0.0.1:8080/app/detail.do")
        )
    );
    // 프레임 안의 프레임은 품은 화면들(맨 위 화면부터)을 함께 알린다.
    let (proxy, _) = page_proxy(&["complete"], "link", true);
    assert_eq!(
        page_loaded(&proxy, "127.0.0.1:8080/app/inner.do", wait, &[]).unwrap(),
        Loaded::Reload(Target::Frame {
            id: "grand".into(),
            url: "http://127.0.0.1:8080/app/inner.do".into(),
            ancestors: vec![LIST.into(), DETAIL.into()],
        })
    );
    // 처음 주소에서 다른 화면으로 간 프레임.
    let (proxy, _) = page_proxy(&["complete"], "link", false);
    assert_eq!(
        page_loaded(&proxy, DETAIL, wait, &[]).unwrap(),
        Loaded::FrameMoved
    );

    // 다 읽지 못하면(읽는 중에 멈춰 있으면) 기다리다 그만둔다. 없는 화면은 알린다.
    let (proxy, _) = page_proxy(&["loading"], "link", true);
    let short = Duration::from_millis(250);
    assert!(!wait_loaded(&proxy, short).unwrap());
    assert!(
        page_loaded(&proxy, LIST, short, &[])
            .unwrap_err()
            .contains("다 읽지 못했습니다")
    );
    // 없는 화면은 알리고, 그 화면의 HTML·블록 글도 다른 화면의 것으로 주지 않는다.
    let gone = "127.0.0.1:8080/app/gone.do";
    assert_eq!(page_loaded(&proxy, gone, short, &[]).unwrap_err(), NOT_OPEN);
    assert_eq!(page_html(&proxy, gone).unwrap_err(), NOT_OPEN);
    assert_eq!(
        script_blocks(&proxy, gone, &[(3, 9)]).unwrap_err(),
        NOT_OPEN
    );
}

#[test]
fn 블록_자리에_멈추지_않는_중단점을_걸어_블록의_글을_받고_바로_지운다() {
    const DETAIL: &str = "127.0.0.1:8080/app/detail.do";
    let (proxy, received) = page_proxy(&["complete"], "link", true);
    assert_eq!(
        script_blocks(&proxy, DETAIL, &[(3, 9), (10, 9)]).unwrap(),
        [(3, 9, "블록 3".to_owned()), (10, 9, "블록 10".to_owned())]
    );
    let seen = requests(&received);
    let set: Vec<(u64, u64)> = seen
        .iter()
        .filter(|(m, _)| m == "Debugger.setBreakpointByUrl")
        .map(|(_, p)| {
            (
                p["lineNumber"].as_u64().unwrap(),
                p["columnNumber"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(set, [(2, 8), (9, 8)], "0부터 센 자리");
    let removed: Vec<&Value> = seen
        .iter()
        .filter(|(m, _)| m == "Debugger.removeBreakpoint")
        .map(|(_, p)| &p["breakpointId"])
        .collect();
    assert_eq!(removed, [&json!("bp3"), &json!("bp10")]);
    // 그 자리의 스크립트가 없거나 여럿이면 가려내지 못한다. 그래도 건 중단점은 지운다.
    for line in [97, 98] {
        let (proxy, received) = page_proxy(&["complete"], "link", true);
        let error = script_blocks(&proxy, DETAIL, &[(3, 9), (line, 1)]).unwrap_err();
        assert!(error.contains("가려내지 못했습니다"), "{error}");
        let removed = requests(&received)
            .iter()
            .filter(|(m, _)| m == "Debugger.removeBreakpoint")
            .count();
        assert_eq!(removed, 2);
    }
}

/// 가짜 프록시, 받은 틀, 품은 프레임에서 받은 `src` 물음(식).
type SessionProxy = (Proxy, Receiver<(u8, Vec<u8>)>, Receiver<String>);

/// 한 세션의 가짜 프록시: 프레임 트리는 `tree`, 화면은 다 읽었고, 품은 프레임에서 `src`를 물으면 `shown`이다.
fn session_proxy(tree: Value, shown: bool) -> SessionProxy {
    let (asked, questions) = channel();
    let asked = std::sync::Mutex::new(asked);
    let (proxy, received) = fake_proxy(move |request| {
        let id = request["id"].as_u64().unwrap();
        let params = &request["params"];
        let result = match request["method"].as_str().unwrap() {
            "Page.getFrameTree" => json!({ "frameTree": tree }),
            "Page.createIsolatedWorld" => json!({ "executionContextId": 9 }),
            "Runtime.evaluate" if params["expression"] == "document.readyState" => {
                json!({ "result": { "type": "string", "value": "complete" } })
            }
            "Runtime.evaluate" => {
                let expression = params["expression"].as_str().unwrap();
                // 다른 사이트의 프레임에서는 `frameElement`를 읽을 수 없다.
                assert!(!expression.contains("frameElement"), "{expression}");
                assert_eq!(params["contextId"], 9);
                let _ = asked.lock().unwrap().send(expression.to_owned());
                json!({ "result": { "type": "boolean", "value": shown } })
            }
            "Page.navigate" => json!({ "frameId": params["frameId"], "loaderId": "L2" }),
            // 다른 사이트의 프레임은 맨 위 대상이 아니다(`Page.reload`·`Page.getNavigationHistory`를 받지 않는다).
            other => panic!("{other}"),
        };
        vec![frame(
            0x1,
            true,
            json!({ "id": id, "result": result }).to_string().as_bytes(),
        )]
    });
    (proxy, received, questions)
}

#[test]
fn 다른_사이트의_프레임은_품은_세션들에서_품은_화면을_찾고_품은_프레임의_src로_가린다() {
    const LIST: &str = "127.0.0.1:8080/app/list.do";
    const DETAIL: &str = "127.0.0.1:8080/app/detail.do";
    const FRAG: &str = "localhost:8080/app/frag.do";
    let wait = Duration::from_secs(5);
    // 페이지 세션: list.do 안에 같은 사이트의 프레임 detail.do. 다른 사이트의 프레임은 이 트리에 없다.
    let page_tree = json!({
        "frame": { "id": "top", "url": "http://127.0.0.1:8080/app/list.do" },
        "childFrames": [{ "frame": { "id": "child", "url": "http://127.0.0.1:8080/app/detail.do" } }]
    });
    // 다른 사이트의 프레임 세션: 트리는 그 프레임에서 시작하고, 품은 프레임(detail.do)을 `parentId`로 안다.
    let far_tree = json!({ "frame": { "id": "far", "parentId": "child",
        "url": "http://localhost:8080/app/frag.do#top" } });
    let (page, _, questions) = session_proxy(page_tree.clone(), true);
    let (far, received, _) = session_proxy(far_tree.clone(), true);
    let target = Target::Frame {
        id: "far".into(),
        url: "http://localhost:8080/app/frag.do#top".into(),
        ancestors: vec![LIST.into(), DETAIL.into()],
    };
    assert_eq!(
        page_loaded(&far, FRAG, wait, std::slice::from_ref(&page)).unwrap(),
        Loaded::Reload(target.clone())
    );
    // 처음 주소인지는 품은 프레임에서 지금 주소(`#` 뒤를 뺀 것)로 묻는다.
    let question = questions.try_recv().unwrap();
    assert!(
        question.contains("querySelectorAll('iframe,frame')")
            && question.contains("\"http://localhost:8080/app/frag.do\""),
        "{question}"
    );
    // 그 프레임만 그 세션에서 다시 연다.
    reload(&far, &target).unwrap();
    let navigate = wait_method(&received, "Page.navigate");
    assert_eq!(
        (&navigate["frameId"], &navigate["url"]),
        (
            &json!("far"),
            &json!("http://localhost:8080/app/frag.do#top")
        )
    );
    // 품은 프레임의 `src`가 지금 주소가 아니면(프레임 안에서 다른 화면으로 갔다) 다시 열지 않는다.
    let (page, ..) = session_proxy(page_tree.clone(), false);
    let (far, ..) = session_proxy(far_tree.clone(), true);
    assert_eq!(
        page_loaded(&far, FRAG, wait, &[page]).unwrap(),
        Loaded::FrameMoved
    );
    // 품은 세션을 모르면(트리에 품은 프레임이 없으면) 알린다.
    let (far, ..) = session_proxy(far_tree, true);
    assert_eq!(page_loaded(&far, FRAG, wait, &[]).unwrap_err(), NOT_OPEN);

    // 다른 사이트의 프레임 안의 다른 사이트 프레임: 세션을 둘 거슬러 오른다. 처음 주소는 가까운 품은 세션에서 본다.
    let mid_tree = json!({ "frame": { "id": "mid", "parentId": "child",
        "url": "http://10.0.0.2:8080/app/mid.do" } });
    let inner_tree = json!({ "frame": { "id": "inner", "parentId": "mid",
        "url": "http://localhost:8080/app/frag.do" } });
    let (page, _, page_questions) = session_proxy(page_tree, true);
    let (mid, _, mid_questions) = session_proxy(mid_tree, true);
    let (inner, ..) = session_proxy(inner_tree, true);
    assert_eq!(
        page_loaded(&inner, FRAG, wait, &[mid, page]).unwrap(),
        Loaded::Reload(Target::Frame {
            id: "inner".into(),
            url: "http://localhost:8080/app/frag.do".into(),
            ancestors: vec![
                LIST.into(),
                DETAIL.into(),
                "10.0.0.2:8080/app/mid.do".into()
            ],
        })
    );
    assert!(mid_questions.try_recv().is_ok());
    assert!(page_questions.try_recv().is_err());
}

#[test]
fn cdp_오류와_웹소켓을_열지_않는_곳을_알린다() {
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
    let error = page_html(&proxy, "127.0.0.1:8080/app/main.do").unwrap_err();
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
