//! 프로젝트 트리의 파일 작업: 새 파일·폴더, 이름 바꾸기, 휴지통으로 삭제.
//! 오류는 사용자에게 그대로 보여 줄 문장으로 돌려준다.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Windows 장치 예약어. 확장자를 붙여도(`con.txt`) 쓸 수 없다.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 파일·폴더 이름 하나를 검사한다. 저장소를 다른 OS에서 받아도 깨지지 않도록
/// Windows 규칙(금지 문자, 예약어, 끝 공백·마침표)을 모든 OS에 똑같이 적용한다.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("이름을 입력하세요".into());
    }
    if name == "." || name == ".." {
        return Err("사용할 수 없는 이름입니다".into());
    }
    if let Some(c) = name.chars().find(|c| {
        matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control()
    }) {
        return Err(format!(
            "'{}' 문자는 이름에 쓸 수 없습니다",
            c.escape_default()
        ));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err("이름은 공백이나 마침표로 끝날 수 없습니다".into());
    }
    let stem = name.split('.').next().unwrap_or(name);
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(format!(
            "'{stem}'은(는) Windows 예약 이름이라 쓸 수 없습니다"
        ));
    }
    if name.len() > 255 {
        return Err("이름이 너무 깁니다(최대 255바이트)".into());
    }
    Ok(())
}

fn already_exists(name: &str) -> String {
    format!("'{name}'이(가) 이미 있습니다")
}

/// 빈 파일을 만든다. 같은 이름이 있으면 덮어쓰지 않고 오류.
pub fn create_file(dir: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let path = dir.join(name);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| match e.kind() {
            io::ErrorKind::AlreadyExists => already_exists(name),
            _ => format!("파일을 만들지 못했습니다: {e}"),
        })?;
    Ok(path)
}

pub fn create_dir(dir: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let path = dir.join(name);
    fs::create_dir(&path).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => already_exists(name),
        _ => format!("폴더를 만들지 못했습니다: {e}"),
    })?;
    Ok(path)
}

/// 같은 폴더 안에서 이름만 바꾼다. 다른 항목을 덮어쓰지 않는다.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    validate_name(new_name)?;
    let old_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let target = path.with_file_name(new_name);
    // 대소문자만 바꾸는 경우, 대소문자를 구분하지 않는 파일 시스템(Windows·macOS)에서는
    // 대상이 "이미 있는" 것으로 보이지만 같은 파일이므로 허용한다.
    let case_only = old_name.to_lowercase() == new_name.to_lowercase();
    if target.exists() && !case_only {
        return Err(already_exists(new_name));
    }
    fs::rename(path, &target).map_err(|e| format!("이름을 바꾸지 못했습니다: {e}"))?;
    Ok(target)
}

/// OS 휴지통으로 옮긴다(영구 삭제하지 않으므로 되살릴 수 있다).
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| format!("휴지통으로 옮기지 못했습니다: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferrite-fsops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 이름_검사는_모든_os에서_windows_규칙을_따른다() {
        assert!(validate_name("Main.java").is_ok());
        assert!(validate_name("한글 파일.txt").is_ok());
        for bad in [
            "", ".", "..", "a/b", r"a\b", "a:b", "a?", "끝.", "끝 ", "con", "CON.txt", "lpt9.log",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?}는 거부되어야 한다");
        }
        assert!(
            validate_name("console.txt").is_ok(),
            "예약어로 시작할 뿐인 이름은 허용"
        );
    }

    #[test]
    fn 새_파일과_폴더는_기존_항목을_덮어쓰지_않는다() {
        let dir = temp_dir("create");
        let file = create_file(&dir, "a.txt").unwrap();
        fs::write(&file, "내용").unwrap();
        assert!(
            create_file(&dir, "a.txt")
                .unwrap_err()
                .contains("이미 있습니다")
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "내용");

        let sub = create_dir(&dir, "sub").unwrap();
        assert!(sub.is_dir());
        assert!(create_dir(&dir, "sub").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 이름_바꾸기는_충돌을_막고_대소문자만_바꾸는_것은_허용() {
        let dir = temp_dir("rename");
        let a = create_file(&dir, "a.txt").unwrap();
        create_file(&dir, "b.txt").unwrap();
        assert!(rename(&a, "b.txt").unwrap_err().contains("이미 있습니다"));

        let upper = rename(&a, "A.txt").unwrap();
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"A.txt".to_owned()), "{names:?}");

        let c = rename(&upper, "c.txt").unwrap();
        assert!(c.exists() && !upper.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
