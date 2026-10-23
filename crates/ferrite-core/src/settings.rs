//! 사용자 설정(TOML). 파일에 없는 항목은 기본값을 쓴다.

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
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub font_size: f32,
    pub theme: Theme,
    pub tab_size: usize,
    pub use_tabs: bool,
    pub indent_size: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            theme: Theme::System,
            tab_size: 4,
            use_tabs: false,
            indent_size: 4,
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
    }
}
