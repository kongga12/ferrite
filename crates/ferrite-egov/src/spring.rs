//! Spring XML 설정(`<beans>`): 빈 정의·참조, 빈 class, property 이름, `${}` 플레이스홀더, import.
//! egov 3.x·4.x의 `context-*.xml`, `dispatcher-servlet.xml`이 대상이다.
//!
//! Spring Boot LS에도 XML 설정 지원이 있지만 XML 문서를 그 서버에 보내려면 jdt.ls와 함께 JVM 둘을
//! 띄워야 하고, 없는 빈·클래스·속성 진단과 플레이스홀더 이동은 없다. 그래서 여기서 직접 다룬다.

use std::ops::Range;

use tree_sitter::{Node, Parser};

use crate::mapper::{attributes, children, tag_name, tag_of};

/// 빈 하나의 이름(id, name, alias). 같은 빈이 이름을 여럿 가질 수 있다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeanDef {
    pub name: String,
    pub class: Option<String>,
    pub range: Range<usize>,
}

/// 이름 하나를 쓴 자리(빈 참조, 클래스 이름, 플레이스홀더 키, import 경로).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub name: String,
    pub range: Range<usize>,
}

/// `<property name>`·`p:이름` 하나. `owner`는 그 빈의 class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub owner: Option<String>,
    pub range: Range<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub defs: Vec<BeanDef>,
    pub refs: Vec<Usage>,
    pub classes: Vec<Usage>,
    pub properties: Vec<Property>,
    pub placeholders: Vec<Usage>,
    pub imports: Vec<Usage>,
}

/// 빈 이름을 받는 속성(`*-ref`는 따로 본다). `bean`은 `<ref bean>`·`<idref bean>`이다.
const REF_ATTRIBUTES: [&str; 7] = [
    "ref",
    "parent",
    "depends-on",
    "factory-bean",
    "transaction-manager",
    "data-source",
    "local",
];

fn is_ref_attribute(name: &str) -> bool {
    REF_ATTRIBUTES.contains(&name) || (name.ends_with("-ref") && !name.starts_with("p:"))
}

/// Spring XML 설정이면 분석 결과, 아니면(다른 XML) None. 루트가 `beans`(`beans:beans` 포함)다.
pub fn parse(text: &str) -> Option<Config> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_xml::LANGUAGE_XML.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = children(tree.root_node()).find(|n| n.kind() == "element")?;
    let name = tag_name(tag_of(root)?, text)?;
    if local(name) != "beans" {
        return None;
    }
    let mut config = Config::default();
    walk(root, text, None, &mut config);
    // `dataSource-${Globals.DbType}`처럼 플레이스홀더로 만든 이름은 실행 전에는 모른다.
    config.refs.retain(|r| !r.name.contains("${"));
    Some(config)
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn walk(node: Node<'_>, text: &str, owner: Option<&str>, config: &mut Config) {
    match node.kind() {
        "Comment" => return,
        "CharData" | "CData" => {
            placeholders(text, node.byte_range(), &mut config.placeholders);
            return;
        }
        _ => {}
    }
    let mut owner = owner.map(str::to_owned);
    if node.kind() == "element"
        && let Some(tag) = tag_of(node)
        && let Some(name) = tag_name(tag, text)
    {
        owner = element(name, tag, text, owner, config);
    }
    for child in children(node) {
        walk(child, text, owner.as_deref(), config);
    }
}

