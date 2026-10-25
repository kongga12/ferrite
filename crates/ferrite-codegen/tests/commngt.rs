//! 공통컴포넌트 추가. 가짜 플러그인으로 가져오기·계획·쓰기를 보고, 공식 개발환경이 있으면 그 목록과
//! zip이 모두 맞는지와 실제로 넣어지는지 본다.

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use ferrite_codegen::commngt::{self, Catalog, State};
use ferrite_codegen::crud::Db;
use zip::write::SimpleFileOptions;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrite-commngt-{name}-{}", std::process::id()));
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

const CATALOG: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<components>
	<category name="#c1nam#" description="#c1des#" >
		<component name="#c1p1nam#" >
			<packageName      >#c1p1pac#</packageName      >
			<description      >#c1p1des#</description      >
		</component>
	</category>
	<category name="#c2nam#" description="#c2des#" >
		<component name="#c2p1nam#" >
			<packageName      >#c2p1pac#</packageName      >
			<dependencyPackage>#c2p1de1#</dependencyPackage>
			<dependencyPackage>#c2p1de2#</dependencyPackage>
			<useTable         >#c2p1use#</useTable         >
		</component>
	</category>
</components>
"##;

/// 공식 플러그인과 같은 모양: 목록, 이름표(CP949), 컴포넌트 zip.
fn plugin_files() -> Vec<(String, Vec<u8>)> {
    let labels = "c1nam=공통\nc1des=기본\nc1p1nam=공통\nc1p1pac=egovframework.com.cmm\nc1p1des=기본 묶음\n\
c2nam=온라인도움말\nc2des=도움말\nc2p1nam=FAQ관리\nc2p1pac=egovframework.com.uss.olh.faq\n\
c2p1de1=egovframework.com.cmm\nc2p1de2=egovframework.com.utl.fcc\nc2p1use=COMTNFAQINFO\n";
    let (labels, _, _) = encoding_rs::EUC_KR.encode(labels);
    let cmm = zip_of(&[
        ("pom.xml", b"<project><dependencies><dependency><groupId>org.egovframe.rte</groupId><artifactId>org.egovframe.rte.ptl.mvc</artifactId></dependency><dependency><groupId>commons-io</groupId><artifactId>commons-io</artifactId></dependency></dependencies></project>"),
        ("src/main/java/egovframework/com/cmm/Base.java", b"class Base {}"),
        ("src/main/resources/egovframework/egovProps/globals.properties", b"Globals.DbType=oracle\nGlobals.fileStorePath=/data/\n"),
        ("src/main/resources/egovframework/mapper/config/mapper-config.xml", b"<configuration>\n</configuration>\n"),
        ("src/script/ddl/oracle/cmm_create_oracle.sql", b"CREATE TABLE A;"),
        ("src/script/dml/oracle/cmm_insert_oracle.sql", b"INSERT;"),
        ("src/script/ddl/postgres/cmm_create_postgres.sql", b"CREATE TABLE A;"),
        ("../escape.txt", b"bad"),
    ]);
    let faq = zip_of(&[
        (
            "src/main/java/egovframework/com/uss/olh/faq/Faq.java",
            b"class Faq {}",
        ),
        (
            "src/main/resources/egovframework/mapper/config/mapper-config.xml",
            b"<configuration>\n</configuration>\n",
        ),
        (
            "src/script/ddl/oracle/uss.olh.faq_create_oracle.sql",
            b"CREATE TABLE F;",
        ),
    ]);
    let fcc = zip_of(&[(
        "src/main/java/egovframework/com/utl/fcc/Util.java",
        b"class Util {}",
    )]);
    vec![
        (
            "examples/components.xml".into(),
            CATALOG.as_bytes().to_vec(),
        ),
        (
            "egovframework/dev/imp/commngt/common/components_ko.properties".into(),
            labels.into_owned(),
        ),
        (
            "examples/egovframework.com.cmm.Wizard.4.2.0.zip".into(),
            cmm,
        ),
        (
            "examples/egovframework.com.uss.olh.faq.Wizard.4.2.0.zip".into(),
            faq,
        ),
        (
            "examples/egovframework.com.utl.fcc.Wizard.4.2.0.zip".into(),
            fcc,
        ),
    ]
}

fn plugin_jar() -> Vec<u8> {
    let files = plugin_files();
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    zip_of(&entries)
}

