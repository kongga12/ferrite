//! egov 언어 서버(LSP, 표준 입출력). 비동기 런타임 없이 메시지를 하나씩 처리한다.
//! 시작할 때 프로젝트의 매퍼와 Java 인터페이스를 모아 색인하고, 열린 문서는 편집 중인 내용으로
//! 덮어쓴다.
//!
//! 위치는 UTF-16으로 주고받는다. 이 서버는 다른 서버(jdt.ls, lemminx)와 함께 한 문서를 맡아
//! 결과가 합쳐지므로, 그 서버들과 같은 단위를 써야 위치가 어긋나지 않는다.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use ferrite_core::text_file;
use ferrite_lsp::position::{PositionEncoding, from_lsp, to_lsp};
use ferrite_lsp::transport::{read_message, write_message};
use ferrite_lsp::uri::{path_to_uri, uri_to_path};
use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Uri};
use ropey::Rope;
use serde_json::{Value, json};

use crate::java::{self, JavaFile, Target, TypeDecl};
use crate::jpa::{self, Word};
use crate::jsp;
use crate::mapper::{self, Aliases, Context, Dialect, Finding, Index, Kind, Severity};
use crate::spring::{self, Config};
use crate::taglib::{self, Library, Site, Tld};
use crate::web;

const ENCODING: PositionEncoding = PositionEncoding::Utf16;
/// 클래스 이름 자동완성에서 한 번에 돌려줄 최대 개수. 넘으면 목록이 미완성이라고 알린다.
const MAX_CLASS_ITEMS: usize = 200;
/// 필드가 아니라 이름으로 값을 꺼내는 타입(`#{아무이름}`). 속성을 완성할 수 없다.
const MAP_TYPES: [&str; 6] = [
    "map",
    "hashmap",
    "java.util.map",
    "java.util.hashmap",
    "egovmap",
    "egovframework.rte.psl.dataaccess.util.egovmap",
];
/// MyBatis 기본 별칭 중 자주 쓰는 것(클래스 이름 자동완성에 함께 보인다).
const BUILTIN_ALIASES: [&str; 10] = [
    "string",
    "int",
    "long",
    "double",
    "boolean",
    "date",
    "map",
    "hashmap",
    "list",
    "arraylist",
];

/// taglib 선언을 따라 읽는 정적 include의 깊이(공통 taglib 파일이 또 다른 파일을 include하는 경우).
const MAX_INCLUDE_DEPTH: usize = 3;

/// 색인할 때 건너뛰는 폴더(빌드 결과물·도구 폴더). 빌드 결과물에는 같은 매퍼의 사본이 있다.
const SKIP_DIRS: [&str; 10] = [
    ".git",
    ".gradle",
    ".idea",
    ".settings",
    ".vscode",
    "bin",
    "build",
    "node_modules",
    "out",
    "target",
];

/// 표준 입출력으로 LSP 서버를 돌린다. `exit` 알림이나 입력이 끝나면 돌아온다.
pub fn run_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run(&mut stdin.lock(), &mut stdout.lock())
}

pub fn run(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<()> {
    let mut server = Server::default();
    while let Some(body) = read_message(input)? {
        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if method == "exit" {
            break;
        }
        let replies = match message.get("id") {
            Some(id) => vec![server.request(id.clone(), method, &params)],
            None => server.notification(method, &params),
        };
        for reply in replies {
            write_message(output, &serde_json::to_vec(&reply)?)?;
        }
        output.flush()?;
    }
    Ok(())
}

fn is_java(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("java"))
}

fn is_jsp(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["jsp", "jspf", "tag"]
            .iter()
            .any(|j| e.eq_ignore_ascii_case(j))
    })
}

#[derive(Default)]
struct Server {
    root: Option<PathBuf>,
    index: Index,
    /// Java 파일의 타입(Mapper 인터페이스, VO 클래스).
    java: HashMap<PathBuf, JavaFile>,
    /// XML 파일의 타입 별칭(iBatis sqlMap, MyBatis 설정).
    aliases: HashMap<PathBuf, Aliases>,
    /// 열린 문서(편집 중인 내용).
    open: HashMap<PathBuf, Rope>,
    /// 진단을 보낸 문서. 더 이상 대상이 아니게 되면 빈 진단으로 지운다.
    published: HashSet<PathBuf>,
    /// 웹 루트(`WEB-INF`가 있는 폴더).
    web_roots: Vec<PathBuf>,
    /// 뷰 리졸버의 (접두사, 접미사).
    views: Vec<(String, String)>,
    /// 태그 라이브러리. `taglib_uris`·`taglib_files`는 이 목록의 번호다.
    libraries: Vec<Library>,
    taglib_uris: HashMap<String, usize>,
    taglib_files: HashMap<PathBuf, usize>,
    /// `web.xml`의 taglib-uri → taglib-location.
    taglib_locations: HashMap<String, String>,
    /// Spring XML 설정.
    spring: HashMap<PathBuf, Config>,
    /// `.properties` 키 → 그 키를 둔 (파일, 범위). 프로필마다 같은 키가 여러 파일에 있다.
    property_keys: HashMap<String, Vec<(PathBuf, Range<usize>)>>,
    /// `classpath:` 경로를 찾는 폴더(`src/main/resources`, `WEB-INF/classes`).
    resource_roots: Vec<PathBuf>,
    /// JPA IDENTITY 진단에 쓰는 DB 단서.
    database: jpa::Database,
}

