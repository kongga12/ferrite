//! CRUD 코드 생성: 표 정의 → VO·DefaultVO·Mapper 인터페이스·매퍼 XML·Service·ServiceImpl·Controller·
//! JSP(목록·등록)·ServiceImpl 테스트.
//!
//! 템플릿 문맥은 eGovFrame 공식 VS Code 확장(egovframe-vscode-initializr)과 같은 이름을 쓴다
//! (`packageName`, `className`, `attributes[].ccName` 등). 그래서 공식 템플릿이나 직접 고친 템플릿을
//! 프로젝트의 `.ferrite/templates/code/`에 두면 그것으로 만든다. 도우미(`concat`, `setVar`, `eq` 등)도
//! 공식 확장과 같다.

use std::path::{Path, PathBuf};

use ferrite_core::text_file::Encoding;
use handlebars::{
    Context, Handlebars, Helper, HelperResult, JsonRender, Output, RenderContext,
    RenderErrorReason, handlebars_helper,
};
use serde_json::{Map, Value, json};

use crate::ddl::{Column, Table};

/// 실행환경 버전. 패키지 접두사만 다르다(`egovframework.rte` / `org.egovframe.rte`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rte {
    V3,
    V4,
}

impl Rte {
    pub fn prefix(self) -> &'static str {
        match self {
            Rte::V3 => "egovframework.rte",
            Rte::V4 => "org.egovframe.rte",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Rte::V3 => "3.x (egovframework.rte)",
            Rte::V4 => "4.x (org.egovframe.rte)",
        }
    }
}

/// 매퍼 SQL의 대상 DB(페이징·LIKE 문법이 다르다).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Db {
    Oracle,
    Tibero,
    PostgreSql,
    MariaDb,
    Cubrid,
}

impl Db {
    pub const ALL: [Db; 5] = [
        Db::Oracle,
        Db::Tibero,
        Db::PostgreSql,
        Db::MariaDb,
        Db::Cubrid,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Db::Oracle => "Oracle",
            Db::Tibero => "Tibero",
            Db::PostgreSql => "PostgreSQL",
            Db::MariaDb => "MariaDB·MySQL",
            Db::Cubrid => "CUBRID",
        }
    }

    /// 매퍼 파일 이름 끝(`Board_SQL_oracle.xml`).
    pub fn suffix(self) -> &'static str {
        match self {
            Db::Oracle => "oracle",
            Db::Tibero => "tibero",
            Db::PostgreSql => "postgres",
            Db::MariaDb => "mysql",
            Db::Cubrid => "cubrid",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JUnit {
    V4,
    V5,
}

/// 생성할 필드 하나(표의 열). 화면에서 고칠 수 있다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub column: String,
    /// Java 필드 이름(camelCase).
    pub name: String,
    /// Java 타입(패키지 포함, `java.lang.String`, 배열은 `byte[]`).
    pub java_type: String,
    /// DB 타입 이름(`VARCHAR2`). 비어 있으면 Java 타입에서 정한다.
    pub data_type: String,
    pub primary_key: bool,
    pub not_null: bool,
    /// 글자 수 상한(문자열 열의 길이). 화면 입력 검증에 쓴다.
    pub max_length: Option<u32>,
    pub comment: Option<String>,
}

impl Field {
    /// 표의 열에서 필드를 만든다.
    pub fn from_column(column: &Column) -> Self {
        let java_type = java_type(column).to_owned();
        let max_length = (java_type == "java.lang.String")
            .then_some(column.length)
            .flatten();
        Field {
            name: camel_case(&column.name),
            column: column.name.clone(),
            java_type,
            data_type: column.data_type.clone(),
            primary_key: column.primary_key,
            not_null: column.not_null,
            max_length,
            comment: column.comment.clone(),
        }
    }
}

