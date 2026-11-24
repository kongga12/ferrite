//! 에디터 커서가 든 테스트(메서드·함수)를 소스 글만 보고 알아본다(언어 서버 없이). 테스트 창이 그것만 돌린다
//! (`test_report::run_only`). 돌릴 때의 묶음(`suite`)과 이름은 그 도구의 보고서가 적는 꼴에 맞춘다.
//! - Java·Kotlin: `@Test`류 어노테이션이 붙은 메서드. 묶음은 `패키지.클래스`(커서 위로 가장 가까운 클래스).
//! - Python: `test`로 시작하는 함수. 묶음은 돌린 폴더 기준 모듈(`tests.test_a`)과 품은 클래스(`.TestX`).
//! - Go: `func TestX(t *testing.T)`. 묶음은 `go.mod`의 모듈 경로와 폴더(`example.com/board/calc`).

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

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
    pub name: String,
}

/// `path` 파일(글 `text`)의 `line`(0부터) 줄이 든 테스트. Python의 모듈 이름은 테스트를 돌릴 폴더(`dir`) 기준이다.
/// 테스트 메서드·함수 안이 아니면 None.
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

/// Java·Kotlin: 커서 위로 가장 가까운 메서드가 커서를 품고 테스트 어노테이션이 붙었으면 그 테스트.
fn jvm(lines: &[&str], line: usize) -> Option<TestAt> {
    let (start, name) = (0..=line).rev().find_map(|i| {
        let m = JAVA_METHOD
            .captures(lines[i])
            .or_else(|| KOTLIN_FUN.captures(lines[i]))?;
        Some((i, m[1].to_owned()))
    })?;
    if block_end(lines, start)? < line {
        return None;
    }
    // 어노테이션은 메서드 줄이나 그 위의 어노테이션·주석·빈 줄에 있다.
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
    if !annotated {
        return None;
    }
    let class = lines[..start]
        .iter()
        .rev()
        .find_map(|l| CLASS.captures(l).map(|c| c[1].to_owned()))?;
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

/// Python: 커서를 품은 가장 안쪽 `def`가 `test`로 시작하면 그 테스트. 품은 클래스들을 묶음에 붙인다.
fn python(path: &Path, lines: &[&str], line: usize, dir: &Path) -> Option<TestAt> {
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
    let (start, depth, name) = found?;
    if !name.starts_with("test") {
        return None;
    }
    // 품은 클래스들(바깥부터).
    let mut classes = Vec::new();
    let mut outer = depth;
    for text in lines[..start].iter().rev() {
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
    let relative = path.strip_prefix(dir).unwrap_or(path).with_extension("");
    let mut module: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    module.extend(classes);
    Some(TestAt {
        lang: TestLang::Python,
        suite: module.join("."),
        name,
    })
}

static GO_TEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^func\s+(Test\w*)\s*\(\s*\w+\s+\*testing\.T\s*\)").unwrap());
static GO_MODULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^module\s+(\S+)").unwrap());

/// Go: 커서를 품은 `func TestX(t *testing.T)`. 묶음은 `go.mod`의 모듈 경로에 그 폴더를 붙인 패키지 경로다.
fn go(path: &Path, lines: &[&str], line: usize) -> Option<TestAt> {
    let (start, name) = (0..=line)
        .rev()
        .find_map(|i| GO_TEST.captures(lines[i]).map(|c| (i, c[1].to_owned())))?;
    if block_end(lines, start)? < line {
        return None;
    }
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
        // 테스트가 아닌 메서드, 메서드 밖.
        assert_eq!(at("CalcTest.java", JAVA, "// C", dir), None);
        assert_eq!(at("CalcTest.java", JAVA, "// D", dir), None);
        assert_eq!(at("CalcTest.java", JAVA, "import", dir), None);
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
        assert_eq!(at(file, text, "# D", dir), None);
        assert_eq!(at(file, text, "# E", dir), None);
        assert_eq!(at(file, text, "import", dir), None);
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
        assert_eq!(test_at(&file, text, line("// B"), &root), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
