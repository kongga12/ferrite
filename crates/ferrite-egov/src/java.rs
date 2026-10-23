//! Java 쪽 분석: Mapper 인터페이스 색인과, 커서가 가리키는 매퍼 문장 찾기.
//! 타입 추론은 하지 않는다. egov 코드의 관례(필드·지역 변수의 선언 타입, import, 같은 패키지)로
//! 호출 대상 Mapper를 알아낸다.

use std::ops::Range;

use tree_sitter::{Node, Parser, Tree};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    pub name: String,
    /// 메서드 이름의 바이트 범위.
    pub range: Range<usize>,
    /// XML 문장이 필요 없는 메서드: `@Select` 같은 SQL 애너테이션을 달았거나 본문이 있다
    /// (default·static 메서드).
    pub needs_statement: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    /// 패키지를 붙인 이름. MyBatis 매퍼의 namespace와 같아야 연결된다.
    pub fqn: String,
    pub methods: Vec<Method>,
}

/// 한 파일에서 모은 인터페이스. 인터페이스가 없는 파일은 색인할 필요가 없다.
pub fn interfaces(text: &str) -> Vec<Interface> {
    let Some(tree) = parse(text) else {
        return Vec::new();
    };
    let package = package(tree.root_node(), text);
    let mut found = Vec::new();
    collect_interfaces(tree.root_node(), text, package.as_deref(), &mut found);
    found
}

/// 커서가 가리키는 매퍼 문장을 찾을 단서.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// 문자열로 쓴 문장 id(`selectList("ns.id", vo)` 같은 DAO 방식, iBatis는 id 그대로).
    StatementId(String),
    /// Mapper 인터페이스(= namespace)의 메서드.
    Method { namespace: String, name: String },
}

