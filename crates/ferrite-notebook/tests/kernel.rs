//! 실제 Jupyter 커널(PATH의 Python에 깔린 ipykernel)로 실행·출력·오류·멈춤을 시험한다. ipykernel이
//! 없으면 건너뛴다(CI에서는 실패).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ferrite_notebook::kernel::{Event, FrameInfo, Kernel, Status};
use serde_json::Value;

/// `modules`를 가져올 수 있는 PATH의 Python.
fn python(modules: &str) -> Option<PathBuf> {
    let name = if cfg!(windows) { "python" } else { "python3" };
    let found = Command::new(name)
        .args(["-c", &format!("import {modules}")])
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 {modules}이(가) 설치되어 있어야 합니다"
        );
        eprintln!("{modules}이(가) 없어 건너뜁니다");
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

/// 데이터 요청 `id`의 답을 기다린다.
fn answer(kernel: &mut Kernel, id: u64) -> Event {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        match kernel.try_event() {
            Some(event @ (Event::Frames { id: of, .. } | Event::Page { id: of, .. }))
                if of == id =>
            {
                return event;
            }
            Some(Event::Ended(reason)) => panic!("커널이 끝났습니다: {reason}"),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    panic!("요청 {id}의 답이 오지 않았습니다");
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
    let Some(python) = python("ipykernel, jupyter_client") else {
        return;
    };
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

#[test]
fn 커널의_dataframe을_페이지로_읽고_정렬한다() {
    let Some(python) = python("ipykernel, jupyter_client, pandas") else {
        return;
    };
    let mut kernel = Kernel::start(&python, None, &std::env::temp_dir(), || {}).unwrap();
    // pandas를 가져오지 않은 커널에서는 가져오지 않고 빈 목록이다.
    let id = kernel.frames();
    assert!(
        matches!(answer(&mut kernel, id), Event::Frames { result: Ok(list), .. } if list.is_empty())
    );
    let id = kernel.execute(
        "import pandas as pd, numpy as np\n\
         df = pd.DataFrame({'이름': ['다', '가', None, '나'], 'n': [3, 1, 4, 2],\n\
             'x': [0.5, np.nan, 2.25, 1e20]}, index=[10, 11, 12, 13])\n\
         df['날짜'] = pd.to_datetime(['2026-01-02', '2026-01-01', None, '2026-01-03'])\n\
         _숨김 = df\nseries = df['n']",
    );
    let (_, count, status) = run(&mut kernel, id, |_, _| {});
    assert_eq!((count, status), (Some(1), Status::Ok));

    // 밑줄로 시작하는 이름과 Series는 빼고 DataFrame만.
    let id = kernel.frames();
    let Event::Frames { result, .. } = answer(&mut kernel, id) else {
        unreachable!()
    };
    assert_eq!(
        result.unwrap(),
        [FrameInfo {
            name: "df".into(),
            rows: 4,
            columns: 4
        }]
    );

    let id = kernel.page("df", 1..3, None);
    let Event::Page { result, .. } = answer(&mut kernel, id) else {
        unreachable!()
    };
    let page = result.unwrap();
    assert_eq!(page.rows, 4);
    assert_eq!(page.columns, ["이름", "n", "x", "날짜"]);
    assert_eq!(page.dtypes[1], "int64");
    assert_eq!(page.index, ["11", "12"]);
    assert_eq!(page.data[0][0], "가");
    assert_eq!(page.data[0][1], 1);
    assert!(page.data[0][2].is_null(), "NaN은 null");
    assert!(
        page.data[0][3]
            .as_str()
            .unwrap()
            .starts_with("2026-01-01T00:00:00")
    );
    assert!(page.data[1][0].is_null() && page.data[1][3].is_null());

    // 정렬은 커널에서: n 내림차순, 이름 오름차순(빈 값은 끝).
    let id = kernel.page("df", 0..4, Some((1, false)));
    let Event::Page { result, .. } = answer(&mut kernel, id) else {
        unreachable!()
    };
    let page = result.unwrap();
    assert_eq!(page.index, ["12", "10", "13", "11"]);
    assert_eq!(page.data[1][2], 0.5);
    assert_eq!(page.data[2][2], 1e20, "실수는 15자리까지");
    let id = kernel.page("df", 0..4, Some((0, true)));
    let Event::Page { result, .. } = answer(&mut kernel, id) else {
        unreachable!()
    };
    assert_eq!(result.unwrap().index, ["11", "13", "10", "12"]);

    // 없는 이름은 오류로 답하고 커널은 그대로다. 조용한 실행은 실행 번호를 쓰지 않는다.
    let id = kernel.page("없음", 0..1, None);
    let Event::Page { result, .. } = answer(&mut kernel, id) else {
        unreachable!()
    };
    assert!(result.unwrap_err().contains("KeyError"));
    let id = kernel.execute("len(df)");
    let (_, count, status) = run(&mut kernel, id, |_, _| {});
    assert_eq!((count, status), (Some(2), Status::Ok));
}