/// 생성 입력 전체.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// DB 표 이름.
    pub table: String,
    pub table_comment: Option<String>,
    pub fields: Vec<Field>,
    /// 기능 패키지(`egovframework.example.board`). 그 아래 `service`·`service.impl`·`web`을 만든다.
    pub package: String,
    /// 클래스 이름 앞부분(`Board` → `BoardVO`, `BoardService`).
    pub class_name: String,
    pub author: String,
    /// `2026-10-25`
    pub date: String,
    pub rte: Rte,
    pub db: Db,
    pub junit: JUnit,
    /// 파일 인코딩. JSP `pageEncoding`·XML 선언에도 쓴다(프로젝트가 MS949면 MS949로 만든다).
    pub encoding: Encoding,
}

impl Spec {
    /// 표 정의로 기본값을 채운다(클래스 이름은 표 이름에서 `TB_` 같은 접두사를 뺀 것).
    pub fn from_table(table: &Table, package: &str, rte: Rte, db: Db, junit: JUnit) -> Self {
        Spec {
            table: table.name.clone(),
            table_comment: table.comment.clone(),
            fields: table.columns.iter().map(Field::from_column).collect(),
            package: package.to_owned(),
            class_name: class_name(&table.name),
            author: String::new(),
            date: String::new(),
            rte,
            db,
            junit,
            encoding: Encoding::Utf8,
        }
    }
}

/// 만든 파일이 놓일 폴더(프로젝트 루트 기준).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub java: PathBuf,
    pub test: PathBuf,
    pub mapper: PathBuf,
    /// 매퍼 XML 파일 이름 모양. `{class}`·`{db}`를 바꾼다. 프로젝트의 `mapperLocations` 패턴에 맞아야
    /// MyBatis가 읽으므로 기존 매퍼 파일 이름을 따른다(`{class}_SQL.xml`, `{class}_SQL_{db}.xml` 등).
    pub mapper_file: String,
    /// JSP 폴더(뷰 리졸버 접두사 자리). 뷰 이름 `{class}/{class}List`가 그 아래 파일이 된다.
    pub jsp: PathBuf,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            java: "src/main/java".into(),
            test: "src/test/java".into(),
            mapper: "src/main/resources/egovframework/mapper".into(),
            mapper_file: "{class}_SQL_{db}.xml".into(),
            jsp: "src/main/webapp/WEB-INF/jsp".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedFile {
    /// 프로젝트 루트 기준 경로.
    pub path: PathBuf,
    pub content: String,
}

/// 만들 파일 종류(템플릿 이름 = 공식 확장의 템플릿 파일 이름과 맞춘 것).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    DefaultVo,
    Vo,
    MapperInterface,
    MapperXml,
    Service,
    ServiceImpl,
    Controller,
    JspList,
    JspRegister,
    Test,
}

impl Part {
    pub const ALL: [Part; 10] = [
        Part::DefaultVo,
        Part::Vo,
        Part::MapperInterface,
        Part::MapperXml,
        Part::Service,
        Part::ServiceImpl,
        Part::Controller,
        Part::JspList,
        Part::JspRegister,
        Part::Test,
    ];