/// `offset`(바이트) 위치가 가리키는 매퍼 문장 단서.
pub fn target_at(text: &str, offset: usize) -> Option<Target> {
    let tree = parse(text)?;
    let root = tree.root_node();
    let node = root.descendant_for_byte_range(offset, offset)?;
    let package = package(root, text);
    match node.kind() {
        "string_fragment" => {
            let id = node.utf8_text(text.as_bytes()).ok()?;
            (!id.is_empty() && !id.contains(char::is_whitespace))
                .then(|| Target::StatementId(id.to_owned()))
        }
        "identifier" => {
            let parent = node.parent()?;
            let name = node.utf8_text(text.as_bytes()).ok()?.to_owned();
            match parent.kind() {
                // 인터페이스에 선언한 메서드 이름.
                "method_declaration" if parent.child_by_field_name("name") == Some(node) => {
                    let interface = ancestor(parent, "interface_declaration")?;
                    let simple = field_text(interface, "name", text)?;
                    Some(Target::Method {
                        namespace: qualify(package.as_deref(), simple),
                        name,
                    })
                }
                // `sampleMapper.selectSample(vo)`: 받는 변수의 선언 타입이 Mapper다.
                "method_invocation" if parent.child_by_field_name("name") == Some(node) => {
                    let object = parent.child_by_field_name("object")?;
                    let variable = match object.kind() {
                        "identifier" => object.utf8_text(text.as_bytes()).ok()?,
                        "field_access" => field_text(object, "field", text)?,
                        _ => return None,
                    };
                    let simple = declared_type(root, variable, text)?;
                    Some(Target::Method {
                        namespace: resolve(root, text, package.as_deref(), &simple),
                        name,
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .ok()?;
    parser.parse(text, None)
}

fn children(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    (0..node.child_count()).filter_map(move |i| node.child(i))
}

fn field_text<'t>(node: Node<'_>, field: &str, text: &'t str) -> Option<&'t str> {
    node.child_by_field_name(field)?
        .utf8_text(text.as_bytes())
        .ok()
}

fn ancestor<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == kind {
            return Some(n);
        }
        current = n.parent();
    }
    None
}

fn package(root: Node<'_>, text: &str) -> Option<String> {
    let declaration = children(root).find(|n| n.kind() == "package_declaration")?;
    let name =
        children(declaration).find(|n| matches!(n.kind(), "identifier" | "scoped_identifier"))?;
    Some(name.utf8_text(text.as_bytes()).ok()?.to_owned())
}

fn qualify(package: Option<&str>, simple: &str) -> String {
    match package {
        Some(package) => format!("{package}.{simple}"),
        None => simple.to_owned(),
    }
}

fn collect_interfaces(
    node: Node<'_>,
    text: &str,
    package: Option<&str>,
    found: &mut Vec<Interface>,
) {
    if node.kind() == "interface_declaration"
        && let Some(simple) = field_text(node, "name", text)
        && let Some(body) = node.child_by_field_name("body")
    {
        let methods = children(body)
            .filter(|n| n.kind() == "method_declaration")
            .filter_map(|method| {
                let name = method.child_by_field_name("name")?;
                Some(Method {
                    name: name.utf8_text(text.as_bytes()).ok()?.to_owned(),
                    range: name.byte_range(),
                    needs_statement: method.child_by_field_name("body").is_none()
                        && !has_sql_annotation(method, text),
                })
            })
            .collect();
        found.push(Interface {
            fqn: qualify(package, simple),
            methods,
        });
    }
    for child in children(node) {
        collect_interfaces(child, text, package, found);
    }
}

/// MyBatis 애너테이션(`@Select`, `@InsertProvider` 등)으로 SQL을 직접 단 메서드인가.
fn has_sql_annotation(method: Node<'_>, text: &str) -> bool {
    const SQL: [&str; 8] = [
        "Select",
        "Insert",
        "Update",
        "Delete",
        "SelectProvider",
        "InsertProvider",
        "UpdateProvider",
        "DeleteProvider",
    ];
    let Some(modifiers) = children(method).find(|n| n.kind() == "modifiers") else {
        return false;
    };
    children(modifiers)
        .filter(|n| matches!(n.kind(), "annotation" | "marker_annotation"))
        .filter_map(|a| field_text(a, "name", text))
        .any(|name| SQL.contains(&name.rsplit('.').next().unwrap_or(name)))
}

/// 파일 안에서 이 이름으로 선언한 필드·지역 변수·매개변수의 타입(단순 이름).
fn declared_type(root: Node<'_>, variable: &str, text: &str) -> Option<String> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let found = match node.kind() {
            "field_declaration" | "local_variable_declaration" => children(node)
                .filter(|n| n.kind() == "variable_declarator")
                .any(|d| field_text(d, "name", text) == Some(variable))
                .then(|| node.child_by_field_name("type"))
                .flatten(),
            "formal_parameter" => (field_text(node, "name", text) == Some(variable))
                .then(|| node.child_by_field_name("type"))
                .flatten(),
            _ => None,
        };
        if let Some(ty) = found {
            return simple_type_name(ty, text);
        }
        stack.extend(children(node));
    }
    None
}

/// `SampleMapper`, `a.b.SampleMapper`, `List<X>`에서 바깥 타입의 단순 이름.
fn simple_type_name(ty: Node<'_>, text: &str) -> Option<String> {
    let name_node = match ty.kind() {
        "generic_type" => children(ty).next()?,
        _ => ty,
    };
    let full = name_node.utf8_text(text.as_bytes()).ok()?;
    Some(full.rsplit('.').next().unwrap_or(full).to_owned())
}

/// 단순 이름을 import(정확히 일치) → 같은 패키지 순으로 정규화한다.
fn resolve(root: Node<'_>, text: &str, package: Option<&str>, simple: &str) -> String {
    let suffix = format!(".{simple}");
    children(root)
        .filter(|n| n.kind() == "import_declaration")
        .filter_map(|import| {
            children(import)
                .find(|n| matches!(n.kind(), "identifier" | "scoped_identifier"))
                .and_then(|n| n.utf8_text(text.as_bytes()).ok())
        })
        .find(|fqn| fqn.ends_with(&suffix))
        .map(str::to_owned)
        .unwrap_or_else(|| qualify(package, simple))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPER: &str = r#"package egov.sample.service.impl;

import org.apache.ibatis.annotations.Select;

@Mapper("sampleMapper")
public interface SampleMapper {
    List<SampleVO> selectSampleList(SampleVO vo);
    @Select("SELECT 1")
    int selectOne();
    default int helper() { return 0; }
}
"#;

    const SERVICE: &str = r#"package egov.sample.service.impl;

import egov.other.OtherMapper;

public class SampleServiceImpl {
    @Resource(name = "sampleMapper")
    private SampleMapper sampleMapper;
    private OtherMapper other;

    public List<SampleVO> list(SampleVO vo) {
        dao.selectList("egov.sample.service.impl.SampleMapper.selectSampleList", vo);
        other.selectX(vo);
        return this.sampleMapper.selectSampleList(vo);
    }
}
"#;

    #[test]
    fn 인터페이스와_xml이_필요한_메서드를_모은다() {
        let found = interfaces(MAPPER);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].fqn, "egov.sample.service.impl.SampleMapper");
        let methods: Vec<(&str, bool)> = found[0]
            .methods
            .iter()
            .map(|m| (m.name.as_str(), m.needs_statement))
            .collect();
        assert_eq!(
            methods,
            [
                ("selectSampleList", true),
                ("selectOne", false),
                ("helper", false)
            ],
            "@Select와 default 메서드는 XML 문장이 필요 없다"
        );
        assert_eq!(
            &MAPPER[found[0].methods[0].range.clone()],
            "selectSampleList"
        );
        assert!(interfaces(SERVICE).is_empty());
    }

    #[test]
    fn 커서_위치의_매퍼_문장_단서를_찾는다() {
        let at = |text: &str, needle: &str| target_at(text, text.find(needle).unwrap() + 2);
        assert_eq!(
            at(MAPPER, "selectSampleList"),
            Some(Target::Method {
                namespace: "egov.sample.service.impl.SampleMapper".into(),
                name: "selectSampleList".into()
            }),
            "선언"
        );
        assert_eq!(
            at(SERVICE, "egov.sample.service.impl.SampleMapper.select"),
            Some(Target::StatementId(
                "egov.sample.service.impl.SampleMapper.selectSampleList".into()
            )),
            "DAO 문자열 id"
        );
        assert_eq!(
            at(SERVICE, "selectX"),
            Some(Target::Method {
                namespace: "egov.other.OtherMapper".into(),
                name: "selectX".into()
            }),
            "import로 정규화"
        );
        let call = SERVICE.rfind("selectSampleList").unwrap() + 2;
        assert_eq!(
            target_at(SERVICE, call),
            Some(Target::Method {
                namespace: "egov.sample.service.impl.SampleMapper".into(),
                name: "selectSampleList".into()
            }),
            "this.필드 호출, 같은 패키지로 정규화"
        );
        assert_eq!(at(SERVICE, "class SampleServiceImpl"), None);
    }
}