/// 요소 하나를 모은다. 돌려주는 값은 자식 요소의 property가 속할 class.
fn element(
    name: &str,
    tag: Node<'_>,
    text: &str,
    owner: Option<String>,
    config: &mut Config,
) -> Option<String> {
    let usage = |name: &str, range: Range<usize>| Usage {
        name: name.to_owned(),
        range,
    };
    let attrs: Vec<(&str, &str, Range<usize>)> = attributes(tag, text).collect();
    for (_, _, range) in &attrs {
        placeholders(text, range.clone(), &mut config.placeholders);
    }
    let class = attrs
        .iter()
        .find(|(n, _, _)| *n == "class")
        .map(|(_, v, _)| v.trim().to_owned());
    let element = local(name);
    let is_bean = element == "bean" && (name == "bean" || name.starts_with("beans:"));
    for (attribute, value, range) in &attrs {
        let attribute = *attribute;
        match (element, attribute) {
            // 다른 네임스페이스의 id(`util:properties`, `tx:advice`, `aop:pointcut` 등)도 이름이다.
            (_, "id") if !value.is_empty() => config.defs.push(BeanDef {
                name: value.to_string(),
                class: if is_bean { class.clone() } else { None },
                range: range.clone(),
            }),
            ("bean", "name") if is_bean => {
                for (name, part) in split_names(value, range.start) {
                    config.defs.push(BeanDef {
                        name: name.to_owned(),
                        class: class.clone(),
                        range: part,
                    });
                }
            }
            ("alias", "alias") => config.defs.push(BeanDef {
                name: value.to_string(),
                class: None,
                range: range.clone(),
            }),
            ("alias", "name") | ("ref" | "idref", "bean") => {
                config.refs.push(usage(value, range.clone()))
            }
            ("bean", "class") if is_bean && !value.contains("${") => {
                config.classes.push(usage(value.trim(), range.clone()))
            }
            ("bean", "depends-on") if is_bean => {
                for (name, part) in split_names(value, range.start) {
                    config.refs.push(usage(name, part));
                }
            }
            ("property", "name") => config.properties.push(Property {
                name: value.to_string(),
                owner: owner.clone(),
                range: range.clone(),
            }),
            // `p:name="값"`, `p:name-ref="빈"`(p 네임스페이스로 줄인 property).
            (_, p) if is_bean && p.starts_with("p:") => {
                let property = &p[2..];
                let (property, is_ref) = match property.strip_suffix("-ref") {
                    Some(name) => (name, true),
                    None => (property, false),
                };
                let start =
                    tag.start_byte() + text[tag.byte_range()].find(p).map_or(0, |at| at + 2);
                config.properties.push(Property {
                    name: property.to_owned(),
                    owner: class.clone(),
                    range: start..start + property.len(),
                });
                if is_ref {
                    config.refs.push(usage(value, range.clone()));
                }
            }
            (_, a) if is_ref_attribute(a) && !value.contains("${") && !value.is_empty() => {
                config.refs.push(usage(value.trim(), range.clone()))
            }
            ("import", "resource") => config.imports.push(usage(value, range.clone())),
            _ => {}
        }
    }
    if is_bean { class } else { owner }
}

/// `a, b; c` 같은 이름 목록을 (이름, 문서 범위)로 나눈다.
fn split_names(value: &str, start: usize) -> impl Iterator<Item = (&str, Range<usize>)> {
    let mut offset = 0;
    value
        .split([',', ';', ' '])
        .map(move |part| {
            let at = offset;
            offset += part.len() + 1;
            (part, start + at..start + at + part.len())
        })
        .filter(|(part, _)| !part.is_empty())
}

/// `${key}`·`${key:기본값}`의 key를 모은다. `#{}`(SpEL)는 뺀다.
fn placeholders(text: &str, span: Range<usize>, found: &mut Vec<Usage>) {
    let Some(segment) = text.get(span.clone()) else {
        return;
    };
    let mut from = 0;
    while let Some(at) = segment[from..].find("${") {
        let start = from + at + 2;
        let Some(end) = segment[start..].find(['}', ':']).map(|e| start + e) else {
            break;
        };
        let key = segment[start..end].trim();
        if !key.is_empty() && !key.contains("${") {
            found.push(Usage {
                name: key.to_owned(),
                range: span.start + start..span.start + end,
            });
        }
        from = end;
    }
}

/// `.properties` 파일의 키와 그 바이트 범위. `#`·`!` 주석과 `\`로 이은 줄을 따른다.
pub fn property_keys(text: &str) -> Vec<(String, Range<usize>)> {
    let mut found = Vec::new();
    let mut offset = 0;
    let mut continued = false;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        let was_continued = continued;
        continued = body.ends_with('\\') && !body.ends_with("\\\\");
        let trimmed = body.trim_start();
        if was_continued || trimmed.is_empty() || trimmed.starts_with(['#', '!']) {
            continue;
        }
        let key_start = start + (body.len() - trimmed.len());
        let key_len = trimmed
            .find(|c: char| c == '=' || c == ':' || c.is_whitespace())
            .unwrap_or(trimmed.len());
        if key_len > 0 {
            found.push((
                trimmed[..key_len].to_owned(),
                key_start..key_start + key_len,
            ));
        }
    }
    found
}

