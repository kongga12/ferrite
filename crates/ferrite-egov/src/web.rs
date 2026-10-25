//! 웹 애플리케이션 구조: 웹 루트(`WEB-INF`가 있는 폴더), JSP include 경로, Spring 뷰 리졸버의
//! 접두사·접미사, `web.xml`의 taglib 매핑. JSP 이동과 taglib 찾기에 쓴다.

use std::path::{Path, PathBuf};

use ferrite_core::jsp::{self, Kind};

use crate::jsp::{attribute, attribute_range};

/// include 경로를 받는 (태그, 속성). 지시자 `<%@ include file %>`는 따로 본다.
const INCLUDE_TAGS: [(&str, &str); 3] = [
    ("jsp:include", "page"),
    ("jsp:forward", "page"),
    ("c:import", "url"),
];

/// 이 파일을 담은 웹 루트. 없으면 첫 번째(웹 루트가 하나뿐인 보통 프로젝트).
pub fn web_root_for<'a>(roots: &'a [PathBuf], file: &Path) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| file.starts_with(root))
        .max_by_key(|root| root.components().count())
        .or(roots.first())
}

/// include 경로를 파일로 푼다. `/`로 시작하면 웹 루트 기준, 아니면 이 JSP의 폴더 기준이다.
/// 쿼리 문자열은 빼고, EL·스크립틀릿이 섞인 경로나 URL, 없는 파일은 None.
pub fn resolve(path: &str, file: &Path, web_root: Option<&Path>) -> Option<PathBuf> {
    let path = path.split(['?', '#']).next()?.trim();
    if path.is_empty() || path.contains("${") || path.contains("<%") || path.contains("://") {
        return None;
    }
    let target = match path.strip_prefix('/') {
        Some(rest) => join(web_root?, rest),
        None => join(file.parent()?, path),
    };
    target.is_file().then_some(target)
}

/// 웹 경로(`/`로 나눈 조각)를 폴더에 잇는다. `.`·빈 조각은 건너뛰고 `..`은 한 단계 올라간다
/// (같은 파일이 다른 경로로 열려 탭이 둘 생기지 않게).
fn join(base: &Path, web_path: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for segment in web_path.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                path.pop();
            }
            _ => path.push(segment),
        }
    }
    path
}

/// 커서가 include 경로 값 안이면 그 값. 지시자 `file`, `jsp:include`·`jsp:forward`의 `page`,
/// `c:import`의 `url`.
pub fn include_at(text: &str, offset: usize) -> Option<&str> {
    let within = |range: &std::ops::Range<usize>| range.start <= offset && offset <= range.end;
    for region in jsp::regions(text) {
        if region.kind != Kind::Directive || !within(&region.range) {
            continue;
        }
        let inner = region.inner();
        let body = text[inner.clone()].trim_start();
        let skipped = inner.len() - body.len();
        let rest = body.strip_prefix("include")?;
        let start = inner.start + skipped + "include".len();
        let value = attribute_range(rest, "file")?;
        let value = start + value.start..start + value.end;
        return within(&value).then(|| &text[value]);
    }
    let tag = jsp::open_tag_at(text, offset).filter(|t| t.in_value)?;
    let (_, name) = INCLUDE_TAGS
        .iter()
        .find(|(t, _)| tag.name.eq_ignore_ascii_case(t))?;
    let end = text[tag.start..]
        .find('>')
        .map_or(text.len(), |e| tag.start + e);
    let value = attribute_range(&text[tag.start..end], name)?;
    let value = tag.start + value.start..tag.start + value.end;
    within(&value).then(|| &text[value])
}

/// 정적 include(`<%@ include file %>`) 경로들. taglib 선언을 공통 파일에 모아 두는 경우를 따라간다.
pub fn static_includes(text: &str) -> Vec<&str> {
    jsp::regions(text)
        .into_iter()
        .filter(|r| r.kind == Kind::Directive)
        .filter_map(|r| {
            let inner = r.inner();
            let body = text[inner.clone()].trim_start();
            let rest = body.strip_prefix("include")?;
            let value = attribute_range(rest, "file")?;
            let start = inner.end - rest.len();
            Some(&text[start + value.start..start + value.end])
        })
        .collect()
}

