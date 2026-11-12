//! 디버그 어댑터를 실제 배포처에서 받아 설치하고(`ferrite_lsp::install`), 언어마다 같은 예제
//! (`add(41, 1)`)의 `return` 줄에서 멈춰 변수 a=41·b=1·total=42를 읽는다. 내려받기가 커서 기본은
//! 건너뛰고, `.github/workflows/adapters.yml`이 3개 OS에서 `--ignored`로 돌린다. 필요한 도구(Go,
//! PHP+Xdebug, node, rustc, Chrome)가 없으면 로컬에서는 건너뛰고 CI에서는 실패한다. js-debug는 브라우저 화면의
//! 외부 스크립트와 인라인 스크립트에서 멈추고, 인라인 스크립트를 만든 JSP의 줄로 중단점을 걸고 멈춘 자리를
//! JSP 줄로 맞춘다. 화면을 다시 불러도 건 중단점에서 멈춘다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ferrite_dap::{
    Adapter, Breakpoint, DebugEvent, DebugSession, Launch, adapters, cdp, page_lines, types,
};
use ferrite_lsp::client::find_program;
use ferrite_lsp::install;
use serde_json::{Value, json};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrite-adapter-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // macOS의 임시 폴더(/var)는 /private/var로 가는 링크다. go는 실제 경로로 모듈을 찾으므로(링크 경로면
    // "모듈 밖 폴더") 실제 경로를 쓴다. 앱은 프로젝트·문서 경로를 이미 실제 경로로 맞춘다.
    // (Windows의 canonicalize는 `\\?\` 경로를 주므로 macOS에서만 바꾼다.)
    if cfg!(target_os = "macos") {
        std::fs::canonicalize(&dir).unwrap()
    } else {
        dir
    }
}

/// 도구가 있는지. 없으면 로컬은 건너뛰고(false) CI는 실패한다.
fn require(what: &str, found: bool) -> bool {
    if !found {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 {what}이(가) 있어야 합니다"
        );
        eprintln!("{what}이(가) 없어 건너뜁니다");
    }
    found
}

/// 어댑터를 받아 설치하고 띄울 방법을 돌려준다.
fn install_adapter(servers: &Path, name: &str, project: &Path) -> Adapter {
    let server = adapters::find(name).unwrap();
    install::install(servers, server, &mut |_, _| {})
        .unwrap_or_else(|e| panic!("{name} 설치 실패: {e}"));
    assert!(install::installed(servers, server).is_some());
    adapters::resolve(name, servers, project).unwrap()
}

/// 아직 보지 않은 이벤트 중 `done`이 참인 첫 이벤트(최대 120초, 첫 빌드·첫 실행이 느리다).
fn wait_for(
    session: &mut DebugSession,
    seen: &mut Vec<DebugEvent>,
    next: &mut usize,
    done: impl Fn(&DebugEvent) -> bool,
) -> DebugEvent {
    let started = Instant::now();
    loop {
        if let Some(offset) = seen[*next..].iter().position(&done) {
            *next += offset + 1;
            return seen[*next - 1].clone();
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "시간 초과. 받은 이벤트: {seen:#?}"
        );
        seen.extend(session.poll());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn response(event: DebugEvent) -> Value {
    match event {
        DebugEvent::Response { result, .. } => result.expect("요청 실패"),
        other => panic!("응답이 아님: {other:?}"),
    }
}

/// `source`의 `line`(1부터)에 중단점을 걸고 띄워, 멈추면 맨 위 프레임의 지역 변수를 읽은 뒤 이어 가서
/// 끝낸다. (변수 이름에서 `$`를 뺀 것, 값)을 돌려준다.
fn locals_at_breakpoint(
    adapter: &Adapter,
    adapter_id: &str,
    mut arguments: Value,
    source: &Path,
    line: usize,
) -> Vec<(String, String)> {
    adapters::complete_arguments(adapter_id, &mut arguments);
    let launch = Launch {
        adapter_id: adapter_id.into(),
        attach: false,
        arguments,
    };
    let breakpoints = BTreeMap::from([(
        source.to_path_buf(),
        vec![Breakpoint {
            line,
            condition: None,
        }],
    )]);
    let mut session = DebugSession::start(adapter, launch, breakpoints, || {}).unwrap();
    let (mut seen, mut next) = (Vec::new(), 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    let event = wait(&mut session, &|e| matches!(e, DebugEvent::Stopped(_)));
    let DebugEvent::Stopped(stopped) = event else {
        unreachable!()
    };
    assert_eq!(stopped.reason, "breakpoint", "{stopped:?}");
    let thread = match stopped.thread_id {
        Some(thread) => thread,
        None => {
            session.threads();
            let threads = types::threads(&response(wait(
                &mut session,
                &|e| matches!(e, DebugEvent::Response { command, .. } if command == "threads"),
            )));
            threads[0].id
        }
    };
    session.stack_trace(thread);
    let frames = types::stack_frames(&response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "stackTrace"),
    )));
    assert_eq!(frames[0].line as usize, line, "멈춘 줄: {frames:?}");
    session.scopes(frames[0].id);
    let scopes = types::scopes(&response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "scopes"),
    )));
    session.variables(scopes[0].variables_reference);
    let variables = types::variables(&response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "variables"),
    )));
    // 끝날 때까지 이어 간다. 한 줄에 걸 자리가 여럿이면(Rust의 println! 펼침) 다시 멈출 수 있다.
    session.resume(thread);
    for _ in 0..10 {
        let event = wait(&mut session, &|e| {
            matches!(
                e,
                DebugEvent::Stopped(_)
                    | DebugEvent::Terminated
                    | DebugEvent::Exited { .. }
                    | DebugEvent::Closed(_)
            )
        });
        match event {
            DebugEvent::Stopped(again) => session.resume(again.thread_id.unwrap_or(thread)),
            _ => break,
        }
    }
    variables
        .into_iter()
        .map(|v| (v.name.trim_start_matches('$').to_owned(), v.value))
        .collect()
}

fn assert_locals(locals: &[(String, String)]) {
    let value = |name: &str| {
        locals
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("{name} 없음: {locals:?}"))
    };
    assert_eq!(
        (value("a"), value("b"), value("total")),
        ("41", "1", "42"),
        "{locals:?}"
    );
}

