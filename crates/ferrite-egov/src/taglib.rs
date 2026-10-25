//! JSP 태그 라이브러리(TLD)의 태그·속성·EL 함수. JSP 자동완성에 쓴다.
//!
//! 찾는 곳(앞이 우선): 프로젝트의 `.tld` 파일, `WEB-INF/lib`의 jar, 로컬 Maven 저장소에 있는 잘
//! 알려진 라이브러리(JSTL, Spring `spring:`·`form:`, egov `ui:`, Tiles, Spring Security, Spring Modules
//! `validator:`, SiteMesh)의 jar.

use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};

use ferrite_core::jsp::{self, Kind};

use tree_sitter::{Node, Parser};

use crate::mapper::{children, tag_name, tag_of};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Library {
    pub tags: Vec<Tag>,
    pub functions: Vec<Function>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub description: String,
    pub attributes: Vec<Attribute>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    /// `int length(java.lang.Object)`
    pub signature: String,
}

/// TLD 파일 하나. `uri`는 TLD가 스스로 밝힌 이름(`http://java.sun.com/jsp/jstl/core`)이다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tld {
    pub uri: Option<String>,
    pub library: Library,
}

/// 로컬 Maven 저장소에서 찾아볼 라이브러리(그룹 경로, artifactId). 같은 uri를 여럿이 담으면
/// 프로젝트 빌드 파일에 적힌 것이 먼저다.
const KNOWN_ARTIFACTS: [(&str, &str); 11] = [
    ("javax/servlet/jstl", "jstl"),
    ("org/apache/taglibs", "taglibs-standard-impl"),
    ("org/glassfish/web", "jakarta.servlet.jsp.jstl"),
    ("taglibs/standard", "standard"),
    ("org/springframework", "spring-webmvc"),
    ("org/springframework/security", "spring-security-taglibs"),
    ("org/egovframe/rte", "org.egovframe.rte.ptl.mvc"),
    ("egovframework/rte", "egovframework.rte.ptl.mvc"),
    ("org/apache/tiles", "tiles-jsp"),
    ("org/springmodules", "spring-modules-validation"),
    ("opensymphony", "sitemesh"),
];
/// jar 안 TLD 하나의 크기 상한(이보다 크면 TLD가 아니라고 본다).
const MAX_TLD_BYTES: u64 = 4 * 1024 * 1024;
/// 자동완성 설명으로 보여 줄 최대 글자 수.
const MAX_DESCRIPTION: usize = 120;

/// TLD 문서를 읽는다. `<taglib>` 문서가 아니면 None.
pub fn parse_tld(text: &str) -> Option<Tld> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_xml::LANGUAGE_XML.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = children(tree.root_node()).find(|n| n.kind() == "element")?;
    if tag_of(root).and_then(|t| tag_name(t, text)) != Some("taglib") {
        return None;
    }
    let mut tld = Tld {
        uri: None,
        library: Library::default(),
    };
    for element in elements(root, text) {
        match element.0 {
            "uri" => tld.uri = Some(content(element.1, text)).filter(|u| !u.is_empty()),
            // JSP 2.0 태그 파일도 태그다(속성은 .tag 파일 안에 있어 모른다).
            "tag" | "tag-file" => {
                let mut tag = Tag {
                    name: String::new(),
                    description: String::new(),
                    attributes: Vec::new(),
                };
                for (name, node) in elements(element.1, text) {
                    match name {
                        "name" => tag.name = content(node, text),
                        // JSP 1.1 TLD는 `info`에 설명을 둔다.
                        "description" | "info" => tag.description = short(&content(node, text)),
                        "attribute" => {
                            let mut attribute = Attribute {
                                name: String::new(),
                                required: false,
                            };
                            for (name, node) in elements(node, text) {
                                match name {
                                    "name" => attribute.name = content(node, text),
                                    "required" => {
                                        let value = content(node, text).to_ascii_lowercase();
                                        attribute.required = value == "true" || value == "yes";
                                    }
                                    _ => {}
                                }
                            }
                            if !attribute.name.is_empty() {
                                tag.attributes.push(attribute);
                            }
                        }
                        _ => {}
                    }
                }
                if !tag.name.is_empty() {
                    tld.library.tags.push(tag);
                }
            }
            "function" => {
                let (mut name, mut signature) = (String::new(), String::new());
                for (child, node) in elements(element.1, text) {
                    match child {
                        "name" => name = content(node, text),
                        "function-signature" => signature = short(&content(node, text)),
                        _ => {}
                    }
                }
                if !name.is_empty() {
                    tld.library.functions.push(Function { name, signature });
                }
            }
            _ => {}
        }
    }
    Some(tld)
}

