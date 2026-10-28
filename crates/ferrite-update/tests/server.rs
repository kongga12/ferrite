//! 로컬 HTTP 서버로 매니페스트를 묻고, 설치 파일을 받아 서명을 검증한다(시험용 키, `fixtures/`).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;

use ferrite_update::{Format, check, download, platform};

const KEY: &str = include_str!("fixtures/test.pub");
const PAYLOAD: &[u8] = include_bytes!("fixtures/payload.bin");
const SIGNATURE: &str = include_str!("fixtures/payload.sig");

/// 경로마다 정한 본문을 돌려주는 서버. 주소(`http://127.0.0.1:포트`)를 돌려준다.
fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            let _ = reader.read_line(&mut request);
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) <= 2 {
                    break;
                }
            }
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let _ = match routes.iter().find(|(p, _)| p == path) {
                Some((_, body)) => write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .and_then(|()| stream.write_all(body)),
                None => stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                ),
            };
        }
    });
    address
}

fn manifest(base: &str, file: &str) -> Vec<u8> {
    format!(
        r#"{{"version": "0.2.0", "notes": "시험", "platforms": {{"{}": {{
            "url": "{base}/{file}", "signature": "{}", "format": "nsis"}}}}}}"#,
        platform(),
        SIGNATURE.trim()
    )
    .into_bytes()
}

#[test]
fn 새_판을_알아보고_서명을_검증해_받는다() {
    let mut tampered = PAYLOAD.to_vec();
    tampered[10] ^= 1;
    // 서버 주소를 알아야 매니페스트를 만들 수 있으므로 두 번 띄운다: 파일 서버, 매니페스트 서버.
    let files = serve(vec![
        ("/setup.exe".into(), PAYLOAD.to_vec()),
        ("/bad.exe".into(), tampered),
    ]);
    let manifests = serve(vec![
        ("/latest.json".into(), manifest(&files, "setup.exe")),
        ("/bad.json".into(), manifest(&files, "bad.exe")),
    ]);
    let dir = std::env::temp_dir().join(format!("ferrite-update-server-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let release = check(&format!("{manifests}/latest.json"), "0.1.0")
        .unwrap()
        .expect("새 판");
    assert_eq!(release.version, "0.2.0");
    let asset = release.asset.unwrap();
    assert_eq!(asset.format, Format::Nsis);
    let dest = dir.join("setup.exe");
    let mut last = (0, None);
    download(&asset, KEY, &dest, &mut |received, total| {
        last = (received, total)
    })
    .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), PAYLOAD);
    assert_eq!(last, (PAYLOAD.len() as u64, Some(PAYLOAD.len() as u64)));

    // 지금 판이 같거나 새로우면 없다.
    assert_eq!(
        check(&format!("{manifests}/latest.json"), "0.2.0").unwrap(),
        None
    );

    // 바뀐 파일은 쓰지 않는다.
    let bad = check(&format!("{manifests}/bad.json"), "0.1.0")
        .unwrap()
        .unwrap()
        .asset
        .unwrap();
    let bad_dest = dir.join("bad.exe");
    let error = download(&bad, KEY, &bad_dest, &mut |_, _| {}).unwrap_err();
    assert!(error.contains("서명이 맞지 않습니다"), "{error}");
    assert!(!bad_dest.exists());

    // 매니페스트가 없으면 오류(시작할 때 확인은 조용히 넘어간다).
    assert!(check(&format!("{manifests}/none.json"), "0.1.0").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