#[test]
#[ignore = "PyPI에서 debugpy(약 5MB)를 받아 가상환경에 설치한다. Python이 필요하다"]
fn debugpy를_설치해_python을_디버그한다() {
    let python = if cfg!(windows) { "python" } else { "python3" };
    if !require("Python", find_program(python).is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("debugpy-servers"), temp_dir("debugpy"));
    let source = project.join("main.py");
    std::fs::write(
        &source,
        "def add(a, b):\n    total = a + b\n    return total\n\n\ny = add(41, 1)\nprint(\"결과\", y)\n",
    )
    .unwrap();
    let adapter = install_adapter(&servers, "debugpy", &project);
    let arguments = json!({ "program": source, "cwd": project, "console": "internalConsole" });
    assert_locals(&locals_at_breakpoint(
        &adapter, "debugpy", arguments, &source, 3,
    ));
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
#[ignore = "GitHub에서 codelldb(약 45~56MB)를 받는다. rustc가 필요하다"]
fn codelldb를_설치해_rust를_디버그한다() {
    if !require("rustc", find_program("rustc").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("codelldb-servers"), temp_dir("codelldb"));
    let source = project.join("main.rs");
    // 마지막 식(`total`)에는 코드가 없을 수 있어(Windows PDB에서는 다음 줄로 옮겨 걸림) 확실히 코드가 있는
    // println! 줄(3번)에 멈춘다.
    std::fs::write(
        &source,
        "fn add(a: i32, b: i32) -> i32 {\n    let total = a + b;\n    println!(\"합 {total}\");\n    total\n}\n\nfn main() {\n    let y = add(41, 1);\n    println!(\"결과 {y}\");\n}\n",
    )
    .unwrap();
    let program = project.join(format!("main{}", std::env::consts::EXE_SUFFIX));
    let built = Command::new("rustc")
        .args(["-g", "-C", "opt-level=0", "-o"])
        .arg(&program)
        .arg(&source)
        .status()
        .unwrap();
    assert!(built.success(), "rustc 실패");
    let adapter = install_adapter(&servers, "codelldb", &project);
    let arguments = json!({
        "program": program,
        "cwd": project,
        "args": [],
        "sourceLanguages": ["rust"],
    });
    assert_locals(&locals_at_breakpoint(
        &adapter, "codelldb", arguments, &source, 3,
    ));
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
#[ignore = "go install로 delve를 받아 빌드한다. Go가 필요하다"]
fn delve를_설치해_go를_디버그한다() {
    if !require("Go", find_program("go").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("delve-servers"), temp_dir("delve"));
    std::fs::write(
        project.join("go.mod"),
        "module example.com/demo\n\ngo 1.21\n",
    )
    .unwrap();
    let source = project.join("main.go");
    std::fs::write(
        &source,
        "package main\n\nimport \"fmt\"\n\nfunc add(a, b int) int {\n\ttotal := a + b\n\treturn total\n}\n\nfunc main() {\n\ty := add(41, 1)\n\tfmt.Println(\"결과\", y)\n}\n",
    )
    .unwrap();
    let adapter = install_adapter(&servers, "delve", &project);
    let arguments = json!({ "mode": "debug", "program": project });
    assert_locals(&locals_at_breakpoint(
        &adapter, "delve", arguments, &source, 7,
    ));
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}

/// PHP에 Xdebug 확장이 켜져 있는지.
fn php_with_xdebug() -> bool {
    Command::new("php")
        .arg("-m")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Xdebug"))
}

#[test]
#[ignore = "GitHub에서 vscode-php-debug(약 2MB)를 받는다. node와 Xdebug가 켜진 PHP가 필요하다"]
fn php_debug를_설치해_php를_디버그한다() {
    if !require("node", find_program("node").is_some())
        || !require("Xdebug가 켜진 PHP", php_with_xdebug())
    {
        return;
    }
    let (servers, project) = (temp_dir("php-servers"), temp_dir("php"));
    let source = project.join("index.php");
    std::fs::write(
        &source,
        "<?php\nfunction add($a, $b) {\n    $total = $a + $b;\n    return $total;\n}\n\n$y = add(41, 1);\necho \"결과 $y\\n\";\n",
    )
    .unwrap();
    let adapter = install_adapter(&servers, "php-debug", &project);
    let arguments = json!({
        "program": source,
        "cwd": project,
        "port": 0,
        "runtimeArgs": ["-dxdebug.start_with_request=yes"],
        "env": { "XDEBUG_MODE": "debug,develop", "XDEBUG_CONFIG": "client_port=${port}" },
    });
    assert_locals(&locals_at_breakpoint(
        &adapter,
        "php-debug",
        arguments,
        &source,
        4,
    ));
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}

/// jdt.ls에 `workspace/executeCommand`를 보내고 응답을 기다린다(최대 120초).
fn execute(
    lsp: &mut ferrite_lsp::client::LanguageServer,
    command: &str,
    arguments: &[Value],
) -> Result<Value, String> {
    let id = lsp.request(
        "workspace/executeCommand",
        json!({ "command": command, "arguments": arguments }),
    );
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(120) {
        for event in lsp.poll_events() {
            match event {
                ferrite_lsp::client::ServerEvent::Response { id: got, result } if got == id => {
                    return result.map_err(|e| e.to_string());
                }
                ferrite_lsp::client::ServerEvent::Exited(reason) => {
                    panic!("jdt.ls가 끝남: {reason}")
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{command} 응답 시간 초과");
}

#[test]
#[ignore = "jdt.ls(약 51MB)와 java-debug(약 3MB)를 받는다. Java 21 이상이 필요하다"]
fn java_debug를_jdtls에_실어_java를_디버그한다() {
    use ferrite_dap::java::{JavaStart, Next, RESOLVE_MAIN_CLASS};
    if !require("Java 21", ferrite_lsp::java::find_java().is_ok()) {
        return;
    }
    let (servers, project) = (temp_dir("java-servers"), temp_dir("java"));
    // 빌드 도구 없는 Eclipse 프로젝트(.project·.classpath). egov 프로젝트도 흔히 이 꼴이고, 의존을 받지 않는다.
    std::fs::write(
        project.join(".project"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription><name>demo</name><comment></comment><projects></projects>\
         <buildSpec><buildCommand><name>org.eclipse.jdt.core.javabuilder</name><arguments></arguments></buildCommand></buildSpec>\
         <natures><nature>org.eclipse.jdt.core.javanature</nature></natures></projectDescription>\n",
    )
    .unwrap();
    std::fs::write(
        project.join(".classpath"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<classpath><classpathentry kind=\"src\" path=\"src\"/>\
         <classpathentry kind=\"con\" path=\"org.eclipse.jdt.launching.JRE_CONTAINER\"/>\
         <classpathentry kind=\"output\" path=\"bin\"/></classpath>\n",
    )
    .unwrap();
    std::fs::create_dir_all(project.join("src/demo")).unwrap();
    let source = project.join("src/demo/Main.java");
    let text = "package demo;\n\npublic class Main {\n    static int add(int a, int b) {\n        int total = a + b;\n        return total;\n    }\n\n    public static void main(String[] args) {\n        int y = add(41, 1);\n        System.out.println(\"결과 \" + y);\n    }\n}\n";
    std::fs::write(&source, text).unwrap();

    let jdtls = ferrite_lsp::catalog::SERVERS
        .iter()
        .find(|s| s.name == "jdtls")
        .unwrap();
    install::install(&servers, jdtls, &mut |_, _| {}).expect("jdt.ls 설치");
    install::install(
        &servers,
        adapters::find("java-debug").unwrap(),
        &mut |_, _| {},
    )
    .expect("java-debug 설치");
    let command = install::command(&servers, jdtls, &project)
        .unwrap()
        .unwrap();
    assert!(
        command
            .init_options
            .to_string()
            .contains("com.microsoft.java.debug.plugin"),
        "java-debug 번들을 싣는다: {}",
        command.init_options
    );
    let mut lsp = ferrite_lsp::client::LanguageServer::start(&command, &project, || {}).unwrap();
    let started = Instant::now();
    while !lsp.is_ready() {
        assert!(
            started.elapsed() < Duration::from_secs(180),
            "jdt.ls 준비 시간 초과"
        );
        for event in lsp.poll_events() {
            if let ferrite_lsp::client::ServerEvent::Exited(reason) = event {
                panic!("jdt.ls가 끝남: {reason}");
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    lsp.did_open(&source, "java", 1, text);

    let mut start = JavaStart::new(
        json!({ "cwd": project, "console": "internalConsole" }),
        false,
        Some(&source),
    );
    let mut next = start.command();
    let (port, arguments) = loop {
        match next {
            Next::Command(name, args) => {
                // 프로젝트를 다 읽기 전에는 메인 클래스가 비어 있다. 잠시 뒤 다시 묻는다.
                let mut result = execute(&mut lsp, name, &args);
                for _ in 0..90 {
                    let empty = result
                        .as_ref()
                        .is_ok_and(|v| v.as_array().is_some_and(Vec::is_empty));
                    if name != RESOLVE_MAIN_CLASS || !empty {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                    result = execute(&mut lsp, name, &args);
                }
                next = start.answer(result);
            }
            Next::Ready { port, arguments } => break (port, arguments),
            Next::Failed(e) => panic!("java-debug 준비 실패: {e}"),
        }
    };
    assert_eq!(arguments["mainClass"], "demo.Main");
    let adapter = Adapter::Tcp(([127, 0, 0, 1], port).into());
    assert_locals(&locals_at_breakpoint(
        &adapter, "java", arguments, &source, 6,
    ));
    drop(lsp);
    let _ = std::fs::remove_dir_all(&servers);
    let _ = std::fs::remove_dir_all(&project);
}

/// 이 PC의 Chrome(없으면 Edge). js-debug의 `type`과 실행 파일.
fn browser() -> Option<&'static str> {
    let found = |paths: &[PathBuf]| paths.iter().any(|p| p.exists());
    let env = |key: &str| std::env::var_os(key).map(PathBuf::from).unwrap_or_default();
    if cfg!(windows) {
        let chrome = [
            env("ProgramFiles").join("Google/Chrome/Application/chrome.exe"),
            env("ProgramFiles(x86)").join("Google/Chrome/Application/chrome.exe"),
            env("LOCALAPPDATA").join("Google/Chrome/Application/chrome.exe"),
        ];
        let edge = [env("ProgramFiles(x86)").join("Microsoft/Edge/Application/msedge.exe")];
        return if found(&chrome) {
            Some("pwa-chrome")
        } else {
            found(&edge).then_some("pwa-msedge")
        };
    }
    if cfg!(target_os = "macos") {
        return found(&[PathBuf::from("/Applications/Google Chrome.app")]).then_some("pwa-chrome");
    }
    [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ]
    .iter()
    .any(|name| find_program(name).is_some())
    .then_some("pwa-chrome")
}

const PAGE: &str = "<!doctype html>
<html><body>
<h1>목록</h1>
<script src=\"/app/js/app.js\"></script>
<script>
var count = 0;
function tick() {
  count = count + 1;
  add(count, 1);
}
setInterval(tick, 200);
</script>
</body></html>
";

/// `PAGE`를 만든 JSP(서버가 실제로 돌리지는 않는다). 지시자 둘이 앞에 있고 EL·`<c:url>`·`<c:out>`이
/// 화면에서 값으로 바뀐다.
const LIST_JSP: &str = "<%@ page contentType=\"text/html; charset=UTF-8\" %>
<%@ taglib prefix=\"c\" uri=\"http://java.sun.com/jsp/jstl/core\" %>
<!doctype html>
<html><body>
<h1>${title}</h1>
<script src=\"<c:url value='/js/app.js'/>\"></script>
<script>
var count = ${start};
function tick() {
  count = count + 1;
  add(count, 1);
}
setInterval(tick, <c:out value=\"${interval}\"/>);
</script>
</body></html>
";

/// Tiles 꼴 화면: 레이아웃(`TILES_LAYOUT`)의 머리 스크립트, 머리글 조각(`TILES_HEADER`)과 본문
/// (`TILES_BODY`)의 스크립트가 한 화면에 든다. 서버는 JSP를 돌리지 않고 합친 결과를 준다.
const TILES_PAGE: &str = "<!doctype html>
<html><head>
<script>
var layoutReady = false;
function markLayout() {
  layoutReady = true;
}
</script>
</head><body>
<div id=\"header\">머리글</div>
<script>
function headerClock() {
  var now = new Date();
  return now.getTime();
}
</script>
<h1>목록</h1>
<script>
var ticks = 0;
function tick() {
  ticks = ticks + 1;
  headerClock();
}
setInterval(tick, 200);
</script>
</body></html>
";

const TILES_LAYOUT: &str = "<%@ page contentType=\"text/html; charset=UTF-8\" %>
<%@ taglib prefix=\"tiles\" uri=\"http://tiles.apache.org/tags-tiles\" %>
<!doctype html>
<html><head>
<script>
var layoutReady = false;
function markLayout() {
  layoutReady = true;
}
</script>
</head><body>
<tiles:insertAttribute name=\"header\"/>
<tiles:insertAttribute name=\"body\"/>
</body></html>
";

const TILES_HEADER: &str = "<%@ page contentType=\"text/html; charset=UTF-8\" %>
<div id=\"header\">머리글</div>
<script>
function headerClock() {
  var now = new Date();
  return now.getTime();
}
</script>
";

const TILES_BODY: &str = "<%@ page contentType=\"text/html; charset=UTF-8\" %>
<h1>${title}</h1>
<script>
var ticks = 0;
function tick() {
  ticks = ticks + 1;
  headerClock();
}
setInterval(tick, <c:out value=\"${interval}\"/>);
</script>
";

/// 조각을 끼우는 화면: 머리의 스크립트가 jQuery `.html()`로 `<script>`가 든 상세 조각을 `#detail`에
/// 끼운다. 조각의 스크립트는 화면 중간(문서 차례로 두 블록 사이)에 남지만 jQuery가 따로 돌려 화면 주소로
/// 알리지 않는다.
const FRAG_PAGE: &str = "<!doctype html>
<html><head>
<script src=\"/app/js/jquery.js\"></script>
<script>
function loadDetail() {
  $(\"#detail\").html(\"<p>상세</p><script>\\nfunction detailReady() {\\n  return 1;\\n}\\n<\\/script>\");
}
</script>
</head><body>
<div id=\"detail\"></div>
<script>
var count = 0;
function tick() {
  count = count + 1;
}
loadDetail();
setInterval(tick, 200);
</script>
</body></html>
";

/// 조각 화면을 프레임(`<iframe>`)으로 보이는 화면.
const MAIN_PAGE: &str = "<!doctype html>
<html><body>
<script>
var mainReady = true;
</script>
<iframe id=\"content\" src=\"/app/frag.do\"></iframe>
</body></html>
";

/// 다른 사이트의 조각 화면을 `<iframe>`으로 보이는 화면. 맨 위 화면은 `127.0.0.1`, 프레임은 `localhost`로
/// 열어 사이트가 다르다(브라우저가 프레임을 다른 프로세스로 띄운다). `{port}`는 웹 서버가 채운다.
const CROSS_PAGE: &str = "<!doctype html>
<html><body>
<script>
var crossReady = true;
</script>
<iframe id=\"content\" src=\"http://localhost:{port}/app/frag.do\"></iframe>
</body></html>
";

/// 옛 egov 화면처럼 `<frameset>`으로 나눈 화면: 왼쪽은 메뉴, 본문 프레임(main)은 조각 화면을 `<iframe>`으로
/// 보이는 화면(프레임 안의 프레임).
const FRAMESET_PAGE: &str = "<!doctype html>
<html>
<frameset cols=\"200,*\">
<frame name=\"left\" src=\"/app/menu.do\">
<frame name=\"main\" src=\"/app/main.do\">
</frameset>
</html>
";

/// 메뉴 프레임: 링크가 본문 프레임(main)에 목록 화면을 연다.
const MENU_PAGE: &str = "<!doctype html>
<html><body>
<script>
var menuReady = true;
</script>
<a id=\"list\" href=\"/app/list.do\" target=\"main\">목록</a>
</body></html>
";

/// jQuery `.html()`이 조각의 스크립트를 다루는 꼴만 흉내 낸 대역: 조각을 넣고, 스크립트마다 새
/// `<script>`로 돌려 바로 떼고, 원래 스크립트에 `globalEval` 표시를 남긴다(`jQuery._data`로 읽는다).
const JQUERY_STUB: &str = "(function () {
  var data = new WeakMap();
  function jq(selector) {
    var el = document.querySelector(selector);
    return {
      html: function (html) {
        el.innerHTML = html;
        el.querySelectorAll('script').forEach(function (s) {
          var run = document.createElement('script');
          run.text = s.text;
          document.head.appendChild(run).parentNode.removeChild(run);
          data.set(s, { globalEval: true });
        });
        return this;
      }
    };
  }
  jq._data = function (el, name) { var d = data.get(el); return d && d[name]; };
  window.jQuery = window.$ = jq;
})();
";

/// 화면에 줄 jQuery: `FERRITE_TEST_JQUERY`에 실제 jQuery 파일이 있으면 그것, 없으면 대역.
fn jquery() -> &'static str {
    static JQUERY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    JQUERY.get_or_init(|| {
        std::env::var_os("FERRITE_TEST_JQUERY")
            .map(|path| std::fs::read_to_string(path).expect("FERRITE_TEST_JQUERY 파일"))
            .unwrap_or_else(|| JQUERY_STUB.to_owned())
    })
}

/// 전용 워커(Web Worker)를 띄우고 메시지를 주고받는 화면.
const WORKER_PAGE: &str = "<!doctype html>
<html><body>
<script>
var worker = new Worker('/app/js/worker.js');
worker.onmessage = function (e) { window.workerAnswer = e.data; };
function ask(n) { worker.postMessage(n); return n; }
</script>
</body></html>
";

/// 워커 스크립트: 맨 위 코드(1줄)와 메시지를 받아 두 배로 돌려주는 함수(3줄).
const WORKER_JS: &str = "var seen = 0;
self.onmessage = function (e) {
  var doubled = e.data * 2;
  seen = seen + 1;
  self.postMessage(doubled);
};
";

const APP_JS: &str = "function add(a, b) {
  var total = a + b;
  return total;
}
";

/// 컨텍스트 경로 /app 아래에 화면(인라인 스크립트)과 외부 스크립트를 주는 웹 서버. 연결마다 따로 받는다:
/// 브라우저는 요청 없이 미리 열어 둔 연결을 쥐고 있기도 해서, 차례로 받으면 그 연결을 읽다 다음 요청에
/// 답하지 못한다(화면을 다시 부를 때 CI의 Windows에서 그랬다). 조각 화면은 폼(POST)으로도 연다(시험은
/// 본문 없는 폼만 보내 본문은 읽지 않는다).
fn web_server() -> u16 {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let cross: &'static str = Box::leak(
        CROSS_PAGE
            .replace("{port}", &port.to_string())
            .into_boxed_str(),
    );
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                let _ = reader.read_line(&mut line);
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                        break;
                    }
                }
                let (kind, body) = if line.starts_with("GET /app/list.do ") {
                    ("text/html; charset=UTF-8", PAGE)
                } else if line.starts_with("GET /app/main.do ") {
                    ("text/html; charset=UTF-8", MAIN_PAGE)
                } else if line.starts_with("GET /app/worker.do ") {
                    ("text/html; charset=UTF-8", WORKER_PAGE)
                } else if line.starts_with("GET /app/js/worker.js ") {
                    ("application/javascript", WORKER_JS)
                } else if line.starts_with("GET /app/cross.do ") {
                    ("text/html; charset=UTF-8", cross)
                } else if line.starts_with("GET /app/frameset.do ") {
                    ("text/html; charset=UTF-8", FRAMESET_PAGE)
                } else if line.starts_with("GET /app/menu.do ") {
                    ("text/html; charset=UTF-8", MENU_PAGE)
                } else if line.starts_with("GET /app/tiles.do ") {
                    ("text/html; charset=UTF-8", TILES_PAGE)
                } else if line.starts_with("GET /app/frag.do ")
                    || line.starts_with("POST /app/frag.do ")
                {
                    ("text/html; charset=UTF-8", FRAG_PAGE)
                } else if line.starts_with("GET /app/js/jquery.js ") {
                    ("application/javascript", jquery())
                } else if line.starts_with("GET /app/js/app.js ") {
                    ("application/javascript", APP_JS)
                } else {
                    ("text/plain", "")
                };
                let status = if body.is_empty() {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    port
}

/// 시험이 실패해 풀릴 때 기록 파일의 끝을 출력한다.
struct DumpOnPanic(PathBuf);

impl Drop for DumpOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let text = std::fs::read_to_string(&self.0).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            eprintln!(
                "--- {} 끝 ---\n{}",
                self.0.display(),
                lines[lines.len().saturating_sub(60)..].join("\n")
            );
        }
    }
}

/// 다음 멈춤을 기다려 (스레드, 맨 위 프레임)을 돌려준다.
fn top_frame(
    session: &mut DebugSession,
    wait: &mut impl FnMut(&mut DebugSession, &dyn Fn(&DebugEvent) -> bool) -> DebugEvent,
) -> (i64, ferrite_dap::StackFrame) {
    let DebugEvent::Stopped(stopped) = wait(session, &|e| matches!(e, DebugEvent::Stopped(_)))
    else {
        unreachable!()
    };
    let thread = stopped.thread_id.expect("멈춘 스레드");
    session.stack_trace(thread);
    let frames = types::stack_frames(&response(wait(
        session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "stackTrace"),
    )));
    (thread, frames[0].clone())
}

#[test]
#[ignore = "GitHub에서 vscode-js-debug(약 1.2MB)를 받는다. node와 Chrome(Windows는 Edge도 된다)이 필요하다"]
fn js_debug를_설치해_브라우저의_외부_스크립트와_화면_스크립트에서_멈춘다() {
    let Some(kind) = browser() else {
        require("Chrome", false);
        return;
    };
    if !require("node", find_program("node").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("js-servers"), temp_dir("js"));
    let web_root = project.join("src/main/webapp");
    std::fs::create_dir_all(web_root.join("js")).unwrap();
    let app_js = web_root.join("js/app.js");
    std::fs::write(&app_js, APP_JS).unwrap();
    let list_jsp = web_root.join("WEB-INF/jsp/list.jsp");
    std::fs::create_dir_all(list_jsp.parent().unwrap()).unwrap();
    std::fs::write(&list_jsp, LIST_JSP).unwrap();
    let port = web_server();
    let adapter = install_adapter(&servers, "js-debug", &project);
    let mut runtime_args = vec!["--headless=new", "--disable-gpu"];
    if cfg!(target_os = "linux") {
        runtime_args.extend(["--no-sandbox", "--disable-dev-shm-usage"]);
    }
    // 실패하면 js-debug의 기록 끝을 보인다(브라우저를 띄우지 못한 까닭 등).
    let trace = project.join("js-debug.log");
    let _dump = DumpOnPanic(trace.clone());
    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({
            "type": kind,
            "url": format!("http://127.0.0.1:{port}/app/list.do"),
            "webRoot": web_root,
            "pathMapping": { "/app/": format!("{}/", web_root.display()) },
            "runtimeArgs": runtime_args,
            // CI 러너에서 처음 띄우는 브라우저는 느리다(기본 10초).
            "timeout": 60000,
            "trace": { "logFile": trace },
        }),
    };
    let breakpoints = BTreeMap::from([(
        app_js.clone(),
        vec![Breakpoint {
            line: 3,
            condition: None,
        }],
    )]);
    let mut session = DebugSession::start(&adapter, launch, breakpoints, || {}).unwrap();
    let (mut seen, mut next) = (Vec::new(), 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    // 0) 화면을 불러오면 인라인 스크립트를 `loadedSource`로 알린다(앱은 이때 내용을 받아 JSP를 찾는다).
    let DebugEvent::LoadedSource { source: loaded, .. } = wait(&mut session, &|e| {
        matches!(e, DebugEvent::LoadedSource { source, .. }
            if source.source_reference > 0 && source.name.as_deref().is_some_and(|n| n.contains("list.do")))
    }) else {
        unreachable!()
    };
    // 1) 외부 스크립트: URL /app/js/app.js가 webapp 폴더의 파일로 맞춰져 그 파일의 중단점에서 멈춘다.
    let (thread, frame) = top_frame(&mut session, &mut wait);
    let source = frame.source.clone().unwrap();
    eprintln!("외부 스크립트 프레임: {frame:?}");
    assert_eq!(frame.line, 3);
    let path = source.path.expect("로컬 경로");
    assert_eq!(
        std::fs::canonicalize(&path).unwrap(),
        std::fs::canonicalize(&app_js).unwrap()
    );

    // 2) 화면 스크립트: 로컬 파일이 없으니 내용을 받아 담아 두고, 그 파일에 건 중단점을 원래 소스로 건다.
    session.loaded_sources();
    let sources = types::sources(&response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "loadedSources"),
    )));
    eprintln!("불러온 스크립트: {sources:#?}");
    let page = sources
        .iter()
        .find(|s| {
            s.source_reference > 0 && s.name.as_deref().is_some_and(|n| n.contains("list.do"))
        })
        .expect("화면 스크립트")
        .clone();
    assert_eq!(loaded.source_reference, page.source_reference);
    session.source(&page, None);
    let content = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "source"),
    ));
    let content = content["content"].as_str().unwrap().to_owned();
    let line = content
        .lines()
        .position(|l| l.contains("count = count + 1"))
        .expect("tick 본문")
        + 1;
    // 내용으로 그 스크립트를 만든 JSP를 찾는다. JSP 10줄(`count = count + 1;`)은 화면 8줄이다.
    let files: Vec<(PathBuf, String)> = page_lines::candidates(&project)
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect();
    let lines = page_lines::find(&content, &files, page.name.as_deref().unwrap()).expect("JSP");
    assert_eq!(lines.file, list_jsp);
    assert_eq!(lines.to_page(9), Some(line - 1));
    assert_eq!(line, 8);
    let local = project.join("list.do.html");
    std::fs::write(&local, &content).unwrap();
    session.map_source(&local, &page);
    session.set_breakpoints(&app_js, Vec::new());
    session.set_breakpoints(
        &local,
        vec![Breakpoint {
            line,
            condition: None,
        }],
    );
    session.resume(thread);
    let (_, frame) = top_frame(&mut session, &mut wait);
    eprintln!("화면 스크립트 프레임: {frame:?}");
    assert_eq!(frame.line as usize, line);
    assert_eq!(
        frame.source.unwrap().source_reference,
        page.source_reference
    );
    assert_eq!(
        lines.to_file(frame.line as usize - 1),
        9,
        "멈춘 자리는 JSP 10줄"
    );

    // 3) 화면을 다시 불러도 건 중단점에서 멈추는지: 맨 위에서 바로 도는 줄(`var count = 0;`, JSP 8줄)에 걸고
    // 다시 부른다. 그 줄은 화면을 불러오는 중에 돌므로 다시 불러야 멈출 수 있다. 멈춘 채로는 화면을 다시
    // 부르지 못하므로 이어 간 뒤에 부르게 한다.
    let top = lines.to_page(7).expect("JSP 8줄의 화면 줄") + 1;
    session.set_breakpoints(
        &local,
        vec![Breakpoint {
            line: top,
            condition: None,
        }],
    );
    session.request(
        "evaluate",
        json!({ "expression": "setTimeout(function () { location.reload(); }, 300)", "frameId": frame.id, "context": "repl" }),
    );
    // js-debug는 요청을 함께 처리해 답을 기다리지 않으면 이어 가기가 먼저 브라우저에 가서 식이 돌지 않는다.
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    );
    session.resume(thread);
    let (_, frame) = top_frame(&mut session, &mut wait);
    eprintln!("다시 부른 화면의 프레임: {frame:?}");
    assert_eq!(frame.line as usize, top);
    let source = frame.source.unwrap();
    assert!(
        source
            .name
            .as_deref()
            .is_some_and(|n| n.contains("list.do")),
        "{source:?}"
    );

    session.stop();
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Terminated | DebugEvent::Closed(_))
    });
    drop(session);
    // js-debug에 임시 폴더로 준 곳(브라우저 프로필)까지 지운다. 브라우저가 늦게 끝나 폴더를 쥐고 있으면
    // 잠깐 기다린다.
    for dir in [servers, project, adapters::scratch_dir()] {
        for _ in 0..40 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!dir.exists(), "폴더가 남음: {}", dir.display());
    }
}