/// Spring XML 설정의 뷰 리졸버 접두사·접미사(`<property name="prefix" value=…>`나 `p:prefix`).
pub fn view_resolvers(xml: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find("<bean") {
        let bean = &rest[at..];
        rest = &bean[5..];
        let Some(tag_end) = bean.find('>') else {
            break;
        };
        let start_tag = &bean[..tag_end];
        if !attribute(start_tag, "class").is_some_and(|c| c.trim().ends_with("ViewResolver")) {
            continue;
        }
        // 속성으로 줄인 p:prefix, 아니면 `</bean>`까지의 property.
        let body = if start_tag.ends_with('/') {
            ""
        } else {
            bean.find("</bean>").map_or("", |end| &bean[tag_end..end])
        };
        let property = |name: &str| {
            attribute(start_tag, &format!("p:{name}"))
                .or_else(|| {
                    body.match_indices("<property").find_map(|(p, _)| {
                        let tag = &body[p..body[p..].find('>').map_or(body.len(), |e| p + e)];
                        (attribute(tag, "name") == Some(name))
                            .then(|| attribute(tag, "value"))
                            .flatten()
                    })
                })
                .map(|v| v.trim().to_owned())
        };
        let (prefix, suffix) = (property("prefix"), property("suffix"));
        if prefix.is_some() || suffix.is_some() {
            found.push((prefix.unwrap_or_default(), suffix.unwrap_or_default()));
        }
    }
    found
}

/// Spring Boot `application.properties`의 `spring.mvc.view.prefix`·`suffix`.
pub fn view_properties(text: &str) -> Option<(String, String)> {
    let value = |key: &str| {
        text.lines().find_map(|line| {
            let rest = line.trim().strip_prefix(key)?.trim_start();
            let rest = rest.strip_prefix(['=', ':'])?;
            Some(rest.trim().to_owned())
        })
    };
    let (prefix, suffix) = (
        value("spring.mvc.view.prefix"),
        value("spring.mvc.view.suffix"),
    );
    (prefix.is_some() || suffix.is_some())
        .then(|| (prefix.unwrap_or_default(), suffix.unwrap_or_default()))
}