    /// 템플릿 파일 이름(`.ferrite/templates/code/` 아래에서 찾는다).
    pub fn template_name(self) -> &'static str {
        match self {
            Part::DefaultVo => "sample-default-vo-template.hbs",
            Part::Vo => "sample-vo-template.hbs",
            Part::MapperInterface => "sample-mapper-interface-template.hbs",
            Part::MapperXml => "sample-mapper-template.hbs",
            Part::Service => "sample-service-template.hbs",
            Part::ServiceImpl => "sample-service-impl-template.hbs",
            Part::Controller => "sample-controller-template.hbs",
            Part::JspList => "sample-jsp-list.hbs",
            Part::JspRegister => "sample-jsp-register.hbs",
            Part::Test => "sample-service-impl-test-template.hbs",
        }
    }

    fn builtin(self) -> &'static str {
        match self {
            Part::DefaultVo => include_str!("templates/default_vo.hbs"),
            Part::Vo => include_str!("templates/vo.hbs"),
            Part::MapperInterface => include_str!("templates/mapper_interface.hbs"),
            Part::MapperXml => include_str!("templates/mapper.hbs"),
            Part::Service => include_str!("templates/service.hbs"),
            Part::ServiceImpl => include_str!("templates/service_impl.hbs"),
            Part::Controller => include_str!("templates/controller.hbs"),
            Part::JspList => include_str!("templates/jsp_list.hbs"),
            Part::JspRegister => include_str!("templates/jsp_register.hbs"),
            Part::Test => include_str!("templates/service_impl_test.hbs"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Part::DefaultVo => "검색 VO(DefaultVO)",
            Part::Vo => "VO",
            Part::MapperInterface => "Mapper 인터페이스",
            Part::MapperXml => "매퍼 XML",
            Part::Service => "Service",
            Part::ServiceImpl => "ServiceImpl",
            Part::Controller => "Controller",
            Part::JspList => "JSP 목록",
            Part::JspRegister => "JSP 등록·수정",
            Part::Test => "ServiceImpl 테스트",
        }
    }

    /// 만들 파일의 경로(프로젝트 루트 기준).
    pub fn path(self, spec: &Spec, layout: &Layout) -> PathBuf {
        let package = spec.package.replace('.', "/");
        let class = &spec.class_name;
        let lower = first_lower(class);
        match self {
            Part::DefaultVo => layout
                .java
                .join(format!("{package}/service/{class}DefaultVO.java")),
            Part::Vo => layout
                .java
                .join(format!("{package}/service/{class}VO.java")),
            Part::MapperInterface => layout
                .java
                .join(format!("{package}/service/impl/{class}Mapper.java")),
            // 폴더·이름에 `{module}`(기능 폴더, 클래스 이름 소문자)·`{db}`·`{class}`를 넣을 수 있다.
            Part::MapperXml => PathBuf::from(
                layout
                    .mapper
                    .join(&layout.mapper_file)
                    .to_string_lossy()
                    .replace("{module}", &class.to_lowercase())
                    .replace("{class}", class)
                    .replace("{db}", spec.db.suffix()),
            ),
            Part::Service => layout
                .java
                .join(format!("{package}/service/{class}Service.java")),
            Part::ServiceImpl => layout
                .java
                .join(format!("{package}/service/impl/{class}ServiceImpl.java")),
            Part::Controller => layout
                .java
                .join(format!("{package}/web/{class}Controller.java")),
            Part::JspList => layout.jsp.join(format!("{lower}/{lower}List.jsp")),
            Part::JspRegister => layout.jsp.join(format!("{lower}/{lower}Register.jsp")),
            Part::Test => layout.test.join(format!(
                "{package}/service/impl/{class}ServiceImplTest.java"
            )),
        }
    }
}

/// 코드를 만든다. `overrides`(프로젝트의 `.ferrite/templates/code`)에 같은 이름의 템플릿이 있으면 그것을 쓴다.
pub fn generate(
    spec: &Spec,
    layout: &Layout,
    parts: &[Part],
    overrides: Option<&Path>,
) -> Result<Vec<GeneratedFile>, String> {
    validate(spec)?;
    let registry = registry();
    let context = context(spec);
    parts
        .iter()
        .map(|part| {
            let custom = overrides
                .map(|dir| dir.join(part.template_name()))
                .and_then(|file| std::fs::read_to_string(file).ok())
                .map(|text| js_compatible(&text));
            let template = custom.as_deref().unwrap_or(part.builtin());
            let content = registry
                .render_template(template, &context)
                .map_err(|e| format!("{} 템플릿 오류: {e}", part.template_name()))?;
            Ok(GeneratedFile {
                path: part.path(spec, layout),
                content,
            })
        })
        .collect()
}