#[test]
#[ignore = "GitHub에서 vscode-js-debug(약 1.2MB)를 받는다. node와 Chrome(Windows는 Edge도 된다)이 필요하다"]
fn js_debug로_여러_jsp를_합친_화면의_블록마다_jsp_줄에서_멈춘다() {
    let Some(kind) = browser() else {
        require("Chrome", false);
        return;
    };
    if !require("node", find_program("node").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("js-tiles-servers"), temp_dir("js-tiles"));
    let web_root = project.join("src/main/webapp");
    let jsp = web_root.join("WEB-INF/jsp");
    let (layout, header, body) = (
        jsp.join("layout/default.jsp"),
        jsp.join("layout/header.jsp"),
        jsp.join("board/BoardList.jsp"),
    );
    for (path, text) in [
        (&layout, TILES_LAYOUT),
        (&header, TILES_HEADER),
        (&body, TILES_BODY),
    ] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let port = web_server();
    let adapter = install_adapter(&servers, "js-debug", &project);
    let mut runtime_args = vec!["--headless=new", "--disable-gpu"];
    if cfg!(target_os = "linux") {
        runtime_args.extend(["--no-sandbox", "--disable-dev-shm-usage"]);
    }
    let trace = project.join("js-debug.log");
    let _dump = DumpOnPanic(trace.clone());
    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({
            "type": kind,
            "url": format!("http://127.0.0.1:{port}/app/tiles.do"),
            "webRoot": web_root,
            "pathMapping": { "/app/": format!("{}/", web_root.display()) },
            "runtimeArgs": runtime_args,
            "timeout": 60000,
            "trace": { "logFile": trace },
        }),
    };
    let mut session = DebugSession::start(&adapter, launch, BTreeMap::new(), || {}).unwrap();
    let (mut seen, mut next) = (Vec::new(), 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    // 0) 인라인 스크립트 셋을 블록마다 알린다(이름 끝 `꞉줄:칸`). js-debug는 같은 화면 주소의 스크립트가 새로
    // 오면 앞의 것을 지워(번호도 다시 쓴다) 내용은 마지막 블록만 받을 수 있다.
    let mut starts = Vec::new();
    let mut live = None;
    while starts.len() < 3 {
        let DebugEvent::LoadedSource { source, .. } = wait(&mut session, &|e| {
            matches!(e, DebugEvent::LoadedSource { source, .. }
                if source.name.as_deref().and_then(page_lines::inline_script)
                    .is_some_and(|(page, ..)| page.ends_with("/app/tiles.do")))
        }) else {
            unreachable!()
        };
        let (_, line, column) = page_lines::inline_script(source.name.as_deref().unwrap()).unwrap();
        starts.push((line, column));
        live = Some(source);
    }
    starts.sort();
    assert_eq!(starts, [(3, 9), (11, 9), (18, 9)]);
    let live = live.unwrap();

    // 1) 화면에서 블록의 글을 읽어(앱과 같은 식) 블록마다 JSP를 찾는다.
    session.request(
        "evaluate",
        json!({ "expression": page_lines::PAGE_SCRIPTS, "context": "clipboard" }),
    );
    let result = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    ));
    let texts = page_lines::page_scripts(result["result"].as_str().unwrap()).expect("글 목록");
    let blocks: Vec<(usize, usize, &str)> = starts
        .iter()
        .zip(&texts)
        .map(|(&(line, column), text)| (line, column, text.as_str()))
        .collect();
    let files: Vec<(PathBuf, String)> = page_lines::candidates(&project)
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect();
    let map = page_lines::PageMap::find(&blocks, &files, "127.0.0.1/app/tiles.do").expect("JSP");
    assert_eq!(
        map.files(),
        [layout.as_path(), header.as_path(), body.as_path()]
    );

    // 2) 머리글 JSP 5줄과 본문 JSP 6줄을 화면 줄로 바꿔 살아 있는 블록 하나로 건다(화면 주소로 걸려 모든
    // 블록에 든다). 본문의 tick이 머리글의 headerClock을 부르므로 두 곳에서 차례로 멈춘다.
    let local = project.join("tiles.do.js");
    std::fs::write(&local, page_lines::assemble(&blocks)).unwrap();
    session.map_source(&local, &live);
    let lines = [map.to_pages(&header, 4)[0], map.to_pages(&body, 5)[0]];
    assert_eq!(lines, [12, 20]);
    session.set_breakpoints(
        &local,
        lines
            .iter()
            .map(|l| Breakpoint {
                line: l + 1,
                condition: None,
            })
            .collect(),
    );
    let mut stops = Vec::new();
    let mut thread = None;
    while stops.len() < 2 {
        if let Some(thread) = thread {
            session.resume(thread);
        }
        let (stopped, frame) = top_frame(&mut session, &mut wait);
        thread = Some(stopped);
        eprintln!("멈춘 프레임: {frame:?}");
        let name = frame
            .source
            .as_ref()
            .and_then(|s| s.name.clone())
            .unwrap_or_default();
        assert!(name.contains("tiles.do"), "{name}");
        let (file, line) = map.to_file(frame.line as usize - 1).expect("JSP 줄");
        stops.push((file.to_path_buf(), line));
    }
    assert_eq!(stops, [(body.clone(), 5), (header.clone(), 4)]);

    session.stop();
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Terminated | DebugEvent::Closed(_))
    });
    drop(session);
    for dir in [servers, project, adapters::scratch_dir()] {
        for _ in 0..40 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!dir.exists(), "폴더가 남음: {}", dir.display());
    }
}

