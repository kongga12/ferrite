//! 공식 프로젝트 템플릿으로 새 프로젝트 만들기. 가짜 zip으로 동작을 보고, 공식 vsix가 있으면
//! 고정한 22종이 모두 맞는지와 실제로 만들어지는지 본다.

use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use ferrite_codegen::scaffold::{self, Pinned, Request, TEMPLATES, Template};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrite-scaffold-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// 테스트용 고정값(정적 수명은 테스트 동안만 쓰므로 leak으로 만든다).
fn pin(file: &'static str, data: &[u8]) -> Pinned {
    let sha256: String = Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Pinned {
        file,
        size: data.len() as u64,
        sha256: Box::leak(sha256.into_boxed_str()),
    }
}

const POM: &str = "<project><groupId>###GROUP_ID###</groupId><artifactId>###ARTIFACT_ID###</artifactId>\
<version>###VERSION###</version><name>###NAME###</name><url>###URL###</url></project>";

/// 가짜 템플릿과 그 파일들을 둔 캐시.
fn fake(cache: &Path, zip: &[u8], pom: Option<&[u8]>) -> Template {
    fs::write(cache.join("fake.zip"), zip).unwrap();
    if let Some(pom) = pom {
        fs::write(cache.join("fake-pom.xml"), pom).unwrap();
    }
    Template {
        category: "테스트",
        name: "가짜",
        description: "",
        project_name: "egov-fake",
        zip: pin("fake.zip", zip),
        pom: pom.map(|p| pin("fake-pom.xml", p)),
    }
}