impl Server {
    fn request(&mut self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "initialize" => {
                self.root = initialize_root(params);
                json!({ "jsonrpc": "2.0", "id": id, "result": {
                    "capabilities": {
                        "positionEncoding": "utf-16",
                        "textDocumentSync": { "openClose": true, "change": 1 },
                        "definitionProvider": true,
                        "completionProvider": { "triggerCharacters": ["{", "#", "\"", "."] },
                    },
                    "serverInfo": { "name": "ferrite-egov", "version": env!("CARGO_PKG_VERSION") },
                } })
            }
            "textDocument/definition" => {
                json!({ "jsonrpc": "2.0", "id": id, "result": self.definition(params) })
            }
            "textDocument/completion" => {
                json!({ "jsonrpc": "2.0", "id": id, "result": self.completion(params) })
            }
            "shutdown" => json!({ "jsonrpc": "2.0", "id": id, "result": null }),
            _ => json!({ "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("지원하지 않는 요청: {method}") } }),
        }
    }

    fn notification(&mut self, method: &str, params: &Value) -> Vec<Value> {
        match method {
            "initialized" => {
                if let Some(root) = self.root.clone() {
                    self.index_project(&root);
                }
                Vec::new()
            }
            "textDocument/didOpen" => {
                let text = params.pointer("/textDocument/text").and_then(Value::as_str);
                self.update(document_path(params), text)
            }
            // 전체 내용 동기화만 쓴다(textDocumentSync.change = Full).
            "textDocument/didChange" => {
                let text = params
                    .pointer("/contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text"))
                    .and_then(Value::as_str);
                self.update(document_path(params), text)
            }
            "textDocument/didClose" => {
                let Some(path) = document_path(params) else {
                    return Vec::new();
                };
                self.open.remove(&path);
                // 닫으면 디스크 내용이 다시 기준이다.
                let text = read_text(&path);
                self.reindex(&path, text.as_deref());
                let mut replies = self.publish_all();
                if self.published.remove(&path) {
                    replies.extend(publish(&path, Vec::new()));
                }
                replies
            }
            _ => Vec::new(),
        }
    }

    /// 열린 문서의 내용을 바꾸고 열린 문서 모두를 다시 진단한다(서로 참조할 수 있어서).
    fn update(&mut self, path: Option<PathBuf>, text: Option<&str>) -> Vec<Value> {
        let (Some(path), Some(text)) = (path, text) else {
            return Vec::new();
        };
        self.reindex(&path, Some(text));
        self.open.insert(path, Rope::from_str(text));
        self.publish_all()
    }

    /// 파일 하나의 색인을 이 내용으로 바꾼다(None이면 뺀다).
    fn reindex(&mut self, path: &Path, text: Option<&str>) {
        if is_jsp(path) {
            // JSP는 색인하지 않는다(열린 내용만 진단한다).
        } else if is_java(path) {
            match text.map(java::parse_file).filter(|f| !f.types.is_empty()) {
                Some(found) => self.java.insert(path.to_path_buf(), found),
                None => self.java.remove(path),
            };
        } else {
            match text.and_then(mapper::parse) {
                Some(found) => self.index.mappers.insert(path.to_path_buf(), found),
                None => self.index.mappers.remove(path),
            };
            match text.and_then(spring::parse) {
                Some(found) => self.spring.insert(path.to_path_buf(), found),
                None => self.spring.remove(path),
            };
            let aliases = text.map(mapper::aliases).unwrap_or_default();
            if aliases == Aliases::default() {
                self.aliases.remove(path);
            } else {
                self.aliases.insert(path.to_path_buf(), aliases);
            }
        }
    }

    fn publish_all(&mut self) -> Vec<Value> {
        let mut replies = Vec::new();
        for (path, rope) in &self.open {
            let findings = if is_jsp(path) {
                Some(jsp::diagnose(&rope.to_string()))
            } else if is_java(path) {
                let mut findings = self
                    .java
                    .get(path)
                    .and_then(|file| missing_statements(&file.types, &self.index));
                // IDENTITY를 못 쓰는 DB면 `@GeneratedValue(IDENTITY)`를 알린다.
                if let Some((severity, message)) = jpa::identity_problem(&self.database) {
                    let identity: Vec<Finding> = java::identity_generators(&rope.to_string())
                        .into_iter()
                        .map(|range| Finding {
                            range,
                            severity,
                            code: "jpa-identity",
                            message: message.clone(),
                        })
                        .collect();
                    if !identity.is_empty() || findings.is_some() {
                        findings.get_or_insert_with(Vec::new).extend(identity);
                    }
                }
                findings
            } else if let Some(mapper) = self.index.mappers.get(path) {
                Some(mapper::diagnose(mapper, &self.index))
            } else {
                self.spring
                    .get(path)
                    .map(|config| self.spring_findings(config))
            };
            let Some(findings) = findings else {
                if self.published.remove(path) {
                    replies.extend(publish(path, Vec::new()));
                }
                continue;
            };
            let diagnostics = findings
                .into_iter()
                .map(|finding| Diagnostic {
                    range: lsp_range(rope, &finding.range),
                    severity: Some(match finding.severity {
                        Severity::Error => DiagnosticSeverity::ERROR,
                        Severity::Warning => DiagnosticSeverity::WARNING,
                        Severity::Information => DiagnosticSeverity::INFORMATION,
                    }),
                    code: Some(NumberOrString::String(finding.code.into())),
                    source: Some("ferrite-egov".into()),
                    message: finding.message,
                    ..Default::default()
                })
                .collect();
            self.published.insert(path.clone());
            replies.extend(publish(path, diagnostics));
        }
        replies
    }

    /// 정의로 이동. 매퍼 XML과 Java 사이를 잇는다(같은 파일·다른 파일).
    fn definition(&self, params: &Value) -> Value {
        let Some(path) = document_path(params) else {
            return Value::Null;
        };
        let (Some(rope), Ok(position)) = (
            self.open.get(&path),
            serde_json::from_value::<Position>(params["position"].clone()),
        ) else {
            return Value::Null;
        };
        let offset = rope.char_to_byte(from_lsp(rope, position, ENCODING));
        let targets = if is_jsp(&path) {
            self.jsp_targets(&path, &rope.to_string(), offset)
        } else if is_java(&path) {
            self.java_targets(&rope.to_string(), offset)
        } else {
            self.xml_targets(&path, offset)
        };
        // 대상 파일마다 한 번만 읽는다(열려 있으면 편집 중인 내용).
        let mut ropes: HashMap<PathBuf, Rope> = HashMap::new();
        let mut locations = Vec::new();
        for (target, range) in targets {
            let rope = ropes.entry(target.clone()).or_insert_with(|| {
                self.open
                    .get(&target)
                    .cloned()
                    .unwrap_or_else(|| Rope::from_str(&read_text(&target).unwrap_or_default()))
            });
            if let Ok(uri) = path_to_uri(&target) {
                let location = json!({ "uri": uri, "range": lsp_range(rope, &range) });
                if !locations.contains(&location) {
                    locations.push(location);
                }
            }
        }
        if locations.is_empty() {
            Value::Null
        } else {
            Value::Array(locations)
        }
    }