#[test]
#[ignore = "GitHub에서 vscode-js-debug(약 1.2MB)를 받는다. node와 Chrome(Windows는 Edge도 된다)이 필요하다"]
fn js_debug로_조각을_끼운_화면도_원래_html에서_블록의_글을_잘라_낸다() {
    let Some(kind) = browser() else {
        require("Chrome", false);
        return;
    };
    if !require("node", find_program("node").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("js-frag-servers"), temp_dir("js-frag"));
    let web_root = project.join("src/main/webapp");
    std::fs::create_dir_all(&web_root).unwrap();
    let port = web_server();
    let adapter = install_adapter(&servers, "js-debug", &project);
    let mut runtime_args = vec!["--headless=new", "--disable-gpu"];
    if cfg!(target_os = "linux") {
        runtime_args.extend(["--no-sandbox", "--disable-dev-shm-usage"]);
    }
    let trace = project.join("js-debug.log");
    let _dump = DumpOnPanic(trace.clone());
    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({
            "type": kind,
            "url": format!("http://127.0.0.1:{port}/app/frag.do"),
            "webRoot": web_root,
            "pathMapping": { "/app/": format!("{}/", web_root.display()) },
            "runtimeArgs": runtime_args,
            "timeout": 60000,
            "trace": { "logFile": trace },
        }),
    };
    let mut session = DebugSession::start(&adapter, launch, BTreeMap::new(), || {}).unwrap();
    // 화면 주소로 알리는 인라인 스크립트는 화면을 읽으며 돈 두 블록뿐이다(조각의 것은 jQuery가 따로 돌린다).
    let mut starts = Vec::new();
    let mut live = None;
    let mut events = Vec::new();
    let started = Instant::now();
    let mut quiet = None;
    while quiet.is_none_or(|at: Instant| at.elapsed() < Duration::from_millis(1500)) {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "시간 초과: {starts:?}"
        );
        for event in session.poll() {
            if let DebugEvent::LoadedSource { source, .. } = &event
                && let Some((page, line, column)) =
                    source.name.as_deref().and_then(page_lines::inline_script)
                && page.ends_with("/app/frag.do")
            {
                starts.push((line, column));
                live = Some(source.clone());
            }
            events.push(event);
        }
        if starts.len() >= 2 && quiet.is_none() {
            quiet = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    starts.sort();
    assert_eq!(starts, [(4, 9), (11, 9)]);
    let page = events
        .iter()
        .find_map(|e| match e {
            DebugEvent::LoadedSource { source, .. } => source
                .name
                .as_deref()
                .and_then(page_lines::inline_script)
                .map(|(page, ..)| page.replace('\u{a789}', ":")),
            _ => None,
        })
        .unwrap();
    let (mut seen, mut next) = (events, 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    // 조각의 스크립트는 화면(문서 차례로 두 블록 사이)에 남아, 화면의 글로는 어느 것이 블록인지 모른다.
    session.request(
        "evaluate",
        json!({ "expression": "Array.from(document.scripts).filter(s => !s.src).length", "context": "clipboard" }),
    );
    let count = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    ));
    assert_eq!(count["result"], "3");
    // CDP 프록시로 받은 원래 HTML에서 블록 자리의 글을 잘라 낸다(앱과 같다).
    session.request("requestCDPProxy", json!({}));
    let proxy = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "requestCDPProxy"),
    ));
    let proxy = cdp::Proxy::from_json(&proxy).expect("프록시 주소");
    let html = cdp::page_html(&proxy, &page).unwrap();
    assert_eq!(html, FRAG_PAGE, "화면이 받은 그대로");
    let blocks = page_lines::scripts_at(&html, &starts).expect("블록");
    assert!(blocks[0].2.contains("function loadDetail()"), "{blocks:?}");
    assert!(blocks[1].2.starts_with("\nvar count = 0;"), "{blocks:?}");

    // 블록 둘(화면 6·14줄)에 건다. 앱은 원래 HTML을 다시 받으면 같은 중단점을 다시 보내는데, js-debug는 그때
    // 빈 목록으로 답한다(걸어 둔 것은 그대로다). 다시 보낸 뒤에도 둘 다 걸린 것으로 보여야 한다.
    let local = project.join("frag.do.js");
    std::fs::write(&local, page_lines::assemble(&blocks)).unwrap();
    session.map_source(&local, &live.unwrap());
    for round in 0..2 {
        session.set_breakpoints(
            &local,
            [6, 14]
                .map(|line| Breakpoint {
                    line,
                    condition: None,
                })
                .to_vec(),
        );
        // 응답 뒤에 오는 "changed" 알림까지 받고 마지막 상태를 본다.
        let mut last = None;
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            for event in session.poll() {
                if let DebugEvent::Breakpoints { path, states } = event
                    && path == local
                {
                    last = Some(states);
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let last = last.unwrap_or_else(|| panic!("{round}번째: 중단점 상태가 오지 않음"));
        assert!(
            last.len() == 2 && last.iter().all(|s| s.verified),
            "{round}번째: {last:?}"
        );
    }

    session.stop();
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Terminated | DebugEvent::Closed(_))
    });
    drop(session);
    for dir in [servers, project, adapters::scratch_dir()] {
        for _ in 0..40 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!dir.exists(), "폴더가 남음: {}", dir.display());
    }
}