/// Spring XML 자동완성 자리. `replace`는 고른 항목으로 바꿀 범위다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Site {
    /// `<bean class="` 안. `typed`는 커서까지 친 값.
    Class {
        replace: Range<usize>,
        typed: String,
    },
    /// 빈 이름을 받는 속성값(`ref`, `<ref bean>`, `*-ref` 등).
    Bean { replace: Range<usize> },
    /// `<property name="` 안. `owner`는 감싼 `<bean>`의 class.
    Property {
        owner: Option<String>,
        replace: Range<usize>,
    },
    /// `${` 뒤.
    Placeholder { replace: Range<usize> },
}

/// `offset`(바이트) 자리의 자동완성 문맥. 입력 중인 XML은 깨져 있기 쉬워 커서 앞 텍스트로 판단한다.
pub fn site_at(text: &str, offset: usize) -> Option<Site> {
    let before = text.get(..offset)?;
    let word_end = |stop: &[char]| {
        text[offset..]
            .find(|c: char| stop.contains(&c) || c == '\n')
            .map_or(offset, |end| offset + end)
    };
    // 플레이스홀더는 속성값·본문 어디서나.
    if let Some(open) = before.rfind("${")
        && !before[open..].contains(['}', '"', '\'', '<', '>'])
    {
        return Some(Site::Placeholder {
            replace: open + 2..word_end(&['}', '"', '\'', '<', ':']),
        });
    }
    let tag_start = before.rfind('<')?;
    let tag = &before[tag_start + 1..];
    if tag.contains('>') || tag.matches(['"', '\'']).count() % 2 == 0 {
        return None;
    }
    let element = tag.split(char::is_whitespace).next()?;
    let quote = tag.rfind(['"', '\''])?;
    let attribute = tag[..quote]
        .trim_end()
        .strip_suffix('=')?
        .trim_end()
        .rsplit(char::is_whitespace)
        .next()?;
    let value_start = tag_start + 1 + quote + 1;
    let replace = value_start..word_end(&['"', '\'', '<']);
    match (local(element), attribute) {
        ("bean", "class") => Some(Site::Class {
            replace,
            typed: text[value_start..offset].to_owned(),
        }),
        ("property", "name") => Some(Site::Property {
            owner: enclosing_bean_class(&before[..tag_start]),
            replace,
        }),
        ("ref" | "idref", "bean") => Some(Site::Bean { replace }),
        (_, a) if is_ref_attribute(a) || (a.starts_with("p:") && a.ends_with("-ref")) => {
            Some(Site::Bean { replace })
        }
        _ => None,
    }
}

