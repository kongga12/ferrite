//! 에디터 커서가 든 테스트(메서드·함수)를 소스 글만 보고 알아본다(언어 서버 없이). 테스트 창이 그것만 돌린다
//! (`test_report::run_only`). 돌릴 때의 묶음(`suite`)과 이름은 그 도구의 보고서가 적는 꼴에 맞춘다. 커서가 테스트
//! 메서드 밖이면 묶음 전체(이름이 빈 것)다: Java·Kotlin은 클래스, Python은 품은 `Test` 클래스나 파일, Go는 패키지.
//! - Java·Kotlin: `@Test`류 어노테이션이 붙은 메서드. 묶음은 `패키지.클래스`(커서 위로 가장 가까운 클래스).
//! - Python: `test`로 시작하는 함수. 묶음은 돌린 폴더 기준 모듈(`tests.test_a`)과 품은 클래스(`.TestX`).
//! - Go: `func TestX(t *testing.T)`. 묶음은 `go.mod`의 모듈 경로와 폴더(`example.com/board/calc`).

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::json;

use crate::debug_config::DebugConfig;

/// 커서의 테스트가 쓰는 테스트 도구의 갈래.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestLang {
    /// Maven·Gradle.
    Jvm,
    Python,
    Go,
}

impl TestLang {
    /// 파일 확장자로 고른다.
    pub fn of(path: &Path) -> Option<TestLang> {
        match path.extension()?.to_str()? {
            "java" | "kt" => Some(TestLang::Jvm),
            "py" => Some(TestLang::Python),
            "go" => Some(TestLang::Go),
            _ => None,
        }
    }
}

/// 커서가 든 테스트.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestAt {
    pub lang: TestLang,
    pub suite: String,
    /// 빈 글이면 묶음 전체다.
    pub name: String,
}

/// `path` 파일(글 `text`)의 `line`(0부터) 줄이 든 테스트. Python의 모듈 이름은 테스트를 돌릴 폴더(`dir`) 기준이다.
/// 테스트 메서드·함수 밖이면 그 묶음 전체다. 파일에 테스트가 없으면 None.
pub fn test_at(path: &Path, text: &str, line: usize, dir: &Path) -> Option<TestAt> {
    let lines: Vec<&str> = text.lines().collect();
    if line >= lines.len() {
        return None;
    }
    match TestLang::of(path)? {
        TestLang::Jvm => jvm(&lines, line),
        TestLang::Python => python(path, &lines, line, dir),
        TestLang::Go => go(path, &lines, line),
    }
}

static JAVA_METHOD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bvoid\s+([A-Za-z_]\w*)\s*\(").unwrap());
static KOTLIN_FUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfun\s+([A-Za-z_]\w*)\s*\(").unwrap());
static TEST_ANNOTATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@(?:[\w.]+\.)?(?:Test|ParameterizedTest|RepeatedTest|TestFactory|TestTemplate)\b")
        .unwrap()
});
static PACKAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*package\s+([\w.]+)").unwrap());
static CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:class|object)\s+([A-Za-z_]\w*)").unwrap());

/// Java·Kotlin: 커서 위로 가장 가까운 메서드가 커서를 품고 테스트 어노테이션이 붙었으면 그 테스트. 아니면 커서 위로
/// 가장 가까운 클래스(위에 없으면 파일의 첫 클래스) 전체다. 파일에 테스트 어노테이션이 없으면 None.
fn jvm(lines: &[&str], line: usize) -> Option<TestAt> {
    if !lines.iter().any(|l| TEST_ANNOTATION.is_match(l)) {
        return None;
    }
    let class_above = |end: usize| {
        lines[..=end]
            .iter()
            .rev()
            .find_map(|l| CLASS.captures(l).map(|c| c[1].to_owned()))
    };
    let (class, name) = match jvm_test_method(lines, line) {
        Some((start, name)) => (class_above(start)?, name),
        None => (
            class_above(line).or_else(|| {
                lines
                    .iter()
                    .find_map(|l| CLASS.captures(l).map(|c| c[1].to_owned()))
            })?,
            String::new(),
        ),
    };
    let package = lines
        .iter()
        .find_map(|l| PACKAGE.captures(l).map(|c| c[1].to_owned()));
    Some(TestAt {
        lang: TestLang::Jvm,
        suite: match package {
            Some(package) => format!("{package}.{class}"),
            None => class,
        },
        name,
    })
}