#[test]
#[ignore = "GitHub에서 vscode-js-debug(약 1.2MB)를 받는다. node와 Chrome(Windows는 Edge도 된다)이 필요하다"]
fn js_debug로_cdp_프록시로_화면을_다시_불러_맨_위_코드에서_멈추고_폼으로_연_화면을_가린다() {
    let Some(kind) = browser() else {
        require("Chrome", false);
        return;
    };
    if !require("node", find_program("node").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("js-reload-servers"), temp_dir("js-reload"));
    let web_root = project.join("src/main/webapp");
    std::fs::create_dir_all(&web_root).unwrap();
    let port = web_server();
    let adapter = install_adapter(&servers, "js-debug", &project);
    let mut runtime_args = vec!["--headless=new", "--disable-gpu"];
    if cfg!(target_os = "linux") {
        runtime_args.extend(["--no-sandbox", "--disable-dev-shm-usage"]);
    }
    let trace = project.join("js-debug.log");
    let _dump = DumpOnPanic(trace.clone());
    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({
            "type": kind,
            "url": format!("http://127.0.0.1:{port}/app/frag.do"),
            "webRoot": web_root,
            "pathMapping": { "/app/": format!("{}/", web_root.display()) },
            "runtimeArgs": runtime_args,
            "timeout": 60000,
            "trace": { "logFile": trace },
        }),
    };
    let mut session = DebugSession::start(&adapter, launch, BTreeMap::new(), || {}).unwrap();
    let mut starts = Vec::new();
    let mut live = None;
    let mut events = Vec::new();
    let started = Instant::now();
    let mut quiet = None;
    while quiet.is_none_or(|at: Instant| at.elapsed() < Duration::from_millis(1500)) {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "시간 초과: {starts:?}"
        );
        for event in session.poll() {
            if let DebugEvent::LoadedSource { source, .. } = &event
                && let Some((page, line, column)) =
                    source.name.as_deref().and_then(page_lines::inline_script)
                && page.ends_with("/app/frag.do")
            {
                starts.push((line, column));
                live = Some(source.clone());
            }
            events.push(event);
        }
        if starts.len() >= 2 && quiet.is_none() {
            quiet = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    starts.sort();
    let live = live.unwrap();
    let page = live
        .name
        .as_deref()
        .and_then(page_lines::inline_script)
        .unwrap()
        .0
        .replace('\u{a789}', ":");
    let (mut seen, mut next) = (events, 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    session.request("requestCDPProxy", json!({}));
    let proxy = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "requestCDPProxy"),
    ));
    let proxy = cdp::Proxy::from_json(&proxy).expect("프록시 주소");
    // 1) 주소를 쳐서 연 화면은 맨 위 화면이고 폼을 보내 연 것이 아니다(다시 불러도 된다).
    assert_eq!(
        cdp::page_loaded(&proxy, &page, Duration::from_secs(10), &[]).unwrap(),
        cdp::Loaded::Reload(cdp::Target::Top)
    );
    // 2) 맨 위 코드(`var count = 0;`, 화면 12줄)는 이미 돌았다. 화면 줄로 걸고 다시 부르면 거기서 멈춘다.
    let blocks = page_lines::scripts_at(FRAG_PAGE, &starts).expect("블록");
    let local = project.join("frag.do.js");
    std::fs::write(&local, page_lines::assemble(&blocks)).unwrap();
    session.map_source(&local, &live);
    session.set_breakpoints(
        &local,
        vec![Breakpoint {
            line: 12,
            condition: None,
        }],
    );
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    cdp::reload(&proxy, &cdp::Target::Top).unwrap();
    let (thread, frame) = top_frame(&mut session, &mut wait);
    eprintln!("다시 부른 화면의 프레임: {frame:?}");
    assert_eq!(frame.line, 12);
    assert!(
        frame
            .source
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .is_some_and(|n| n.contains("frag.do")),
        "{frame:?}"
    );
    // 3) 화면을 읽는 중에 멈춰 있다(그동안은 CDP가 화면 HTML에 답하지 않아 앱은 이어 간 뒤에 받는다).
    assert!(!cdp::wait_loaded(&proxy, Duration::from_millis(500)).unwrap());
    session.set_breakpoints(&local, Vec::new());
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    session.resume(thread);
    assert!(cdp::wait_loaded(&proxy, Duration::from_secs(10)).unwrap());
    assert_eq!(
        cdp::page_html(&proxy, &page).unwrap(),
        FRAG_PAGE,
        "이어 가면 받는다"
    );
    // 4) 폼(POST)을 보내 연 화면은 다시 부르면 폼을 다시 보낼 수 있다.
    session.request(
        "evaluate",
        json!({ "expression": "var f = document.createElement('form'); f.method = 'post'; f.action = '/app/frag.do'; document.body.appendChild(f); setTimeout(function () { f.submit(); }, 100); 1", "context": "repl" }),
    );
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let loaded = cdp::page_loaded(&proxy, &page, Duration::from_secs(10), &[]);
        if loaded == Ok(cdp::Loaded::FormSubmit) {
            break;
        }
        assert!(Instant::now() < deadline, "폼으로 연 화면: {loaded:?}");
        std::thread::sleep(Duration::from_millis(200));
    }

    session.stop();
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Terminated | DebugEvent::Closed(_))
    });
    drop(session);
    for dir in [servers, project, adapters::scratch_dir()] {
        for _ in 0..40 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!dir.exists(), "폴더가 남음: {}", dir.display());
    }
}