    /// XML: 참조(refid·resultMap·select) → 정의, MyBatis 문장 id → Mapper 인터페이스 메서드.
    /// Spring XML이면 빈 참조·class·property·플레이스홀더·import.
    fn xml_targets(&self, path: &Path, offset: usize) -> Vec<(PathBuf, Range<usize>)> {
        let Some(mapper) = self.index.mappers.get(path) else {
            return self
                .spring
                .get(path)
                .map(|config| self.spring_targets(path, config, offset))
                .unwrap_or_default();
        };
        let within = |range: &Range<usize>| range.start <= offset && offset <= range.end;
        if let Some(reference) = mapper.refs.iter().find(|r| within(&r.range)) {
            return self
                .index
                .targets(mapper, reference)
                .into_iter()
                .map(|(found, def)| {
                    (
                        found.cloned().unwrap_or_else(|| path.to_path_buf()),
                        def.range.clone(),
                    )
                })
                .collect();
        }
        let statement = mapper
            .defs
            .iter()
            .find(|d| d.kind == Kind::Statement && within(&d.range));
        match (statement, &mapper.namespace, mapper.dialect) {
            (Some(def), Some(namespace), Dialect::MyBatis) => self
                .java
                .iter()
                .flat_map(|(file, java)| {
                    java.types
                        .iter()
                        .filter(|t| t.is_interface && &t.fqn == namespace)
                        .flat_map(|t| t.methods.iter().filter(|m| m.name == def.id))
                        .map(|m| (file.clone(), m.range.clone()))
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// JSP: include 경로 → 그 파일.
    fn jsp_targets(&self, path: &Path, text: &str, offset: usize) -> Vec<(PathBuf, Range<usize>)> {
        let root = web::web_root_for(&self.web_roots, path).map(PathBuf::as_path);
        web::include_at(text, offset)
            .and_then(|include| web::resolve(include, path, root))
            .map(|target| vec![(target, 0..0)])
            .unwrap_or_default()
    }

    /// Java: Mapper 메서드(선언·호출)나 문자열 문장 id → 매퍼 XML 문장, 문자열 뷰 이름 → JSP.
    fn java_targets(&self, text: &str, offset: usize) -> Vec<(PathBuf, Range<usize>)> {
        let target = java::target_at(text, offset);
        // 컨트롤러가 돌려주는 뷰 이름(`return "sample/list";`, `new ModelAndView("…")`). 파일이
        // 있을 때만 대상이라 다른 문자열에 잘못 붙지 않는다.
        let views = match &target {
            Some(Target::StatementId(name)) => web::view_files(name, &self.views, &self.web_roots),
            _ => Vec::new(),
        };
        let found = match target {
            Some(Target::Method { namespace, name }) => self.index.find(
                Kind::Statement,
                |m| m.namespace.as_deref() == Some(namespace.as_str()),
                &name,
            ),
            // DAO 방식: "namespace.id"(MyBatis·iBatis), iBatis는 id 그대로도 쓴다.
            Some(Target::StatementId(id)) => {
                let mut found = self.index.qualified(Kind::Statement, &id);
                found.extend(self.index.find(
                    Kind::Statement,
                    |m| m.dialect == Dialect::IBatis,
                    &id,
                ));
                found
            }
            None => Vec::new(),
        };
        found
            .into_iter()
            .filter_map(|(file, def)| Some((file?.clone(), def.range.clone())))
            .chain(views.into_iter().map(|view| (view, 0..0)))
            .collect()
    }

    /// 자동완성: `#{}` 안의 속성(VO 필드·`@Param` 이름), 클래스 이름을 받는 속성값,
    /// resultMap의 `property`.
    fn completion(&self, params: &Value) -> Value {
        let Some(path) = document_path(params) else {
            return Value::Null;
        };
        let (Some(rope), Ok(position)) = (
            self.open.get(&path),
            serde_json::from_value::<Position>(params["position"].clone()),
        ) else {
            return Value::Null;
        };
        let text = rope.to_string();
        let offset = rope.char_to_byte(from_lsp(rope, position, ENCODING));
        if is_jsp(&path) {
            return match self.jsp_items(&path, rope, &text, offset) {
                Some(items) => json!({ "isIncomplete": false, "items": items }),
                None => Value::Null,
            };
        }
        if is_java(&path) {
            return match self.jpql_items(rope, &text, offset) {
                Some(items) => json!({ "isIncomplete": false, "items": items }),
                None => Value::Null,
            };
        }
        let Some(mapper) = self.index.mappers.get(&path) else {
            if !self.spring.contains_key(&path) {
                return Value::Null;
            }
            return match self.spring_items(rope, &text, offset) {
                Some((items, incomplete)) => json!({ "isIncomplete": incomplete, "items": items }),
                None => Value::Null,
            };
        };
        let (items, incomplete) = match mapper::context_at(&text, mapper.dialect, offset) {
            Some(Context::Parameter {
                parameter_type,
                statement_id,
                path,
            }) => (
                self.parameter_items(
                    mapper,
                    parameter_type.as_deref(),
                    statement_id.as_deref(),
                    &path,
                ),
                false,
            ),
            Some(Context::Property { owner }) => {
                let fields = owner
                    .and_then(|owner| self.class(&self.resolve_alias(&owner)))
                    .map(|(file, decl)| self.fields(file, decl))
                    .unwrap_or_default();
                (field_items(&fields), false)
            }
            Some(Context::ClassName { replace, typed }) => {
                self.class_items(rope, replace, &typed, true)
            }
            None => return Value::Null,
        };
        json!({ "isIncomplete": incomplete, "items": items })
    }

    /// JSP: 태그 라이브러리의 태그 이름, 사용자 정의 태그의 속성 이름, EL 함수.
    fn jsp_items(&self, path: &Path, rope: &Rope, text: &str, offset: usize) -> Option<Vec<Value>> {
        let site = taglib::site_at(text, offset)?;
        let libraries = self.taglibs_for(path, text);
        let library = |prefix: &str| {
            libraries
                .iter()
                .find(|(p, _)| p == prefix)
                .map(|(_, library)| *library)
        };
        let edit = |label: String,
                    filter: &str,
                    detail: &str,
                    kind: u8,
                    range: &Range<usize>,
                    new_text: &str| {
            json!({
                "label": label,
                "kind": kind,
                "detail": detail,
                "filterText": filter,
                "textEdit": { "range": lsp_range(rope, range), "newText": new_text },
            })
        };
        let items = match site {
            // 접두사를 치는 중: 선언된 모든 라이브러리의 `접두사:태그`.
            Site::Tag {
                prefix: None,
                replace,
            } => libraries
                .iter()
                .flat_map(|(prefix, library)| library.tags.iter().map(move |t| (prefix, t)))
                .map(|(prefix, tag)| {
                    let label = format!("{prefix}:{}", tag.name);
                    edit(label.clone(), &label, &tag.description, 1, &replace, &label)
                })
                .collect(),
            Site::Tag {
                prefix: Some(prefix),
                replace,
            } => library(&prefix)?
                .tags
                .iter()
                .map(|tag| {
                    edit(
                        format!("{prefix}:{}", tag.name),
                        &tag.name,
                        &tag.description,
                        1,
                        &replace,
                        &tag.name,
                    )
                })
                .collect(),
            Site::Attribute {
                prefix,
                tag,
                written,
                replace,
            } => {
                let tag = library(&prefix)?.tags.iter().find(|t| t.name == tag)?;
                let written = &text[written];
                tag.attributes
                    .iter()
                    .filter(|a| jsp::attribute(written, &a.name).is_none())
                    .map(|a| {
                        let mut item = edit(
                            a.name.clone(),
                            &a.name,
                            if a.required { "필수" } else { "" },
                            1,
                            &replace,
                            &a.name,
                        );
                        // 필수 속성을 먼저 보여 준다.
                        item["sortText"] = json!(format!("{}{}", u8::from(!a.required), a.name));
                        item
                    })
                    .collect()
            }
            Site::Function { prefix, replace } => library(&prefix)?
                .functions
                .iter()
                .map(|f| edit(f.name.clone(), &f.name, &f.signature, 3, &replace, &f.name))
                .collect(),
        };
        Some(items)
    }

    /// 이 JSP에서 쓸 수 있는 (접두사, 라이브러리). 정적 include로 읽는 공통 파일의 선언도 따라간다.
    fn taglibs_for(&self, path: &Path, text: &str) -> Vec<(String, &Library)> {
        let root = web::web_root_for(&self.web_roots, path).map(PathBuf::as_path);
        let mut declared = taglib::declared(text);
        let mut pending: Vec<(PathBuf, usize)> = web::static_includes(text)
            .into_iter()
            .filter_map(|include| web::resolve(include, path, root))
            .map(|file| (file, 1))
            .collect();
        let mut seen = HashSet::new();
        while let Some((file, depth)) = pending.pop() {
            if !seen.insert(file.clone()) {
                continue;
            }
            let Some(included) = self
                .open
                .get(&file)
                .map(Rope::to_string)
                .or_else(|| read_text(&file))
            else {
                continue;
            };
            declared.extend(taglib::declared(&included));
            if depth < MAX_INCLUDE_DEPTH {
                pending.extend(
                    web::static_includes(&included)
                        .into_iter()
                        .filter_map(|include| web::resolve(include, &file, root))
                        .map(|next| (next, depth + 1)),
                );
            }
        }
        let mut found: Vec<(String, &Library)> = Vec::new();
        for (prefix, uri) in declared {
            if found.iter().any(|(p, _)| *p == prefix) {
                continue;
            }
            if let Some(library) = self.library(&uri, path, root) {
                found.push((prefix, library));
            }
        }
        found
    }

    /// taglib uri의 라이브러리: TLD가 밝힌 uri, `web.xml` 매핑, 또는 `.tld` 경로 그대로.
    fn library(&self, uri: &str, path: &Path, root: Option<&Path>) -> Option<&Library> {
        if let Some(&i) = self.taglib_uris.get(uri) {
            return self.libraries.get(i);
        }
        let location = self.taglib_locations.get(uri).map_or(uri, String::as_str);
        let file = web::resolve(location, path, root)?;
        self.libraries.get(*self.taglib_files.get(&file)?)
    }

    /// 라이브러리를 넣는다. 같은 uri가 이미 있으면 먼저 넣은 것(프로젝트 것)을 쓴다.
    fn add_library(&mut self, file: Option<PathBuf>, tld: Tld) {
        let i = self.libraries.len();
        self.libraries.push(tld.library);
        if let Some(uri) = tld.uri {
            self.taglib_uris.entry(uri).or_insert(i);
        }
        if let Some(file) = file {
            self.taglib_files.insert(file, i);
        }
    }

    /// `#{}` 안 속성. 타입은 문장의 parameterType(별칭 풀기), 없으면 namespace 인터페이스 메서드의
    /// 매개변수에서 찾는다. `path`(`user.` 같은 앞 조각)를 따라 필드의 타입으로 내려간다.
    fn parameter_items(
        &self,
        mapper: &mapper::Mapper,
        parameter_type: Option<&str>,
        statement_id: Option<&str>,
        path: &[String],
    ) -> Vec<Value> {
        let mut current: Option<(&JavaFile, &TypeDecl)> = None;
        let mut rest = path;
        if let Some(ty) = parameter_type {
            if is_map(ty) {
                return Vec::new();
            }
            current = self.class(&self.resolve_alias(ty));
        } else if let (Some(namespace), Some(id)) = (&mapper.namespace, statement_id) {
            let method = self.java.values().find_map(|file| {
                file.types
                    .iter()
                    .filter(|t| t.is_interface && &t.fqn == namespace)
                    .flat_map(|t| t.methods.iter())
                    .find(|m| m.name == id)
                    .map(|m| (file, m))
            });
            let Some((file, method)) = method else {
                return Vec::new();
            };
            let bound: Vec<&java::Param> =
                method.params.iter().filter(|p| p.bound.is_some()).collect();
            if !bound.is_empty() {
                // `@Param`이 있으면 그 이름이 첫 조각이다.
                let Some((first, more)) = path.split_first() else {
                    return bound
                        .iter()
                        .map(|p| item(p.bound.as_deref().unwrap_or_default(), &p.ty, 6))
                        .collect();
                };
                let param = bound
                    .iter()
                    .find(|p| p.bound.as_deref() == Some(first.as_str()));
                current = param.and_then(|p| self.class(&file.resolve(&p.ty)));
                rest = more;
            } else if let [param] = method.params.as_slice() {
                current = self.class(&file.resolve(&param.ty));
            }
        }
        for segment in rest {
            let Some((file, decl)) = current else {
                return Vec::new();
            };
            let field = self
                .fields(file, decl)
                .into_iter()
                .find(|(f, _)| &f.name == segment);
            current = field.and_then(|(f, owner)| self.class(&owner.resolve(&f.ty)));
        }
        current
            .map(|(file, decl)| field_items(&self.fields(file, decl)))
            .unwrap_or_default()
    }

    /// 클래스 이름 후보: 프로젝트 클래스(단순 이름이나 FQN이 친 값으로 시작), 별칭, MyBatis 기본 별칭.
    /// 고르면 속성값 전체를 FQN(별칭이면 별칭)으로 바꾼다.
    /// `mybatis`면 MyBatis 별칭(typeAlias·기본 별칭)도 함께 보여 준다.
    fn class_items(
        &self,
        rope: &Rope,
        replace: Range<usize>,
        typed: &str,
        mybatis: bool,
    ) -> (Vec<Value>, bool) {
        let range = lsp_range(rope, &replace);
        let typed = typed.to_lowercase();
        let simple_typed = typed.rsplit('.').next().unwrap_or(&typed);
        let edit = |label: &str, detail: &str, kind: u8, new_text: &str| {
            json!({
                "label": label,
                "kind": kind,
                "detail": detail,
                "filterText": label,
                "textEdit": { "range": range, "newText": new_text },
            })
        };
        let mut items = Vec::new();
        let mut classes: Vec<&str> = self
            .java
            .values()
            .flat_map(|f| f.types.iter().map(|t| t.fqn.as_str()))
            .filter(|fqn| {
                let lower = fqn.to_lowercase();
                let simple = lower.rsplit('.').next().unwrap_or(&lower);
                simple.starts_with(simple_typed) || lower.starts_with(&typed)
            })
            .collect();
        classes.sort_unstable();
        classes.dedup();
        let incomplete = classes.len() > MAX_CLASS_ITEMS;
        for fqn in classes.into_iter().take(MAX_CLASS_ITEMS) {
            let (package, simple) = fqn.rsplit_once('.').unwrap_or(("", fqn));
            items.push(edit(simple, package, 7, fqn));
        }
        if !mybatis {
            return (items, incomplete);
        }
        for aliases in self.aliases.values() {
            for (alias, ty) in &aliases.explicit {
                if alias.to_lowercase().starts_with(simple_typed) {
                    items.push(edit(alias, &format!("별칭 → {ty}"), 18, alias));
                }
            }
        }
        for alias in BUILTIN_ALIASES
            .iter()
            .filter(|a| a.starts_with(simple_typed))
        {
            items.push(edit(alias, "MyBatis 기본 별칭", 18, alias));
        }
        (items, incomplete)
    }

    fn resolve_alias(&self, name: &str) -> String {
        let aliases: Vec<&Aliases> = self.aliases.values().collect();
        let classes: Vec<&str> = self
            .java
            .values()
            .flat_map(|f| f.types.iter().map(|t| t.fqn.as_str()))
            .collect();
        mapper::resolve_alias(name, &aliases, &classes)
    }

    fn class(&self, fqn: &str) -> Option<(&JavaFile, &TypeDecl)> {
        self.java
            .values()
            .find_map(|file| file.types.iter().find(|t| t.fqn == fqn).map(|t| (file, t)))
    }

    /// 프로젝트의 모든 빈 이름(Spring XML의 id·name·alias, `@Component` 계열·`@Bean`).
    fn bean_names(&self) -> HashSet<&str> {
        self.spring
            .values()
            .flat_map(|c| c.defs.iter().map(|d| d.name.as_str()))
            .chain(
                self.java
                    .values()
                    .flat_map(|f| f.types.iter())
                    .flat_map(|t| t.beans.iter().map(|(name, _)| name.as_str())),
            )
            .collect()
    }

    /// 이 빈 이름의 정의(XML 빈, Java 클래스·`@Bean` 메서드).
    fn bean_targets(&self, name: &str) -> Vec<(PathBuf, Range<usize>)> {
        let xml = self.spring.iter().flat_map(|(file, config)| {
            config
                .defs
                .iter()
                .filter(|d| d.name == name)
                .map(|d| (file.clone(), d.range.clone()))
        });
        let java = self.java.iter().flat_map(|(file, java)| {
            java.types
                .iter()
                .flat_map(|t| t.beans.iter())
                .filter(|(bean, _)| bean == name)
                .map(|(_, range)| (file.clone(), range.clone()))
        });
        xml.chain(java).collect()
    }

    /// 클래스와 상위 클래스들(프로젝트 안에서 찾은 것만). 끝까지 프로젝트 안이면 true도 함께 준다
    /// (상위 클래스가 라이브러리면 그쪽 property를 몰라 검증하지 않는다).
    fn hierarchy(&self, fqn: &str) -> (Vec<(&JavaFile, &TypeDecl)>, bool) {
        let mut found = Vec::new();
        let mut current = self.class(fqn);
        let mut seen = HashSet::new();
        while let Some((file, decl)) = current {
            if !seen.insert(decl.fqn.as_str()) {
                break;
            }
            found.push((file, decl));
            let Some(parent) = &decl.superclass else {
                return (found, true);
            };
            current = self.class(&file.resolve(parent));
        }
        let complete = !found.is_empty() && current.is_some();
        (found, complete)
    }

    /// 클래스의 property(setter `setX`가 있거나 같은 이름의 필드): (이름, 타입, 파일, 범위).
    fn bean_properties(&self, fqn: &str) -> Vec<(String, String, PathBuf, Range<usize>)> {
        let mut found: Vec<(String, String, PathBuf, Range<usize>)> = Vec::new();
        let (hierarchy, _) = self.hierarchy(fqn);
        let file_of = |decl: &TypeDecl| {
            self.java
                .iter()
                .find(|(_, f)| f.types.iter().any(|t| t.fqn == decl.fqn))
                .map(|(path, _)| path.clone())
                .unwrap_or_default()
        };
        for (_, decl) in hierarchy {
            let path = file_of(decl);
            for method in &decl.methods {
                if let Some(rest) = method.name.strip_prefix("set")
                    && method.params.len() == 1
                    && let Some(first) = rest.chars().next()
                {
                    let name: String = first.to_lowercase().chain(rest.chars().skip(1)).collect();
                    if !found.iter().any(|(n, ..)| *n == name) {
                        found.push((
                            name,
                            method.params[0].ty.clone(),
                            path.clone(),
                            method.range.clone(),
                        ));
                    }
                }
            }
            for field in &decl.fields {
                if !found.iter().any(|(n, ..)| *n == field.name) {
                    found.push((
                        field.name.clone(),
                        field.ty.clone(),
                        path.clone(),
                        field.range.clone(),
                    ));
                }
            }
        }
        found
    }

    /// Spring XML 진단: 없는 빈 참조, 프로젝트 패키지인데 없는 클래스, 프로젝트 클래스에 없는 property.
    fn spring_findings(&self, config: &Config) -> Vec<Finding> {
        let beans = self.bean_names();
        let packages: HashSet<&str> = self
            .java
            .values()
            .filter_map(|f| f.package.as_deref())
            .collect();
        let mut findings = Vec::new();
        for usage in &config.refs {
            if !beans.contains(usage.name.as_str()) {
                findings.push(Finding {
                    range: usage.range.clone(),
                    severity: Severity::Warning,
                    code: "unresolved-bean",
                    message: format!(
                        "빈 '{}'을(를) 찾을 수 없습니다(Spring XML 설정, @Component 계열, @Bean에 없음).",
                        usage.name
                    ),
                });
            }
        }
        for usage in &config.classes {
            let package = usage.name.rsplit_once('.').map(|(p, _)| p);
            if !usage.name.contains('$')
                && package.is_some_and(|p| packages.contains(p))
                && self.class(&usage.name).is_none()
            {
                findings.push(Finding {
                    range: usage.range.clone(),
                    severity: Severity::Warning,
                    code: "unresolved-class",
                    message: format!("클래스 {}을(를) 프로젝트에서 찾을 수 없습니다.", usage.name),
                });
            }
        }
        for property in &config.properties {
            let Some(owner) = &property.owner else {
                continue;
            };
            let (hierarchy, complete) = self.hierarchy(owner);
            if !complete || hierarchy.first().is_some_and(|(_, d)| d.is_interface) {
                continue;
            }
            // `a.b`처럼 이어 쓴 경로는 첫 조각만 본다.
            let name = property.name.split('.').next().unwrap_or(&property.name);
            if !self.bean_properties(owner).iter().any(|(n, ..)| n == name) {
                findings.push(Finding {
                    range: property.range.clone(),
                    severity: Severity::Warning,
                    code: "unknown-property",
                    message: format!(
                        "{owner}에 property '{name}'이(가) 없습니다(setter·필드 없음). 실행하면 빈 생성 오류가 납니다."
                    ),
                });
            }
        }
        findings
    }

    /// Spring XML 정의로 이동: 빈 참조 → 빈, class → Java 클래스, property → setter·필드,
    /// `${키}` → 프로퍼티 파일, import → 파일.
    fn spring_targets(
        &self,
        path: &Path,
        config: &Config,
        offset: usize,
    ) -> Vec<(PathBuf, Range<usize>)> {
        let within = |range: &Range<usize>| range.start <= offset && offset <= range.end;
        if let Some(usage) = config.placeholders.iter().find(|u| within(&u.range)) {
            return self
                .property_keys
                .get(&usage.name)
                .cloned()
                .unwrap_or_default();
        }
        if let Some(usage) = config.refs.iter().find(|u| within(&u.range)) {
            return self.bean_targets(&usage.name);
        }
        if let Some(usage) = config.classes.iter().find(|u| within(&u.range)) {
            return self
                .java
                .iter()
                .flat_map(|(file, java)| {
                    java.types
                        .iter()
                        .filter(|t| t.fqn == usage.name)
                        .map(|t| (file.clone(), t.range.clone()))
                })
                .collect();
        }
        if let Some(property) = config.properties.iter().find(|p| within(&p.range)) {
            let name = property.name.split('.').next().unwrap_or(&property.name);
            return property
                .owner
                .iter()
                .flat_map(|owner| self.bean_properties(owner))
                .filter(|(n, ..)| n == name)
                .map(|(_, _, file, range)| (file, range))
                .take(1)
                .collect();
        }
        if let Some(usage) = config.imports.iter().find(|u| within(&u.range)) {
            return self
                .import_file(path, &usage.name)
                .map(|f| vec![(f, 0..0)])
                .unwrap_or_default();
        }
        Vec::new()
    }

    /// `<import resource>` 경로: `classpath:`는 리소스 폴더 기준, `/`는 웹 루트, 아니면 이 파일 기준.
    fn import_file(&self, path: &Path, resource: &str) -> Option<PathBuf> {
        if resource.contains('*') || resource.contains("${") {
            return None;
        }
        if let Some(rest) = resource
            .strip_prefix("classpath*:")
            .or_else(|| resource.strip_prefix("classpath:"))
        {
            return self.resource_roots.iter().find_map(|root| {
                web::resolve(
                    &format!("/{}", rest.trim_start_matches('/')),
                    path,
                    Some(root),
                )
            });
        }
        let root = web::web_root_for(&self.web_roots, path).map(PathBuf::as_path);
        web::resolve(resource.trim_start_matches("file:"), path, root)
    }

    /// Spring XML 자동완성: class 이름, 빈 이름, property 이름, `${` 프로퍼티 키.
    fn spring_items(&self, rope: &Rope, text: &str, offset: usize) -> Option<(Vec<Value>, bool)> {
        let edit = |label: &str, detail: &str, kind: u8, range: &Range<usize>| {
            json!({
                "label": label,
                "kind": kind,
                "detail": detail,
                "filterText": label,
                "textEdit": { "range": lsp_range(rope, range), "newText": label },
            })
        };
        Some(match spring::site_at(text, offset)? {
            spring::Site::Class { replace, typed } => {
                self.class_items(rope, replace, &typed, false)
            }
            spring::Site::Bean { replace } => {
                let classes: HashMap<&str, &str> = self
                    .spring
                    .values()
                    .flat_map(|c| c.defs.iter())
                    .filter_map(|d| Some((d.name.as_str(), d.class.as_deref()?)))
                    .collect();
                let mut names: Vec<&str> = self.bean_names().into_iter().collect();
                names.sort_unstable();
                let items = names
                    .into_iter()
                    .map(|name| edit(name, classes.get(name).copied().unwrap_or(""), 6, &replace))
                    .collect();
                (items, false)
            }
            spring::Site::Property { owner, replace } => {
                let items = owner
                    .map(|owner| self.bean_properties(&owner))
                    .unwrap_or_default()
                    .iter()
                    .map(|(name, ty, ..)| edit(name, ty, 10, &replace))
                    .collect();
                (items, false)
            }
            spring::Site::Placeholder { replace } => {
                let mut keys: Vec<_> = self.property_keys.iter().collect();
                keys.sort_by(|a, b| a.0.cmp(b.0));
                let items = keys
                    .into_iter()
                    .map(|(key, files)| {
                        let file = files[0]
                            .0
                            .file_name()
                            .map(|f| f.to_string_lossy())
                            .unwrap_or_default();
                        edit(key, &file, 10, &replace)
                    })
                    .collect();
                (items, false)
            }
        })
    }

    /// JPQL(`@Query` 등) 안: `FROM` 뒤 엔티티 이름, `별칭.` 뒤 엔티티 필드.
    fn jpql_items(&self, rope: &Rope, text: &str, offset: usize) -> Option<Vec<Value>> {
        let site = java::jpql_at(text, offset)?;
        let (word, typed) = jpa::word_at(&site.before)?;
        let replace = offset - typed..offset;
        let edit = |label: &str, detail: &str, kind: u8| {
            json!({
                "label": label,
                "kind": kind,
                "detail": detail,
                "filterText": label,
                "textEdit": { "range": lsp_range(rope, &replace), "newText": label },
            })
        };
        let entities = || {
            self.java
                .values()
                .flat_map(|f| f.types.iter().map(move |t| (f, t)))
                .filter(|(_, t)| t.entity.is_some())
        };
        match word {
            Word::Entity => Some(
                entities()
                    .map(|(_, t)| edit(t.entity.as_deref().unwrap_or_default(), &t.fqn, 7))
                    .collect(),
            ),
            Word::Field { alias, path } => {
                let entity = jpa::aliases(&site.query)
                    .into_iter()
                    .find(|(a, _)| *a == alias)
                    .map(|(_, e)| e)?;
                let (mut file, mut decl) =
                    entities().find(|(_, t)| t.entity.as_deref() == Some(entity.as_str()))?;
                // `b.writer.` → writer 필드의 타입으로 내려간다.
                for segment in &path {
                    let (field, owner) = self
                        .fields(file, decl)
                        .into_iter()
                        .find(|(f, _)| &f.name == segment)?;
                    (file, decl) = self.class(&owner.resolve(&field.ty))?;
                }
                Some(
                    self.fields(file, decl)
                        .iter()
                        .map(|(f, _)| edit(&f.name, &f.ty, 5))
                        .collect(),
                )
            }
        }
    }

    /// 상속한 필드까지(상위 클래스는 선언한 파일 기준으로 이름을 푼다). 가까운 클래스가 먼저다.
    fn fields<'a>(
        &'a self,
        file: &'a JavaFile,
        decl: &'a TypeDecl,
    ) -> Vec<(&'a java::Field, &'a JavaFile)> {
        let mut found = Vec::new();
        let mut current = Some((file, decl));
        let mut seen = HashSet::new();
        while let Some((file, decl)) = current {
            if !seen.insert(decl.fqn.as_str()) {
                break;
            }
            found.extend(decl.fields.iter().map(|f| (f, file)));
            current = decl
                .superclass
                .as_ref()
                .and_then(|parent| self.class(&file.resolve(parent)));
        }
        found
    }

    /// 프로젝트 폴더의 매퍼와 Java 인터페이스를 모두 읽어 색인한다. 열린 문서는 그대로 둔다.
    /// 웹 구조(웹 루트, 뷰 리졸버, 태그 라이브러리)도 함께 모은다.
    fn index_project(&mut self, root: &Path) {
        // pom.xml·build.gradle 내용(Maven 저장소에서 라이브러리 버전을 고를 때 본다).
        let mut build = String::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if entry.file_name() == "WEB-INF" {
                        self.web_roots.push(dir.clone());
                        self.resource_roots.push(path.join("classes"));
                    }
                    if path.ends_with("src/main/resources") {
                        self.resource_roots.push(path.clone());
                    }
                    if !SKIP_DIRS.iter().any(|s| entry.file_name() == *s) {
                        stack.push(path);
                    }
                    continue;
                }
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if self.index_web_file(&path, &name, &ext.to_ascii_lowercase(), &mut build) {
                    continue;
                }
                // 대부분의 파일은 대상이 아니므로 내용을 풀기 전에 바이트로 거른다.
                let wanted: &[&[u8]] = match ext.to_ascii_lowercase().as_str() {
                    "xml" => &[b"<mapper", b"<sqlMap", b"<typeAlias", b"<beans"],
                    // VO 클래스의 필드도 자동완성에 쓰므로 Java 파일은 모두 읽는다.
                    "java" => &[],
                    _ => continue,
                };
                if self.open.contains_key(&path) {
                    continue;
                }
                let Ok(bytes) = text_file::read(&path) else {
                    continue;
                };
                if wanted.is_empty()
                    || wanted
                        .iter()
                        .any(|w| bytes.windows(w.len()).any(|b| b == *w))
                {
                    let text = text_file::decode(&bytes).ok().map(|c| c.text);
                    self.reindex(&path, text.as_deref());
                }
            }
        }
        // 프로젝트에 없는 잘 알려진 라이브러리는 로컬 Maven 저장소의 jar에서 읽는다.
        if !self.web_roots.is_empty()
            && let Some(repo) = taglib::maven_repository()
        {
            for jar in taglib::maven_jars(&repo, &build) {
                for tld in taglib::jar_tlds(&jar) {
                    if tld
                        .uri
                        .as_ref()
                        .is_some_and(|u| !self.taglib_uris.contains_key(u))
                    {
                        self.add_library(None, tld);
                    }
                }
            }
        }
    }

