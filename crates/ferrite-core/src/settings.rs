//! 사용자 설정(TOML). 파일에 없는 항목은 기본값을 쓴다.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::buffer::Indent;

/// 설정 파일을 처음 만들 때 쓰는 내용. 모든 항목과 기본값을 주석과 함께 보여 준다.
pub const TEMPLATE: &str = r#"# Ferrite 설정. 저장하면 바로 적용됩니다.

# 에디터 글꼴 크기(pt)
font_size = 14.0

# 테마: "system"(OS 설정 따름), "dark", "light"
theme = "system"

# 탭 문자 하나의 표시 폭(칸)
tab_size = 4

# 파일에서 들여쓰기 방식을 알아내지 못했을 때 쓸 기본값
use_tabs = false
indent_size = 4

# 시작할 때 새 판이 있는지 확인한다. 망이 막혀 있으면 조용히 넘어간다.
update_check = true

# 업데이트 정보(latest.json) 주소. 비우면 공식 릴리스. 사내 미러를 쓸 때만 바꾼다.
update_url = ""

# AI 에이전트 연결(MCP): 앱을 켤 때 이 PC(127.0.0.1)의 이 포트에서 듣는다. 에이전트는 토큰으로 붙어
# 열린 파일·진단·디버거 상태를 읽는다(파일은 바꾸지 않는다). [도구 > AI 에이전트 연결]에서 켜고 끄며 설정을 복사한다.
mcp_server = false
mcp_port = 47711

# 단축키 배치: "default"(VS Code식), "eclipse"(egov 개발환경·Eclipse식)
keymap = "default"

# AI 에이전트(ACP). Gemini CLI·Claude Code는 기본으로 있고, 다른 ACP 에이전트를 [[agent]]로 더한다.
# Ferrite는 에이전트를 설치하거나 로그인하지 않는다(터미널에서 한 번 실행해 로그인한다).
# agent_mode: 세션을 열 때 바꿀 권한 모드. 비우면 에이전트 자기 설정(Claude Code는 permissions.defaultMode)을
# 따른다. "default"는 Claude Code·Gemini CLI에서 고치기 전에 묻는 모드다. 에이전트 창 위에서 언제든 바꾼다.
agent_mode = ""
# [[agent]]
# name = "내 에이전트"
# command = "my-agent"
# args = ["--acp"]
# env = { MY_AGENT_HOME = "C:/my-agent" }

# 명령마다 키를 바꾼다. 명령 이름과 지금 키는 [보기 > 단축키 목록]에서 본다.
# "Ctrl"은 macOS에서 Cmd다(실제 Ctrl은 "Control"). 키 여럿은 목록으로, 빈 값("")은 키를 뺀다.
# [keys]
# "edit.deleteLine" = "Ctrl+D"
# "code.action" = ["Ctrl+1", "Alt+Enter"]
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

/// 단축키 배치의 바탕.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Keymap {
    #[default]
    Default,
    Eclipse,
}

/// `[keys]`의 값: 키 하나 또는 여럿(`"Ctrl+S"`, `["Ctrl+.", "Alt+Enter"]`). 키 글은 앱이 읽는다.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Keys {
    One(String),
    Many(Vec<String>),
}

impl Keys {
    /// 적은 키들(빈 글은 뺀다. 그래서 `""`·`[]`은 키 없음).
    pub fn list(&self) -> Vec<&str> {
        match self {
            Keys::One(key) => vec![key.as_str()],
            Keys::Many(keys) => keys.iter().map(String::as_str).collect(),
        }
        .into_iter()
        .filter(|k| !k.trim().is_empty())
        .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub font_size: f32,
    pub theme: Theme,
    pub tab_size: usize,
    pub use_tabs: bool,
    pub indent_size: u8,
    pub update_check: bool,
    pub update_url: String,
    pub mcp_server: bool,
    pub mcp_port: u16,
    pub keymap: Keymap,
    /// 명령 이름(`"file.save"`) → 키. 배치의 키를 덮는다.
    pub keys: BTreeMap<String, Keys>,
    /// 사용자가 더한 ACP 에이전트(`[[agent]]`).
    #[serde(rename = "agent")]
    pub agents: Vec<AgentSetting>,
    /// 세션을 열 때 바꿀 에이전트 권한 모드(빈 글이면 에이전트 설정 그대로).
    pub agent_mode: String,
}

/// ACP 에이전트 하나(`[[agent]]`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSetting {
    pub name: String,
    /// 띄울 프로그램(PATH의 이름이나 경로).
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            theme: Theme::System,
            tab_size: 4,
            use_tabs: false,
            indent_size: 4,
            update_check: true,
            update_url: String::new(),
            mcp_server: false,
            mcp_port: 47711,
            keymap: Keymap::Default,
            keys: BTreeMap::new(),
            agents: Vec::new(),
            agent_mode: String::new(),
        }
    }
}