#[test]
#[ignore = "GitHub에서 vscode-js-debug(약 1.2MB)를 받는다. node와 Chrome(Windows는 Edge도 된다)이 필요하다"]
fn js_debug로_프레임_안_화면의_블록을_디버거에서_받고_그_프레임만_다시_연다() {
    let Some(kind) = browser() else {
        require("Chrome", false);
        return;
    };
    if !require("node", find_program("node").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("js-frame-servers"), temp_dir("js-frame"));
    let web_root = project.join("src/main/webapp");
    std::fs::create_dir_all(&web_root).unwrap();
    let port = web_server();
    let adapter = install_adapter(&servers, "js-debug", &project);
    let mut runtime_args = vec!["--headless=new", "--disable-gpu"];
    if cfg!(target_os = "linux") {
        runtime_args.extend(["--no-sandbox", "--disable-dev-shm-usage"]);
    }
    let trace = project.join("js-debug.log");
    let _dump = DumpOnPanic(trace.clone());
    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({
            "type": kind,
            "url": format!("http://127.0.0.1:{port}/app/main.do"),
            "webRoot": web_root,
            "pathMapping": { "/app/": format!("{}/", web_root.display()) },
            "runtimeArgs": runtime_args,
            "timeout": 60000,
            "trace": { "logFile": trace },
        }),
    };
    let mut session = DebugSession::start(&adapter, launch, BTreeMap::new(), || {}).unwrap();
    // 프레임 안 화면의 인라인 스크립트도 그 화면(프레임) 주소로 알린다.
    let mut starts = Vec::new();
    let mut live = None;
    let mut events = Vec::new();
    let started = Instant::now();
    let mut quiet = None;
    while quiet.is_none_or(|at: Instant| at.elapsed() < Duration::from_millis(1500)) {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "시간 초과: {starts:?}"
        );
        for event in session.poll() {
            if let DebugEvent::LoadedSource { source, .. } = &event
                && let Some((page, line, column)) =
                    source.name.as_deref().and_then(page_lines::inline_script)
                && page.ends_with("/app/frag.do")
            {
                starts.push((line, column));
                live = Some(source.clone());
            }
            events.push(event);
        }
        if starts.len() >= 2 && quiet.is_none() {
            quiet = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    starts.sort();
    assert_eq!(starts, [(4, 9), (11, 9)]);
    let live = live.unwrap();
    let page = live
        .name
        .as_deref()
        .and_then(page_lines::inline_script)
        .unwrap()
        .0
        .replace('\u{a789}', ":");
    let (mut seen, mut next) = (events, 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    session.request("requestCDPProxy", json!({}));
    let proxy = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "requestCDPProxy"),
    ));
    let proxy = cdp::Proxy::from_json(&proxy).expect("프록시 주소");
    // 1) 프레임 안 화면은 브라우저가 원래 HTML을 남겨 두지 않는다. 블록의 글은 디버거에서 블록마다 받는다.
    eprintln!(
        "프레임 화면의 원래 HTML: {:?}",
        cdp::page_html(&proxy, &page).map(|h| h.len())
    );
    let expected: Vec<(usize, usize, String)> = page_lines::scripts_at(FRAG_PAGE, &starts)
        .unwrap()
        .into_iter()
        .map(|(line, column, text)| (line, column, text.to_owned()))
        .collect();
    assert_eq!(
        cdp::script_blocks(&proxy, &page, &starts).unwrap(),
        expected
    );
    // 2) 처음 주소(src)를 보이는 프레임은 그 프레임만 다시 열어도 된다.
    let Ok(cdp::Loaded::Reload(target)) =
        cdp::page_loaded(&proxy, &page, Duration::from_secs(10), &[])
    else {
        panic!("프레임을 다시 열 수 있어야 한다");
    };
    assert!(matches!(&target, cdp::Target::Frame { url, .. } if url.ends_with("/app/frag.do")));
    // 3) 블록을 받으며 건 중단점은 지웠다(조건이 거짓이라 멈추지도 않았다). 맨 위 코드(화면 12줄)에 걸고
    // 프레임만 다시 열면 거기서 멈춘다.
    let local = project.join("frag.do.js");
    std::fs::write(
        &local,
        page_lines::assemble(&page_lines::scripts_at(FRAG_PAGE, &starts).unwrap()),
    )
    .unwrap();
    session.map_source(&local, &live);
    session.set_breakpoints(
        &local,
        vec![Breakpoint {
            line: 12,
            condition: None,
        }],
    );
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    cdp::reload(&proxy, &target).unwrap();
    let (thread, frame) = top_frame(&mut session, &mut wait);
    eprintln!("다시 연 프레임의 프레임: {frame:?}");
    assert_eq!(frame.line, 12);
    assert!(
        frame
            .source
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .is_some_and(|n| n.contains("frag.do")),
        "{frame:?}"
    );
    // 프레임이 다시 열리는 중에 멈춰 있어도 블록의 글을 받는다.
    assert_eq!(
        cdp::script_blocks(&proxy, &page, &starts).unwrap(),
        expected
    );
    session.set_breakpoints(&local, Vec::new());
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    session.resume(thread);
    // 4) 프레임이 링크로 다른 화면으로 가면(처음 주소와 다르다) 다시 열지 않는다.
    session.request(
        "evaluate",
        json!({ "expression": "document.getElementById('content').contentWindow.location.href = '/app/list.do'; 1", "context": "repl" }),
    );
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    );
    let moved = page.replace("/app/frag.do", "/app/list.do");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let loaded = cdp::page_loaded(&proxy, &moved, Duration::from_secs(10), &[]);
        if loaded == Ok(cdp::Loaded::FrameMoved) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "다른 화면으로 간 프레임: {loaded:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    session.stop();
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Terminated | DebugEvent::Closed(_))
    });
    drop(session);
    for dir in [servers, project, adapters::scratch_dir()] {
        for _ in 0..40 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!dir.exists(), "폴더가 남음: {}", dir.display());
    }
}

