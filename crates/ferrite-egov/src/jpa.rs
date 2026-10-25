//! JPA 보완: `@Query` JPQL 안의 엔티티·필드 완성, IDENTITY 키 생성을 지원하지 않는 DB 경고.
//! JPQL 문법 검증은 Spring Boot LS의 몫이다.

use crate::mapper::Severity;

/// 프로젝트 설정에서 모은 DB 단서.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    /// Hibernate 방언 클래스의 단순 이름(`Oracle10gDialect`). 프로필마다 다를 수 있다.
    pub dialects: Vec<String>,
    /// Oracle을 쓴다는 단서(드라이버, JDBC URL, egov `Globals.DbType`). 버전은 모른다.
    pub oracle: bool,
}

/// IDENTITY 컬럼이 없는 Hibernate Oracle 방언(Oracle12cDialect부터 지원한다).
const OLD_ORACLE_DIALECTS: [&str; 4] = [
    "Oracle8iDialect",
    "Oracle9Dialect",
    "Oracle9iDialect",
    "Oracle10gDialect",
];

/// 설정 파일(.properties, .yml, Spring XML, persistence.xml) 텍스트에서 DB 단서를 모은다.
pub fn scan_database(text: &str, db: &mut Database) {
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("dialect") || lower.contains("platform") {
            for token in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
            {
                let simple = token.rsplit('.').next().unwrap_or(token);
                if simple.ends_with("Dialect")
                    && simple.len() > "Dialect".len()
                    && !db.dialects.iter().any(|d| d == simple)
                {
                    db.dialects.push(simple.to_owned());
                }
            }
        }
        let trimmed = lower.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with("<!--") {
            continue;
        }
        if lower.contains("jdbc:oracle:")
            || lower.contains("oracle.jdbc")
            || (lower.contains("dbtype") && lower.contains("oracle"))
        {
            db.oracle = true;
        }
    }
}

/// `@GeneratedValue(strategy = IDENTITY)`를 이 DB에서 쓰면 낼 진단(심각도, 메시지). 괜찮으면 None.
pub fn identity_problem(db: &Database) -> Option<(Severity, String)> {
    const ADVICE: &str = "SEQUENCE와 @SequenceGenerator를 쓰세요";
    if let Some(old) = db
        .dialects
        .iter()
        .find(|d| OLD_ORACLE_DIALECTS.contains(&d.as_str()))
    {
        return Some((
            Severity::Warning,
            format!(
                "Hibernate 방언 {old}는 IDENTITY 키 생성을 지원하지 않습니다(Oracle12cDialect부터). {ADVICE}."
            ),
        ));
    }
    let oracle_dialect = db.dialects.iter().any(|d| d.starts_with("Oracle"));
    let modern = db
        .dialects
        .iter()
        .any(|d| d.starts_with("Oracle") && d != "OracleDialect");
    let other = !db.dialects.is_empty() && !oracle_dialect;
    // 버전을 모르는 Oracle(드라이버만 보임, Hibernate 6의 버전 감지형 OracleDialect).
    if !modern && !other && (db.oracle || oracle_dialect) {
        return Some((
            Severity::Information,
            format!(
                "대상 DB가 Oracle입니다. Oracle 11g 이하는 IDENTITY 컬럼이 없습니다(12c부터). 11g 이하라면 {ADVICE}."
            ),
        ));
    }
    None
}

/// JPQL에서 커서 앞 낱말이 채울 것.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Word {
    /// `FROM`·`JOIN`·`UPDATE` 뒤(또는 `FROM A a, ` 뒤): 엔티티 이름.
    Entity,
    /// `b.wri`: 별칭 `b`에서 시작해 `path`(가운데 필드들)를 따라간 엔티티의 필드.
    Field { alias: String, path: Vec<String> },
}

