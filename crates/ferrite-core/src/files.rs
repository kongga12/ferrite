//! 프로젝트 파일 둘러보기: 보고서가 적은 상대 경로(`com/example/CalcTest.java`)로 프로젝트의 실제 파일을
//! 찾는다(테스트 보고서·SARIF).

use std::path::{Path, PathBuf};

/// 폴더를 내려가며 `visit(경로, 폴더인지)`를 부른다. 버전 관리·의존성·편집기 폴더는 건너뛰고, `visit`이
/// false를 주면 그 폴더 안으로 들어가지 않는다(파일이면 멈춘다).
// ponytail: 깊이와 항목 수로만 자른다. 아주 큰 저장소에서 느리면 git이 아는 파일 목록으로 바꾼다.
pub fn walk(root: &Path, depth: usize, visit: &mut dyn FnMut(&Path, bool) -> bool) {
    const SKIP: [&str; 8] = [
        ".git",
        "node_modules",
        ".gradle",
        ".idea",
        ".venv",
        "__pycache__",
        ".mvn",
        ".ferrite",
    ];
    let mut stack = vec![(root.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((dir, level)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 200_000 {
                return;
            }
            let path = entry.path();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_dir {
                let name = entry.file_name();
                if SKIP.iter().any(|s| name == *s) || level >= depth {
                    continue;
                }
                if visit(&path, true) {
                    stack.push((path, level + 1));
                }
            } else if !visit(&path, false) {
                return;
            }
        }
    }
}

/// 경로 끝이 `relative`(`/`나 `\`로 나눈 조각들)와 같은 프로젝트 파일.
pub fn find_suffix(root: &Path, relative: &str) -> Option<PathBuf> {
    let wanted: Vec<&str> = relative
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if wanted.is_empty() {
        return None;
    }
    let mut found = None;
    walk(root, 20, &mut |path, is_dir| {
        if is_dir {
            return found.is_none();
        }
        let parts: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts.len() >= wanted.len() && parts[parts.len() - wanted.len()..] == wanted[..] {
            found = Some(path.to_path_buf());
        }
        found.is_none()
    });
    found
}