    /// 웹 구조 파일이면 읽고 true(매퍼·Java 색인은 하지 않는다). 매퍼일 수 있는 XML은 false.
    fn index_web_file(&mut self, path: &Path, name: &str, ext: &str, build: &mut String) -> bool {
        let text = || read_text(path);
        match ext {
            "tld" => {
                if let Some(tld) = text().as_deref().and_then(taglib::parse_tld) {
                    self.add_library(Some(path.to_path_buf()), tld);
                }
            }
            // 예전 방식 프로젝트는 라이브러리 jar를 `WEB-INF/lib`에 둔다.
            "jar" if path.parent().is_some_and(|p| p.ends_with("WEB-INF/lib")) => {
                for tld in taglib::jar_tlds(path) {
                    self.add_library(None, tld);
                }
            }
            "properties" => {
                let Some(text) = text() else {
                    return true;
                };
                for (key, range) in spring::property_keys(&text) {
                    self.property_keys
                        .entry(key)
                        .or_default()
                        .push((path.to_path_buf(), range));
                }
                jpa::scan_database(&text, &mut self.database);
                if name.starts_with("application")
                    && let Some(view) = web::view_properties(&text)
                {
                    self.views.push(view);
                }
            }
            "yml" | "yaml" if name.starts_with("application") => {
                jpa::scan_database(&text().unwrap_or_default(), &mut self.database);
            }
            "gradle" | "kts" if name.starts_with("build.gradle") => {
                build.push_str(&text().unwrap_or_default());
            }
            "xml" => {
                // 바이트로 먼저 거른다(대부분의 XML은 해당 없음). 매퍼 색인은 이어서 따로 한다.
                let Ok(bytes) = text_file::read(path) else {
                    return true;
                };
                let has = |needle: &[u8]| bytes.windows(needle.len()).any(|b| b == needle);
                let decoded = || text_file::decode(&bytes).ok().map(|c| c.text);
                if name == "pom.xml" {
                    build.push_str(&decoded().unwrap_or_default());
                    return true;
                }
                if has(b"ViewResolver")
                    && let Some(xml) = decoded()
                {
                    self.views.extend(web::view_resolvers(&xml));
                }
                if name == "web.xml"
                    && has(b"<taglib>")
                    && let Some(xml) = decoded()
                {
                    self.taglib_locations.extend(web::taglib_locations(&xml));
                }
                // persistence.xml·데이터소스 설정의 방언·드라이버.
                if (has(b"Dialect") || has(b"jdbc:oracle") || has(b"OracleDriver"))
                    && let Some(xml) = decoded()
                {
                    jpa::scan_database(&xml, &mut self.database);
                }
                return false;
            }
            _ => return false,
        }
        true
    }
}

