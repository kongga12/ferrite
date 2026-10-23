//! Java 쪽 분석: 타입 색인(Mapper 인터페이스, VO 클래스의 필드·상속, 메서드 매개변수)과
//! 커서가 가리키는 매퍼 문장 찾기.
//! 타입 추론은 하지 않는다. egov 코드의 관례(선언 타입, import, 같은 패키지)로 타입을 알아낸다.

use std::ops::Range;

use tree_sitter::{Node, Parser, Tree};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    /// 선언 타입의 단순 이름(제네릭 바깥 타입).
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: String,
    /// `@Param("x")`로 붙인 이름. MyBatis는 이 이름으로 `#{x}`를 찾는다.
    pub bound: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    pub name: String,
    /// 메서드 이름의 바이트 범위.
    pub range: Range<usize>,
    /// XML 문장이 필요한가. `@Select` 같은 SQL 애너테이션을 달았거나 본문이 있으면(default·static)
    /// 필요 없다.
    pub needs_statement: bool,
    pub params: Vec<Param>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDecl {
    /// 패키지를 붙인 이름. MyBatis 매퍼의 namespace·parameterType과 같아야 연결된다.
    pub fqn: String,
    pub is_interface: bool,
    /// `extends`로 쓴 상위 클래스 이름(쓴 그대로, 제네릭 바깥).
    pub superclass: Option<String>,
    /// 인스턴스 필드(static 제외).
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JavaFile {
    pub package: Option<String>,
    /// `import a.b.C;`의 `a.b.C`(static·`*` 제외).
    pub imports: Vec<String>,
    pub types: Vec<TypeDecl>,
}

impl JavaFile {
    /// 이 파일에서 쓴 타입 이름을 FQN으로: 점이 있으면 그대로, import(정확히 일치), 같은 패키지 순.
    pub fn resolve(&self, name: &str) -> String {
        if name.contains('.') {
            return name.to_owned();
        }
        let suffix = format!(".{name}");
        self.imports
            .iter()
            .find(|i| i.ends_with(&suffix))
            .cloned()
            .unwrap_or_else(|| qualify(self.package.as_deref(), name))
    }
}

/// 파일 하나를 분석한다. 실패하면 빈 결과.
pub fn parse_file(text: &str) -> JavaFile {
    let Some(tree) = parse(text) else {
        return JavaFile::default();
    };
    let root = tree.root_node();
    let package = package(root, text);
    let imports = children(root)
        .filter(|n| n.kind() == "import_declaration")
        // `import static`과 `import a.b.*`는 타입 하나를 가리키지 않는다.
        .filter(|n| !children(*n).any(|c| matches!(c.kind(), "static" | "asterisk")))
        .filter_map(|n| {
            children(n)
                .find(|c| matches!(c.kind(), "identifier" | "scoped_identifier"))
                .and_then(|c| c.utf8_text(text.as_bytes()).ok())
                .map(str::to_owned)
        })
        .collect();
    let mut types = Vec::new();
    collect_types(root, text, package.as_deref(), &mut types);
    JavaFile {
        package,
        imports,
        types,
    }
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
    match node.kind() {
        "string_fragment" => {
            let id = node.utf8_text(text.as_bytes()).ok()?;
            (!id.is_empty() && !id.contains(char::is_whitespace))
                .then(|| Target::StatementId(id.to_owned()))
        }
        "identifier" => {
            let parent = node.parent()?;
            let name = node.utf8_text(text.as_bytes()).ok()?.to_owned();
            let file = || JavaFile {
                package: package(root, text),
                imports: parse_file(text).imports,
                types: Vec::new(),
            };
            match parent.kind() {
                // 인터페이스에 선언한 메서드 이름.
                "method_declaration" if parent.child_by_field_name("name") == Some(node) => {
                    let interface = ancestor(parent, "interface_declaration")?;
                    let simple = field_text(interface, "name", text)?;
                    Some(Target::Method {
                        namespace: qualify(package(root, text).as_deref(), simple),
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
                        namespace: file().resolve(&simple),
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

fn modifiers<'a>(node: Node<'a>) -> Option<Node<'a>> {
    children(node).find(|n| n.kind() == "modifiers")
}

/// 애너테이션 이름들(마지막 이름만: `org.x.Param` → `Param`)과 그 첫 문자열 인자.
fn annotations<'t>(node: Node<'_>, text: &'t str) -> Vec<(&'t str, Option<&'t str>)> {
    let Some(modifiers) = modifiers(node) else {
        return Vec::new();
    };
    children(modifiers)
        .filter(|n| matches!(n.kind(), "annotation" | "marker_annotation"))
        .filter_map(|a| {
            let name = field_text(a, "name", text)?;
            let argument = a.child_by_field_name("arguments").and_then(|args| {
                children(args)
                    .find(|n| n.kind() == "string_literal")
                    .and_then(|s| children(s).find(|n| n.kind() == "string_fragment"))
                    .and_then(|s| s.utf8_text(text.as_bytes()).ok())
            });
            Some((name.rsplit('.').next().unwrap_or(name), argument))
        })
        .collect()
}

fn is_static(node: Node<'_>) -> bool {
    modifiers(node).is_some_and(|m| children(m).any(|c| c.kind() == "static"))
}

fn collect_types(node: Node<'_>, text: &str, package: Option<&str>, found: &mut Vec<TypeDecl>) {
    let is_interface = node.kind() == "interface_declaration";
    if (is_interface || node.kind() == "class_declaration")
        && let Some(simple) = field_text(node, "name", text)
        && let Some(body) = node.child_by_field_name("body")
    {
        let superclass = node
            .child_by_field_name("superclass")
            .and_then(|s| children(s).find(|c| c.is_named()))
            .and_then(|ty| simple_type_name(ty, text));
        let fields = children(body)
            .filter(|n| n.kind() == "field_declaration" && !is_static(*n))
            .flat_map(|declaration| {
                let ty = declaration
                    .child_by_field_name("type")
                    .and_then(|t| simple_type_name(t, text))
                    .unwrap_or_default();
                children(declaration)
                    .filter(|d| d.kind() == "variable_declarator")
                    .filter_map(|d| field_text(d, "name", text))
                    .map(move |name| Field {
                        name: name.to_owned(),
                        ty: ty.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let methods = children(body)
            .filter(|n| n.kind() == "method_declaration")
            .filter_map(|method| method_of(method, text))
            .collect();
        found.push(TypeDecl {
            fqn: qualify(package, simple),
            is_interface,
            superclass,
            fields,
            methods,
        });
    }
    for child in children(node) {
        collect_types(child, text, package, found);
    }
}

fn method_of(method: Node<'_>, text: &str) -> Option<Method> {
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
    let name = method.child_by_field_name("name")?;
    let annotated = annotations(method, text)
        .iter()
        .any(|(a, _)| SQL.contains(a));
    let params = method
        .child_by_field_name("parameters")
        .map(|list| {
            children(list)
                .filter(|p| p.kind() == "formal_parameter")
                .filter_map(|p| {
                    Some(Param {
                        name: field_text(p, "name", text)?.to_owned(),
                        ty: simple_type_name(p.child_by_field_name("type")?, text)?,
                        bound: annotations(p, text)
                            .into_iter()
                            .find(|(a, _)| *a == "Param")
                            .and_then(|(_, value)| value.map(str::to_owned)),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Method {
        name: name.utf8_text(text.as_bytes()).ok()?.to_owned(),
        range: name.byte_range(),
        needs_statement: method.child_by_field_name("body").is_none() && !annotated,
        params,
    })
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

/// `SampleMapper`, `a.b.SampleMapper`, `List<X>`에서 바깥 타입 이름(점이 있으면 그대로 둔다).
fn simple_type_name(ty: Node<'_>, text: &str) -> Option<String> {
    let name_node = match ty.kind() {
        "generic_type" => children(ty).next()?,
        _ => ty,
    };
    Some(name_node.utf8_text(text.as_bytes()).ok()?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPER: &str = r#"package egov.sample.service.impl;

import org.apache.ibatis.annotations.Select;
import org.apache.ibatis.annotations.Param;
import static java.util.Collections.emptyList;
import egov.com.*;

@Mapper("sampleMapper")
public interface SampleMapper {
    List<SampleVO> selectSampleList(SampleVO vo);
    @Select("SELECT 1")
    int selectOne();
    default int helper() { return 0; }
    int updateName(@Param("id") String id, @Param("name") String name);
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

    const VO: &str = r#"package egov.sample.service;

import egov.com.cmm.ComDefaultVO;

public class SampleVO extends ComDefaultVO implements Serializable {
    private static final long serialVersionUID = 1L;
    private String id;
    private String name, description;
    private List<FileVO> files;
}
"#;

    #[test]
    fn 인터페이스와_xml이_필요한_메서드와_매개변수를_모은다() {
        let file = parse_file(MAPPER);
        assert_eq!(
            file.imports,
            [
                "org.apache.ibatis.annotations.Select",
                "org.apache.ibatis.annotations.Param"
            ],
            "static·* import 제외"
        );
        let mapper = &file.types[0];
        assert!(mapper.is_interface);
        assert_eq!(mapper.fqn, "egov.sample.service.impl.SampleMapper");
        let methods: Vec<(&str, bool)> = mapper
            .methods
            .iter()
            .map(|m| (m.name.as_str(), m.needs_statement))
            .collect();
        assert_eq!(
            methods,
            [
                ("selectSampleList", true),
                ("selectOne", false),
                ("helper", false),
                ("updateName", true)
            ],
            "@Select와 default 메서드는 XML 문장이 필요 없다"
        );
        assert_eq!(&MAPPER[mapper.methods[0].range.clone()], "selectSampleList");
        assert_eq!(mapper.methods[0].params[0].ty, "SampleVO");
        let bound: Vec<_> = mapper.methods[3]
            .params
            .iter()
            .map(|p| p.bound.as_deref())
            .collect();
        assert_eq!(bound, [Some("id"), Some("name")]);
        assert!(parse_file(SERVICE).types.iter().all(|t| !t.is_interface));
    }

    #[test]
    fn vo의_인스턴스_필드와_상위_클래스를_모은다() {
        let file = parse_file(VO);
        let vo = &file.types[0];
        assert_eq!(vo.fqn, "egov.sample.service.SampleVO");
        assert_eq!(vo.superclass.as_deref(), Some("ComDefaultVO"));
        let fields: Vec<(&str, &str)> = vo
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.ty.as_str()))
            .collect();
        assert_eq!(
            fields,
            [
                ("id", "String"),
                ("name", "String"),
                ("description", "String"),
                ("files", "List")
            ],
            "static 상수 제외, 한 줄 여러 선언"
        );
        assert_eq!(
            file.resolve("ComDefaultVO"),
            "egov.com.cmm.ComDefaultVO",
            "import"
        );
        assert_eq!(
            file.resolve("FileVO"),
            "egov.sample.service.FileVO",
            "같은 패키지"
        );
        assert_eq!(file.resolve("a.b.C"), "a.b.C");
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