fn validate(spec: &Spec) -> Result<(), String> {
    let identifier = |s: &str| {
        s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    if !identifier(&spec.class_name) {
        return Err(format!(
            "클래스 이름이 Java 식별자가 아닙니다: {}",
            spec.class_name
        ));
    }
    if spec.package.is_empty() || !spec.package.split('.').all(identifier) {
        return Err(format!("패키지 이름이 올바르지 않습니다: {}", spec.package));
    }
    if spec.fields.is_empty() {
        return Err("필드가 없습니다".into());
    }
    if !spec.fields.iter().any(|f| f.primary_key) {
        return Err("기본 키 필드가 없습니다(수정·삭제·상세 조회에 쓴다)".into());
    }
    for field in &spec.fields {
        if !identifier(&field.name) {
            return Err(format!(
                "필드 이름이 Java 식별자가 아닙니다: {}",
                field.name
            ));
        }
        // 열 이름은 SQL에 그대로 들어간다. 식별자만 받아 SQL 조작을 막는다.
        if !identifier(&field.column) {
            return Err(format!("열 이름이 올바르지 않습니다: {}", field.column));
        }
    }
    if !identifier(&spec.table) {
        return Err(format!("표 이름이 올바르지 않습니다: {}", spec.table));
    }
    Ok(())
}

fn registry() -> Handlebars<'static> {
    let mut registry = Handlebars::new();
    // Java·XML·JSP를 만들므로 HTML 이스케이프를 끈다(공식 확장의 결과와 같게).
    registry.register_escape_fn(handlebars::no_escape);
    registry.set_strict_mode(false);
    handlebars_helper!(eq: |a: Value, b: Value| a == b);
    handlebars_helper!(ne: |a: Value, b: Value| a != b);
    handlebars_helper!(empty: |a: Value| a.is_null() || a.as_str() == Some(""));
    handlebars_helper!(lowercase: |a: Value| a.as_str().map(str::to_lowercase).unwrap_or_default());
    handlebars_helper!(capitalize_helper: |a: str| capitalize(a));
    handlebars_helper!(trim: |a: Value| a.render().trim().to_owned());
    // JS처럼 정수 합은 정수로 쓴다(`{{add @index 1}}` → `1`, `1.0`이 아니라).
    handlebars_helper!(add: |a: Value, b: Value| {
        let sum = number(&a) + number(&b);
        if sum.fract() == 0.0 && sum.abs() < 1e15 { json!(sum as i64) } else { json!(sum) }
    });
    handlebars_helper!(or: |*args| args.iter().any(|v| truthy(v)));
    registry.register_helper("eq", Box::new(eq));
    registry.register_helper("ne", Box::new(ne));
    registry.register_helper("empty", Box::new(empty));
    registry.register_helper("lowercase", Box::new(lowercase));
    registry.register_helper("capitalize", Box::new(capitalize_helper));
    registry.register_helper("trim", Box::new(trim));
    registry.register_helper("add", Box::new(add));
    registry.register_helper("or", Box::new(or));
    registry.register_helper("concat", Box::new(concat_helper));
    registry.register_helper("setVar", Box::new(set_var));
    handlebars_helper!(len: |a: Value| match &a {
        Value::Array(items) => items.len(),
        Value::String(s) => s.chars().count(),
        Value::Object(map) => map.len(),
        _ => 0,
    });
    registry.register_helper("len", Box::new(len));
    registry
}

/// JS Handlebars의 `배열.length`를 handlebars-rust가 읽는 `len` 도우미로 바꾼다(공식 확장 템플릿이 쓴다).
/// `{{x.length}}` → `{{len x}}`, 그 밖의 자리(`{{#if x.length}}`) → `(len x)`.
fn js_compatible(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find("}}").map(|c| open + c) else {
            break;
        };
        let tag = &rest[open + 2..close];
        let mut rewritten = String::new();
        let mut first = true;
        for (i, token) in tag.split(' ').enumerate() {
            if i > 0 {
                rewritten.push(' ');
            }
            let marker = token.trim_start_matches(['~', '#', '^', '(']);
            let lead = &token[..token.len() - marker.len()];
            let trail_start = marker
                .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '@' | '[' | ']')))
                .unwrap_or(marker.len());
            let (path, trail) = marker.split_at(trail_start);
            match path.strip_suffix(".length") {
                Some(base) if !base.is_empty() => {
                    // 태그의 첫 낱말이면 도우미 호출, 아니면 하위 식.
                    if first && lead.is_empty() || first && lead == "~" {
                        rewritten.push_str(&format!("{lead}len {base}{trail}"));
                    } else {
                        rewritten.push_str(&format!("{lead}(len {base}){trail}"));
                    }
                }
                _ => rewritten.push_str(token),
            }
            first = first && token.is_empty();
        }
        out.push_str("{{");
        out.push_str(&rewritten);
        out.push_str("}}");
        rest = &rest[close + 2..];
    }
    out.push_str(rest);
    out
}