fn is_map(ty: &str) -> bool {
    MAP_TYPES.contains(&ty.to_lowercase().as_str())
}

/// 자동완성 항목 하나. `kind`는 LSP CompletionItemKind(5 필드, 6 변수, 7 클래스, 18 참조).
fn item(label: &str, detail: &str, kind: u8) -> Value {
    json!({ "label": label, "kind": kind, "detail": detail })
}

fn field_items(fields: &[(&java::Field, &JavaFile)]) -> Vec<Value> {
    let mut seen = HashSet::new();
    fields
        .iter()
        .filter(|(f, _)| seen.insert(f.name.as_str()))
        .map(|(f, _)| item(&f.name, &f.ty, 5))
        .collect()
}

/// XML 문장이 없는 Mapper 메서드(실행하면 BindingException). 매퍼 XML의 namespace와 이어진
/// 인터페이스만 본다. 이어진 인터페이스가 없는 파일이면 None(진단 대상이 아님).
fn missing_statements(types: &[TypeDecl], index: &Index) -> Option<Vec<Finding>> {
    let mut bound = false;
    let mut findings = Vec::new();
    for interface in types.iter().filter(|t| t.is_interface) {
        let namespace = |m: &mapper::Mapper| {
            m.dialect == Dialect::MyBatis && m.namespace.as_deref() == Some(interface.fqn.as_str())
        };
        if !index.mappers.values().any(namespace) {
            continue;
        }
        bound = true;
        for method in interface.methods.iter().filter(|m| m.needs_statement) {
            if index
                .find(Kind::Statement, namespace, &method.name)
                .is_empty()
            {
                findings.push(Finding {
                    range: method.range.clone(),
                    severity: Severity::Warning,
                    code: "missing-statement",
                    message: format!(
                        "매퍼 XML(namespace {})에 id '{}'인 문장이 없습니다. 실행하면 바인딩 오류(BindingException)가 납니다.",
                        interface.fqn, method.name
                    ),
                });
            }
        }
    }
    bound.then_some(findings)
}