impl Settings {
    /// TOML을 읽고 값의 범위를 검사한다. 오류 메시지는 사용자에게 그대로 보여 준다.
    pub fn parse(text: &str) -> Result<Self, String> {
        let settings: Settings = toml::from_str(text).map_err(|e| e.to_string())?;
        if !(6.0..=72.0).contains(&settings.font_size) {
            return Err(format!(
                "font_size는 6~72 사이여야 합니다: {}",
                settings.font_size
            ));
        }
        if !(1..=16).contains(&settings.tab_size) {
            return Err(format!(
                "tab_size는 1~16 사이여야 합니다: {}",
                settings.tab_size
            ));
        }
        if !(1..=8).contains(&settings.indent_size) {
            return Err(format!(
                "indent_size는 1~8 사이여야 합니다: {}",
                settings.indent_size
            ));
        }
        Ok(settings)
    }

    pub fn default_indent(&self) -> Indent {
        if self.use_tabs {
            Indent::Tabs
        } else {
            Indent::Spaces(self.indent_size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 템플릿은_기본값과_같다() {
        assert_eq!(Settings::parse(TEMPLATE).unwrap(), Settings::default());
    }

    #[test]
    fn 빠진_항목은_기본값() {
        let s = Settings::parse("theme = \"dark\"\nuse_tabs = true").unwrap();
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.font_size, 14.0);
        assert_eq!(s.default_indent(), Indent::Tabs);
    }

    #[test]
    fn 잘못된_값과_모르는_항목은_오류() {
        assert!(Settings::parse("font_size = 200").is_err());
        assert!(Settings::parse("theme = \"blue\"").is_err());
        assert!(
            Settings::parse("fontsize = 12").is_err(),
            "오타를 조용히 무시하지 않는다"
        );
        assert!(Settings::parse("keymap = \"emacs\"").is_err());
    }

    #[test]
    fn 사용자_에이전트() {
        let s = Settings::parse(
            "[[agent]]\nname = \"내 에이전트\"\ncommand = \"my-agent\"\nargs = [\"--acp\"]\n",
        )
        .unwrap();
        assert_eq!(s.agents[0].name, "내 에이전트");
        assert_eq!(s.agents[0].args, ["--acp"]);
        assert_eq!(
            Settings::parse("agent_mode = \"default\"")
                .unwrap()
                .agent_mode,
            "default"
        );
        assert!(
            Settings::parse("[[agent]]\nname = \"a\"").is_err(),
            "command 빠짐"
        );
    }

    #[test]
    fn 단축키_배치와_명령별_키() {
        let s = Settings::parse(
            "keymap = \"eclipse\"\n[keys]\n\"edit.deleteLine\" = \"Ctrl+D\"\n\"code.action\" = [\"Ctrl+1\", \"Alt+Enter\"]\n\"file.openFolder\" = \"\"\n",
        )
        .unwrap();
        assert_eq!(s.keymap, Keymap::Eclipse);
        assert_eq!(s.keys["edit.deleteLine"].list(), ["Ctrl+D"]);
        assert_eq!(s.keys["code.action"].list(), ["Ctrl+1", "Alt+Enter"]);
        assert!(
            s.keys["file.openFolder"].list().is_empty(),
            "빈 값은 키 없음"
        );
    }
}
