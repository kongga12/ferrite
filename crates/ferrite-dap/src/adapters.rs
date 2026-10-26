//! 디버그 구성의 어댑터 이름으로 어댑터를 띄울 방법을 고른다. 어댑터 목록과 설치는 언어 서버와 같은 것을
//! 쓴다(`ferrite_lsp::catalog::ADAPTERS`, `ferrite_lsp::install`). Ferrite가 설치한 것이 없으면 PATH에서
//! 찾는다.

use std::path::{Path, PathBuf};

use ferrite_lsp::catalog::{ADAPTERS, Server};
use ferrite_lsp::client::find_program;
use ferrite_lsp::install;
use serde_json::{Value, json};

use crate::Adapter;

pub fn find(name: &str) -> Option<&'static Server> {
    ADAPTERS.iter().find(|s| s.name == name)
}

/// 어댑터를 띄울 방법. `servers`는 언어 서버·어댑터 설치 폴더, `cwd`는 프로젝트 루트.
/// 쓸 수 없으면 사용자에게 보일 안내.
pub fn resolve(name: &str, servers: &Path, cwd: &Path) -> Result<Adapter, String> {
    let server = find(name).ok_or_else(|| format!("알 수 없는 디버그 어댑터입니다: {name}"))?;
    let command = install::command(servers, server, cwd)
        .ok_or_else(|| format!("{name}이(가) 없습니다. 도구 > 언어 서버에서 설치하세요"))??;
    let mut env = Vec::new();
    // Ferrite가 설치하지 않았으면 PATH의 Python으로 띄운다. 따로 둔 debugpy는 FERRITE_DEBUGPY_PATH로 알린다.
    if name == "debugpy"
        && install::installed(servers, server).is_none()
        && let Ok(path) = std::env::var("FERRITE_DEBUGPY_PATH")
    {
        env.push(("PYTHONPATH".to_owned(), path));
    }
    let (program, mut args, cwd) = (command.program, command.args, Some(cwd.to_path_buf()));
    Ok(if name == "delve" {
        // 빈 포트로 듣게 하고, 알린 주소로 붙는다(정해 둔 포트가 이미 쓰이는 일이 없다).
        args.push("--listen=127.0.0.1:0".into());
        Adapter::Listening {
            program,
            args,
            cwd,
            env,
        }
    } else {
        Adapter::Stdio {
            program,
            args,
            cwd,
            env,
        }
    })
}

/// 구성 인자에 어댑터별 기본값을 채운다. 사용자가 적은 값은 그대로 둔다.
pub fn complete_arguments(name: &str, arguments: &mut Value) {
    let Some(map) = arguments.as_object_mut() else {
        return;
    };
    let (key, program) = match name {
        // 디버그 대상은 사용자의 Python으로 띄운다. 설치한 debugpy 가상환경에는 프로젝트 패키지가 없다.
        "debugpy" => ("python", python()),
        "php-debug" => ("runtimeExecutable", find_program("php")),
        _ => return,
    };
    if !map.contains_key(key)
        && let Some(program) = program
    {
        map.insert(key.to_owned(), json!(program));
    }
}

/// 어댑터가 곧바로 끝났을 때 stderr로 이유를 짐작해 고칠 방법을 덧붙인다.
pub fn hint(name: &str, stderr: &str) -> Option<&'static str> {
    (name == "debugpy" && stderr.contains("No module named debugpy"))
        .then_some("debugpy가 없습니다. 도구 > 언어 서버에서 debugpy를 설치하세요")
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
    fn 모르는_어댑터와_없는_어댑터를_알린다() {
        let empty = std::env::temp_dir().join(format!("ferrite-adapters-{}", std::process::id()));
        assert!(
            resolve("없는-어댑터", &empty, Path::new("."))
                .unwrap_err()
                .contains("없는-어댑터")
        );
        // php-debug는 PATH에서 찾을 이름이 없으므로 설치하지 않았으면 쓸 수 없다.
        assert!(
            resolve("php-debug", &empty, Path::new("."))
                .unwrap_err()
                .contains("도구 > 언어 서버")
        );
        assert!(hint("debugpy", "ModuleNotFoundError: No module named debugpy").is_some());
        assert!(hint("debugpy", "다른 오류").is_none());
    }

    #[test]
    fn 사용자가_적은_인자는_두고_빠진_기본값만_채운다() {
        let mut arguments = json!({ "python": "/my/python", "program": "a.py" });
        complete_arguments("debugpy", &mut arguments);
        assert_eq!(arguments["python"], "/my/python");
        let mut arguments = json!({ "program": "main.go" });
        complete_arguments("delve", &mut arguments);
        assert_eq!(arguments, json!({ "program": "main.go" }));
    }
}
