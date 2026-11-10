use super::*;

fn changed(pom: &str) -> String {
    match add_to_pom(pom).unwrap() {
        PomEdit::Changed(text) => {
            // 고친 pom도 XML이고, 다시 넣으면 이미 있다고 한다.
            Document::parse(&text).unwrap();
            assert_eq!(add_to_pom(&text), Ok(PomEdit::Already));
            text
        }
        PomEdit::Already => panic!("고치지 않음"),
    }
}

#[test]
fn egov_템플릿의_설정_없는_pmd_플러그인에_룰셋을_넣는다() {
    // egov 웹 템플릿(v5.0.7)의 꼴: 탭 들여쓰기, 판·설정 없는 maven-pmd-plugin. 줄 끝은 CRLF로 했다.
    let pom = "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\r\n\t<modelVersion>4.0.0</modelVersion>\r\n\t<build>\r\n\t\t<plugins>\r\n\t\t\t<!-- PMD manven plugin -->\r\n\t\t\t<plugin>\r\n\t\t\t\t<groupId>org.apache.maven.plugins</groupId>\r\n\t\t\t\t<artifactId>maven-pmd-plugin</artifactId>\r\n\t\t\t</plugin>\r\n\t\t</plugins>\r\n\t</build>\r\n</project>\r\n";
    assert_eq!(
        changed(pom),
        pom.replace(
            "maven-pmd-plugin</artifactId>\r\n",
            "maven-pmd-plugin</artifactId>\r\n\t\t\t\t<configuration>\r\n\t\t\t\t\t<rulesets>\r\n\t\t\t\t\t\t<ruleset>${basedir}/egov-inspection-rules.xml</ruleset>\r\n\t\t\t\t\t</rulesets>\r\n\t\t\t\t</configuration>\r\n",
        )
    );
}

#[test]
fn 플러그인이_없으면_판을_적어_새로_넣는다() {
    // 빈칸 네 칸 들여쓰기, 여러 줄에 걸친 project 속성.
    let pom = "<project xmlns=\"http://maven.apache.org/POM/4.0.0\"\n  xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n    <modelVersion>4.0.0</modelVersion>\n    <build>\n        <plugins>\n            <plugin>\n                <artifactId>maven-war-plugin</artifactId>\n            </plugin>\n        </plugins>\n    </build>\n</project>\n";
    assert_eq!(
        changed(pom),
        pom.replace(
            "            </plugin>\n        </plugins>",
            "            </plugin>\n            <plugin>\n                <groupId>org.apache.maven.plugins</groupId>\n                <artifactId>maven-pmd-plugin</artifactId>\n                <version>3.28.0</version>\n                <configuration>\n                    <rulesets>\n                        <ruleset>${basedir}/egov-inspection-rules.xml</ruleset>\n                    </rulesets>\n                </configuration>\n            </plugin>\n        </plugins>",
        )
    );
    // build가 없으면 build째 넣는다.
    let bare = "<project>\n  <modelVersion>4.0.0</modelVersion>\n</project>\n";
    let text = changed(bare);
    assert!(
        text.starts_with("<project>\n  <modelVersion>4.0.0</modelVersion>\n  <build>\n    <plugins>\n      <plugin>\n"),
        "{text}"
    );
    assert!(text.ends_with("      </plugin>\n    </plugins>\n  </build>\n</project>\n"));
}

#[test]
fn 설정이_있으면_그_안에_넣고_한_줄_태그도_고친다() {
    let pom = "<project>\n\t<build>\n\t\t<plugins>\n\t\t\t<plugin>\n\t\t\t\t<artifactId>maven-pmd-plugin</artifactId>\n\t\t\t\t<configuration>\n\t\t\t\t\t<targetJdk>17</targetJdk>\n\t\t\t\t</configuration>\n\t\t\t</plugin>\n\t\t</plugins>\n\t</build>\n</project>\n";
    assert_eq!(
        changed(pom),
        pom.replace(
            "<targetJdk>17</targetJdk>\n",
            "<targetJdk>17</targetJdk>\n\t\t\t\t\t<rulesets>\n\t\t\t\t\t\t<ruleset>${basedir}/egov-inspection-rules.xml</ruleset>\n\t\t\t\t\t</rulesets>\n",
        )
    );
    let one_line = "<project>\n\t<build>\n\t\t<plugins>\n\t\t\t<plugin><artifactId>maven-pmd-plugin</artifactId></plugin>\n\t\t</plugins>\n\t</build>\n</project>\n";
    assert!(
        changed(one_line)
            .contains("<artifactId>maven-pmd-plugin</artifactId>\n\t\t\t\t<configuration>\n")
    );
}

#[test]
fn 다른_룰셋이나_여러_모듈이면_고치지_않는다() {
    let other = "<project><build><plugins><plugin><artifactId>maven-pmd-plugin</artifactId><configuration><rulesets><ruleset>my.xml</ruleset></rulesets></configuration></plugin></plugins></build></project>";
    assert!(add_to_pom(other).unwrap_err().contains("다른 룰셋"));
    let modules = "<project><modules><module>web</module></modules></project>";
    assert!(add_to_pom(modules).unwrap_err().contains("여러 모듈"));
    assert!(add_to_pom("<project>").is_err());
}

#[test]
fn 룰셋은_44개_규칙을_pmd_7_기본_규칙으로_가리킨다() {
    let doc = Document::parse(RULESET).unwrap();
    let rules: Vec<Node> = doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("rule"))
        .collect();
    assert_eq!(rules.len(), 44);
    for rule in rules {
        let target = rule.attribute("ref").unwrap();
        assert!(target.starts_with("category/java/"), "{target}");
        let message = rule.attribute("message").unwrap();
        assert!(!message.is_ascii(), "한글 메시지: {target}");
        let priority = child(rule, "priority").and_then(|p| p.text()).unwrap();
        assert!(matches!(priority, "1" | "2" | "3" | "4"), "{target}");
    }
}
