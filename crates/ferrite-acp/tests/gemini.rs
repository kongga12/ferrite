//! 실제 에이전트(Gemini CLI `gemini --acp`, Claude Code의 ACP 어댑터)와 대화해 파일을 고치게 한다.
//! 고치는 길은 둘이다: `fs/write_text_file`로 클라이언트에 쓰기를 맡기거나(Gemini),
//! 권한 요청에 diff를 보이고 허락받은 뒤 직접 디스크에 쓴다(Claude Code 어댑터 0.100).
//! 로그인한 에이전트가 필요하고 사용자 계정으로 질의하므로 기본으로는 돌지 않는다.
//! `cargo test -p ferrite-acp --test gemini -- --ignored --nocapture <이름>`

use std::thread;
use std::time::{Duration, Instant};

use ferrite_acp::{Agent, Event};

/// 에이전트에게 파일 하나를 고치게 하고, 권한은 한 번 허락, 쓰기는 받는다.
fn edit_with(program: &str, args: &[&str], name: &str) {
    let root = std::env::temp_dir().join(format!("ferrite-acp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("Board.java");
    std::fs::write(
        &file,
        "public class Board {\n    private int pageSize = 10;\n}\n",
    )
    .unwrap();
    // Claude Code는 사용자 설정의 권한 모드(예: auto)면 묻지 않고 고친다. 이 폴더에서만 묻게 한다.
    std::fs::create_dir_all(root.join(".claude")).unwrap();
    std::fs::write(
        root.join(".claude/settings.local.json"),
        r#"{"permissions":{"defaultMode":"default"}}"#,
    )
    .unwrap();
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let mut agent = Agent::spawn(program, &args, &[], &root, Vec::new()).unwrap();
    let started = Instant::now();
    let mut asked = false;
    let mut wrote = None;
    let mut permissions = 0;
    let mut permitted = None;
    let mut said = String::new();
    loop {
        for event in agent.poll() {
            match &event {
                Event::Ready => {
                    eprintln!("세션 열림 ({:.1}초)", started.elapsed().as_secs_f64());
                    agent
                        .prompt(&format!(
                            "{}의 pageSize를 20으로 바꿔 줘. 파일 편집 도구로 고치고 다른 일은 하지 마.",
                            file.display()
                        ))
                        .unwrap();
                    asked = true;
                }
                Event::Message(_, text) => said.push_str(text),
                Event::ToolCall(call) => {
                    eprintln!(
                        "도구: {:?} {:?} {:?} diff {}개",
                        call.title,
                        call.kind,
                        call.status,
                        call.diffs.len()
                    )
                }
                Event::ReadFile { id, path, .. } => {
                    eprintln!("읽기: {}", path.display());
                    agent.answer_read(id, std::fs::read_to_string(path).map_err(|e| e.to_string()));
                }
                Event::Permission { id, tool, options } => {
                    permissions += 1;
                    eprintln!(
                        "권한: {:?} diff {}개 {:?}",
                        tool.title,
                        tool.diffs.len(),
                        options.iter().map(|o| &o.kind).collect::<Vec<_>>()
                    );
                    if let Some(diff) = tool.diffs.iter().find(|d| d.path == file) {
                        permitted = Some(diff.new_text.clone());
                    }
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
                    eprintln!("끝: {reason} (권한 요청 {permissions}번)\n답: {said}");
                    if let Some((path, content)) = wrote {
                        assert_eq!(path, file);
                        assert!(content.contains("pageSize = 20"), "{content}");
                    } else {
                        let diff = permitted.expect("쓰기 요청도, diff가 붙은 권한 요청도 없었다");
                        assert!(diff.contains("pageSize = 20"), "{diff}");
                        let disk = std::fs::read_to_string(&file).unwrap();
                        assert!(
                            disk.contains("pageSize = 20"),
                            "허락했는데 고치지 않았다: {disk}"
                        );
                    }
                    // 에이전트를 닫으면 표준 입력이 끊겨 손자 프로세스(node·claude)도 끝나고 폴더가 풀린다.
                    drop(agent);
                    let closed = Instant::now();
                    while std::fs::remove_dir_all(&root).is_err() {
                        assert!(
                            closed.elapsed() < Duration::from_secs(15),
                            "에이전트가 끝나지 않는다"
                        );
                        thread::sleep(Duration::from_millis(200));
                    }
                    eprintln!("에이전트 끝남 ({:.1}초)", closed.elapsed().as_secs_f64());
                    return;
                }
                Event::Error(e) | Event::Exited(e) => panic!("에이전트 오류: {e}"),
                _ => {}
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(if asked { 300 } else { 120 }),
            "시간 초과. 답: {said}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "로그인한 Gemini CLI가 필요하고 사용자 계정으로 질의한다"]
fn gemini_cli와_대화하고_파일_쓰기를_제안받는다() {
    edit_with("gemini", &["--acp"], "gemini");
}

#[test]
#[ignore = "로그인한 Claude Code와 npm의 ACP 어댑터(약 390MB)가 필요하고 사용자 계정으로 질의한다"]
fn claude_code와_대화하고_권한을_받아_파일을_고친다() {
    edit_with(
        "npx",
        &["-y", "@agentclientprotocol/claude-agent-acp@0.100.0"],
        "claude",
    );
}
