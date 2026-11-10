//! egov 표준 점검 룰셋(PMD 7용)을 프로젝트에 넣는다: 룰셋 파일과 pom의 maven-pmd-plugin `<rulesets>` 설정.
//! 공식 4.3 룰셋은 XPath 규칙 5개가 PMD 7 정식판에서 아무것도 잡지 못해, 같은 44개 규칙을 PMD가 관리하는
//! 기본 규칙으로 가리키는 룰셋을 쓴다(파일 머리의 주석 참고).

use roxmltree::{Document, Node};

/// 프로젝트 루트(pom.xml 옆)에 쓰는 룰셋 파일 이름.
pub const FILE: &str = "egov-inspection-rules.xml";
pub const RULESET: &str = include_str!("egov_pmd/egov-inspection-rules.xml");
/// 플러그인을 새로 적을 때의 판(PMD 7.17.0). 확인한 판이다.
const PLUGIN_VERSION: &str = "3.28.0";

/// pom을 어떻게 고쳤는지.
#[derive(Debug, PartialEq)]
pub enum PomEdit {
    Changed(String),
    /// 이미 이 룰셋을 쓴다.
    Already,
}

/// pom의 maven-pmd-plugin 설정에 이 룰셋을 더한다. egov 템플릿처럼 설정 없이 적힌 플러그인이면 그
/// 플러그인에, 없으면 `<build><plugins>`에 새로 적는다. 다른 룰셋이 이미 있거나 여러 모듈을 묶는 pom이면
/// 고치지 않는다(룰셋 경로 `${basedir}`가 모듈마다 달라진다).
pub fn add_to_pom(pom: &str) -> Result<PomEdit, String> {
    let doc = Document::parse(pom).map_err(|e| format!("pom.xml을 읽지 못했습니다: {e}"))?;
    let project = doc.root_element();
    if child(project, "modules").is_some() {
        return Err(
            "여러 모듈을 묶는 pom이라 고치지 않았습니다(룰셋 경로가 모듈마다 달라집니다). \
             모듈의 pom에 직접 적으세요."
                .to_owned(),
        );
    }
    let ruleset = format!("<ruleset>${{basedir}}/{FILE}</ruleset>");
    let rulesets = wrap("rulesets", vec![(0, ruleset)]);
    let configuration = wrap("configuration", rulesets.clone());
    let build = child(project, "build");
    let plugins = build.and_then(|b| child(b, "plugins"));
    let pmd = plugins.and_then(|p| {
        p.children().find(|c| {
            c.is_element()
                && c.tag_name().name() == "plugin"
                && child(*c, "artifactId")
                    .and_then(|a| a.text())
                    .map(str::trim)
                    == Some("maven-pmd-plugin")
        })
    });
    let (node, lines) = match (pmd, plugins, build) {
        (Some(pmd), ..) => {
            let config = child(pmd, "configuration");
            if let Some(existing) = config.and_then(|c| child(c, "rulesets")) {
                return if existing
                    .descendants()
                    .any(|n| n.text().is_some_and(|t| t.contains(FILE)))
                {
                    Ok(PomEdit::Already)
                } else {
                    Err(format!(
                        "pom.xml의 PMD 설정에 다른 룰셋(<rulesets>)이 있어 고치지 않았습니다. \
                         그 목록에 ${{basedir}}/{FILE}을 더하세요."
                    ))
                };
            }
            match config {
                Some(config) => (config, rulesets),
                None => (pmd, configuration),
            }
        }
        (None, Some(plugins), _) => (plugins, plugin(configuration)),
        (None, None, Some(build)) => (build, wrap("plugins", plugin(configuration))),
        (None, None, None) => (
            project,
            wrap("build", wrap("plugins", plugin(configuration))),
        ),
    };
    insert(pom, node, &lines).map(PomEdit::Changed)
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

/// (깊이, 줄)을 `<tag>`로 감싼다.
fn wrap(tag: &str, inner: Vec<(usize, String)>) -> Vec<(usize, String)> {
    let mut lines = vec![(0, format!("<{tag}>"))];
    lines.extend(inner.into_iter().map(|(depth, line)| (depth + 1, line)));
    lines.push((0, format!("</{tag}>")));
    lines
}

fn plugin(configuration: Vec<(usize, String)>) -> Vec<(usize, String)> {
    let mut inner = vec![
        (0, "<groupId>org.apache.maven.plugins</groupId>".to_owned()),
        (0, "<artifactId>maven-pmd-plugin</artifactId>".to_owned()),
        (0, format!("<version>{PLUGIN_VERSION}</version>")),
    ];
    inner.extend(configuration);
    wrap("plugin", inner)
}

/// `node`의 닫는 태그 앞에 줄들을 넣는다. 들여쓰기는 pom이 쓰는 단위(탭·빈칸)를 따르고 `node`보다 한 단계
/// 깊게, 줄 끝도 pom을 따른다(egov 템플릿은 탭과 CRLF인 것이 있다).
fn insert(pom: &str, node: Node, lines: &[(usize, String)]) -> Result<String, String> {
    let range = node.range();
    let element = &pom[range.clone()];
    let name = node.tag_name().name();
    if element.ends_with("/>") {
        return Err(format!(
            "pom.xml의 <{name}/>를 고치지 못했습니다. 직접 적으세요."
        ));
    }
    let close = range.start + element.rfind("</").unwrap_or(0);
    let line_of = |at: usize| pom[..at].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = pom[line_of(range.start)..range.start]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    // 들여쓴 첫 태그 줄의 앞 공백이 한 단계다(여러 줄에 걸친 `<project …>`의 속성 줄은 건너뛴다).
    let unit = pom
        .lines()
        .filter(|l| l.trim_start().starts_with('<'))
        .map(|l| {
            l.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>()
        })
        .find(|ws| !ws.is_empty())
        .unwrap_or_else(|| "\t".to_owned());
    let eol = if pom.contains("\r\n") { "\r\n" } else { "\n" };
    let block: String = lines
        .iter()
        .map(|(depth, line)| format!("{indent}{}{line}{eol}", unit.repeat(depth + 1)))
        .collect();
    let close_line = line_of(close);
    Ok(if pom[close_line..close].trim().is_empty() {
        // 닫는 태그가 제 줄에 있다: 그 줄 앞에 넣는다.
        format!("{}{block}{}", &pom[..close_line], &pom[close_line..])
    } else {
        // 한 줄에 열고 닫는다(`<plugin>…</plugin>`): 줄을 바꿔 넣는다.
        format!("{}{eol}{block}{indent}{}", &pom[..close], &pom[close..])
    })
}

#[cfg(test)]
mod tests;
