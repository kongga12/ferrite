//! CRUD 생성 결과를 Phase 2b 진단(매퍼 SQL Injection·참조, JSP XSS)에 돌려 경고가 없는지 보고,
//! DB·실행환경 버전에 따라 달라지는 부분을 확인한다.

use ferrite_codegen::crud::{self, Db, GeneratedFile, JUnit, Layout, Part, Rte, Spec};
use ferrite_codegen::ddl;

const DDL: &str = r#"
CREATE TABLE TB_BOARD (
    BOARD_ID   NUMBER(10)     NOT NULL,
    TITLE      VARCHAR2(200)  NOT NULL,
    CONTENT    CLOB,
    WRITER_NM  VARCHAR2(50),
    VIEW_CNT   NUMBER(9),
    ATTACH     BLOB,
    REG_DT     DATE,
    CONSTRAINT PK_TB_BOARD PRIMARY KEY (BOARD_ID)
);
COMMENT ON TABLE TB_BOARD IS '게시판 <공지>';
COMMENT ON COLUMN TB_BOARD.TITLE IS '제목 "필수" */';
"#;

fn generate(rte: Rte, db: Db, junit: JUnit) -> Vec<GeneratedFile> {
    let table = ddl::parse(DDL).unwrap();
    let mut spec = Spec::from_table(&table, "egovframework.example.board", rte, db, junit);
    spec.author = "홍길동".into();
    spec.date = "2026-10-25".into();
    crud::generate(&spec, &Layout::default(), &Part::ALL, None).unwrap()
}

fn file<'a>(files: &'a [GeneratedFile], suffix: &str) -> &'a str {
    &files
        .iter()
        .find(|f| {
            f.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with(suffix)
        })
        .unwrap_or_else(|| panic!("{suffix} 없음"))
        .content
}

#[test]
fn 만든_매퍼와_jsp는_2b_진단에_걸리지_않는다() {
    for db in Db::ALL {
        let files = generate(Rte::V3, db, JUnit::V4);
        let xml = file(&files, &format!("Board_SQL_{}.xml", db.suffix()));
        let mapper = ferrite_egov::mapper::parse(xml).expect("매퍼로 읽혀야 한다");
        let mut index = ferrite_egov::mapper::Index::default();
        index.mappers.insert("Board_SQL.xml".into(), mapper.clone());
        let findings = ferrite_egov::mapper::diagnose(&mapper, &index);
        assert!(findings.is_empty(), "{db:?}: {findings:?}\n{xml}");
        assert_eq!(
            mapper.namespace.as_deref(),
            Some("egovframework.example.board.service.impl.BoardMapper")
        );
        assert!(!xml.contains("${"), "문자열 치환 없음");
    }
    let files = generate(Rte::V4, Db::Oracle, JUnit::V5);
    for jsp in ["boardList.jsp", "boardRegister.jsp"] {
        let findings = ferrite_egov::jsp::diagnose(file(&files, jsp));
        assert!(findings.is_empty(), "{jsp}: {findings:?}");
    }
}

#[test]
fn db마다_페이징과_like가_다르다() {
    let oracle = generate(Rte::V3, Db::Oracle, JUnit::V4);
    let xml = file(&oracle, "Board_SQL_oracle.xml");
    assert!(xml.contains("ROWNUM &lt;= #{lastIndex}"), "{xml}");
    assert!(xml.contains("'%' || #{searchKeyword} || '%'"));
    assert!(xml.contains("#{title,jdbcType=VARCHAR}"));
    assert!(xml.contains("#{content,jdbcType=CLOB}"));
    assert!(xml.contains("<id property=\"boardId\" column=\"BOARD_ID\"/>"));
    let maria = generate(Rte::V3, Db::MariaDb, JUnit::V4);
    let xml = file(&maria, "Board_SQL_mysql.xml");
    assert!(xml.contains("LIMIT #{firstIndex}, #{recordCountPerPage}"));
    assert!(xml.contains("CONCAT('%', #{searchKeyword}, '%')"));
    let pg = generate(Rte::V3, Db::PostgreSql, JUnit::V4);
    assert!(
        file(&pg, "Board_SQL_postgres.xml")
            .contains("LIMIT #{recordCountPerPage} OFFSET #{firstIndex}")
    );
}

