//! MyBatis 3 매퍼와 iBatis 2 sqlMap 분석.
//! - 정의: 문장(select 등), `<sql>` 조각, `<resultMap>`의 id
//! - 참조: `<include refid>`, `resultMap`·`extends`, 다른 문장을 부르는 `select` 속성
//! - 문자열 치환: MyBatis `${x}`, iBatis `$x$`. 값을 SQL 문자열에 그대로 넣으므로 SQL Injection 위험이다.
//!
//! 치환은 SQL 본문(텍스트·CDATA)에서만 찾는다. 주석과 속성값(`<if test>` 등)은 SQL이 아니므로 뺀다.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;

use tree_sitter::{Node, Parser};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    MyBatis,
    IBatis,
}

/// 정의·참조의 종류. 종류마다 id 공간이 따로다(같은 id의 select와 resultMap은 겹치지 않는다).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Statement,
    Sql,
    ResultMap,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Statement => "문장",
            Kind::Sql => "<sql> 조각",
            Kind::ResultMap => "resultMap",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    pub kind: Kind,
    pub id: String,
    /// id 속성값의 바이트 범위(따옴표 제외).
    pub range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    pub kind: Kind,
    /// 참조 이름. `namespace.id` 꼴일 수 있다.
    pub name: String,
    pub range: Range<usize>,
}

/// 문자열 치환 하나(`${x}` 또는 `$x$` 전체의 바이트 범위와 그 안의 이름).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Substitution {
    pub range: Range<usize>,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Mapper {
    pub dialect: Dialect,
    pub namespace: Option<String>,
    pub defs: Vec<Def>,
    pub refs: Vec<Ref>,
    pub substitutions: Vec<Substitution>,
}

impl Mapper {
    pub fn has(&self, kind: Kind, id: &str) -> bool {
        self.defs.iter().any(|d| d.kind == kind && d.id == id)
    }
}

const MYBATIS_STATEMENTS: [&str; 4] = ["select", "insert", "update", "delete"];
const IBATIS_STATEMENTS: [&str; 6] = [
    "select",
    "insert",
    "update",
    "delete",
    "statement",
    "procedure",
];

/// 매퍼 문서면 분석 결과, 아니면(다른 XML) None.
pub fn parse(text: &str) -> Option<Mapper> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_xml::LANGUAGE_XML.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = children(tree.root_node()).find(|n| n.kind() == "element")?;
    let tag = tag_of(root)?;
    let dialect = match tag_name(tag, text)? {
        "mapper" => Dialect::MyBatis,
        "sqlMap" => Dialect::IBatis,
        _ => return None,
    };
    let mut mapper = Mapper {
        dialect,
        namespace: attributes(tag, text)
            .find(|(name, _, _)| *name == "namespace")
            .map(|(_, value, _)| value.to_owned()),
        defs: Vec::new(),
        refs: Vec::new(),
        substitutions: Vec::new(),
    };
    walk(root, text, &mut mapper);
    Some(mapper)
}

fn children(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    (0..node.child_count()).filter_map(move |i| node.child(i))
}

/// 요소의 여는 태그(`<a ...>` 또는 `<a .../>`).
fn tag_of(element: Node<'_>) -> Option<Node<'_>> {
    children(element).find(|n| matches!(n.kind(), "STag" | "EmptyElemTag"))
}

fn tag_name<'t>(tag: Node<'_>, text: &'t str) -> Option<&'t str> {
    let name = children(tag).find(|n| n.kind() == "Name")?;
    text.get(name.byte_range())
}

/// 태그의 속성: (이름, 값, 값의 바이트 범위). 값은 따옴표를 뺀다.
fn attributes<'a>(
    tag: Node<'a>,
    text: &'a str,
) -> impl Iterator<Item = (&'a str, &'a str, Range<usize>)> + 'a {
    children(tag)
        .filter(|n| n.kind() == "Attribute")
        .filter_map(move |attribute| {
            let name = children(attribute).find(|n| n.kind() == "Name")?;
            let value = children(attribute).find(|n| n.kind() == "AttValue")?;
            let quoted = value.byte_range();
            let range = quoted.start + 1..quoted.end.saturating_sub(1).max(quoted.start + 1);
            Some((
                text.get(name.byte_range())?,
                text.get(range.clone())?,
                range,
            ))
        })
}

fn walk(node: Node<'_>, text: &str, mapper: &mut Mapper) {
    match node.kind() {
        "Comment" => return,
        "CharData" | "CData" => scan_substitutions(node.byte_range(), text, mapper),
        "element" => {
            if let Some(tag) = tag_of(node) {
                element(tag, text, mapper);
            }
        }
        _ => {}
    }
    for child in children(node) {
        walk(child, text, mapper);
    }
}

