//! 실제 셸을 PTY(Windows는 ConPTY)에 띄워 출력·입력·한글·크기 변경·종료를 본다.

use std::time::{Duration, Instant};

use ferrite_term::{Options, Terminal};

fn shell(args: &[&str]) -> (String, Vec<String>) {
    let args = args.iter().map(|a| (*a).to_owned()).collect();
    if cfg!(windows) {
        ("cmd.exe".to_owned(), args)
    } else {
        ("sh".to_owned(), args)
    }
}

fn spawn(shell: (String, Vec<String>)) -> Terminal {
    Terminal::spawn(
        &Options {
            working_directory: std::env::temp_dir(),
            columns: 80,
            lines: 24,
            shell: Some(shell),
        },
        || {},
    )
    .expect("셸을 띄우지 못함")
}

/// `done`이 참이 될 때까지(최대 20초) 기다린다.
fn wait(terminal: &mut Terminal, what: &str, done: impl Fn(&Terminal) -> bool) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        terminal.poll();
        if done(terminal) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "{what} 시간 초과. 종료: {:?}, 화면:\n{}",
        terminal.exited(),
        terminal.snapshot().text()
    );
}

#[test]
fn 명령의_출력과_종료_코드를_받는다() {
    // ConPTY는 곧바로 끝나는 프로세스의 마지막 화면을 다 내보내지 않을 때가 있어(실험으로 확인) 끝나기
    // 전에 1초 쉰다. 통합 터미널의 셸은 계속 떠 있으므로 상관없다.
    let args: &[&str] = if cfg!(windows) {
        &[
            "/c",
            "echo hello-ferrite& echo 한글출력& ping -n 2 127.0.0.1 >nul& exit 3",
        ]
    } else {
        &["-c", "echo hello-ferrite; echo 한글출력; exit 3"]
    };
    let mut terminal = spawn(shell(args));
    // 종료 알림은 다른 스레드에서 오므로 마지막 출력보다 먼저 올 수 있다. 둘 다 기다린다.
    wait(&mut terminal, "출력과 종료", |t| {
        t.exited().is_some() && t.snapshot().text().contains("한글출력")
    });
    let text = terminal.snapshot().text();
    assert!(text.contains("hello-ferrite"), "{text}");
    assert_eq!(terminal.exited(), Some(Some(3)));
}

#[test]
fn 입력한_명령을_실행하고_한글은_두_칸이다() {
    let mut terminal = spawn(shell(&[]));
    terminal.write(&b"echo abc-ferrite\r"[..]);
    // 셸이 입력을 되돌려 보여 준 줄과 출력 줄, 두 번 보인다.
    wait(&mut terminal, "출력", |t| {
        t.snapshot().text().matches("abc-ferrite").count() >= 2
    });

    terminal.write("echo 가나다\r".as_bytes().to_vec());
    wait(&mut terminal, "한글 출력", |t| {
        t.snapshot().text().matches("가나다").count() >= 2
    });
    let snapshot = terminal.snapshot();
    let (line, column) = (0..snapshot.lines)
        .rev()
        .find_map(|line| {
            let cells = snapshot.line(line);
            cells
                .iter()
                .position(|c| c.c == '가')
                .map(|column| (line, column))
        })
        .unwrap();
    let cells = snapshot.line(line);
    assert!(cells[column].wide);
    assert!(cells[column + 1].spacer, "넓은 글자의 뒤 칸");
    assert_eq!(cells[column + 2].c, '나');

    terminal.resize(40, 10);
    assert_eq!(terminal.size(), (40, 10));
    assert_eq!(terminal.snapshot().cells.len(), 400);

    terminal.write(&b"exit\r"[..]);
    wait(&mut terminal, "종료", |t| t.exited().is_some());
}

#[test]
fn 고른_글자를_꺼낸다() {
    let args: &[&str] = if cfg!(windows) {
        &["/c", "echo select-me"]
    } else {
        &["-c", "echo select-me"]
    };
    let mut terminal = spawn(shell(args));
    wait(&mut terminal, "종료", |t| t.exited().is_some());
    let snapshot = terminal.snapshot();
    let line = (0..snapshot.lines)
        .find(|&l| {
            snapshot
                .line(l)
                .iter()
                .map(|c| c.c)
                .collect::<String>()
                .starts_with("select-me")
        })
        .unwrap();
    terminal.select((line, 0), (line, 5));
    assert_eq!(terminal.selection_text().as_deref(), Some("select"));
    assert!(terminal.snapshot().line(line)[0].selected);
    terminal.clear_selection();
    assert_eq!(terminal.selection_text(), None);
}