/// 뷰 이름을 JSP 파일로: 웹 루트 + 접두사 + 이름 + 접미사. `redirect:`·`forward:`는 뷰가 아니다.
pub fn view_files(name: &str, views: &[(String, String)], roots: &[PathBuf]) -> Vec<PathBuf> {
    if name.contains(':') || name.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (prefix, suffix) in views {
        let relative = format!("{prefix}/{name}{suffix}");
        for root in roots {
            let path = join(root, &relative);
            if path.is_file() && !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// `web.xml`의 `<taglib><taglib-uri>…</taglib-uri><taglib-location>…</taglib-location>` 매핑.
pub fn taglib_locations(xml: &str) -> Vec<(String, String)> {
    let text_of = |block: &str, name: &str| {
        let open = format!("<{name}>");
        let start = block.find(&open)? + open.len();
        let end = block[start..].find(&format!("</{name}>"))? + start;
        Some(block[start..end].trim().to_owned())
    };
    xml.split("<taglib>")
        .skip(1)
        .filter_map(|block| {
            let block = &block[..block.find("</taglib>")?];
            Some((
                text_of(block, "taglib-uri")?,
                text_of(block, "taglib-location")?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 커서_자리의_include_경로를_안다() {
        let page = r#"<%@ include file="/WEB-INF/jsp/include/taglib.jsp" %>
<jsp:include page="../common/top.jsp?menu=1" flush="true"/>
<c:import url="${path}/x.jsp"/><c:out value="/no.jsp"/>"#;
        let at = |needle: &str| page.find(needle).unwrap() + 3;
        assert_eq!(
            include_at(page, at("WEB-INF")),
            Some("/WEB-INF/jsp/include/taglib.jsp")
        );
        assert_eq!(
            include_at(page, at("common")),
            Some("../common/top.jsp?menu=1")
        );
        assert_eq!(
            include_at(page, at("${path}")),
            None,
            "EL 안은 경로로 풀 수 없다"
        );
        assert_eq!(include_at(page, at("/x.jsp")), Some("${path}/x.jsp"));
        assert_eq!(
            include_at(page, at("no.jsp")),
            None,
            "c:out은 include가 아니다"
        );
        assert_eq!(include_at(page, at("flush")), None, "값 밖");
        assert_eq!(
            static_includes(page),
            vec!["/WEB-INF/jsp/include/taglib.jsp"]
        );
    }

    #[test]
    fn 뷰_리졸버와_taglib_매핑을_읽는다() {
        let servlet = r#"<beans>
  <bean class="org.springframework.web.servlet.view.BeanNameViewResolver" p:order="0"/>
  <bean class="org.springframework.web.servlet.view.UrlBasedViewResolver" p:order="1"
        p:viewClass="org.springframework.web.servlet.view.JstlView"
        p:prefix="/WEB-INF/jsp/egovframework/example/" p:suffix=".jsp"/>
  <bean id="v" class="org.springframework.web.servlet.view.InternalResourceViewResolver">
    <property name="prefix" value="/WEB-INF/views/" />
    <property name="suffix" value=".jsp"/>
  </bean>
</beans>"#;
        assert_eq!(
            view_resolvers(servlet),
            vec![
                ("/WEB-INF/jsp/egovframework/example/".into(), ".jsp".into()),
                ("/WEB-INF/views/".into(), ".jsp".into()),
            ]
        );
        assert_eq!(
            view_properties(
                "server.port=8080\nspring.mvc.view.prefix = /WEB-INF/jsp/\nspring.mvc.view.suffix=.jsp\n"
            ),
            Some(("/WEB-INF/jsp/".into(), ".jsp".into()))
        );
        let web = "<jsp-config><taglib>\n<taglib-uri>/customs</taglib-uri>\n<taglib-location>/WEB-INF/tlds/customs.tld</taglib-location></taglib></jsp-config>";
        assert_eq!(
            taglib_locations(web),
            vec![("/customs".into(), "/WEB-INF/tlds/customs.tld".into())]
        );
    }

    #[test]
    fn 뷰_이름과_include를_파일로_푼다() {
        let root = std::env::temp_dir().join(format!("ferrite-egov-web-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let web = root.join("src/main/webapp");
        std::fs::create_dir_all(web.join("WEB-INF/jsp/sample")).unwrap();
        let list = web.join("WEB-INF/jsp/sample/list.jsp");
        std::fs::write(&list, "").unwrap();
        let views = vec![("/WEB-INF/jsp/".to_owned(), ".jsp".to_owned())];
        assert_eq!(
            resolve(
                "../sample/./list.jsp",
                &web.join("WEB-INF/jsp/x/a.jsp"),
                Some(&web)
            ),
            Some(list.clone()),
            "..은 경로에서 접는다"
        );
        let roots = vec![web.clone()];
        assert_eq!(
            view_files("sample/list", &views, &roots),
            vec![list.clone()]
        );
        assert_eq!(
            view_files("/sample/list", &views, &roots),
            vec![list.clone()]
        );
        assert!(view_files("redirect:/list.do", &views, &roots).is_empty());
        assert!(view_files("sample/none", &views, &roots).is_empty());

        let other = web.join("WEB-INF/jsp/sample/other.jsp");
        assert_eq!(
            resolve("/WEB-INF/jsp/sample/list.jsp?x=1", &other, Some(&web)),
            Some(list.clone())
        );
        assert_eq!(resolve("list.jsp", &other, Some(&web)), Some(list.clone()));
        assert_eq!(resolve("${ctx}/list.jsp", &other, Some(&web)), None);
        assert_eq!(web_root_for(&roots, &other), Some(&web));
        let _ = std::fs::remove_dir_all(&root);
    }
}
