//! 웹 프로젝트(egov 등)의 화면 폴더와, JS 언어 서버가 전역 스크립트(jQuery 등)를 알게 하는 jsconfig.json.
//!
//! 언어 서버는 열린 파일과 그것이 불러오는 모듈만 본다. 화면이 `<script>`로 따로 불러오는 전역
//! 라이브러리는 모른다. jsconfig.json이 화면 폴더의 스크립트를 한 프로젝트로 묶으면 TS 서버가 라이브러리
//! 파일 이름을 보고 타입(`@types/jquery`)을 받아 쓴다(자동 타입 받기). 다만 TypeScript 7은
//! `jquery-3.7.1.min.js` 같은 `.min.js` 이름으로는 받지 않아서 jQuery는 jsconfig.json에 직접 적는다.

use std::path::Path;

/// 화면 폴더(웹 루트) 후보: Maven(`src/main/webapp`), Eclipse 동적 웹 프로젝트(`WebContent`), 그 밖(`web`).
pub const WEB_DIRS: [&str; 3] = ["src/main/webapp", "WebContent", "web"];

/// 프로젝트의 화면 폴더(루트 기준 상대 경로).
pub fn web_dir(root: &Path) -> Option<&'static str> {
    WEB_DIRS.iter().copied().find(|dir| root.join(dir).is_dir())
}

/// jsconfig.json을 만들자고 할 화면 폴더: `file`이 화면 폴더 안의 스크립트이고, 그 파일의 폴더부터
/// 루트까지 jsconfig.json·tsconfig.json이 없을 때.
pub fn jsconfig_target(root: &Path, file: &Path) -> Option<&'static str> {
    let extension = file.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(extension.as_str(), "js" | "mjs" | "cjs" | "jsx") {
        return None;
    }
    let dir = web_dir(root)?;
    if !file.starts_with(root.join(dir)) {
        return None;
    }
    let configured = file
        .ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(root))
        .any(|d| d.join("jsconfig.json").is_file() || d.join("tsconfig.json").is_file());
    (!configured).then_some(dir)
}

/// 루트에 만들 jsconfig.json: 화면 폴더의 스크립트를 모두 묶고, jQuery 파일이 있으면 그 타입을 받게 한다.
pub fn jsconfig_text(root: &Path, web_dir: &str) -> String {
    let jquery = if has_jquery(&root.join(web_dir)) {
        ",\n  \"typeAcquisition\": { \"include\": [\"jquery\"] }"
    } else {
        ""
    };
    format!("{{\n  \"include\": [\"{web_dir}/**/*.js\"]{jquery}\n}}\n")
}

/// 폴더(하위 포함)에 jQuery 파일(`jquery*.js`, jQuery UI·플러그인 포함)이 있는지.
// ponytail: jQuery만 본다. 다른 전역 라이브러리(moment 등)도 .min.js로만 두는 일이 많아지면 목록으로 늘린다.
fn has_jquery(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 50_000 {
                return false;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(entry.path());
            } else if name.starts_with("jquery") && name.ends_with(".js") {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 화면_폴더의_스크립트에_설정이_없으면_jsconfig를_만들자고_한다() {
        let root = std::env::temp_dir().join(format!("ferrite-web-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let js = root.join("src/main/webapp/js");
        std::fs::create_dir_all(&js).unwrap();
        let board = js.join("board.js");
        assert_eq!(web_dir(&root), Some("src/main/webapp"));
        assert_eq!(jsconfig_target(&root, &board), Some("src/main/webapp"));
        assert_eq!(
            jsconfig_target(&root, &js.join("x.ts")),
            None,
            "TS는 tsconfig"
        );
        assert_eq!(
            jsconfig_target(&root, &root.join("tools/build.js")),
            None,
            "화면 폴더 밖"
        );
        // 화면 폴더 안이나 루트에 설정이 있으면 묻지 않는다.
        std::fs::write(root.join("src/main/webapp/tsconfig.json"), "{}").unwrap();
        assert_eq!(jsconfig_target(&root, &board), None);
        std::fs::remove_file(root.join("src/main/webapp/tsconfig.json")).unwrap();
        let read = |text: String| serde_json::from_str::<serde_json::Value>(&text).unwrap();
        // jQuery 파일이 없으면 묶기만, 있으면(.min.js, 하위 폴더) 그 타입도 받게 한다.
        let config = read(jsconfig_text(&root, "src/main/webapp"));
        assert_eq!(config["include"][0], "src/main/webapp/**/*.js");
        assert!(config.get("typeAcquisition").is_none());
        std::fs::create_dir_all(js.join("lib")).unwrap();
        std::fs::write(js.join("lib/jQuery-1.11.2.min.js"), "").unwrap();
        let config = read(jsconfig_text(&root, "src/main/webapp"));
        assert_eq!(config["typeAcquisition"]["include"][0], "jquery");
        std::fs::write(
            root.join("jsconfig.json"),
            jsconfig_text(&root, "src/main/webapp"),
        )
        .unwrap();
        assert_eq!(jsconfig_target(&root, &board), None);
        assert_eq!(web_dir(&root.join("js")), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
