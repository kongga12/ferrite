//! 실제 Gemini CLI(`gemini --acp`)와 대화한다. 로그인한 Gemini CLI가 필요하고 사용자 계정으로 질의하므로
//! 기본으로는 돌지 않는다. `cargo test -p ferrite-acp --test gemini -- --ignored --nocapture`

use std::thread;
use std::time::{Duration, Instant};

use ferrite_acp::{Agent, Event};

#[test]
#[ignore = "로그인한 Gemini CLI가 필요하고 사용자 계정으로 질의한다"]
fn gemini_cli와_대화하고_파일_쓰기를_제안받는다() {
    let root = std::env::temp_dir().join(format!("ferrite-acp-gemini-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("Board.java");
    std::fs::write(
        &file,
        "public class Board {\n    private int pageSize = 10;\n}\n",
    )
    .unwrap();
    let mut agent = Agent::spawn("gemini", &["--acp".into()], &[], &root, Vec::new()).unwrap();
    let started = Instant::now();
    let mut asked = false;
    let mut wrote = None;
    let mut said = String::new();
    loop {
        for event in agent.poll() {
            match &event {
                Event::Ready => {
                    eprintln!("세션 열림 ({:.1}초)", started.elapsed().as_secs_f64());
                    agent
                        .prompt(&format!(
                            "{}의 pageSize를 20으로 바꿔 줘. 파일 쓰기 도구로 고치고 다른 일은 하지 마.",
                            file.display()
                        ))
                        .unwrap();
                    asked = true;
                }
                Event::Message(_, text) => said.push_str(text),
                Event::ToolCall(call) => {
                    eprintln!("도구: {:?} {:?} {:?}", call.title, call.kind, call.status)
                }
                Event::ReadFile { id, path, .. } => {
                    eprintln!("읽기: {}", path.display());
                    agent.answer_read(id, std::fs::read_to_string(path).map_err(|e| e.to_string()));
                }
                Event::Permission { id, tool, options } => {
                    eprintln!(
                        "권한: {:?} {:?}",
                        tool.title,
                        options.iter().map(|o| &o.kind).collect::<Vec<_>>()
                    );
                    let allow = options
                        .iter()
                        .find(|o| o.kind == "allow_once")
                        .or(options.first());
                    agent.answer_permission(id, allow.map(|o| o.id.as_str()));
                }
                Event::WriteFile { id, path, content } => {
                    eprintln!("쓰기 제안: {}", path.display());
                    wrote = Some((path.clone(), content.clone()));
                    agent.answer_write(id, Ok(()));
                }
                Event::TurnEnded(reason) => {
                    eprintln!("끝: {reason}\n답: {said}");
                    let (path, content) = wrote.expect("파일 쓰기를 제안하지 않았다");
                    assert_eq!(path, file);
                    assert!(content.contains("pageSize = 20"), "{content}");
                    std::fs::remove_dir_all(&root).unwrap();
                    return;
                }
                Event::Error(e) | Event::Exited(e) => panic!("에이전트 오류: {e}"),
                _ => {}
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(if asked { 300 } else { 90 }),
            "시간 초과. 답: {said}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