/// `{{concat "#{" ccName "}"}}`: 인자를 모두 이어 쓴다.
fn concat_helper(
    h: &Helper<'_>,
    _: &Handlebars<'_>,
    _: &Context,
    _: &mut RenderContext<'_, '_>,
    out: &mut dyn Output,
) -> HelperResult {
    for param in h.params() {
        out.write(&param.value().render())?;
    }
    Ok(())
}

/// `{{setVar "isFirstSet" true}}`: 최상위 문맥에 값을 둔다(공식 확장 템플릿의 `@root.이름`).
fn set_var(
    h: &Helper<'_>,
    _: &Handlebars<'_>,
    ctx: &Context,
    rc: &mut RenderContext<'_, '_>,
    _: &mut dyn Output,
) -> HelperResult {
    let name = h
        .param(0)
        .and_then(|p| p.value().as_str())
        .ok_or_else(|| RenderErrorReason::ParamNotFoundForIndex("setVar", 0))?
        .to_owned();
    let value = h.param(1).map(|p| p.value().clone()).unwrap_or(Value::Null);
    let mut data = rc
        .context()
        .map_or_else(|| ctx.data().clone(), |c| c.data().clone());
    if let Some(root) = data.as_object_mut() {
        root.insert(name, value);
    }
    rc.set_context(Context::wraps(data)?);
    Ok(())
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse().unwrap_or(0.0),
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(_) => true,
    }
}