/// 커서 앞에서 아직 닫히지 않은 가장 안쪽 `<bean>`의 class. 앞에서부터 태그를 쌓아 본다
/// (`<bean …/>`처럼 스스로 닫힌 빈은 쌓지 않는다).
fn enclosing_bean_class(before: &str) -> Option<String> {
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut rest = before;
    while let Some(at) = rest.find('<') {
        rest = &rest[at..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[1..end];
        rest = &rest[end.min(rest.len())..];
        // `/bean`(닫는 태그), `bean/`(빈 요소)에서 이름만.
        let name = tag
            .split(char::is_whitespace)
            .next()
            .unwrap_or("")
            .trim_end_matches('/');
        if let Some(closing) = name.strip_prefix('/') {
            if local(closing) == "bean" {
                stack.pop();
            }
        } else if local(name) == "bean" && !tag.ends_with('/') {
            stack.push(crate::jsp::attribute(tag, "class").map(|c| c.trim().to_owned()));
        }
    }
    stack.pop().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTEXT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<beans xmlns="http://www.springframework.org/schema/beans"
       xmlns:p="http://www.springframework.org/schema/p">
  <!-- <bean id="commented" class="x.Y"/> -->
  <import resource="classpath:/egovframework/spring/context-common.xml"/>
  <bean id="dataSource" name="ds, mainDs" class="org.apache.commons.dbcp2.BasicDataSource">
    <property name="driverClassName" value="${Globals.DriverClassName}"/>
    <property name="url" value="${Globals.Url:jdbc:h2:mem}"/>
  </bean>
  <alias name="dataSource" alias="egov.dataSource"/>
  <bean id="sampleService" class="egov.sample.service.impl.SampleServiceImpl"
        p:pageUnit="10" p:sampleDAO-ref="sampleDAO" depends-on="dataSource">
    <property name="helper">
      <bean class="egov.sample.Helper"><property name="inner" ref="ds"/></bean>
    </property>
    <property name="list"><list><ref bean="mainDs"/></list></property>
  </bean>
  <tx:advice id="txAdvice" transaction-manager="txManager"/>
  <value>${Globals.FileStorePath}</value>
</beans>"#;

    #[test]
    fn 빈_정의와_참조와_속성을_모은다() {
        let config = parse(CONTEXT).unwrap();
        let names = |defs: &[BeanDef]| defs.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(&config.defs),
            [
                "dataSource",
                "ds",
                "mainDs",
                "egov.dataSource",
                "sampleService",
                "txAdvice"
            ],
            "주석 안의 빈은 뺀다"
        );
        assert_eq!(&CONTEXT[config.defs[2].range.clone()], "mainDs");
        let refs: Vec<&str> = config.refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            refs,
            [
                "dataSource",
                "sampleDAO",
                "dataSource",
                "ds",
                "mainDs",
                "txManager"
            ]
        );
        let properties: Vec<(&str, Option<&str>)> = config
            .properties
            .iter()
            .map(|p| (p.name.as_str(), p.owner.as_deref()))
            .collect();
        let service = Some("egov.sample.service.impl.SampleServiceImpl");
        assert_eq!(
            properties,
            [
                (
                    "driverClassName",
                    Some("org.apache.commons.dbcp2.BasicDataSource")
                ),
                ("url", Some("org.apache.commons.dbcp2.BasicDataSource")),
                ("pageUnit", service),
                ("sampleDAO", service),
                ("helper", service),
                ("inner", Some("egov.sample.Helper")),
                ("list", service),
            ],
            "안쪽 빈의 property는 안쪽 빈 소속"
        );
        let p = config
            .properties
            .iter()
            .find(|p| p.name == "sampleDAO")
            .unwrap();
        assert_eq!(&CONTEXT[p.range.clone()], "sampleDAO");
        let keys: Vec<&str> = config
            .placeholders
            .iter()
            .map(|u| u.name.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "Globals.DriverClassName",
                "Globals.Url",
                "Globals.FileStorePath"
            ]
        );
        assert_eq!(
            config.imports[0].name,
            "classpath:/egovframework/spring/context-common.xml"
        );
        assert_eq!(config.classes.len(), 3);
        assert!(parse("<mapper namespace=\"x\"/>").is_none());
    }

    #[test]
    fn 프로퍼티_키를_찾는다() {
        let text = "# 주석\nGlobals.DbType = oracle\n! 주석\nGlobals.Url:jdbc:oracle:thin:@a\\\n  :1521\n  spaced.key value\n";
        let keys: Vec<(String, &str)> = property_keys(text)
            .into_iter()
            .map(|(k, r)| (k, &text[r]))
            .collect();
        assert_eq!(
            keys.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["Globals.DbType", "Globals.Url", "spaced.key"],
            "이은 줄은 키가 아니다"
        );
        assert!(keys.iter().all(|(k, s)| k == s));
    }

    #[test]
    fn 자동완성_자리를_안다() {
        let doc = r#"<beans><bean id="a" class="egov.sam" p:x-ref="d"><property name="he" ref="ds"/><property name="v" value="${Glo"/></bean><bean class="b.B"/><property name="z"/></beans>"#;
        let at = |needle: &str| doc.find(needle).unwrap() + needle.len();
        assert_eq!(
            site_at(doc, at("egov.sam")),
            Some(Site::Class {
                replace: at("class=\"")..at("egov.sam"),
                typed: "egov.sam".into()
            })
        );
        assert_eq!(
            site_at(doc, at("name=\"he")),
            Some(Site::Property {
                owner: Some("egov.sam".into()),
                replace: at("name=\"")..at("name=\"he"),
            })
        );
        assert!(matches!(
            site_at(doc, at("ref=\"d")),
            Some(Site::Bean { .. })
        ));
        assert!(matches!(
            site_at(doc, at("p:x-ref=\"d")),
            Some(Site::Bean { .. })
        ));
        assert_eq!(
            site_at(doc, at("${Glo")),
            Some(Site::Placeholder {
                replace: at("${")..at("${Glo")
            })
        );
        assert_eq!(
            site_at(doc, at("name=\"z")),
            Some(Site::Property {
                owner: None,
                replace: at("name=\"z") - 1..at("name=\"z"),
            }),
            "스스로 닫힌 <bean/>은 감싸지 않는다"
        );
        assert_eq!(site_at(doc, at("id=\"a")), None);
    }
}