fn lsp_range(rope: &Rope, range: &Range<usize>) -> lsp_types::Range {
    let to = |byte: usize| {
        to_lsp(
            rope,
            rope.byte_to_char(byte.min(rope.len_bytes())),
            ENCODING,
        )
    };
    lsp_types::Range::new(to(range.start), to(range.end))
}

/// 디스크 파일을 읽는다. 예전 egov 프로젝트의 EUC-KR(CP949) 파일도 읽는다.
fn read_text(path: &Path) -> Option<String> {
    let bytes = text_file::read(path).ok()?;
    Some(text_file::decode(&bytes).ok()?.text)
}

fn initialize_root(params: &Value) -> Option<PathBuf> {
    let uri = params
        .pointer("/workspaceFolders/0/uri")
        .or_else(|| params.get("rootUri"))
        .and_then(Value::as_str)?;
    uri_to_path(&uri.parse::<Uri>().ok()?)
}

fn document_path(params: &Value) -> Option<PathBuf> {
    let uri = params
        .pointer("/textDocument/uri")
        .and_then(Value::as_str)?;
    uri_to_path(&uri.parse::<Uri>().ok()?)
}

fn publish(path: &Path, diagnostics: Vec<Diagnostic>) -> Option<Value> {
    let uri = path_to_uri(path).ok()?;
    Some(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": diagnostics },
    }))
}