/// 요소 하나의 정의·참조를 모은다.
fn element(tag: Node<'_>, text: &str, mapper: &mut Mapper) {
    let Some(name) = tag_name(tag, text) else {
        return;
    };
    let statements: &[&str] = match mapper.dialect {
        Dialect::MyBatis => &MYBATIS_STATEMENTS,
        Dialect::IBatis => &IBATIS_STATEMENTS,
    };
    let defines = match name {
        "sql" => Some(Kind::Sql),
        "resultMap" => Some(Kind::ResultMap),
        _ if statements.contains(&name) => Some(Kind::Statement),
        _ => None,
    };
    for (attribute, value, range) in attributes(tag, text) {
        // `${}`가 든 이름은 실행할 때 정해지므로 확인할 수 없다.
        if value.contains("${") {
            continue;
        }
        let kind = match (name, attribute) {
            (_, "id") => {
                if let Some(kind) = defines {
                    mapper.defs.push(Def {
                        kind,
                        id: value.to_owned(),
                        range,
                    });
                }
                continue;
            }
            ("include", "refid") => Kind::Sql,
            (_, "resultMap") | ("resultMap", "extends") => Kind::ResultMap,
            // association·collection(MyBatis)이나 result(iBatis)가 다른 문장을 부른다.
            (_, "select") => Kind::Statement,
            _ => continue,
        };
        // MyBatis는 `resultMap="a,b"`처럼 여러 개를 쓸 수 있다.
        let mut offset = range.start;
        for part in value.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                let start = offset + (part.len() - part.trim_start().len());
                mapper.refs.push(Ref {
                    kind,
                    name: trimmed.to_owned(),
                    range: start..start + trimmed.len(),
                });
            }
            offset += part.len() + 1;
        }
    }
}

/// 텍스트 구간에서 문자열 치환을 찾는다. MyBatis `${이름}`, iBatis `$이름$`.
fn scan_substitutions(span: Range<usize>, text: &str, mapper: &mut Mapper) {
    let Some(chunk) = text.get(span.clone()) else {
        return;
    };
    let bytes = chunk.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let found = match mapper.dialect {
            Dialect::MyBatis => (bytes.get(i + 1) == Some(&b'{'))
                .then(|| {
                    chunk[i + 2..]
                        .find('}')
                        .map(|end| (i + 2, i + 2 + end, i + 3 + end))
                })
                .flatten(),
            Dialect::IBatis => {
                let name_len = bytes[i + 1..]
                    .iter()
                    .take_while(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'[' | b']')
                    })
                    .count();
                let starts_well = bytes
                    .get(i + 1)
                    .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
                (starts_well && bytes.get(i + 1 + name_len) == Some(&b'$')).then_some((
                    i + 1,
                    i + 1 + name_len,
                    i + 2 + name_len,
                ))
            }
        };
        match found {
            Some((name_start, name_end, end)) => {
                mapper.substitutions.push(Substitution {
                    range: span.start + i..span.start + end,
                    name: chunk[name_start..name_end].trim().to_owned(),
                });
                i = end;
            }
            None => i += 1,
        }
    }
}

/// 프로젝트의 매퍼 모음. 다른 파일의 `<sql>`·resultMap을 참조할 수 있어 진단에 필요하다.
#[derive(Default)]
pub struct Index {
    pub mappers: HashMap<PathBuf, Mapper>,
}

/// 정의 하나와 그 파일. 같은 파일 안에서 찾은 정의는 경로가 None이다(부른 쪽이 안다).
pub type Found<'a> = (Option<&'a PathBuf>, &'a Def);

impl Index {
    /// 참조가 가리키는 정의들. 앞 단계에서 찾으면 거기서 멈춘다(MyBatis가 찾는 순서와 같다).
    /// 1. 같은 파일의 id
    /// 2. `namespace.id` 꼴이면 그 namespace의 id
    /// 3. iBatis는 문장 namespace를 끄고 쓰는 일이 흔해(`useStatementNamespaces=false`) 전체에서 찾는다.
    pub fn targets<'a>(&'a self, from: &'a Mapper, reference: &Ref) -> Vec<Found<'a>> {
        let same: Vec<Found<'a>> = from
            .defs
            .iter()
            .filter(|d| d.kind == reference.kind && d.id == reference.name)
            .map(|d| (None, d))
            .collect();
        if !same.is_empty() {
            return same;
        }
        let qualified = self.qualified(reference.kind, &reference.name);
        if !qualified.is_empty() || from.dialect == Dialect::MyBatis {
            return qualified;
        }
        self.find(
            reference.kind,
            |m| m.dialect == Dialect::IBatis,
            &reference.name,
        )
    }