/// 요소의 자식 요소들(이름, 노드).
fn elements<'t>(element: Node<'t>, text: &'t str) -> impl Iterator<Item = (&'t str, Node<'t>)> {
    children(element)
        .filter(|n| n.kind() == "content")
        .flat_map(children)
        .filter(|n| n.kind() == "element")
        .filter_map(move |n| Some((tag_name(tag_of(n)?, text)?, n)))
}

/// 요소 안의 글자(CDATA 표시는 벗기고 공백은 한 칸으로).
fn content(element: Node<'_>, text: &str) -> String {
    let Some(body) = children(element).find(|n| n.kind() == "content") else {
        return String::new();
    };
    let raw = text.get(body.byte_range()).unwrap_or_default();
    let raw = raw.replace("<![CDATA[", "").replace("]]>", "");
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short(text: &str) -> String {
    match text.char_indices().nth(MAX_DESCRIPTION) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// jar 안의 TLD(`META-INF/` 아래 `.tld`)를 모두 읽는다. 읽지 못하면 빈 목록.
pub fn jar_tlds(jar: &Path) -> Vec<Tld> {
    let Ok(mut archive) = File::open(jar)
        .map_err(drop)
        .and_then(|f| zip::ZipArchive::new(f).map_err(drop))
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            continue;
        };
        let Ok(name) = entry.name().map(|n| n.to_ascii_lowercase()) else {
            continue;
        };
        if !(name.starts_with("meta-inf/") && name.ends_with(".tld"))
            || entry.size() > MAX_TLD_BYTES
        {
            continue;
        }
        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_ok()
            && let Some(tld) = String::from_utf8(bytes).ok().as_deref().and_then(parse_tld)
        {
            found.push(tld);
        }
    }
    found
}

/// 로컬 Maven 저장소. `~/.m2/settings.xml`의 `localRepository`(전자정부 개발환경처럼 바꾼 경우),
/// 없으면 `~/.m2/repository`.
pub fn maven_repository() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?);
    let configured = std::fs::read_to_string(home.join(".m2/settings.xml"))
        .ok()
        .and_then(|settings| {
            let start = settings.find("<localRepository>")? + "<localRepository>".len();
            let end = settings[start..].find("</localRepository>")? + start;
            let value = settings[start..end].trim();
            (!value.is_empty())
                .then(|| PathBuf::from(value.replace("${user.home}", &home.to_string_lossy())))
        });
    configured
        .or_else(|| Some(home.join(".m2/repository")))
        .filter(|p| p.is_dir())
}

/// 잘 알려진 라이브러리의 jar들. 버전은 빌드 파일(`build`: pom.xml·build.gradle 내용)에 적힌 것을
/// 먼저, 없으면 가장 높은 것을 고른다. 빌드 파일에 이름이 나오는 라이브러리가 앞에 온다.
// ponytail: 빌드 파일의 의존성을 풀지 않고 이름·버전 글자만 본다(부모 POM·BOM으로 정한 버전은
// 가장 높은 버전으로 대신한다). 태그 목록은 버전 사이에 거의 같아서 자동완성에는 충분하다.
pub fn maven_jars(repo: &Path, build: &str) -> Vec<PathBuf> {
    let mut found: Vec<(bool, PathBuf)> = KNOWN_ARTIFACTS
        .iter()
        .filter_map(|(group, artifact)| {
            let dir = repo.join(group).join(artifact);
            let mut versions: Vec<String> = std::fs::read_dir(&dir)
                .ok()?
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            versions.sort_by_key(|v| version_key(v));
            let version = versions
                .iter()
                .rev()
                .find(|v| build.contains(v.as_str()))
                .or(versions.last())?;
            let jar = dir.join(version).join(format!("{artifact}-{version}.jar"));
            jar.is_file().then(|| (build.contains(artifact), jar))
        })
        .collect();
    // 빌드 파일에 나오는 것 먼저(안정 정렬이라 같은 묶음 안에서는 목록 순서).
    found.sort_by_key(|(mentioned, _)| !mentioned);
    found.into_iter().map(|(_, jar)| jar).collect()
}

