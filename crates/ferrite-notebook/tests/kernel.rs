//! 실제 Jupyter 커널(PATH의 Python에 깔린 ipykernel)로 실행·출력·오류·멈춤을 시험한다. ipykernel이
//! 없으면 건너뛴다(CI에서는 실패).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ferrite_notebook::kernel::{Event, Kernel, Status};
use serde_json::Value;

fn python() -> Option<PathBuf> {
    let name = if cfg!(windows) { "python" } else { "python3" };
    let found = Command::new(name)
        .args(["-c", "import ipykernel, jupyter_client"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 ipykernel이 설치되어 있어야 합니다"
        );
        eprintln!("ipykernel이 없어 건너뜁니다");
        return None;
    }
    Some(PathBuf::from(name))
}

/// 실행 `id`가 끝날 때까지 이벤트를 모은다. `on_output`은 출력이 올 때마다 불린다.
fn run(
    kernel: &mut Kernel,
    id: u64,
    mut on_output: impl FnMut(&mut Kernel, &Value),
) -> (Vec<Event>, Option<u64>, Status) {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        let Some(event) = kernel.try_event() else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        match event {
            Event::Done {
                id: done,
                execution_count,
                status,
            } if done == id => return (events, execution_count, status),
            Event::Ended(reason) => panic!("커널이 끝났습니다: {reason}"),
            Event::Output { id: of, ref output } if of == id => {
                on_output(kernel, output);
                events.push(event);
            }
            other => events.push(other),
        }
    }
    panic!("실행 {id}가 끝나지 않았습니다: {events:?}");
}

fn outputs(events: &[Event]) -> Vec<&Value> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Output { output, .. } => Some(output),
            _ => None,
        })
        .collect()
}

#[test]
fn 커널로_셀을_실행하고_멈춘다() {
    let Some(python) = python() else { return };
    let dir = std::env::temp_dir().join(format!("ferrite-kernel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 노트북의 커널이 없으면 이 Python의 python3 커널로 띄운다.
    let mut kernel = Kernel::start(&python, Some("ferrite-없는-커널"), &dir, || {}).unwrap();

    let id = kernel.execute(
        "import os, sys\nprint('안녕')\nprint('오류', file=sys.stderr)\nprint(os.path.basename(os.getcwd()))\n1+1",
    );
    let (events, count, status) = run(&mut kernel, id, |_, _| {});
    assert!(
        matches!(&events[0], Event::Started { name, language, missing: Some(m), .. }
            if name == "python3" && language == "python" && m == "ferrite-없는-커널"),
        "{events:?}"
    );
    assert_eq!((count, status), (Some(1), Status::Ok));
    let out = outputs(&events);
    let folder = dir.file_name().unwrap().to_str().unwrap();
    let stdout: String = out
        .iter()
        .filter(|o| o["name"] == "stdout")
        .map(|o| o["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        stdout,
        format!("안녕\n{folder}\n"),
        "커널은 노트북 폴더에서 돈다"
    );
    assert!(
        out.iter()
            .any(|o| o["name"] == "stderr" && o["text"] == "오류\n")
    );
    let result = out.last().unwrap();
    assert_eq!(result["output_type"], "execute_result");
    assert_eq!(result["data"]["text/plain"], "2");
    assert_eq!(result["execution_count"], 1);

    // display_data와 clear_output(wait=True): 다음 출력이 올 때 지운다.
    let id = kernel.execute(
        "from IPython.display import display, HTML, clear_output\n\
         display(HTML('<b>굵게</b>'))\nclear_output(wait=True)\nprint('뒤')",
    );
    let (events, _, status) = run(&mut kernel, id, |_, _| {});
    assert_eq!(status, Status::Ok);
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            Event::Output { output, .. } => output["output_type"].as_str().unwrap(),
            Event::Clear { .. } => "clear",
            _ => "?",
        })
        .collect();
    assert_eq!(kinds, ["display_data", "clear", "stream"]);
    assert_eq!(outputs(&events)[0]["data"]["text/html"], "<b>굵게</b>");

    // 오류가 나면 줄 서 있던 실행은 하지 않는다.
    let failing = kernel.execute("x = 41\n1/0");
    let skipped = kernel.execute("print('돌면 안 된다')");
    let (events, count, status) = run(&mut kernel, failing, |_, _| {});
    assert_eq!((count, status), (Some(3), Status::Error));
    assert_eq!(outputs(&events)[0]["ename"], "ZeroDivisionError");
    let (events, count, status) = run(&mut kernel, skipped, |_, _| {});
    assert_eq!((count, status), (None, Status::Aborted));
    assert!(outputs(&events).is_empty());

    // 멈추면 KeyboardInterrupt로 끝나고, 상태(x)는 남는다. Windows의 ipykernel은 다음 Python 줄에서
    // 멈추므로(긴 time.sleep 같은 C 호출은 끝나야 멈춘다) 짧게 쉬며 돈다.
    let sleeping =
        kernel.execute("import time\nprint('시작', flush=True)\nwhile True:\n    time.sleep(0.05)");
    let started = Instant::now();
    let (events, _, status) = run(&mut kernel, sleeping, |kernel, output| {
        if output["name"] == "stdout" {
            kernel.interrupt();
        }
    });
    assert_eq!(status, Status::Error);
    assert!(started.elapsed() < Duration::from_secs(30), "멈추지 않았다");
    assert_eq!(
        outputs(&events).last().unwrap()["ename"],
        "KeyboardInterrupt"
    );
    let id = kernel.execute("x + 1");
    let (events, _, status) = run(&mut kernel, id, |_, _| {});
    assert_eq!(status, Status::Ok);
    assert_eq!(outputs(&events)[0]["data"]["text/plain"], "42");

    drop(kernel);
    let _ = std::fs::remove_dir_all(Path::new(&dir));
}