/// 템플릿 문맥. 공식 확장과 같은 이름 + Ferrite 템플릿이 쓰는 것.
pub fn context(spec: &Spec) -> Value {
    let class = &spec.class_name;
    let lower = first_lower(class);
    let attribute = |field: &Field, index: usize| {
        let simple = simple_type(&field.java_type);
        let mut map = Map::new();
        map.insert("ccName".into(), json!(field.name));
        map.insert("pcName".into(), json!(capitalize(&field.name)));
        map.insert("columnName".into(), json!(field.column));
        map.insert("isPrimaryKey".into(), json!(field.primary_key));
        map.insert("dataType".into(), json!(field.data_type));
        map.insert("javaType".into(), json!(field.java_type));
        map.insert("javaSimpleType".into(), json!(simple));
        map.insert("jdbcType".into(), json!(jdbc_type(field)));
        map.insert(
            "isString".into(),
            json!(field.java_type == "java.lang.String"),
        );
        map.insert("notNull".into(), json!(field.not_null));
        map.insert("maxLength".into(), json!(field.max_length));
        let label = field
            .comment
            .clone()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| field.name.clone());
        map.insert("labelDoc".into(), json!(doc_text(&label)));
        map.insert("labelJava".into(), json!(java_string(&label)));
        map.insert("labelHtml".into(), json!(html_text(&label)));
        map.insert("label".into(), json!(label));
        map.insert("isBinary".into(), json!(field.java_type == "byte[]"));
        map.insert("index".into(), json!(index));
        Value::Object(map)
    };
    let attributes: Vec<Value> = spec
        .fields
        .iter()
        .enumerate()
        .map(|(i, f)| attribute(f, i))
        .collect();
    let filtered = |keep: &dyn Fn(&Field) -> bool| -> Vec<Value> {
        spec.fields
            .iter()
            .enumerate()
            .filter(|(_, f)| keep(f))
            .map(|(i, f)| attribute(f, i))
            .collect()
    };
    let mut imports: Vec<&str> = spec
        .fields
        .iter()
        .map(|f| f.java_type.as_str())
        .filter(|t| t.contains('.') && !t.starts_with("java.lang."))
        .collect();
    imports.sort_unstable();
    imports.dedup();
    let oracle_like = matches!(spec.db, Db::Oracle | Db::Tibero);
    let table_comment = spec
        .table_comment
        .clone()
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| class.clone());
    let (file_encoding, xml_encoding) = match spec.encoding {
        Encoding::Cp949 => ("MS949", "EUC-KR"),
        Encoding::Utf8 | Encoding::Utf8Bom => ("UTF-8", "UTF-8"),
    };
    json!({
        // 공식 확장과 같은 이름.
        "tableName": spec.table,
        "dbTableName": spec.table,
        "attributes": attributes,
        "pkAttributes": filtered(&|f: &Field| f.primary_key),
        "packageName": spec.package,
        "className": class,
        "classNameFirstCharLower": lower,
        "voInstanceName": format!("{lower}VO"),
        "author": spec.author,
        "date": spec.date,
        "version": "1.0",
        "listPath": format!("/{lower}/{lower}List.do"),
        "addViewPath": format!("/{lower}/add{class}View.do"),
        "addPath": format!("/{lower}/add{class}.do"),
        "updateViewPath": format!("/{lower}/update{class}View.do"),
        "updatePath": format!("/{lower}/update{class}.do"),
        "deletePath": format!("/{lower}/delete{class}.do"),
        // Ferrite 템플릿.
        "rte": spec.rte.prefix(),
        "tableComment": table_comment,
        "tableCommentDoc": doc_text(&table_comment),
        "tableCommentHtml": html_text(&table_comment),
        "fileEncoding": file_encoding,
        "xmlEncoding": xml_encoding,
        "listAttributes": filtered(&|f: &Field| f.java_type != "byte[]"),
        "listColumnCount": spec.fields.iter().filter(|f| f.java_type != "byte[]").count(),
        "formAttributes": filtered(&|f: &Field| f.java_type != "byte[]"),
        "nonPkAttributes": filtered(&|f: &Field| !f.primary_key),
        "searchAttributes": filtered(&|f: &Field| !f.primary_key && f.java_type == "java.lang.String"),
        "voImports": imports,
        "oracleLike": oracle_like,
        "limitOffset": spec.db == Db::PostgreSql,
        "limitComma": matches!(spec.db, Db::MariaDb | Db::Cubrid),
        "concatFunction": spec.db == Db::MariaDb,
        "junit5": spec.junit == JUnit::V5,
        "dbLabel": spec.db.label(),
    })
}

/// SQL 타입 → Java 타입(패키지 포함). 공식 확장의 표를 따르되 Oracle·Tibero·CUBRID 타입을 보탰고
/// `NUMBER(p,s)`는 자릿수로 고른다(공식 확장은 언제나 Integer).
pub fn java_type(column: &Column) -> &'static str {
    let ty = column.data_type.as_str();
    match ty {
        "VARCHAR" | "VARCHAR2" | "NVARCHAR" | "NVARCHAR2" | "CHAR" | "NCHAR" | "CHARACTER"
        | "CHARACTER VARYING" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" | "CLOB"
        | "NCLOB" | "LONG" | "STRING" | "ENUM" | "SET" | "UUID" | "JSON" | "JSONB" | "XMLTYPE" => {
            "java.lang.String"
        }
        "INT" | "INTEGER" | "INT4" | "MEDIUMINT" | "SERIAL" => "java.lang.Integer",
        "BIGINT" | "INT8" | "BIGSERIAL" => "java.lang.Long",
        "SMALLINT" | "INT2" | "SMALLSERIAL" => "java.lang.Short",
        "TINYINT" if column.length == Some(1) => "java.lang.Boolean",
        "TINYINT" => "java.lang.Byte",
        "NUMBER" | "NUMERIC" | "DECIMAL" => match (column.length, column.scale) {
            (Some(p), None | Some(0)) if p <= 9 => "java.lang.Integer",
            (Some(p), None | Some(0)) if p <= 18 => "java.lang.Long",
            _ => "java.math.BigDecimal",
        },
        "FLOAT" | "BINARY_FLOAT" => "java.lang.Float",
        "REAL" | "DOUBLE" | "DOUBLE PRECISION" | "BINARY_DOUBLE" | "MONETARY" => "java.lang.Double",
        "DATE" => "java.sql.Date",
        "TIME" => "java.sql.Time",
        "DATETIME" => "java.util.Date",
        "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIMETZ" | "DATETIMELTZ" => "java.sql.Timestamp",
        "BOOLEAN" | "BOOL" | "BIT" => "java.lang.Boolean",
        "BLOB" | "BYTEA" | "RAW" | "LONG RAW" | "BINARY" | "VARBINARY" | "LONGBLOB"
        | "BIT VARYING" => "byte[]",
        // 모르는 타입은 문자열로 받는다(화면 입력·표시가 되도록). 화면에서 바꿀 수 있다.
        _ => "java.lang.String",
    }
}