#[test]
fn 실행환경_버전과_인코딩과_주석을_반영한다() {
    let v3 = generate(Rte::V3, Db::Oracle, JUnit::V4);
    assert!(
        file(&v3, "BoardMapper.java")
            .contains("import egovframework.rte.psl.dataaccess.mapper.Mapper;")
    );
    assert!(file(&v3, "BoardServiceImplTest.java").contains("import org.junit.Test;"));
    let v4 = generate(Rte::V4, Db::Oracle, JUnit::V5);
    assert!(
        file(&v4, "BoardServiceImpl.java")
            .contains("import org.egovframe.rte.fdl.cmmn.EgovAbstractServiceImpl;")
    );
    assert!(file(&v4, "BoardServiceImplTest.java").contains("import org.junit.jupiter.api.Test;"));

    let vo = file(&v3, "BoardVO.java");
    assert!(vo.contains("private Long boardId;"), "NUMBER(10) → Long");
    assert!(
        vo.contains("private Integer viewCnt;"),
        "NUMBER(9) → Integer"
    );
    assert!(vo.contains("import java.sql.Date;"));
    assert!(vo.contains("private byte[] attach;"));
    assert!(
        vo.contains("/** 제목 \"필수\" * / */"),
        "주석이 */로 끝나지 않게"
    );
    let controller = file(&v3, "BoardController.java");
    assert!(
        controller
            .contains(r#"rejectValue("title", "required", "제목 \"필수\" */을(를) 입력하세요.")"#),
        "Java 문자열 이스케이프"
    );
    assert!(
        controller.contains("length() > 200"),
        "VARCHAR2(200) 길이 검증"
    );
    let list = file(&v3, "boardList.jsp");
    assert!(
        list.contains("<title>게시판 &lt;공지&gt; 목록</title>"),
        "HTML 이스케이프"
    );
    assert!(!list.contains("attach"), "BLOB은 목록에서 뺀다");
    assert!(list.contains("pageEncoding=\"UTF-8\""));

    let table = ddl::parse(DDL).unwrap();
    let mut spec = Spec::from_table(&table, "egov.board", Rte::V3, Db::Oracle, JUnit::V4);
    spec.encoding = ferrite_core::text_file::Encoding::Cp949;
    let files = crud::generate(
        &spec,
        &Layout::default(),
        &[Part::JspList, Part::MapperXml],
        None,
    )
    .unwrap();
    assert!(files[0].content.contains("pageEncoding=\"MS949\""));
    assert!(
        files[1]
            .content
            .starts_with("<?xml version=\"1.0\" encoding=\"EUC-KR\"?>")
    );
}

#[test]
fn 프로젝트_템플릿이_있으면_그것을_쓴다() {
    let dir = std::env::temp_dir().join(format!("ferrite-codegen-override-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(Part::Vo.template_name()),
        "{{className}}:{{#each attributes}}{{concat \"#{\" ccName \"}\"}}{{#unless @last}},{{/unless}}{{/each}}",
    )
    .unwrap();
    let table = ddl::parse("CREATE TABLE T (A_B INT PRIMARY KEY, C VARCHAR(9))").unwrap();
    let spec = Spec::from_table(&table, "x.y", Rte::V4, Db::PostgreSql, JUnit::V5);
    let files = crud::generate(
        &spec,
        &Layout::default(),
        &[Part::Vo, Part::Service],
        Some(&dir),
    )
    .unwrap();
    assert_eq!(files[0].content, "T:#{aB},#{c}");
    assert!(
        files[1].content.contains("interface TService"),
        "없는 템플릿은 내장"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 공식 VS Code 확장(egovframe-vscode-initializr)의 CRUD 템플릿을 그대로 렌더링해 본다.
/// 확장 파일은 저장소에 넣지 않으므로 경로를 환경 변수로 받는다.
/// `FERRITE_EGOV_VSIX=ref/egovframe-vscode-initializr-5.0.7.vsix cargo test -p ferrite-codegen -- --ignored`
#[test]
#[ignore = "공식 확장 vsix가 필요하다(FERRITE_EGOV_VSIX)"]
fn 공식_확장_템플릿을_렌더링한다() {
    use std::io::Read;

    let Some(vsix) = std::env::var_os("FERRITE_EGOV_VSIX") else {
        eprintln!("FERRITE_EGOV_VSIX가 없어 건너뜁니다");
        return;
    };
    let dir = std::env::temp_dir().join(format!("ferrite-codegen-official-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(vsix).unwrap()).unwrap();
    for part in Part::ALL {
        let name = format!("extension/templates/code/{}", part.template_name());
        if let Ok(mut entry) = archive.by_name(&name) {
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            std::fs::write(dir.join(part.template_name()), text).unwrap();
        }
    }
    let table = ddl::parse(DDL).unwrap();
    let spec = Spec::from_table(
        &table,
        "egovframework.example.board",
        Rte::V4,
        Db::Oracle,
        JUnit::V5,
    );
    let files = crud::generate(&spec, &Layout::default(), &Part::ALL, Some(&dir)).unwrap();
    let xml = file(&files, "Board_SQL_oracle.xml");
    let update = &xml[xml.find("<update").unwrap()..xml.find("</update>").unwrap()];
    // SET 절: 공백을 접어 보면 첫 항목에만 쉼표가 없어야 한다(setVar로 첫 항목을 가린다).
    let set = &update[update.find("SET").unwrap() + 3..update.find("WHERE").unwrap()];
    let set = set.split_whitespace().collect::<Vec<_>>().join(" ");
    eprintln!("{set}");
    assert_eq!(
        set,
        "TITLE=#{title} , CONTENT=#{content} , WRITER_NM=#{writerNm} , VIEW_CNT=#{viewCnt} , ATTACH=#{attach} , REG_DT=#{regDt}"
    );
    let mapper = ferrite_egov::mapper::parse(xml).unwrap();
    assert!(mapper.substitutions.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 실제 프로젝트(코퍼스 바로 아래 폴더)마다 기본값을 어떻게 잡는지 본다(눈으로 확인하는 통계).
/// `FERRITE_EGOV_CORPUS=<경로> cargo test -p ferrite-codegen --test crud 코퍼스 -- --ignored --nocapture`
#[test]
#[ignore = "로컬 egov 코퍼스가 필요하다(FERRITE_EGOV_CORPUS)"]
fn 코퍼스_프로젝트의_기본값() {
    let Some(root) = std::env::var_os("FERRITE_EGOV_CORPUS") else {
        eprintln!("FERRITE_EGOV_CORPUS가 없어 건너뜁니다");
        return;
    };
    for project in std::fs::read_dir(root).unwrap().flatten() {
        let path = project.path();
        if !path.join("pom.xml").is_file() && !path.join("build.gradle").is_file() {
            continue;
        }
        let found = ferrite_codegen::project::defaults(&path);
        eprintln!(
            "{}: {:?} {:?} {:?} {:?} 매퍼 {} / {} JSP {} 패키지 {}",
            project.file_name().to_string_lossy(),
            found.rte,
            found.db,
            found.junit,
            found.encoding,
            found.layout.mapper.display(),
            found.layout.mapper_file,
            found.layout.jsp.display(),
            found.base_package
        );
    }
}