/// 커서를 품은 테스트 메서드의 (시작 줄, 이름). 커서 위로 가장 가까운 메서드가 중괄호로 커서를 품고 그 줄이나 위의
/// 어노테이션·주석·빈 줄에 테스트 어노테이션이 있어야 한다.
fn jvm_test_method(lines: &[&str], line: usize) -> Option<(usize, String)> {
    let (start, name) = (0..=line).rev().find_map(|i| {
        let m = JAVA_METHOD
            .captures(lines[i])
            .or_else(|| KOTLIN_FUN.captures(lines[i]))?;
        Some((i, m[1].to_owned()))
    })?;
    if block_end(lines, start)? < line {
        return None;
    }
    let mut annotated = TEST_ANNOTATION.is_match(lines[start]);
    for above in lines[..start].iter().rev() {
        let trimmed = above.trim();
        let note = trimmed.is_empty()
            || trimmed.starts_with('@')
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*');
        if !note {
            break;
        }
        annotated |= TEST_ANNOTATION.is_match(trimmed);
    }
    annotated.then_some((start, name))
}

/// `start` 줄에서 처음 여는 중괄호의 블록이 닫히는 줄. 글자열·글자·주석 안의 중괄호는 세지 않는다(Java·Kotlin·Go).
fn block_end(lines: &[&str], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut opened = false;
    let mut in_block_comment = false;
    for (i, line) in lines.iter().enumerate().skip(start) {
        let mut chars = line.chars().peekable();
        let mut quote: Option<char> = None;
        while let Some(c) = chars.next() {
            if in_block_comment {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    in_block_comment = false;
                }
                continue;
            }
            if let Some(q) = quote {
                if c == '\\' {
                    chars.next();
                } else if c == q {
                    quote = None;
                }
                continue;
            }
            match c {
                '"' | '\'' | '`' => quote = Some(c),
                '/' if chars.peek() == Some(&'/') => break,
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    in_block_comment = true;
                }
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    if opened && depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

static PY_DEF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(").unwrap());
static PY_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)class\s+([A-Za-z_]\w*)").unwrap());

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Python: 커서를 품은 가장 안쪽 `def`가 `test`로 시작하면 그 테스트(품은 클래스들을 묶음에 붙인다). 아니면 커서를
/// 품은 가장 안쪽 클래스가 `Test`로 시작하면 그 클래스 전체, 아니면 파일 전체다. 파일에 `test` 함수가 없으면 None.
fn python(path: &Path, lines: &[&str], line: usize, dir: &Path) -> Option<TestAt> {
    if !lines
        .iter()
        .any(|l| PY_DEF.captures(l).is_some_and(|c| c[2].starts_with("test")))
    {
        return None;
    }
    // 커서 줄부터 위로, 커서를 품는(들여쓰기가 더 얕은) 첫 `def`.
    let mut limit = usize::MAX;
    let mut found = None;
    for i in (0..=line).rev() {
        let text = lines[i];
        if text.trim().is_empty() {
            continue;
        }
        let depth = indent(text);
        if let Some(c) = PY_DEF.captures(text)
            && (i == line || depth < limit)
        {
            found = Some((i, depth, c[2].to_owned()));
            break;
        }
        limit = limit.min(depth);
    }
    let mut module = python_module(path, dir);
    match found {
        Some((start, depth, name)) if name.starts_with("test") => {
            module.extend(python_classes(lines, start, depth));
            Some(TestAt {
                lang: TestLang::Python,
                suite: module.join("."),
                name,
            })
        }
        _ => {
            // 커서 줄(빈 줄이면 들여쓰기를 모른다)을 품은 클래스들.
            let depth = lines[..=line]
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map_or(0, |l| indent(l) + 1);
            let mut classes = python_classes(lines, line + 1, depth);
            while classes.last().is_some_and(|c| !c.starts_with("Test")) {
                classes.pop();
            }
            module.extend(classes);
            Some(TestAt {
                lang: TestLang::Python,
                suite: module.join("."),
                name: String::new(),
            })
        }
    }
}

/// `end` 줄 앞에서 들여쓰기 `depth`보다 얕게 품은 클래스들(바깥부터).
fn python_classes(lines: &[&str], end: usize, depth: usize) -> Vec<String> {
    let mut classes = Vec::new();
    let mut outer = depth;
    for text in lines[..end].iter().rev() {
        if outer == 0 {
            break;
        }
        if text.trim().is_empty() {
            continue;
        }
        let d = indent(text);
        if d < outer {
            if let Some(c) = PY_CLASS.captures(text) {
                classes.push(c[2].to_owned());
            }
            outer = d;
        }
    }
    classes.reverse();
    classes
}

/// 돌릴 폴더(`dir`) 기준 모듈 이름의 마디(`tests`, `test_calc`).
fn python_module(path: &Path, dir: &Path) -> Vec<String> {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .with_extension("")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

static GO_TEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^func\s+(Test\w*)\s*\(\s*\w+\s+\*testing\.T\s*\)").unwrap());
static GO_MODULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^module\s+(\S+)").unwrap());

/// Go: 커서를 품은 `func TestX(t *testing.T)`. 아니면 패키지 전체다. 묶음은 `go.mod`의 모듈 경로에 그 폴더를 붙인
/// 패키지 경로다. 파일에 테스트 함수가 없으면 None.
fn go(path: &Path, lines: &[&str], line: usize) -> Option<TestAt> {
    if !lines.iter().any(|l| GO_TEST.is_match(l)) {
        return None;
    }
    let name = (0..=line)
        .rev()
        .find_map(|i| GO_TEST.captures(lines[i]).map(|c| (i, c[1].to_owned())))
        .filter(|(start, _)| block_end(lines, *start).is_some_and(|end| end >= line))
        .map(|(_, name)| name)
        .unwrap_or_default();
    let dir = path.parent()?;
    let root = dir.ancestors().find(|d| d.join("go.mod").is_file())?;
    let text = std::fs::read_to_string(root.join("go.mod")).ok()?;
    let module = GO_MODULE.captures(&text)?[1].to_owned();
    let relative = dir.strip_prefix(root).ok()?;
    let mut suite = module;
    for part in relative.components() {
        suite.push('/');
        suite.push_str(&part.as_os_str().to_string_lossy());
    }
    Some(TestAt {
        lang: TestLang::Go,
        suite,
        name,
    })
}

/// 커서의 테스트(`at`, `path` 파일)를 디버거로 띄우는 구성. Python은 debugpy로 pytest 모듈을 노드 ID와 함께
/// 띄우고(작업 폴더는 `dir`, 디버그 대상 Python은 어댑터가 사용자의 것으로 채운다), Go는 delve의 test 모드로 그
/// 패키지 폴더를 `-test.run`과 함께 띄운다. 이름이 빈 것(묶음 전체)은 거르지 않는다. Java·Kotlin은 None이다: 테스트
/// 창이 테스트 JVM을 디버거를 기다리게 띄우고(`test_report::with_jvm_debug`) `jvm_attach_config`로 붙는다.
pub fn debug_config(at: &TestAt, path: &Path, dir: &Path) -> Option<DebugConfig> {
    let (name, adapter, arguments) = match at.lang {
        TestLang::Python => {
            let module = python_module(path, dir).len();
            let mut id = path
                .strip_prefix(dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            for class in at.suite.split('.').skip(module) {
                id.push_str("::");
                id.push_str(class);
            }
            if !at.name.is_empty() {
                id.push_str("::");
                id.push_str(&at.name);
            }
            let arguments = json!({
                "module": "pytest",
                "args": [id],
                "cwd": dir,
                "console": "internalConsole",
                "justMyCode": true,
            });
            (id, "debugpy", arguments)
        }
        TestLang::Go => {
            let mut arguments = json!({ "mode": "test", "program": path.parent()? });
            if !at.name.is_empty() {
                arguments["args"] = json!(["-test.run", format!("^{}$", at.name)]);
            }
            let name = if at.name.is_empty() {
                at.suite.clone()
            } else {
                format!("{} {}", at.suite, at.name)
            };
            (name, "delve", arguments)
        }
        TestLang::Jvm => return None,
    };
    Some(DebugConfig {
        name: format!("테스트 디버그: {name}"),
        adapter: adapter.to_owned(),
        request: "launch".to_owned(),
        arguments,
        build: None,
    })
}

/// 결과 트리의 테스트(묶음 `suite`·이름 `name`, 테스트를 돌린 폴더 `dir`)를 `debug_config`로 띄울 때 넘길 것: 커서의
/// 테스트와 같은 꼴과 그 파일. Python은 묶음(`tests.test_a.TestX`)의 앞쪽과 맞는 `dir` 아래 `.py` 파일(가장 긴 것),
/// Go는 `go.mod`의 모듈 경로를 뺀 패키지 폴더의 `_test.go` 파일(`debug_config`는 그 폴더만 쓴다). Go 패키지 결과
/// (`PACKAGE_CASE`)는 패키지 전체다. 못 찾으면 None. Java·Kotlin은 테스트 창이 띄우므로 None.
pub fn debug_target(
    lang: TestLang,
    suite: &str,
    name: &str,
    dir: &Path,
) -> Option<(TestAt, std::path::PathBuf)> {
    let file = match lang {
        TestLang::Python => {
            let parts: Vec<&str> = suite.split('.').collect();
            (1..=parts.len())
                .rev()
                .map(|k| dir.join(format!("{}.py", parts[..k].join("/"))))
                .find(|f| f.is_file())?
        }
        TestLang::Go => {
            let root = dir.ancestors().find(|d| d.join("go.mod").is_file())?;
            let text = std::fs::read_to_string(root.join("go.mod")).ok()?;
            let module = &GO_MODULE.captures(&text)?[1];
            let rest = suite.strip_prefix(module)?;
            if !(rest.is_empty() || rest.starts_with('/')) {
                return None;
            }
            let folder = root.join(rest.trim_start_matches('/'));
            std::fs::read_dir(folder)
                .ok()?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .find(|p| p.to_string_lossy().ends_with("_test.go"))?
        }
        TestLang::Jvm => return None,
    };
    let name = if name == crate::test_report::PACKAGE_CASE {
        ""
    } else {
        name
    };
    Some((
        TestAt {
            lang,
            suite: suite.to_owned(),
            name: name.to_owned(),
        },
        file,
    ))
}

/// 디버거를 기다리는 테스트 JVM(`test_report::with_jvm_debug`)에 java-debug로 붙는 구성.
pub fn jvm_attach_config() -> DebugConfig {
    DebugConfig {
        name: format!("테스트 디버그: Java({} 포트에 붙기)", crate::run::JDWP_PORT),
        adapter: "java-debug".to_owned(),
        request: "attach".to_owned(),
        arguments: json!({ "hostName": "127.0.0.1", "port": crate::run::JDWP_PORT }),
        build: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, text: &str, marker: &str, dir: &Path) -> Option<TestAt> {
        let line = text
            .lines()
            .position(|l| l.contains(marker))
            .expect("표시 줄");
        test_at(Path::new(path), text, line, dir)
    }

    const JAVA: &str = "package com.example.board;

import org.junit.jupiter.api.Test;

class CalcTest {
    @Test
    void adds() {
        assertEquals(3, 1 + 2); // A
        String s = \"} 닫힘 아님\";
    }

    @org.junit.jupiter.params.ParameterizedTest
    @ValueSource(ints = {1, 2})
    void param(int n) {
        check(n); // B
    }

    private void helper() {
        work(); // C
    }
    // D
}
";

    #[test]
    fn java_커서가_든_테스트_메서드() {
        let dir = Path::new(".");
        let suite = "com.example.board.CalcTest";
        let case = |name: &str| {
            Some(TestAt {
                lang: TestLang::Jvm,
                suite: suite.to_owned(),
                name: name.to_owned(),
            })
        };
        assert_eq!(at("CalcTest.java", JAVA, "// A", dir), case("adds"));
        assert_eq!(at("CalcTest.java", JAVA, "닫힘 아님", dir), case("adds"));
        assert_eq!(at("CalcTest.java", JAVA, "void adds", dir), case("adds"));
        assert_eq!(at("CalcTest.java", JAVA, "// B", dir), case("param"));
        // 테스트가 아닌 메서드, 메서드 밖, 클래스 위(import)는 클래스 전체.
        for marker in ["// C", "// D", "import", "class CalcTest"] {
            assert_eq!(at("CalcTest.java", JAVA, marker, dir), case(""), "{marker}");
        }
        // 테스트 어노테이션이 없는 파일은 아니다.
        assert_eq!(
            at(
                "Calc.java",
                "class Calc {\n    void add() {\n    }\n}\n",
                "add",
                dir
            ),
            None
        );
    }

    #[test]
    fn kotlin_테스트_함수() {
        let text = "package demo\n\nclass BoardTest {\n    @Test\n    fun lists() {\n        check() // A\n    }\n}\n";
        assert_eq!(
            at("BoardTest.kt", text, "// A", Path::new(".")),
            Some(TestAt {
                lang: TestLang::Jvm,
                suite: "demo.BoardTest".to_owned(),
                name: "lists".to_owned(),
            })
        );
    }

    #[test]
    fn python_함수와_클래스_안의_테스트() {
        let text = "import pytest\n\n\ndef test_add():\n    x = 1  # A\n\n    assert x  # B\n\n\nclass TestCalc:\n    def test_sub(self):\n        assert 1  # C\n\n    def helper(self):\n        pass  # D\n\n\ndef setup():\n    pass  # E\n";
        let dir = Path::new("/p");
        let file = "/p/tests/test_calc.py";
        let case = |suite: &str, name: &str| {
            Some(TestAt {
                lang: TestLang::Python,
                suite: suite.to_owned(),
                name: name.to_owned(),
            })
        };
        assert_eq!(
            at(file, text, "# A", dir),
            case("tests.test_calc", "test_add")
        );
        assert_eq!(
            at(file, text, "# B", dir),
            case("tests.test_calc", "test_add")
        );
        assert_eq!(
            at(file, text, "# C", dir),
            case("tests.test_calc.TestCalc", "test_sub")
        );
        // 클래스 안 도우미는 그 클래스 전체, 파일의 다른 곳은 파일 전체.
        assert_eq!(
            at(file, text, "# D", dir),
            case("tests.test_calc.TestCalc", "")
        );
        assert_eq!(at(file, text, "# E", dir), case("tests.test_calc", ""));
        assert_eq!(at(file, text, "import", dir), case("tests.test_calc", ""));
        assert_eq!(at(file, "def helper():\n    pass\n", "pass", dir), None);
    }

    #[test]
    fn 커서의_테스트를_디버거로_띄우는_구성() {
        let dir = Path::new("/p");
        let python = |suite: &str, name: &str| TestAt {
            lang: TestLang::Python,
            suite: suite.to_owned(),
            name: name.to_owned(),
        };
        let file = Path::new("/p/tests/test_calc.py");
        let config =
            debug_config(&python("tests.test_calc.TestCalc", "test_sub"), file, dir).unwrap();
        assert_eq!(config.adapter, "debugpy");
        assert_eq!(config.arguments["module"], "pytest");
        assert_eq!(
            config.arguments["args"],
            json!(["tests/test_calc.py::TestCalc::test_sub"])
        );
        // 묶음 전체는 파일까지.
        let whole = debug_config(&python("tests.test_calc", ""), file, dir).unwrap();
        assert_eq!(whole.arguments["args"], json!(["tests/test_calc.py"]));
        let go = TestAt {
            lang: TestLang::Go,
            suite: "example.com/board/calc".to_owned(),
            name: "TestAdd".to_owned(),
        };
        let config = debug_config(&go, Path::new("/p/calc/calc_test.go"), dir).unwrap();
        assert_eq!(config.adapter, "delve");
        assert_eq!(config.arguments["mode"], "test");
        assert_eq!(config.arguments["args"], json!(["-test.run", "^TestAdd$"]));
        let package = TestAt {
            name: String::new(),
            ..go.clone()
        };
        let config = debug_config(&package, Path::new("/p/calc/calc_test.go"), dir).unwrap();
        assert!(config.arguments.get("args").is_none());
        let jvm = TestAt {
            lang: TestLang::Jvm,
            ..go
        };
        assert_eq!(debug_config(&jvm, Path::new("/p/A.java"), dir), None);
        let attach = jvm_attach_config();
        assert!(attach.attach());
        assert_eq!(attach.arguments["port"], crate::run::JDWP_PORT);
    }

    #[test]
    fn 결과의_테스트를_디버거로_띄울_파일() {
        let dir = std::env::temp_dir().join(format!("ferrite-debug-target-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::create_dir_all(dir.join("calc")).unwrap();
        std::fs::write(dir.join("tests/test_calc.py"), "").unwrap();
        std::fs::write(
            dir.join("go.mod"),
            "module example.com/board
",
        )
        .unwrap();
        std::fs::write(dir.join("calc/calc_test.go"), "").unwrap();
        std::fs::write(dir.join("calc/calc.go"), "").unwrap();
        // Python: 클래스까지 든 묶음에서 파일을 찾고, 노드 ID가 커서의 것과 같다.
        let (at, file) = debug_target(
            TestLang::Python,
            "tests.test_calc.TestCalc",
            "test_sub",
            &dir,
        )
        .unwrap();
        assert_eq!(file, dir.join("tests/test_calc.py"));
        let config = debug_config(&at, &file, &dir).unwrap();
        assert_eq!(
            config.arguments["args"],
            json!(["tests/test_calc.py::TestCalc::test_sub"])
        );
        assert_eq!(
            debug_target(TestLang::Python, "tests.test_none", "t", &dir),
            None
        );
        // Go: 모듈 경로를 뺀 폴더. 패키지 결과는 패키지 전체.
        let (at, file) =
            debug_target(TestLang::Go, "example.com/board/calc", "TestAdd", &dir).unwrap();
        assert_eq!(file, dir.join("calc/calc_test.go"));
        assert_eq!(
            debug_config(&at, &file, &dir).unwrap().arguments["args"],
            json!(["-test.run", "^TestAdd$"])
        );
        let (at, _) = debug_target(
            TestLang::Go,
            "example.com/board/calc",
            crate::test_report::PACKAGE_CASE,
            &dir,
        )
        .unwrap();
        assert_eq!(at.name, "");
        assert_eq!(
            debug_target(TestLang::Go, "example.com/boardx", "TestA", &dir),
            None
        );
        assert_eq!(debug_target(TestLang::Jvm, "a.B", "c", &dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn go_테스트_함수와_패키지_경로() {
        let root = std::env::temp_dir().join(format!("ferrite-test-at-go-{}", std::process::id()));
        std::fs::create_dir_all(root.join("calc")).unwrap();
        std::fs::write(root.join("go.mod"), "module example.com/board\n\ngo 1.21\n").unwrap();
        let file = root.join("calc/calc_test.go");
        let text = "package calc\n\nimport \"testing\"\n\nfunc TestAdd(t *testing.T) {\n\tif Add(1, 2) != 3 { // A\n\t\tt.Fatal(\"add\")\n\t}\n}\n\nfunc helper() {\n\t// B\n}\n";
        let line = |marker: &str| text.lines().position(|l| l.contains(marker)).unwrap();
        assert_eq!(
            test_at(&file, text, line("// A"), &root),
            Some(TestAt {
                lang: TestLang::Go,
                suite: "example.com/board/calc".to_owned(),
                name: "TestAdd".to_owned(),
            })
        );
        assert_eq!(
            test_at(&file, text, line("// B"), &root),
            Some(TestAt {
                lang: TestLang::Go,
                suite: "example.com/board/calc".to_owned(),
                name: String::new(),
            })
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