#[test]
fn 개발환경_폴더_jar_업데이트_사이트에서_가져온다() {
    let dir = temp("import");
    // 개발환경 폴더: <dev>/eclipse/plugins/egovframework.dev.imp.commngt_4.2.0.x/ (모바일용은 무시한다)
    let plugins = dir.join("dev/eclipse/plugins");
    let plugin = plugins.join("egovframework.dev.imp.commngt_4.2.0.202402021010");
    for (name, data) in plugin_files() {
        let path = plugin.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
    fs::create_dir_all(plugins.join("egovframework.mdev.imp.commngt_4.2.0/examples")).unwrap();
    let root = dir.join("cache");
    let catalog = commngt::import(&dir.join("dev"), &root).unwrap();
    assert_eq!(catalog.version, "4.2.0");
    assert_eq!(catalog.categories.len(), 2);
    assert_eq!(
        catalog.categories[1].name, "온라인도움말",
        "CP949 이름표를 읽는다"
    );
    let faq = catalog.component("egovframework.com.uss.olh.faq").unwrap();
    assert_eq!(faq.name, "FAQ관리");
    assert_eq!(faq.tables, ["COMTNFAQINFO"]);

    // 플러그인 jar 하나, 그 jar를 담은 업데이트 사이트 zip
    let jar = dir.join("egovframework.dev.imp.commngt_4.2.0.202402021010.jar");
    fs::write(&jar, plugin_jar()).unwrap();
    commngt::import(&jar, &root).unwrap();
    let site = dir.join("site.zip");
    fs::write(
        &site,
        zip_of(&[
            ("artifacts.jar", b"x"),
            (
                "plugins/egovframework.dev.imp.commngt_4.2.0.202402021010.jar",
                &plugin_jar(),
            ),
        ]),
    )
    .unwrap();
    let again = commngt::import(&site, &root).unwrap();
    assert_eq!(again.categories, catalog.categories);
    let imported = commngt::imported(&root);
    assert_eq!(imported.len(), 1, "같은 버전은 바꾼다");
    assert!(!root.join(".commngt-import").exists());

    let err = commngt::import(&dir.join("cache"), &root.join("other")).unwrap_err();
    assert!(err.contains("찾지 못했습니다"), "{err}");
    fs::remove_dir_all(&dir).unwrap();
}

fn imported_catalog(dir: &Path) -> Catalog {
    let jar = dir.join("commngt.jar");
    fs::write(&jar, plugin_jar()).unwrap();
    commngt::import(&jar, &dir.join("cache")).unwrap()
}

#[test]
fn 의존_컴포넌트까지_넣고_다른_기존_파일은_건너뛴다() {
    let dir = temp("apply");
    let catalog = imported_catalog(&dir);
    let project = dir.join("project");
    let write = |path: &str, text: &str| {
        let file = project.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    };
    write(
        "pom.xml",
        "<project><dependencies><dependency><groupId>org.egovframe.rte</groupId><artifactId>org.egovframe.rte.ptl.mvc</artifactId></dependency></dependencies></project>",
    );
    write(
        "src/main/resources/egovframework/egovProps/globals.properties",
        "Globals.DbType=oracle\n",
    );
    write(
        "src/main/resources/egovframework/mapper/config/mapper-config.xml",
        "<configuration>\r\n</configuration>\r\n",
    );

    let selected = vec!["egovframework.com.uss.olh.faq".to_owned()];
    assert_eq!(
        catalog.closure(&selected),
        [
            "egovframework.com.cmm",
            "egovframework.com.uss.olh.faq",
            "egovframework.com.utl.fcc"
        ],
        "기본 묶음이 먼저"
    );
    assert!(!commngt::installed(
        &project,
        "egovframework.com.uss.olh.faq"
    ));
    let plan = commngt::plan(&catalog, &selected, &project).unwrap();
    assert!(plan.missing.is_empty());
    assert!(
        plan.entries.iter().all(|e| !e.path.contains("..")),
        "폴더 밖을 가리키는 항목은 넣지 않는다"
    );
    let state = |path: &str| plan.entries.iter().find(|e| e.path == path).unwrap().state;
    assert_eq!(state("pom.xml"), State::Differs);
    assert_eq!(
        state("src/main/resources/egovframework/mapper/config/mapper-config.xml"),
        State::Same,
        "줄바꿈만 다르면 같다"
    );
    assert_eq!(
        state("src/main/java/egovframework/com/uss/olh/faq/Faq.java"),
        State::New
    );

    let outcome = commngt::apply(&plan, &project, false, Db::Oracle).unwrap();
    assert_eq!(outcome.created, plan.count(State::New));
    assert_eq!(outcome.skipped.len(), 2, "{:?}", outcome.skipped);
    assert_eq!(outcome.missing_dependencies, ["commons-io:commons-io"]);
    assert_eq!(
        outcome.missing_keys,
        [(
            "src/main/resources/egovframework/egovProps/globals.properties".to_owned(),
            vec!["Globals.fileStorePath".to_owned()]
        )]
    );
    assert_eq!(
        outcome.scripts,
        [
            "src/script/ddl/oracle/cmm_create_oracle.sql",
            "src/script/ddl/oracle/uss.olh.faq_create_oracle.sql",
            "src/script/dml/oracle/cmm_insert_oracle.sql",
        ]
    );
    assert!(commngt::installed(
        &project,
        "egovframework.com.uss.olh.faq"
    ));
    assert_eq!(
        fs::read_to_string(
            project.join("src/main/resources/egovframework/egovProps/globals.properties")
        )
        .unwrap(),
        "Globals.DbType=oracle\n",
        "기존 설정은 그대로"
    );

    // 덮어쓰기: 백업하고 바꾼다. pom.xml은 그래도 두고 빠진 의존성만 알린다.
    let plan = commngt::plan(&catalog, &selected, &project).unwrap();
    assert_eq!(plan.count(State::New), 0);
    let outcome = commngt::apply(&plan, &project, true, Db::Oracle).unwrap();
    let globals = "src/main/resources/egovframework/egovProps/globals.properties";
    assert_eq!(outcome.overwritten.len(), 1);
    assert_eq!(outcome.overwritten[0].0, globals);
    assert_eq!(
        fs::read_to_string(&outcome.overwritten[0].1).unwrap(),
        "Globals.DbType=oracle\n"
    );
    assert!(
        fs::read_to_string(project.join(globals))
            .unwrap()
            .contains("fileStorePath")
    );
    assert_eq!(outcome.skipped, ["pom.xml"]);
    assert!(
        fs::read_to_string(project.join("pom.xml"))
            .unwrap()
            .contains("rte.ptl.mvc")
    );
    assert!(
        !fs::read_to_string(project.join("pom.xml"))
            .unwrap()
            .contains("commons-io")
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn zip이_없는_의존_패키지는_알린다() {
    let dir = temp("missing");
    let catalog = imported_catalog(&dir);
    fs::remove_file(
        catalog
            .dir
            .join("egovframework.com.utl.fcc.Wizard.4.2.0.zip"),
    )
    .unwrap();
    let plan = commngt::plan(
        &catalog,
        &["egovframework.com.uss.olh.faq".to_owned()],
        &dir.join("p"),
    )
    .unwrap();
    assert_eq!(plan.missing, ["egovframework.com.utl.fcc"]);
    fs::remove_dir_all(&dir).unwrap();
}

/// 공식 개발환경(폴더, 플러그인 jar, 업데이트 사이트 zip, 개발환경 zip)마다 목록을 가져와, 모든
/// 컴포넌트를 빈 프로젝트에 넣어 본다. 여러 개는 경로 구분자(Windows `;`)로 잇는다.
/// `FERRITE_EGOV_DEV=<경로> cargo test --release -p ferrite-codegen --test commngt -- --ignored --nocapture`
#[test]
#[ignore = "공식 개발환경이 필요하다(FERRITE_EGOV_DEV)"]
fn 공식_개발환경의_공통컴포넌트를_모두_넣는다() {
    let Some(sources) = std::env::var_os("FERRITE_EGOV_DEV") else {
        eprintln!("FERRITE_EGOV_DEV가 없어 건너뜁니다");
        return;
    };
    let dir = temp("official");
    for source in std::env::split_paths(&sources) {
        let catalog = commngt::import(&source, &dir.join("cache"))
            .unwrap_or_else(|e| panic!("{}: {e}", source.display()));
        let packages: Vec<String> = catalog
            .categories
            .iter()
            .flat_map(|c| &c.components)
            .map(|c| c.package.clone())
            .collect();
        eprintln!(
            "{}: {} {}개 분류, 컴포넌트 {}개",
            source.display(),
            catalog.version,
            catalog.categories.len(),
            packages.len()
        );
        assert!(packages.len() > 100);
        assert!(
            catalog
                .categories
                .iter()
                .flat_map(|c| &c.components)
                .all(|c| !c.name.contains('#') && !c.name.contains("&#"))
        );

        let project = dir.join(format!("project-{}", catalog.version));
        let plan = commngt::plan(&catalog, &packages, &project).unwrap();
        assert!(
            plan.missing.is_empty(),
            "zip이 없는 패키지: {:?}",
            plan.missing
        );
        assert_eq!(plan.count(State::New), plan.entries.len());
        let outcome = commngt::apply(&plan, &project, false, Db::Oracle).unwrap();
        eprintln!(
            "  패키지 {}개, 파일 {}개, 스크립트 {}개",
            plan.packages.len(),
            outcome.created,
            outcome.scripts.len()
        );
        assert!(
            outcome.scripts[0].ends_with("cmm_create_oracle.sql"),
            "{:?}",
            outcome.scripts
        );
        assert!(project.join("pom.xml").is_file());
        for package in &packages {
            assert!(
                commngt::installed(&project, package)
                    // 소스 없이 화면·설정만 있는 컴포넌트도 있다.
                    || plan.entries.iter().any(|e| &e.package == package),
                "{package}"
            );
        }
        let again = commngt::plan(&catalog, &packages, &project).unwrap();
        assert_eq!(
            again.count(State::Same),
            again.entries.len(),
            "다시 넣으면 모두 같다"
        );
    }
    fs::remove_dir_all(&dir).unwrap();
}