/// MyBatis `jdbcType`. Oracle·Tibero는 null 값에 jdbcType이 없으면 오류가 나므로 모든 값에 붙인다.
fn jdbc_type(field: &Field) -> &'static str {
    match field.java_type.as_str() {
        "java.lang.String" if matches!(field.data_type.as_str(), "CLOB" | "NCLOB") => "CLOB",
        "java.lang.String" => "VARCHAR",
        "java.lang.Integer" => "INTEGER",
        "java.lang.Long" => "BIGINT",
        "java.lang.Short" => "SMALLINT",
        "java.lang.Byte" => "TINYINT",
        "java.math.BigDecimal" => "NUMERIC",
        "java.lang.Float" => "FLOAT",
        "java.lang.Double" => "DOUBLE",
        "java.sql.Date" => "DATE",
        "java.sql.Time" => "TIME",
        "java.util.Date" | "java.sql.Timestamp" => "TIMESTAMP",
        "java.lang.Boolean" => "BOOLEAN",
        "byte[]" => "BLOB",
        _ => "OTHER",
    }
}

/// Javadoc·주석 안에 넣을 글자(`*/`로 주석이 끝나지 않게, 한 줄로).
fn doc_text(text: &str) -> String {
    one_line(text).replace("*/", "* /")
}

/// Java 문자열 리터럴 안에 넣을 글자.
fn java_string(text: &str) -> String {
    one_line(text).replace('\\', "\\\\").replace('"', "\\\"")
}