#[test]
#[ignore = "GitHub에서 vscode-js-debug(약 1.2MB)를 받는다. node와 Chrome(Windows는 Edge도 된다)이 필요하다"]
fn js_debug로_프레임셋과_프레임_안의_프레임도_블록을_받고_그_프레임만_다시_연다() {
    let Some(kind) = browser() else {
        require("Chrome", false);
        return;
    };
    if !require("node", find_program("node").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("js-frameset-servers"), temp_dir("js-frameset"));
    let web_root = project.join("src/main/webapp");
    std::fs::create_dir_all(&web_root).unwrap();
    let port = web_server();
    let adapter = install_adapter(&servers, "js-debug", &project);
    let mut runtime_args = vec!["--headless=new", "--disable-gpu"];
    if cfg!(target_os = "linux") {
        runtime_args.extend(["--no-sandbox", "--disable-dev-shm-usage"]);
    }
    let trace = project.join("js-debug.log");
    let _dump = DumpOnPanic(trace.clone());
    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({
            "type": kind,
            "url": format!("http://127.0.0.1:{port}/app/frameset.do"),
            "webRoot": web_root,
            "pathMapping": { "/app/": format!("{}/", web_root.display()) },
            "runtimeArgs": runtime_args,
            "timeout": 60000,
            "trace": { "logFile": trace },
        }),
    };
    let mut session = DebugSession::start(&adapter, launch, BTreeMap::new(), || {}).unwrap();
    // 프레임셋의 프레임(메뉴·본문)과 본문 프레임 안의 프레임(조각 화면)의 인라인 스크립트를 저마다 그 화면
    // 주소로 알린다.
    let mut blocks: BTreeMap<String, (Vec<(usize, usize)>, ferrite_dap::Source)> = BTreeMap::new();
    let mut events = Vec::new();
    let started = Instant::now();
    let mut quiet = None;
    while quiet.is_none_or(|at: Instant| at.elapsed() < Duration::from_millis(1500)) {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "시간 초과: {:?}",
            blocks.keys().collect::<Vec<_>>()
        );
        for event in session.poll() {
            if let DebugEvent::LoadedSource { source, .. } = &event
                && let Some((page, line, column)) =
                    source.name.as_deref().and_then(page_lines::inline_script)
            {
                let entry = blocks
                    .entry(page.replace('\u{a789}', ":"))
                    .or_insert_with(|| (Vec::new(), source.clone()));
                entry.0.push((line, column));
                entry.1 = source.clone();
            }
            events.push(event);
        }
        if blocks
            .values()
            .map(|(starts, _)| starts.len())
            .sum::<usize>()
            >= 4
            && quiet.is_none()
        {
            quiet = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let find = |path: &str| {
        let (page, (starts, live)) = blocks
            .iter()
            .find(|(page, _)| page.ends_with(path))
            .unwrap_or_else(|| panic!("{path}: {:?}", blocks.keys().collect::<Vec<_>>()));
        let mut starts = starts.clone();
        starts.sort();
        (page.clone(), starts, live.clone())
    };
    let (menu, menu_starts, _) = find("/app/menu.do");
    let (main, main_starts, _) = find("/app/main.do");
    let (frag, frag_starts, frag_live) = find("/app/frag.do");
    assert_eq!(menu_starts, [(3, 9)]);
    assert_eq!(main_starts, [(3, 9)]);
    assert_eq!(frag_starts, [(4, 9), (11, 9)]);
    let (mut seen, mut next) = (events, 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    session.request("requestCDPProxy", json!({}));
    let proxy = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "requestCDPProxy"),
    ));
    let proxy = cdp::Proxy::from_json(&proxy).expect("프록시 주소");
    // 1) 프레임의 블록 글은 디버거에서 받는다(프레임셋의 프레임과 프레임 안의 프레임 모두).
    let owned = |page: &str, starts: &[(usize, usize)]| -> Vec<(usize, usize, String)> {
        page_lines::scripts_at(page, starts)
            .unwrap()
            .into_iter()
            .map(|(line, column, text)| (line, column, text.to_owned()))
            .collect()
    };
    assert_eq!(
        cdp::script_blocks(&proxy, &menu, &menu_starts).unwrap(),
        owned(MENU_PAGE, &menu_starts)
    );
    assert_eq!(
        cdp::script_blocks(&proxy, &main, &main_starts).unwrap(),
        owned(MAIN_PAGE, &main_starts)
    );
    let expected = owned(FRAG_PAGE, &frag_starts);
    assert_eq!(
        cdp::script_blocks(&proxy, &frag, &frag_starts).unwrap(),
        expected
    );
    // 2) 처음 주소를 보이는 프레임(`<frame>`·프레임 안의 `<iframe>`)은 그 프레임만 다시 열어도 된다.
    let mut targets = BTreeMap::new();
    for page in [&menu, &main, &frag] {
        let loaded = cdp::page_loaded(&proxy, page, Duration::from_secs(10), &[]);
        let Ok(cdp::Loaded::Reload(target @ cdp::Target::Frame { .. })) = loaded else {
            panic!("{page}: {loaded:?}");
        };
        targets.insert(page.clone(), target);
    }
    // 프레임 안의 프레임은 품은 화면들(프레임셋, 본문 프레임)을 함께 알린다. 앱은 품은 화면을 다시 부르면 이
    // 프레임을 따로 부르지 않는다.
    let frameset = main.replace("/app/main.do", "/app/frameset.do");
    assert_eq!(targets[&frag].ancestors(), [frameset.clone(), main.clone()]);
    assert_eq!(targets[&main].ancestors(), [frameset]);
    // 3) 프레임 안의 프레임만 다시 열면 그 화면의 맨 위 코드(화면 12줄)에서 멈춘다.
    let local = project.join("frag.do.js");
    std::fs::write(
        &local,
        page_lines::assemble(&page_lines::scripts_at(FRAG_PAGE, &frag_starts).unwrap()),
    )
    .unwrap();
    session.map_source(&local, &frag_live);
    session.set_breakpoints(
        &local,
        vec![Breakpoint {
            line: 12,
            condition: None,
        }],
    );
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    cdp::reload(&proxy, &targets[&frag]).unwrap();
    let (thread, frame) = top_frame(&mut session, &mut wait);
    eprintln!("다시 연 프레임 안 프레임의 프레임: {frame:?}");
    assert_eq!(frame.line, 12);
    assert!(
        frame
            .source
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .is_some_and(|n| n.contains("frag.do")),
        "{frame:?}"
    );
    assert_eq!(
        cdp::script_blocks(&proxy, &frag, &frag_starts).unwrap(),
        expected
    );
    session.set_breakpoints(&local, Vec::new());
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    session.resume(thread);
    // 4) 메뉴의 링크로 본문 프레임에 다른 화면을 열면(처음 주소와 다르다) 다시 열지 않는다.
    session.request(
        "evaluate",
        json!({ "expression": "window.frames['left'].document.getElementById('list').click(); 1", "context": "repl" }),
    );
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    );
    let moved = main.replace("/app/main.do", "/app/list.do");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let loaded = cdp::page_loaded(&proxy, &moved, Duration::from_secs(10), &[]);
        if loaded == Ok(cdp::Loaded::FrameMoved) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "다른 화면으로 간 본문 프레임: {loaded:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    session.stop();
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Terminated | DebugEvent::Closed(_))
    });
    drop(session);
    for dir in [servers, project, adapters::scratch_dir()] {
        for _ in 0..40 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!dir.exists(), "폴더가 남음: {}", dir.display());
    }
}

