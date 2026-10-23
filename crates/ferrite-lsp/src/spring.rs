//! Spring Boot LS ↔ jdt.ls 중계 규칙. 공개 명세가 없어 VS Code 확장(vscode-spring-boot의
//! `commons-vscode/src/java-data.ts`, `classpath.ts`)의 동작을 그대로 따른다.
//! - Spring Boot LS가 클라이언트에 `sts/*` 요청을 보내면 jdt.ls의 `sts.java.*` 명령으로 바꿔 보낸다.
//! - jdt.ls가 `workspace/executeClientCommand`로 콜백하면 Spring Boot LS의 명령으로 넘긴다.

use serde_json::{Value, json};

/// 시작한 Spring Boot LS에 보내는 명령. jdt.ls 클래스패스 연동을 켠다(인자 `[true]`).
pub const ENABLE_CLASSPATH: &str = "sts.vscode-spring-boot.enableClasspathListening";
/// jdt.ls 확장이 Spring 프로젝트를 찾으면 보내는 클라이언트 명령. 연동을 켜라는 뜻이다.
pub const LS_START: &str = "vscode-spring-boot.ls.start";

/// `sts/*` 요청 → jdt.ls 명령. 매개변수 객체를 그대로 첫 인자로 넘긴다.
const JDT_COMMANDS: [(&str, &str); 10] = [
    ("sts/javaType", "sts.java.type"),
    ("sts/javadocHoverLink", "sts.java.javadocHoverLink"),
    ("sts/javaLocation", "sts.java.location"),
    ("sts/javadoc", "sts.java.javadoc"),
    ("sts/javaSearchTypes", "sts.java.search.types"),
    ("sts/javaSearchPackages", "sts.java.search.packages"),
    ("sts/javaSubTypes", "sts.java.hierarchy.subtypes"),
    ("sts/javaSuperTypes", "sts.java.hierarchy.supertypes"),
    ("sts/javaCodeComplete", "sts.java.code.completions"),
    ("sts/project/gav", "sts.project.gav"),
];

/// Spring Boot LS의 `sts/*` 요청을 jdt.ls에 보낼 `workspace/executeCommand` 매개변수로 바꾼다.
/// 중계할 요청이 아니면 None(그때는 null로 답한다).
pub fn jdt_command(method: &str, params: &Value) -> Option<Value> {
    // 클래스패스 리스너는 콜백 명령 ID만 인자로 넘긴다.
    let listener = match method {
        "sts/addClasspathListener" => Some("sts.java.addClasspathListener"),
        "sts/removeClasspathListener" => Some("sts.java.removeClasspathListener"),
        _ => None,
    };
    if let Some(command) = listener {
        let callback = params.get("callbackCommandId")?;
        return Some(json!({ "command": command, "arguments": [callback] }));
    }
    let (_, command) = JDT_COMMANDS.iter().find(|(m, _)| *m == method)?;
    Some(json!({ "command": command, "arguments": [params] }))
}

/// Spring Boot LS 설정(`workspace/didChangeConfiguration`). VS Code 확장 기본값에서 egov에 맞게
/// Spring XML 설정 지원을 켰다(`context-*.xml`의 빈을 Java 쪽 기능이 알도록).
pub fn settings() -> Value {
    json!({
        "boot-java": {
            "jpql": true,
            "support-spring-xml-config": {
                "on": true,
                "scan-folders": "src/main",
                "hyperlinks": true,
                "content-assist": true,
            },
            "scan-java-test-sources": { "on": false },
            "java": {
                "reconcilers": true,
                "completions": { "inject-bean": true },
                "codelens-over-query-methods": true,
            },
            "properties": { "completions": { "elide-prefix": false } },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sts_요청을_jdtls_명령으로_바꾼다() {
        let params = json!({ "projectUri": "file:///p", "bindingKey": "Lx;" });
        assert_eq!(
            jdt_command("sts/javaType", &params),
            Some(json!({ "command": "sts.java.type", "arguments": [params] }))
        );
        assert_eq!(
            jdt_command(
                "sts/addClasspathListener",
                &json!({ "callbackCommandId": "cb-1" })
            ),
            Some(json!({ "command": "sts.java.addClasspathListener", "arguments": ["cb-1"] }))
        );
        assert_eq!(jdt_command("sts/addClasspathListener", &json!({})), None);
        assert_eq!(jdt_command("sts/highlight", &params), None);
    }
}
