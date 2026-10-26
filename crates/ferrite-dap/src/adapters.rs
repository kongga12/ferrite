//! 구성의 어댑터 이름으로 어댑터를 띄울 방법을 고른다. 설치 관리와 다른 언어는 4-4에서 더한다.

use std::path::{Path, PathBuf};

use ferrite_lsp::client::find_program;

use crate::Adapter;

/// 알려진 어댑터를 띄울 방법. 쓸 수 없으면 사용자에게 보일 안내.
pub fn resolve(name: &str, cwd: &Path) -> Result<Adapter, String> {
    match name {
        // debugpy는 디버그 대상과 같은 Python의 모듈로 돈다. 따로 둔 debugpy(`pip install --target`)는
        // FERRITE_DEBUGPY_PATH로 알려 준다.
        "debugpy" => {
            let python = python().ok_or("Python을 찾지 못했습니다(PATH)")?;
            let env = std::env::var("FERRITE_DEBUGPY_PATH")
                .map(|path| vec![("PYTHONPATH".to_owned(), path)])
                .unwrap_or_default();
            Ok(Adapter::Stdio {
                program: python,
                args: vec!["-m".into(), "debugpy.adapter".into()],
                cwd: Some(cwd.to_path_buf()),
                env,
            })
        }
        other => Err(format!("알 수 없는 디버그 어댑터입니다: {other}")),
    }
}

/// 어댑터가 곧바로 끝났을 때 stderr로 이유를 짐작해 고칠 방법을 덧붙인다.
pub fn hint(name: &str, stderr: &str) -> Option<&'static str> {
    (name == "debugpy" && stderr.contains("No module named debugpy"))
        .then_some("debugpy가 없습니다. `python -m pip install debugpy`로 설치하세요")
}

fn python() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["python", "python3"]
    } else {
        &["python3", "python"]
    };
    names.iter().find_map(|name| find_program(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 모르는_어댑터와_debugpy가_없을_때를_알린다() {
        assert!(
            resolve("없는-어댑터", Path::new("."))
                .unwrap_err()
                .contains("없는-어댑터")
        );
        assert!(hint("debugpy", "ModuleNotFoundError: No module named debugpy").is_some());
        assert!(hint("debugpy", "다른 오류").is_none());
    }
}