#[test]
#[ignore = "GitHub에서 vscode-js-debug(약 1.2MB)를 받는다. node와 Chrome(Windows는 Edge도 된다)이 필요하다"]
fn js_debug로_다른_사이트_iframe은_그_세션의_cdp로_블록을_받고_그_프레임만_다시_연다() {
    let Some(kind) = browser() else {
        require("Chrome", false);
        return;
    };
    if !require("node", find_program("node").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("js-cross-servers"), temp_dir("js-cross"));
    let web_root = project.join("src/main/webapp");
    std::fs::create_dir_all(&web_root).unwrap();
    let port = web_server();
    let adapter = install_adapter(&servers, "js-debug", &project);
    let mut runtime_args = vec!["--headless=new", "--disable-gpu"];
    if cfg!(target_os = "linux") {
        runtime_args.extend(["--no-sandbox", "--disable-dev-shm-usage"]);
    }
    let trace = project.join("js-debug.log");
    let _dump = DumpOnPanic(trace.clone());
    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({
            "type": kind,
            "url": format!("http://127.0.0.1:{port}/app/cross.do"),
            "webRoot": web_root,
            "pathMapping": { "/app/": format!("{}/", web_root.display()) },
            "runtimeArgs": runtime_args,
            "timeout": 60000,
            "trace": { "logFile": trace },
        }),
    };
    let mut session = DebugSession::start(&adapter, launch, BTreeMap::new(), || {}).unwrap();
    let top = format!("127.0.0.1:{port}/app/cross.do");
    let frame = format!("localhost:{port}/app/frag.do");
    // 다른 사이트의 프레임은 브라우저가 다른 프로세스로 띄워 js-debug가 페이지 세션 아래의 세션으로 따로 알린다.
    let (mut top_session, mut frame_session) = (None, None);
    let mut starts = Vec::new();
    let mut live = None;
    let mut events = Vec::new();
    let started = Instant::now();
    let mut quiet = None;
    while quiet.is_none_or(|at: Instant| at.elapsed() < Duration::from_millis(1500)) {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "시간 초과: {starts:?}"
        );
        for event in session.poll() {
            if let DebugEvent::LoadedSource {
                source, session, ..
            } = &event
                && let Some((page, line, column)) =
                    source.name.as_deref().and_then(page_lines::inline_script)
            {
                let page = page.replace('\u{a789}', ":");
                if page == frame {
                    starts.push((line, column));
                    live = Some(source.clone());
                    frame_session = Some(*session);
                } else if page == top {
                    top_session = Some(*session);
                }
            }
            events.push(event);
        }
        if starts.len() >= 2 && top_session.is_some() && quiet.is_none() {
            quiet = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    starts.sort();
    assert_eq!(starts, [(4, 9), (11, 9)]);
    let (top_session, frame_session) = (top_session.unwrap(), frame_session.unwrap());
    assert_ne!(top_session, frame_session);
    assert_eq!(session.parents(frame_session), [top_session]);
    let live = live.unwrap();
    let (mut seen, mut next) = (events, 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    let mut proxy_of = |session: &mut DebugSession, id: u64| {
        session
            .request_in(id, "requestCDPProxy", json!({}))
            .unwrap();
        let proxy = response(wait(
            session,
            &|e| matches!(e, DebugEvent::Response { command, .. } if command == "requestCDPProxy"),
        ));
        cdp::Proxy::from_json(&proxy).expect("프록시 주소")
    };
    let top_proxy = proxy_of(&mut session, top_session);
    let frame_proxy = proxy_of(&mut session, frame_session);
    // 1) 페이지 세션의 프록시로 맨 위 화면을 다룬다(그 세션의 프레임 트리에는 다른 사이트의 프레임이 없다).
    assert_eq!(
        cdp::page_html(&top_proxy, &top).unwrap(),
        CROSS_PAGE.replace("{port}", &port.to_string())
    );
    assert_eq!(
        cdp::page_loaded(&top_proxy, &top, Duration::from_secs(10), &[]).unwrap(),
        cdp::Loaded::Reload(cdp::Target::Top)
    );
    assert_eq!(
        cdp::script_blocks(&top_proxy, &frame, &starts).unwrap_err(),
        cdp::NOT_OPEN
    );
    // 2) 프레임은 그 세션의 프록시로: 블록의 글은 디버거에서 받고, 품은 화면은 페이지 세션에서 찾는다.
    let expected: Vec<(usize, usize, String)> = page_lines::scripts_at(FRAG_PAGE, &starts)
        .unwrap()
        .into_iter()
        .map(|(line, column, text)| (line, column, text.to_owned()))
        .collect();
    assert_eq!(
        cdp::script_blocks(&frame_proxy, &frame, &starts).unwrap(),
        expected
    );
    let parents = [top_proxy.clone()];
    let Ok(cdp::Loaded::Reload(target)) =
        cdp::page_loaded(&frame_proxy, &frame, Duration::from_secs(10), &parents)
    else {
        panic!("다른 사이트의 프레임을 다시 열 수 있어야 한다");
    };
    assert!(
        matches!(&target, cdp::Target::Frame { url, ancestors, .. }
            if url.ends_with("/app/frag.do") && *ancestors == [top.clone()]),
        "{target:?}"
    );
    // 3) 멈추지 않았으면 콘솔의 식은 맨 위 화면에서 돈다(나중에 열린 프레임 세션이 아니다).
    session.request(
        "evaluate",
        json!({ "expression": "location.href", "context": "repl" }),
    );
    let href = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    ));
    assert!(
        href["result"].as_str().unwrap_or_default().contains(&top),
        "{href}"
    );
    // 4) 프레임의 맨 위 코드(화면 12줄)에 걸고 그 프레임만 다시 열면 거기서 멈춘다.
    let local = project.join("frag.do.js");
    std::fs::write(
        &local,
        page_lines::assemble(&page_lines::scripts_at(FRAG_PAGE, &starts).unwrap()),
    )
    .unwrap();
    session.map_source(&local, &live);
    session.set_breakpoints(
        &local,
        vec![Breakpoint {
            line: 12,
            condition: None,
        }],
    );
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    cdp::reload(&frame_proxy, &target).unwrap();
    let (thread, frame_top) = top_frame(&mut session, &mut wait);
    assert_eq!(frame_top.line, 12);
    assert!(
        frame_top
            .source
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .is_some_and(|n| n.contains("frag.do")),
        "{frame_top:?}"
    );
    session.set_breakpoints(&local, Vec::new());
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, .. } if *path == local),
    );
    session.resume(thread);
    // 5) 프레임 안에서 다른 화면으로 가면(품은 프레임의 src와 다르다) 다시 열지 않는다.
    session
        .request_in(
            frame_session,
            "evaluate",
            json!({ "expression": "location.href = '/app/list.do'; 1", "context": "repl" }),
        )
        .unwrap();
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    );
    let moved = frame.replace("/app/frag.do", "/app/list.do");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let loaded = cdp::page_loaded(&frame_proxy, &moved, Duration::from_secs(10), &parents);
        if loaded == Ok(cdp::Loaded::FrameMoved) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "다른 화면으로 간 프레임: {loaded:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    session.stop();
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Terminated | DebugEvent::Closed(_))
    });
    drop(session);
    for dir in [servers, project, adapters::scratch_dir()] {
        for _ in 0..40 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!dir.exists(), "폴더가 남음: {}", dir.display());
    }
}

#[test]
#[ignore = "GitHub에서 vscode-js-debug(약 1.2MB)를 받는다. node와 Chrome(Windows는 Edge도 된다)이 필요하다"]
fn js_debug로_워커_스크립트의_중단점에서_멈추고_멈추지_않았으면_화면에서_식을_돈다() {
    let Some(kind) = browser() else {
        require("Chrome", false);
        return;
    };
    if !require("node", find_program("node").is_some()) {
        return;
    }
    let (servers, project) = (temp_dir("js-worker-servers"), temp_dir("js-worker"));
    let web_root = project.join("src/main/webapp");
    std::fs::create_dir_all(web_root.join("js")).unwrap();
    let worker_js = web_root.join("js/worker.js");
    std::fs::write(&worker_js, WORKER_JS).unwrap();
    let port = web_server();
    let adapter = install_adapter(&servers, "js-debug", &project);
    let mut runtime_args = vec!["--headless=new", "--disable-gpu"];
    if cfg!(target_os = "linux") {
        runtime_args.extend(["--no-sandbox", "--disable-dev-shm-usage"]);
    }
    let trace = project.join("js-debug.log");
    let _dump = DumpOnPanic(trace.clone());
    let launch = Launch {
        adapter_id: "js-debug".into(),
        attach: false,
        arguments: json!({
            "type": kind,
            "url": format!("http://127.0.0.1:{port}/app/worker.do"),
            "webRoot": web_root,
            "pathMapping": { "/app/": format!("{}/", web_root.display()) },
            "runtimeArgs": runtime_args,
            "timeout": 60000,
            "trace": { "logFile": trace },
        }),
    };
    // 워커 스크립트(로컬 파일)의 메시지 함수(3줄)에 건다.
    let breakpoints = BTreeMap::from([(
        worker_js.clone(),
        vec![Breakpoint {
            line: 3,
            condition: None,
        }],
    )]);
    let mut session = DebugSession::start(&adapter, launch, breakpoints, || {}).unwrap();
    let (mut seen, mut next) = (Vec::new(), 0);
    let mut wait = |session: &mut DebugSession, done: &dyn Fn(&DebugEvent) -> bool| {
        wait_for(session, &mut seen, &mut next, done)
    };
    // 1) 워커는 페이지 세션 아래의 세션으로 열린다. 로컬 파일에 건 중단점은 걸린다.
    let DebugEvent::LoadedSource { session: page, .. } = wait(&mut session, &|e| {
        matches!(e, DebugEvent::LoadedSource { source, .. }
            if source.name.as_deref().is_some_and(|n| n.contains("worker.do")))
    }) else {
        unreachable!()
    };
    let DebugEvent::LoadedSource {
        session: worker, ..
    } = wait(&mut session, &|e| {
        matches!(e, DebugEvent::LoadedSource { source, .. }
            if source.name.as_deref().is_some_and(|n| n.ends_with("worker.js")))
    })
    else {
        unreachable!()
    };
    assert_eq!(session.parents(worker), [page]);
    wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Breakpoints { path, states } if *path == worker_js && states[0].verified),
    );
    // 2) 멈추지 않았으면 콘솔의 식은 화면에서 돈다(워커 세션이 나중에 열렸어도). 화면이 워커에 메시지를 보내면
    // 워커 스크립트의 중단점에서 멈추고, 그때 요청은 워커 세션에 간다.
    assert_eq!(session.focus_id(), page);
    session.request(
        "evaluate",
        json!({ "expression": "ask(21)", "context": "repl" }),
    );
    let (thread, frame) = top_frame(&mut session, &mut wait);
    assert_eq!(frame.line, 3);
    assert_eq!(
        std::fs::canonicalize(frame.source.as_ref().and_then(|s| s.path.clone()).unwrap()).unwrap(),
        std::fs::canonicalize(&worker_js).unwrap()
    );
    assert_eq!(session.focus_id(), worker);
    session.evaluate("e.data", Some(frame.id));
    let data = response(wait(
        &mut session,
        &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
    ));
    assert_eq!(data["result"], "21");
    session.resume(thread);
    // 3) 이어 가면 다시 화면에서 돈다: 워커가 돌려준 답을 화면이 받았다.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        session.request(
            "evaluate",
            json!({ "expression": "String(window.workerAnswer)", "context": "repl" }),
        );
        let answer = response(wait(
            &mut session,
            &|e| matches!(e, DebugEvent::Response { command, .. } if command == "evaluate"),
        ));
        if answer["result"] == "'42'" || answer["result"] == "42" {
            break;
        }
        assert!(Instant::now() < deadline, "워커의 답: {answer}");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(session.focus_id(), page);
    session.stop();
    wait(&mut session, &|e| {
        matches!(e, DebugEvent::Terminated | DebugEvent::Closed(_))
    });
    drop(session);
    for dir in [servers, project, adapters::scratch_dir()] {
        for _ in 0..40 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!dir.exists(), "폴더가 남음: {}", dir.display());
    }
}