/// `5.3.39` → [5, 3, 39]. 숫자가 아닌 조각(`RELEASE`)은 비교에서 뺀다.
fn version_key(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse().ok())
        .collect()
}

/// JSP의 `<%@ taglib prefix="c" uri="…" %>` 선언들(접두사, uri). `tagdir`은 뺀다.
// ponytail: tagdir(`WEB-INF/tags`의 .tag 파일)은 코퍼스에서 쓰지 않아 뺐다. 쓰면 .tag 파일의
// `<%@ attribute name %>`를 읽어 태그로 만든다.
pub fn declared(text: &str) -> Vec<(String, String)> {
    jsp::regions(text)
        .into_iter()
        .filter(|r| r.kind == Kind::Directive)
        .filter_map(|r| {
            let inner = text[r.inner()].trim_start();
            let body = inner.strip_prefix("taglib")?;
            let prefix = crate::jsp::attribute(body, "prefix")?;
            let uri = crate::jsp::attribute(body, "uri")?;
            Some((prefix.trim().to_owned(), uri.trim().to_owned()))
        })
        .collect()
}

/// JSP 자동완성 자리. `replace`는 고른 항목으로 바꿀 범위다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Site {
    /// `<c:o`(접두사까지 침) 또는 `<c`(접두사를 치는 중): 태그 이름.
    Tag {
        prefix: Option<String>,
        replace: Range<usize>,
    },
    /// `<c:out va`: 속성 이름. `written`은 커서까지의 태그 텍스트(이미 쓴 속성을 빼는 데 쓴다).
    Attribute {
        prefix: String,
        tag: String,
        written: Range<usize>,
        replace: Range<usize>,
    },
    /// `${fn:le`: EL 함수.
    Function {
        prefix: String,
        replace: Range<usize>,
    },
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// 커서 앞 이름 조각의 길이(바이트).
fn trailing_name(before: &str) -> usize {
    before.len() - before.trim_end_matches(is_name_char).len()
}

