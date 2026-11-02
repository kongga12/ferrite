//! 의존성 취약점 검사: 프로젝트가 쓰는 라이브러리 버전(Maven·Gradle·npm·PyPI·crates.io)을 공개 취약점
//! DB(OSV)와 견주고 고친 버전을 알린다. 온라인은 api.osv.dev에 이름·버전만 묻고, 오프라인은 생태계별
//! DB 묶음(`all.zip`)을 읽는다.

pub mod manifest;
pub mod osv;
pub mod version;

use std::path::PathBuf;

/// 라이브러리 생태계(OSV의 이름).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ecosystem {
    Maven,
    Npm,
    PyPI,
    Crates,
}

impl Ecosystem {
    pub const ALL: [Ecosystem; 4] = [
        Ecosystem::Maven,
        Ecosystem::Npm,
        Ecosystem::PyPI,
        Ecosystem::Crates,
    ];

    /// OSV의 생태계 이름(DB 묶음 주소에도 쓴다).
    pub fn osv(self) -> &'static str {
        match self {
            Ecosystem::Maven => "Maven",
            Ecosystem::Npm => "npm",
            Ecosystem::PyPI => "PyPI",
            Ecosystem::Crates => "crates.io",
        }
    }

    pub fn from_osv(name: &str) -> Option<Ecosystem> {
        Ecosystem::ALL.into_iter().find(|e| e.osv() == name)
    }

    /// 이름을 같은 꼴로(PyPI는 PEP 503: 소문자, `_`·`.`을 `-`로).
    pub fn normalize(self, name: &str) -> String {
        match self {
            Ecosystem::PyPI => name
                .to_ascii_lowercase()
                .chars()
                .map(|c| if c == '_' || c == '.' { '-' } else { c })
                .collect(),
            _ => name.to_owned(),
        }
    }
}

/// 프로젝트가 쓰는 라이브러리 하나.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub ecosystem: Ecosystem,
    /// OSV의 꼴(Maven은 `그룹:아티팩트`).
    pub name: String,
    pub version: String,
    /// 이것을 적은 파일과 줄(0부터). 전이 의존은 그것을 끌어온 직접 의존의 줄.
    pub file: PathBuf,
    pub line: usize,
    /// 전이 의존이면 끌어온 직접 의존.
    pub via: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Critical,
    High,
    Moderate,
    Low,
    Unknown,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Critical => "치명",
            Severity::High => "높음",
            Severity::Moderate => "보통",
            Severity::Low => "낮음",
            Severity::Unknown => "모름",
        }
    }
}

/// 취약한 의존성 하나와 그 취약점.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub dependency: Dependency,
    pub id: String,
    /// CVE 등 다른 이름.
    pub aliases: Vec<String>,
    pub summary: String,
    pub severity: Severity,
    /// 이 버전이 든 범위를 고친 버전.
    pub fixed: Option<String>,
}

impl Finding {
    /// 보일 이름: CVE가 있으면 CVE, 없으면 OSV 이름.
    pub fn title(&self) -> &str {
        self.aliases
            .iter()
            .find(|a| a.starts_with("CVE-"))
            .unwrap_or(&self.id)
    }
}

#[cfg(test)]
mod tests;