    pub fn resolves(&self, from: &Mapper, reference: &Ref) -> bool {
        !self.targets(from, reference).is_empty()
    }

    /// `namespace.id` 꼴 이름의 정의.
    pub fn qualified(&self, kind: Kind, name: &str) -> Vec<Found<'_>> {
        name.rsplit_once('.')
            .map_or_else(Vec::new, |(namespace, id)| {
                self.find(kind, |m| m.namespace.as_deref() == Some(namespace), id)
            })
    }

    /// 조건에 맞는 매퍼에서 이 종류·id인 정의. 결과는 파일 경로순이다(HashMap 순서에 기대지 않게).
    pub fn find(&self, kind: Kind, mapper: impl Fn(&Mapper) -> bool, id: &str) -> Vec<Found<'_>> {
        let mut found: Vec<Found<'_>> = self
            .mappers
            .iter()
            .filter(|(_, m)| mapper(m))
            .flat_map(|(path, m)| {
                m.defs
                    .iter()
                    .filter(|d| d.kind == kind && d.id == id)
                    .map(move |d| (Some(path), d))
            })
            .collect();
        found.sort_by(|a, b| (a.0, a.1.range.start).cmp(&(b.0, b.1.range.start)));
        found
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// 진단 하나. 범위는 바이트 단위다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub range: Range<usize>,
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
}