pub fn site_at(text: &str, offset: usize) -> Option<Site> {
    let offset = offset.min(text.len());
    let in_el = jsp::regions(text).into_iter().find(|r| {
        let inner = r.inner();
        r.kind == Kind::El && inner.start <= offset && offset <= inner.end
    });
    if let Some(el) = in_el {
        // `${fn:le` → 접두사 `fn`. `vo.fn:x` 같은 속성 접근은 아니다.
        let before = &text[el.inner().start..offset];
        let word = trailing_name(before);
        let rest = before[..before.len() - word].strip_suffix(':')?;
        let prefix = &rest[rest.len() - trailing_name(rest)..];
        let preceded = rest[..rest.len() - prefix.len()].chars().next_back();
        if prefix.is_empty() || preceded == Some('.') {
            return None;
        }
        return Some(Site::Function {
            prefix: prefix.to_owned(),
            replace: offset - word..offset,
        });
    }
    let tag = jsp::open_tag_at(text, offset).filter(|t| !t.in_value)?;
    let name_end = tag.start + 1 + tag.name.len();
    if offset <= name_end {
        let typed = &text[tag.start + 1..offset];
        return Some(match typed.split_once(':') {
            Some((prefix, _)) => Site::Tag {
                prefix: Some(prefix.to_owned()),
                replace: tag.start + 1 + prefix.len() + 1..offset,
            },
            None => Site::Tag {
                prefix: None,
                replace: tag.start + 1..offset,
            },
        });
    }
    // 사용자 정의 태그의 속성 이름 자리(앞이 공백). HTML 태그 속성은 다른 서버의 몫이다.
    let (prefix, local) = tag.name.split_once(':')?;
    let before = &text[name_end..offset];
    let word = trailing_name(before);
    if !before[..before.len() - word].ends_with(char::is_whitespace) {
        return None;
    }
    Some(Site::Attribute {
        prefix: prefix.to_owned(),
        tag: local.to_owned(),
        written: tag.start..offset,
        replace: offset - word..offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TLD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<taglib xmlns="http://java.sun.com/xml/ns/javaee" version="2.1">
  <description>시험용 태그</description>
  <short-name>t</short-name>
  <uri>http://example.com/tags/test</uri>
  <tag>
    <description><![CDATA[값을   출력한다]]></description>
    <name>out</name>
    <tag-class>example.OutTag</tag-class>
    <attribute><name>value</name><required>true</required></attribute>
    <attribute><name>escapeXml</name><required>false</required></attribute>
  </tag>
  <tag><name>legacy</name><info>JSP 1.1 설명</info><attribute><name>id</name><required>yes</required></attribute></tag>
  <function>
    <name>length</name>
    <function-class>example.Fn</function-class>
    <function-signature>int length(java.lang.Object)</function-signature>
  </function>
</taglib>"#;

    #[test]
    fn tld의_태그와_속성과_함수를_읽는다() {
        let tld = parse_tld(TLD).unwrap();
        assert_eq!(tld.uri.as_deref(), Some("http://example.com/tags/test"));
        let tags = &tld.library.tags;
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name, "out");
        assert_eq!(tags[0].description, "값을 출력한다");
        assert_eq!(
            tags[0].attributes,
            vec![
                Attribute {
                    name: "value".into(),
                    required: true
                },
                Attribute {
                    name: "escapeXml".into(),
                    required: false
                },
            ]
        );
        assert_eq!(tags[1].description, "JSP 1.1 설명");
        assert!(tags[1].attributes[0].required);
        assert_eq!(
            tld.library.functions,
            vec![Function {
                name: "length".into(),
                signature: "int length(java.lang.Object)".into()
            }]
        );
        assert!(parse_tld("<beans/>").is_none());
    }

    #[test]
    fn 버전은_빌드_파일에_적힌_것을_먼저_고른다() {
        let repo = std::env::temp_dir().join(format!("ferrite-egov-m2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        for version in ["4.3.25.RELEASE", "5.3.39", "5.3.6"] {
            let dir = repo.join("org/springframework/spring-webmvc").join(version);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("spring-webmvc-{version}.jar")), b"").unwrap();
        }
        let jar = |build: &str| {
            maven_jars(&repo, build)[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(jar(""), "spring-webmvc-5.3.39.jar", "5.3.39 > 5.3.6");
        assert_eq!(
            jar("<spring.version>4.3.25.RELEASE</spring.version>"),
            "spring-webmvc-4.3.25.RELEASE.jar"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn 자동완성_자리를_안다() {
        let page = r#"<c:o <c <form:input path="x" cssC ${fn:le} ${vo.fn:x} <c:out value="v"#;
        let at = |needle: &str| page.find(needle).unwrap() + needle.len();
        let tag = |prefix: Option<&str>, replace: Range<usize>| Site::Tag {
            prefix: prefix.map(str::to_owned),
            replace,
        };
        assert_eq!(site_at(page, at("<c:o")), Some(tag(Some("c"), 3..4)));
        assert_eq!(site_at(page, at("<c:o <c")), Some(tag(None, 6..7)));
        let start = page.find("<form").unwrap();
        assert_eq!(
            site_at(page, at("cssC")),
            Some(Site::Attribute {
                prefix: "form".into(),
                tag: "input".into(),
                written: start..at("cssC"),
                replace: at("cssC") - 4..at("cssC"),
            })
        );
        assert_eq!(
            site_at(page, at("fn:le")),
            Some(Site::Function {
                prefix: "fn".into(),
                replace: at("fn:le") - 2..at("fn:le"),
            })
        );
        assert_eq!(site_at(page, at("vo.fn:x")), None, "속성 접근");
        assert_eq!(site_at(page, at("path=\"x")), None, "속성값 안");
        assert_eq!(site_at(page, page.len()), None, "속성값 안(닫히지 않음)");
    }

    #[test]
    fn taglib_선언을_찾는다() {
        let page = r#"<%@ page contentType="text/html" %>
<%@ taglib prefix="c" uri="http://java.sun.com/jsp/jstl/core"%>
<%@taglib uri="/WEB-INF/tlds/ui.tld" prefix = "ui" %>
<%@ taglib prefix="t" tagdir="/WEB-INF/tags" %>"#;
        assert_eq!(
            declared(page),
            vec![
                ("c".into(), "http://java.sun.com/jsp/jstl/core".into()),
                ("ui".into(), "/WEB-INF/tlds/ui.tld".into()),
            ]
        );
    }
}