#[test]
fn 템플릿을_풀고_pom_틀을_채운다() {
    let dir = temp("create");
    let cache = dir.join("cache");
    fs::create_dir(&cache).unwrap();
    let zip = zip_of(&[
        ("pom.xml", b"<project>zip pom</project>"),
        (
            "src/main/resources/egovframework/message/message-common_ko.properties",
            b"a=1",
        ),
        ("src/main/webapp/한글.jsp", "<p>안녕</p>".as_bytes()),
    ]);
    let template = fake(&cache, &zip, Some(POM.as_bytes()));
    let mut request = Request::new(&template, dir.clone());
    request.name = "my.app".into();
    request.group_id = "com.example".into();
    request.artifact_id = "my-app".into();
    request.version = "1.0.0-SNAPSHOT".into();

    assert!(template.cached(&cache));
    let created = scaffold::create(&template, &request, &cache).unwrap();
    assert_eq!(created, dir.join("my.app"));
    assert_eq!(
        fs::read_to_string(created.join("pom.xml")).unwrap(),
        "<project><groupId>com.example</groupId><artifactId>my-app</artifactId>\
<version>1.0.0-SNAPSHOT</version><name>my.app</name><url>https://www.egovframe.go.kr</url></project>",
        "zip의 pom.xml은 pom 틀로 바꾼다"
    );
    assert_eq!(
        fs::read_to_string(created.join("src/main/webapp/한글.jsp")).unwrap(),
        "<p>안녕</p>"
    );

    let err = scaffold::create(&template, &request, &cache).unwrap_err();
    assert!(err.contains("이미 있습니다"), "{err}");
    assert_eq!(
        fs::read_to_string(created.join("src/main/webapp/한글.jsp")).unwrap(),
        "<p>안녕</p>",
        "있던 폴더는 건드리지 않는다"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn pom_틀이_없으면_그대로_풀고_groupid는_보지_않는다() {
    let dir = temp("nopom");
    let cache = dir.join("cache");
    fs::create_dir(&cache).unwrap();
    let template = fake(&cache, &zip_of(&[("package.json", b"{}")]), None);
    let mut request = Request::new(&template, dir.clone());
    request.group_id.clear();
    request.artifact_id.clear();

    let created = scaffold::create(&template, &request, &cache).unwrap();
    assert_eq!(
        fs::read_to_string(created.join("package.json")).unwrap(),
        "{}"
    );
    assert!(!created.join("pom.xml").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn 실패하면_만들던_폴더를_지우고_바뀐_캐시는_쓰지_않는다() {
    let dir = temp("fail");
    let cache = dir.join("cache");
    fs::create_dir(&cache).unwrap();
    // 해시는 맞지만 zip이 아니다: 폴더를 만든 뒤 풀다가 실패한다.
    let template = fake(&cache, b"not a zip", Some(POM.as_bytes()));
    let request = Request::new(&template, dir.clone());
    assert!(scaffold::create(&template, &request, &cache).is_err());
    assert!(!request.target().exists(), "반쯤 만든 폴더를 남기지 않는다");

    // 받아 둔 파일이 바뀌었다(크기는 같다): 화면에는 받아 둔 것으로 보여도 만들 때 막는다.
    let zip = zip_of(&[("a.txt", b"a")]);
    let template = fake(&cache, &zip, None);
    let mut tampered = zip.clone();
    *tampered.last_mut().unwrap() ^= 1;
    fs::write(cache.join("fake.zip"), &tampered).unwrap();
    assert!(template.cached(&cache));
    let err = scaffold::create(&template, &request, &cache).unwrap_err();
    assert!(err.contains("손상 또는 변조"), "{err}");
    assert!(!request.target().exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn 입력값은_공식_확장과_같은_규칙으로_검사한다() {
    let template = &TEMPLATES[0];
    let ok = Request::new(template, PathBuf::from("out"));
    assert!(
        ok.problems(template).is_empty(),
        "{:?}",
        ok.problems(template)
    );

    let check = |change: fn(&mut Request)| {
        let mut request = ok.clone();
        change(&mut request);
        request.problems(template)
    };
    for bad in [
        check(|r| r.name = "../escape".into()),
        check(|r| r.name = "a..b".into()),
        check(|r| r.name = "a/b".into()),
        check(|r| r.name = String::new()),
        check(|r| r.group_id = "Com.example".into()),
        check(|r| r.group_id = "com..example".into()),
        check(|r| r.group_id = "com.1example".into()),
        check(|r| r.artifact_id = "My-app".into()),
        check(|r| r.artifact_id = "my_app".into()),
        check(|r| r.version = "1.0 <x>".into()),
        check(|r| r.version = String::new()),
        check(|r| r.output = PathBuf::new()),
    ] {
        assert_eq!(bad.len(), 1, "{bad:?}");
    }
    assert!(check(|r| r.name = "My_App.v2".into()).is_empty());
}

#[test]
fn 다른_곳에서_가져온_파일이나_vsix로_캐시를_채운다() {
    let dir = temp("import");
    let cache = dir.join("cache");
    let zip = zip_of(&[("a.txt", b"a")]);
    let pom = POM.as_bytes();
    let template = Template {
        category: "",
        name: "",
        description: "",
        project_name: "egov-fake",
        zip: pin("fake.zip", &zip),
        pom: Some(pin("fake-pom.xml", pom)),
    };

    // 이름이 달라도 해시로 맞춘다.
    let renamed = dir.join("downloaded (1).zip");
    fs::write(&renamed, &zip).unwrap();
    let err = scaffold::import(&template, std::slice::from_ref(&renamed), &cache).unwrap_err();
    assert!(err.contains("fake-pom.xml"), "{err}");
    assert!(!err.contains("fake.zip"), "zip은 찾았다: {err}");
    assert!(!template.cached(&cache));

    // vsix: 템플릿이 그 안에 들어 있다. 항목 경로가 아니라 캐시 이름으로 쓴다.
    let vsix = dir.join("egov.vsix");
    fs::write(
        &vsix,
        zip_of(&[
            ("extension/package.json", b"{}"),
            ("extension/templates/projects/examples/fake.zip", &zip),
            ("extension/templates/projects/pom/fake-pom.xml", pom),
        ]),
    )
    .unwrap();
    scaffold::import(&template, &[vsix], &cache).unwrap();
    assert!(template.cached(&cache));
    for (pinned, _) in template.files() {
        scaffold::verify(&cache.join(pinned.file), pinned).unwrap();
    }
    let leftovers: Vec<_> = fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n.to_string_lossy().ends_with(".import"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // 내용이 다른 vsix 항목은 받지 않는다.
    let other = dir.join("other.vsix");
    let mut wrong = zip.clone();
    *wrong.last_mut().unwrap() ^= 1;
    fs::write(
        &other,
        zip_of(&[("extension/templates/projects/examples/fake.zip", &wrong)]),
    )
    .unwrap();
    let fresh = dir.join("fresh");
    assert!(scaffold::import(&template, &[other], &fresh).is_err());
    assert!(!fresh.join("fake.zip").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn 목록은_공식_확장의_22종을_고정한다() {
    assert_eq!(TEMPLATES.len(), 22);
    let mut names = HashSet::new();
    for template in TEMPLATES {
        assert!(
            Request::new(template, PathBuf::from("out"))
                .problems(template)
                .is_empty()
        );
        for (pinned, url) in template.files() {
            assert!(
                names.insert(pinned.file),
                "캐시 이름이 겹친다: {}",
                pinned.file
            );
            assert_eq!(pinned.sha256.len(), 64);
            assert!(pinned.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(
                url.starts_with("https://") && url.ends_with(pinned.file),
                "{url}"
            );
            assert!(url.contains(scaffold::RELEASE));
        }
    }
}

/// 고정값이 공식 vsix와 맞고, 22종 모두 만들어지는지 본다.
/// `FERRITE_EGOV_VSIX=<vsix 절대 경로> cargo test --release -p ferrite-codegen --test scaffold -- --ignored`
#[test]
#[ignore = "공식 확장 vsix가 필요하다(FERRITE_EGOV_VSIX)"]
fn 공식_vsix의_템플릿으로_모두_만든다() {
    let Some(vsix) = std::env::var_os("FERRITE_EGOV_VSIX") else {
        eprintln!("FERRITE_EGOV_VSIX가 없어 건너뜁니다");
        return;
    };
    let vsix = std::fs::canonicalize(vsix).unwrap();
    let dir = temp("official");
    let cache = dir.join("cache");
    for template in TEMPLATES {
        scaffold::import(template, std::slice::from_ref(&vsix), &cache)
            .unwrap_or_else(|e| panic!("{}: {e}", template.name));
        let request = Request::new(template, dir.clone());
        let created = scaffold::create(template, &request, &cache)
            .unwrap_or_else(|e| panic!("{}: {e}", template.name));
        if template.pom.is_some() {
            let pom = fs::read_to_string(created.join("pom.xml")).unwrap();
            assert!(!pom.contains("###"), "{}: 채우지 않은 자리", template.name);
            assert!(pom.contains("<artifactId>egov-"), "{}", template.name);
        }
        assert!(fs::read_dir(&created).unwrap().count() > 0);
    }
    fs::remove_dir_all(&dir).unwrap();
}