/// HTML 본문에 넣을 글자.
fn html_text(text: &str) -> String {
    one_line(text)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn simple_type(java_type: &str) -> &str {
    java_type.rsplit('.').next().unwrap_or(java_type)
}

/// 열 이름 → camelCase. 공식 확장처럼 소문자로 바꾸고 `_x`를 `X`로 잇는다. 밑줄 없는 혼합 대소문자
/// (`boardId`)는 이미 camelCase라 그대로 둔다.
pub fn camel_case(column: &str) -> String {
    let mixed = !column.contains('_')
        && column.chars().any(|c| c.is_ascii_lowercase())
        && column.chars().any(|c| c.is_ascii_uppercase());
    if mixed {
        return first_lower(column);
    }
    let mut out = String::with_capacity(column.len());
    let mut upper_next = false;
    for c in column.to_lowercase().chars() {
        if c == '_' {
            upper_next = !out.is_empty();
            continue;
        }
        if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// 표 이름 → 클래스 이름(`TB_BOARD` → `Board`, `comtn_bbs_master` → `ComtnBbsMaster`).
pub fn class_name(table: &str) -> String {
    let upper = table.to_ascii_uppercase();
    let stripped = ["TB_", "TBL_", "T_"]
        .iter()
        .find_map(|p| upper.starts_with(p).then(|| &table[p.len()..]))
        .unwrap_or(table);
    capitalize(&camel_case(stripped))
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn first_lower(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddl;

    #[test]
    fn 이름과_타입을_정한다() {
        assert_eq!(camel_case("BOARD_ID"), "boardId");
        assert_eq!(camel_case("view_count"), "viewCount");
        assert_eq!(camel_case("boardId"), "boardId", "이미 camelCase");
        assert_eq!(camel_case("ID"), "id");
        assert_eq!(class_name("TB_BOARD"), "Board");
        assert_eq!(class_name("comtn_bbs_master"), "ComtnBbsMaster");
        let column = |ty: &str, length, scale| Column {
            name: "X".into(),
            data_type: ty.into(),
            length,
            scale,
            primary_key: false,
            not_null: false,
            comment: None,
        };
        assert_eq!(
            java_type(&column("NUMBER", Some(10), None)),
            "java.lang.Long"
        );
        assert_eq!(
            java_type(&column("NUMBER", Some(5), None)),
            "java.lang.Integer"
        );
        assert_eq!(
            java_type(&column("NUMBER", Some(12), Some(2))),
            "java.math.BigDecimal"
        );
        assert_eq!(
            java_type(&column("NUMBER", None, None)),
            "java.math.BigDecimal"
        );
        assert_eq!(java_type(&column("CLOB", None, None)), "java.lang.String");
        assert_eq!(
            java_type(&column("TINYINT", Some(1), None)),
            "java.lang.Boolean"
        );
        assert_eq!(java_type(&column("BLOB", None, None)), "byte[]");
    }

    #[test]
    fn 잘못된_입력을_거른다() {
        let table =
            ddl::parse("CREATE TABLE TB_X (ID NUMBER(5) PRIMARY KEY, NM VARCHAR2(10))").unwrap();
        let mut spec = Spec::from_table(&table, "egov.x", Rte::V3, Db::Oracle, JUnit::V4);
        assert!(generate(&spec, &Layout::default(), &[Part::Vo], None).is_ok());
        spec.fields[1].column = "NM; DROP TABLE X".into();
        assert!(
            generate(&spec, &Layout::default(), &[Part::Vo], None).is_err(),
            "열 이름은 식별자만"
        );
        spec.fields[1].column = "NM".into();
        spec.fields[0].primary_key = false;
        assert!(
            generate(&spec, &Layout::default(), &[Part::Vo], None).is_err(),
            "기본 키 필요"
        );
    }

    #[test]
    fn 공식_확장의_도우미가_동작한다() {
        let registry = registry();
        let render =
            |template: &str, data: Value| registry.render_template(template, &data).unwrap();
        assert_eq!(
            render("{{concat \"#{\" name \"}\"}}", json!({"name": "id"})),
            "#{id}"
        );
        assert_eq!(
            render(
                "{{~setVar \"first\" true~}}{{#each items}}{{#if @root.first}}{{this}}{{~setVar \"first\" false~}}{{else}}, {{this}}{{/if}}{{/each}}",
                json!({"items": ["a", "b", "c"]})
            ),
            "a, b, c"
        );
        assert_eq!(render("{{add 1 2}}", json!({})), "3");
        assert_eq!(
            render("{{#if (eq a \"x\")}}y{{/if}}", json!({"a": "x"})),
            "y"
        );
        assert_eq!(
            render(
                "{{capitalize a}} {{lowercase b}}",
                json!({"a": "ab", "b": "CD"})
            ),
            "Ab cd"
        );
        assert_eq!(
            render("{{#if (or a b)}}t{{/if}}", json!({"a": false, "b": 1})),
            "t"
        );
        assert_eq!(
            render("<{{a}}>", json!({"a": "<&>"})),
            "<<&>>",
            "이스케이프하지 않는다"
        );
        let js = |template: &str, data: Value| render(&js_compatible(template), data);
        assert_eq!(
            js(
                "{{#if items.length}}y{{else}}n{{/if}}",
                json!({"items": [1]})
            ),
            "y"
        );
        assert_eq!(
            js(
                "{{#if items.length}}y{{else}}n{{/if}}",
                json!({"items": []})
            ),
            "n"
        );
        assert_eq!(js("{{items.length}}개", json!({"items": [1, 2]})), "2개");
        assert_eq!(js("{{~a.length~}}", json!({"a": "abc"})), "3");
    }
}