const ENTITY_KEYWORDS: [&str; 4] = ["FROM", "JOIN", "UPDATE", "INTO"];
/// 별칭 자리에 오지 않는 JPQL 예약어.
const CLAUSE_KEYWORDS: [&str; 18] = [
    "WHERE", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "FETCH", "ON", "SET", "ORDER", "GROUP",
    "HAVING", "AS", "AND", "OR", "UNION", "WITH", "BY",
];

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// 커서 앞 쿼리(`before`)에서 지금 치는 낱말의 자리와 그 길이(바이트).
pub fn word_at(before: &str) -> Option<(Word, usize)> {
    let typed = before.len() - before.trim_end_matches(is_ident).len();
    let head = &before[..before.len() - typed];
    if let Some(path) = head.strip_suffix('.') {
        let start = path
            .trim_end_matches(|c: char| is_ident(c) || c == '.')
            .len();
        let mut segments: Vec<String> = path[start..].split('.').map(str::to_owned).collect();
        if segments.iter().any(String::is_empty) {
            return None;
        }
        let alias = segments.remove(0);
        return Some((
            Word::Field {
                alias,
                path: segments,
            },
            typed,
        ));
    }
    let trimmed = head.trim_end();
    if trimmed.len() == head.len() && !head.is_empty() {
        // 낱말 앞이 공백이 아니다(`(x`, `:param` 등).
        return None;
    }
    let previous = trimmed
        .rsplit(|c: char| !is_ident(c))
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    let listed = trimmed.ends_with(',') && from_clause_open(trimmed);
    (ENTITY_KEYWORDS.contains(&previous.as_str()) || listed).then_some((Word::Entity, typed))
}

/// `FROM A a,` 처럼 FROM 절 안의 쉼표인가(뒤에 WHERE 등이 아직 없다).
fn from_clause_open(before: &str) -> bool {
    let upper = before.to_ascii_uppercase();
    let from = upper.rfind("FROM ");
    let clause = ["WHERE", "SELECT", "SET", "ORDER", "GROUP"]
        .iter()
        .filter_map(|k| upper.rfind(k))
        .max();
    match (from, clause) {
        (Some(from), Some(clause)) => from > clause,
        (Some(_), None) => true,
        _ => false,
    }
}

/// FROM 절을 끝내는 예약어.
const CLAUSE_ENDS: [&str; 7] = [
    "WHERE", "SET", "ORDER", "GROUP", "HAVING", "SELECT", "UNION",
];

/// 쿼리의 `엔티티 [AS] 별칭`들(별칭, 엔티티 이름). `JOIN b.files f`처럼 경로로 붙인 별칭은 뺀다.
// ponytail: 경로 조인 별칭은 컬렉션 요소 타입(제네릭 인자)을 알아야 해서 뺐다. 필요하면 Java 색인의
// 필드 타입에 제네릭 인자를 함께 담는다.
pub fn aliases(query: &str) -> Vec<(String, String)> {
    // 쉼표는 따로 낱말로 둔다(`FROM A a, B b`).
    let spaced = query.replace(',', " , ");
    let words: Vec<&str> = spaced
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|w| !w.is_empty())
        .collect();
    let mut found = Vec::new();
    let mut in_from = false;
    let mut i = 0;
    while i < words.len() {
        let upper = words[i].to_ascii_uppercase();
        if ENTITY_KEYWORDS.contains(&upper.as_str()) || (in_from && words[i] == ",") {
            // JOIN·쉼표는 FROM 절 안이다. UPDATE·INSERT INTO 뒤에는 FROM 절이 없다.
            in_from = matches!(upper.as_str(), "FROM" | "JOIN" | ",");
            i += 1;
            if words
                .get(i)
                .is_some_and(|w| w.eq_ignore_ascii_case("FETCH"))
            {
                i += 1;
            }
            push_alias(&words, &mut i, &mut found);
            continue;
        }
        if CLAUSE_ENDS.contains(&upper.as_str()) {
            in_from = false;
        }
        i += 1;
    }
    found
}