/// 매퍼 하나를 진단한다.
pub fn diagnose(mapper: &Mapper, index: &Index) -> Vec<Finding> {
    let mut findings = Vec::new();
    for substitution in &mapper.substitutions {
        let name = &substitution.name;
        let (used, bound) = match mapper.dialect {
            Dialect::MyBatis => (format!("${{{name}}}"), format!("#{{{name}}}")),
            Dialect::IBatis => (format!("${name}$"), format!("#{name}#")),
        };
        findings.push(Finding {
            range: substitution.range.clone(),
            severity: Severity::Warning,
            code: "sql-injection",
            message: format!(
                "SQL Injection 위험: {used}는 값을 SQL에 문자열 그대로 넣습니다. {bound} 바인딩을 쓰세요. \
                 정렬 컬럼·테이블 이름처럼 바인딩할 수 없으면 허용 목록으로 검증하세요."
            ),
        });
    }
    let mut seen = std::collections::HashSet::new();
    for def in &mapper.defs {
        if !seen.insert((def.kind, def.id.as_str())) {
            findings.push(Finding {
                range: def.range.clone(),
                severity: Severity::Error,
                code: "duplicate-id",
                message: format!(
                    "id '{}'인 {}이(가) 이 파일에 이미 있습니다. 같은 종류끼리 겹치면 시작할 때 오류가 납니다.",
                    def.id,
                    def.kind.label()
                ),
            });
        }
    }
    for reference in &mapper.refs {
        if !index.resolves(mapper, reference) {
            findings.push(Finding {
                range: reference.range.clone(),
                severity: Severity::Warning,
                code: "unresolved-ref",
                message: format!(
                    "'{}'에 해당하는 {}을(를) 찾지 못했습니다.",
                    reference.name,
                    reference.kind.label()
                ),
            });
        }
    }
    findings.sort_by_key(|f| f.range.start);
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    const MYBATIS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE mapper PUBLIC "-//mybatis.org//DTD Mapper 3.0//EN" "http://mybatis.org/dtd/mybatis-3-mapper.dtd">
<mapper namespace="egov.sample.SampleMapper">
  <!-- 주석 안의 ${commented}는 SQL이 아니다 -->
  <resultMap id="sample" type="egov.sample.SampleVO" extends="base">
    <collection property="items" select="selectItems" resultMap="common.Item"/>
  </resultMap>
  <sql id="where">WHERE use_yn = 'Y'</sql>
  <select id="selectList" resultMap="sample, missingMap">
    SELECT * FROM ${tableName}
    <include refid="where"/>
    <if test="sort != null and ${notSql}">ORDER BY ${sort}</if>
    <![CDATA[ AND title LIKE '%${keyword}%' ]]>
  </select>
  <select id="selectList">SELECT 1</select>
  <select id="selectItems">SELECT #{id}</select>
</mapper>
"#;

    const IBATIS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE sqlMap PUBLIC "-//iBATIS.com//DTD SQL Map 2.0//EN" "http://www.ibatis.com/dtd/sql-map-2.dtd">
<sqlMap namespace="Sample">
  <!-- $searchKeyword$를 #searchKeyword#로 바꿔야 한다 -->
  <select id="sampleDAO.selectList" resultMap="sample">
    SELECT * FROM V$SESSION WHERE name LIKE '%$searchKeyword$%' AND id = #id#
    <isNotEmpty property="list"> AND x IN ($list[]$)</isNotEmpty>
    <include refid="paging"/>
  </select>
</sqlMap>
"#;

    fn at<'t>(text: &'t str, range: &Range<usize>) -> &'t str {
        &text[range.clone()]
    }

    #[test]
    fn mybatis_치환은_본문과_cdata에서만_찾는다() {
        let mapper = parse(MYBATIS).unwrap();
        assert_eq!(mapper.dialect, Dialect::MyBatis);
        assert_eq!(
            mapper.namespace.as_deref(),
            Some("egov.sample.SampleMapper")
        );
        let found: Vec<&str> = mapper
            .substitutions
            .iter()
            .map(|s| at(MYBATIS, &s.range))
            .collect();
        assert_eq!(
            found,
            ["${tableName}", "${sort}", "${keyword}"],
            "주석·속성(test)은 뺀다"
        );
        assert_eq!(mapper.substitutions[2].name, "keyword");
    }

    #[test]
    fn ibatis_치환은_달러로_감싼_이름이고_오라클_뷰_이름은_아니다() {
        let mapper = parse(IBATIS).unwrap();
        assert_eq!(mapper.dialect, Dialect::IBatis);
        let found: Vec<&str> = mapper
            .substitutions
            .iter()
            .map(|s| at(IBATIS, &s.range))
            .collect();
        assert_eq!(
            found,
            ["$searchKeyword$", "$list[]$"],
            "V$SESSION과 주석은 뺀다"
        );
    }

    #[test]
    fn 정의와_참조를_모으고_진단한다() {
        let mapper = parse(MYBATIS).unwrap();
        let refs: Vec<(Kind, &str)> = mapper
            .refs
            .iter()
            .map(|r| (r.kind, r.name.as_str()))
            .collect();
        assert_eq!(
            refs,
            [
                (Kind::ResultMap, "base"),
                (Kind::Statement, "selectItems"),
                (Kind::ResultMap, "common.Item"),
                (Kind::ResultMap, "sample"),
                (Kind::ResultMap, "missingMap"),
                (Kind::Sql, "where"),
            ]
        );
        assert_eq!(
            at(MYBATIS, &mapper.refs[4].range),
            "missingMap",
            "쉼표 뒤 공백을 뺀 범위"
        );

        let mut index = Index::default();
        let common =
            parse(r#"<mapper namespace="common"><resultMap id="Item" type="x"/></mapper>"#)
                .unwrap();
        index.mappers.insert("common.xml".into(), common);
        let findings = diagnose(&mapper, &index);
        let summary: Vec<(&str, &str)> = findings
            .iter()
            .map(|f| (f.code, at(MYBATIS, &f.range)))
            .collect();
        assert_eq!(
            summary,
            [
                ("unresolved-ref", "base"),
                ("unresolved-ref", "missingMap"),
                ("sql-injection", "${tableName}"),
                ("sql-injection", "${sort}"),
                ("sql-injection", "${keyword}"),
                ("duplicate-id", "selectList"),
            ],
            "common.Item은 다른 파일 namespace로 찾는다"
        );
        assert!(findings[5].message.contains("이미 있습니다"));
    }

    #[test]
    fn ibatis는_다른_파일의_id도_namespace_없이_찾는다() {
        let mapper = parse(IBATIS).unwrap();
        let mut index = Index::default();
        assert_eq!(
            diagnose(&mapper, &index)
                .iter()
                .filter(|f| f.code == "unresolved-ref")
                .count(),
            2,
            "sample, paging 모두 없음"
        );
        let other = parse(
            r#"<sqlMap namespace="Common"><sql id="paging">LIMIT 10</sql><resultMap id="sample" class="x"/></sqlMap>"#,
        )
        .unwrap();
        index.mappers.insert("common.xml".into(), other);
        assert!(
            diagnose(&mapper, &index)
                .iter()
                .all(|f| f.code != "unresolved-ref")
        );
    }

    #[test]
    fn 매퍼가_아닌_xml은_무시한다() {
        assert!(parse("<project><modelVersion>4.0.0</modelVersion></project>").is_none());
        assert!(parse("").is_none());
    }
}