fn push_alias(words: &[&str], i: &mut usize, found: &mut Vec<(String, String)>) {
    let Some(entity) = words.get(*i) else {
        return;
    };
    *i += 1;
    if words.get(*i).is_some_and(|w| w.eq_ignore_ascii_case("AS")) {
        *i += 1;
    }
    let Some(alias) = words.get(*i) else {
        return;
    };
    let keyword = CLAUSE_KEYWORDS.contains(&alias.to_ascii_uppercase().as_str());
    if !entity.contains('.') && !keyword && alias.chars().all(is_ident) {
        found.push((alias.to_string(), entity.to_string()));
        *i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 방언과_드라이버로_identity_지원을_가린다() {
        let db = |text: &str| {
            let mut db = Database::default();
            scan_database(text, &mut db);
            db
        };
        let old =
            db("spring.jpa.properties.hibernate.dialect=org.hibernate.dialect.Oracle10gDialect\n");
        assert_eq!(old.dialects, ["Oracle10gDialect"]);
        assert_eq!(identity_problem(&old).unwrap().0, Severity::Warning);
        let xml = db(
            r#"<property name="hibernate.dialect" value="org.hibernate.dialect.Oracle12cDialect"/>"#,
        );
        assert_eq!(identity_problem(&xml), None, "12c부터 지원");
        let driver = db("Globals.DriverClassName=oracle.jdbc.driver.OracleDriver\n");
        assert!(driver.oracle);
        assert_eq!(
            identity_problem(&driver).unwrap().0,
            Severity::Information,
            "버전을 모른다"
        );
        let egov = db("Globals.DbType = oracle\n");
        assert!(egov.oracle);
        let postgres = db(
            "spring.jpa.database-platform: org.hibernate.dialect.PostgreSQLDialect\nspring.datasource.url: jdbc:oracle:thin:@x\n",
        );
        assert_eq!(
            identity_problem(&postgres),
            None,
            "방언이 Oracle이 아니면 방언을 따른다"
        );
        assert_eq!(
            identity_problem(&db("# url=jdbc:oracle:thin\n")),
            None,
            "주석"
        );
        assert_eq!(identity_problem(&Database::default()), None);
    }

    #[test]
    fn jpql_낱말_자리를_안다() {
        assert_eq!(word_at("SELECT b FROM Bo"), Some((Word::Entity, 2)));
        assert_eq!(word_at("select b from "), Some((Word::Entity, 0)));
        assert_eq!(
            word_at("SELECT b FROM Board b JOIN "),
            Some((Word::Entity, 0))
        );
        assert_eq!(
            word_at("SELECT b FROM Board b, Mem"),
            Some((Word::Entity, 3))
        );
        assert_eq!(
            word_at("SELECT b FROM Board b WHERE b.ti"),
            Some((
                Word::Field {
                    alias: "b".into(),
                    path: vec![]
                },
                2
            ))
        );
        assert_eq!(
            word_at("SELECT b FROM Board b WHERE b.writer.na"),
            Some((
                Word::Field {
                    alias: "b".into(),
                    path: vec!["writer".into()]
                },
                2
            ))
        );
        assert_eq!(word_at("SELECT b FROM Board b WHERE b.id = :id AND "), None);
        assert_eq!(
            word_at("SELECT b FROM Board b WHERE b.id = :i"),
            None,
            "매개변수"
        );
        assert_eq!(
            word_at("SELECT b FROM Board b WHERE b.id, "),
            None,
            "FROM 절 밖의 쉼표"
        );
    }

    #[test]
    fn 별칭을_모은다() {
        assert_eq!(
            aliases(
                "SELECT b FROM Board AS b LEFT JOIN FETCH Member m ON m.id = b.writerId, Code c JOIN b.files f WHERE b.id = :id"
            ),
            vec![
                ("b".to_owned(), "Board".to_owned()),
                ("m".to_owned(), "Member".to_owned()),
                ("c".to_owned(), "Code".to_owned()),
            ]
        );
        assert_eq!(
            aliases("UPDATE Board b SET b.title = :t"),
            vec![("b".to_owned(), "Board".to_owned())]
        );
    }
}
