//! 설정 파일 생성: Datasource·Transaction·ID 생성·Logging·Scheduling·Cache·Property.
//!
//! 종류마다 입력 칸(이름·종류·기본값)과 형식(XML·Java Config·YAML·properties)별 템플릿을 표로 둔다.
//! 화면은 이 표로 입력 칸을 그린다. 입력 칸 이름(`txtDatasourceName` 등)과 고르는 값(`DBCP`,
//! `External File` 등)은 eGovFrame 공식 VS Code 확장과 같아, 공식 템플릿이나 고친 템플릿을 프로젝트의
//! `.ferrite/templates/config/<폴더>/<파일>`에 두면 그것으로 만든다.
//!
//! 공식 확장과 다른 점(Ferrite 템플릿):
//! - 실행환경 3.x(`egovframework.rte`)·4.x(`org.egovframe.rte`)를 함께 지원한다.
//! - Java Config는 `javax.sql.DataSource`를 쓴다(공식 확장은 없는 `jakarta.sql.DataSource`를 쓴다).
//! - 로깅은 log4j2만 만든다(공식 확장의 Java 로깅은 지원이 끝난 log4j 1을 쓴다). 로깅에는 Java 형식이 없다.
//! - 캐시는 Ehcache 2(Spring 4.3·5.3이 지원하는 `net.sf.ehcache`)로 만든다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::crud::{Db, GeneratedFile, Rte, js_compatible, registry};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Xml,
    JavaConfig,
    Yaml,
    Properties,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Xml => "XML",
            Format::JavaConfig => "Java Config",
            Format::Yaml => "YAML",
            Format::Properties => "properties",
        }
    }

    /// 공식 확장의 템플릿 파일 이름 끝(`datasource-java.hbs`).
    fn suffix(self) -> &'static str {
        match self {
            Format::Xml => "",
            Format::JavaConfig => "-java",
            Format::Yaml => "-yaml",
            Format::Properties => "-properties",
        }
    }
}

/// 입력 칸 하나.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    /// 문맥 이름(공식 확장과 같다).
    pub key: &'static str,
    pub label: &'static str,
    pub kind: InputKind,
    /// 다른 칸이 이 값일 때만 보인다(`("rdoType", "External File")`).
    pub when: Option<(&'static str, &'static str)>,
}

#[derive(Clone, Copy, Debug)]
pub enum InputKind {
    Text,
    /// 고르는 값(첫 값이 기본).
    Choice(&'static [&'static str]),
    Check,
}

/// 설정 종류 하나.
#[derive(Clone, Copy, Debug)]
pub struct Kind {
    pub category: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// 공식 확장의 템플릿 폴더·파일 이름(확장자 제외).
    pub folder: &'static str,
    pub template: &'static str,
    pub formats: &'static [Format],
    /// XML 파일 이름(확장자 제외)·Java 클래스 이름의 기본값.
    pub xml_name: &'static str,
    pub java_name: &'static str,
    pub inputs: &'static [Input],
}

const fn text(key: &'static str, label: &'static str) -> Input {
    Input {
        key,
        label,
        kind: InputKind::Text,
        when: None,
    }
}

const fn choice(key: &'static str, label: &'static str, values: &'static [&'static str]) -> Input {
    Input {
        key,
        label,
        kind: InputKind::Choice(values),
        when: None,
    }
}

const fn check(key: &'static str, label: &'static str) -> Input {
    Input {
        key,
        label,
        kind: InputKind::Check,
        when: None,
    }
}

const fn when(input: Input, key: &'static str, value: &'static str) -> Input {
    Input {
        when: Some((key, value)),
        ..input
    }
}

const XML_JAVA: &[Format] = &[Format::Xml, Format::JavaConfig];
const LOGGING: &[Format] = &[Format::Xml, Format::Yaml, Format::Properties];
const PROPAGATION: &[&str] = &[
    "REQUIRED",
    "SUPPORTS",
    "MANDATORY",
    "REQUIRES_NEW",
    "NOT_SUPPORTED",
    "NEVER",
    "NESTED",
];
const ISOLATION: &[&str] = &[
    "DEFAULT",
    "READ_UNCOMMITTED",
    "READ_COMMITTED",
    "REPEATABLE_READ",
    "SERIALIZABLE",
];
/// 트랜잭션 공통 입력(어드바이스·포인트컷).
const TX_ADVICE: [Input; 11] = [
    check("chkAnnotationTransaction", "@Transactional 사용"),
    check("chkAopConfigTransaction", "AOP로 메서드에 적용"),
    when(
        text("txtAdviceName", "어드바이스 이름"),
        "chkAopConfigTransaction",
        "true",
    ),
    when(
        text("txtMethodName", "메서드 이름 패턴"),
        "chkAopConfigTransaction",
        "true",
    ),
    when(
        choice("cmbPropagation", "전파", PROPAGATION),
        "chkAopConfigTransaction",
        "true",
    ),
    when(
        choice("cmbIsolation", "격리 수준", ISOLATION),
        "chkAopConfigTransaction",
        "true",
    ),
    when(
        check("chkReadOnly", "읽기 전용"),
        "chkAopConfigTransaction",
        "true",
    ),
    when(
        text("txtTimeout", "제한 시간(초, -1은 없음)"),
        "chkAopConfigTransaction",
        "true",
    ),
    when(
        text("txtRollbackFor", "롤백할 예외"),
        "chkAopConfigTransaction",
        "true",
    ),
    when(
        text("txtPointCutName", "포인트컷 이름"),
        "chkAopConfigTransaction",
        "true",
    ),
    when(
        text("txtPointCutExpression", "포인트컷 식"),
        "chkAopConfigTransaction",
        "true",
    ),
];

macro_rules! inputs {
    ($($item:expr),* $(,)?) => { &[$($item),*] };
}

pub const KINDS: &[Kind] = &[
    Kind {
        category: "Datasource",
        label: "데이터소스(DBCP·C3P0·JDBC)",
        description: "커넥션 풀 데이터소스. 값에 ${Globals.Url} 같은 플레이스홀더를 써도 된다.",
        folder: "datasource",
        template: "datasource",
        formats: XML_JAVA,
        xml_name: "context-datasource",
        java_name: "EgovConfigDatasource",
        inputs: inputs![
            choice("rdoType", "풀 종류", &["DBCP", "C3P0", "JDBC"]),
            text("txtDatasourceName", "빈 이름"),
            text("txtDriver", "드라이버 클래스"),
            text("txtUrl", "JDBC URL"),
            text("txtUser", "사용자"),
            text("txtPasswd", "비밀번호"),
        ],
    },
    Kind {
        category: "Datasource",
        label: "JNDI 데이터소스",
        description: "WAS(JEUS·WebLogic·Tomcat)가 제공하는 데이터소스를 JNDI로 찾는다.",
        folder: "datasource",
        template: "jndiDatasource",
        formats: XML_JAVA,
        xml_name: "context-datasource",
        java_name: "EgovConfigJndiDatasource",
        inputs: inputs![
            text("txtDatasourceName", "빈 이름"),
            text("txtJndiName", "JNDI 이름"),
            choice(
                "cboResourceRef",
                "java:comp/env/ 붙이기(Tomcat은 true, JEUS·WebLogic 전역 이름은 false)",
                &["true", "false"]
            ),
        ],
    },
    Kind {
        category: "Transaction",
        label: "데이터소스 트랜잭션",
        description: "DataSourceTransactionManager. MyBatis·JDBC 트랜잭션.",
        folder: "transaction",
        template: "datasource",
        formats: XML_JAVA,
        xml_name: "context-transaction",
        java_name: "EgovConfigTransaction",
        inputs: &[
            text("txtTransactionName", "트랜잭션 관리자 이름"),
            text("txtDataSourceName", "데이터소스 빈"),
            TX_ADVICE[0],
            TX_ADVICE[1],
            TX_ADVICE[2],
            TX_ADVICE[3],
            TX_ADVICE[4],
            TX_ADVICE[5],
            TX_ADVICE[6],
            TX_ADVICE[7],
            TX_ADVICE[8],
            TX_ADVICE[9],
            TX_ADVICE[10],
        ],
    },
    Kind {
        category: "Transaction",
        label: "JPA 트랜잭션",
        description: "엔티티 매니저 팩토리(Hibernate)와 JpaTransactionManager.",
        folder: "transaction",
        template: "jpa",
        formats: XML_JAVA,
        xml_name: "context-jpa",
        java_name: "EgovConfigJpa",
        inputs: &[
            text("txtTransactionName", "트랜잭션 관리자 이름"),
            text("txtDataSourceName", "데이터소스 빈"),
            text("txtEntityManagerFactory", "엔티티 매니저 팩토리 이름"),
            text("txtPackagesToScan", "엔티티 패키지"),
            text("cmbDialectName", "Hibernate 방언"),
            text(
                "txtSpringDataJpaRepositoriesPackage",
                "Spring Data 리포지토리 패키지(없으면 비움)",
            ),
            TX_ADVICE[0],
            TX_ADVICE[1],
            TX_ADVICE[2],
            TX_ADVICE[3],
            TX_ADVICE[4],
            TX_ADVICE[5],
            TX_ADVICE[6],
            TX_ADVICE[7],
            TX_ADVICE[8],
            TX_ADVICE[9],
            TX_ADVICE[10],
        ],
    },
    Kind {
        category: "Transaction",
        label: "JTA 트랜잭션",
        description: "WAS의 JTA 트랜잭션 관리자(여러 자원에 걸친 트랜잭션).",
        folder: "transaction",
        template: "jta",
        formats: XML_JAVA,
        xml_name: "context-transaction",
        java_name: "EgovConfigJtaTransaction",
        inputs: &[
            text("txtTransactionName", "트랜잭션 관리자 이름"),
            text("txtGlobalTimeout", "기본 제한 시간(초)"),
            TX_ADVICE[0],
            TX_ADVICE[1],
            TX_ADVICE[2],
            TX_ADVICE[3],
            TX_ADVICE[4],
            TX_ADVICE[5],
            TX_ADVICE[6],
            TX_ADVICE[7],
            TX_ADVICE[8],
            TX_ADVICE[9],
            TX_ADVICE[10],
        ],
    },
    Kind {
        category: "ID 생성",
        label: "시퀀스 ID 생성",
        description: "DB 시퀀스로 ID를 만든다(EgovSequenceIdGnrServiceImpl). IDENTITY가 없는 Oracle 11g에 맞다.",
        folder: "idGeneration",
        template: "xml-id-gnr-sequence-service",
        formats: XML_JAVA,
        xml_name: "context-idgen",
        java_name: "EgovConfigIdGeneration",
        inputs: inputs![
            text("txtIdServiceName", "서비스 이름"),
            text("txtDatasourceName", "데이터소스 빈"),
            text("txtQuery", "다음 값 조회 SQL"),
            choice("rdoIdType", "ID 타입", &["BigDecimal", "Long"]),
        ],
    },
    Kind {
        category: "ID 생성",
        label: "테이블 ID 생성",
        description: "ID 테이블(TABLE_NAME·NEXT_ID)로 ID를 만든다(EgovTableIdGnrServiceImpl). 접두어·자릿수 전략을 붙일 수 있다.",
        folder: "idGeneration",
        template: "xml-id-gnr-table-service",
        formats: XML_JAVA,
        xml_name: "context-idgen",
        java_name: "EgovConfigIdGeneration",
        inputs: inputs![
            text("txtIdServiceName", "서비스 이름"),
            text("txtDatasourceName", "데이터소스 빈"),
            text("txtTable", "ID 테이블"),
            text("txtTableNameFieldValue", "TABLE_NAME 값"),
            text("txtBlockSize", "블록 크기"),
            check("chkStrategy", "ID 모양 전략 사용"),
            when(
                text("txtStrategyName", "전략 빈 이름"),
                "chkStrategy",
                "true"
            ),
            when(text("txtPrefix", "접두어"), "chkStrategy", "true"),
            when(text("txtCipers", "전체 자릿수"), "chkStrategy", "true"),
            when(text("txtFillChar", "채울 글자"), "chkStrategy", "true"),
        ],
    },
    Kind {
        category: "ID 생성",
        label: "UUID 생성",
        description: "UUID로 ID를 만든다(EgovUUIdGnrServiceImpl). 주소(MAC·IP)를 넣으면 그것으로 만든다.",
        folder: "idGeneration",
        template: "xml-id-gnr-uuid-service",
        formats: XML_JAVA,
        xml_name: "context-idgen",
        java_name: "EgovConfigIdGeneration",
        inputs: inputs![
            text("txtIdServiceName", "서비스 이름"),
            choice("rdoIdType", "방식", &["Random", "Address"]),
            when(
                text("txtAddress", "주소(MAC 또는 IP)"),
                "rdoIdType",
                "Address"
            ),
        ],
    },
    Kind {
        category: "Logging",
        label: "콘솔 로그",
        description: "log4j2 콘솔 출력.",
        folder: "logging",
        template: "console",
        formats: LOGGING,
        xml_name: "log4j2",
        java_name: "",
        inputs: inputs![
            text("txtAppenderName", "어펜더 이름"),
            text("txtConversionPattern", "출력 모양")
        ],
    },
    Kind {
        category: "Logging",
        label: "파일 로그",
        description: "log4j2 파일 출력.",
        folder: "logging",
        template: "file",
        formats: LOGGING,
        xml_name: "log4j2",
        java_name: "",
        inputs: inputs![
            text("txtAppenderName", "어펜더 이름"),
            text("txtConversionPattern", "출력 모양"),
            text("txtLogFileName", "로그 파일"),
            choice("cboAppend", "이어 쓰기", &["true", "false"]),
        ],
    },
    Kind {
        category: "Logging",
        label: "크기별 롤링 파일 로그",
        description: "log4j2 RollingFile: 파일이 정한 크기를 넘으면 번호를 붙여 넘긴다.",
        folder: "logging",
        template: "rollingFile",
        formats: LOGGING,
        xml_name: "log4j2",
        java_name: "",
        inputs: inputs![
            text("txtAppenderName", "어펜더 이름"),
            text("txtConversionPattern", "출력 모양"),
            text("txtLogFileName", "로그 파일"),
            text("txtLogFileNamePattern", "넘긴 파일 이름(%i는 번호)"),
            text("txtMaxFileSize", "최대 크기"),
            text("txtMaxIndex", "보관 개수"),
        ],
    },
    Kind {
        category: "Logging",
        label: "날짜별 롤링 파일 로그",
        description: "log4j2 RollingFile: 정한 주기(날짜 패턴의 단위)마다 넘긴다.",
        folder: "logging",
        template: "timeBasedRollingFile",
        formats: LOGGING,
        xml_name: "log4j2",
        java_name: "",
        inputs: inputs![
            text("txtAppenderName", "어펜더 이름"),
            text("txtConversionPattern", "출력 모양"),
            text("txtLogFileName", "로그 파일"),
            text("txtLogFileNamePattern", "넘긴 파일 이름(%d{…}는 날짜)"),
            text("txtInterval", "주기"),
            choice("cboModulate", "경계에 맞추기", &["true", "false"]),
        ],
    },
    Kind {
        category: "Logging",
        label: "DB(JDBC) 로그",
        description: "log4j2 JDBC 어펜더: 로그를 표에 쓴다(LOG_DATE·LOG_LEVEL·LOGGER·MESSAGE·EXCEPTION 열).",
        folder: "logging",
        template: "jdbc",
        formats: LOGGING,
        xml_name: "log4j2",
        java_name: "",
        inputs: inputs![
            text("txtAppenderName", "어펜더 이름"),
            text("txtTableName", "로그 테이블"),
            choice(
                "rdoConnectionType",
                "연결 방식",
                &["DriverManager", "ConnectionFactory"]
            ),
            when(
                text("txtDriver", "드라이버 클래스"),
                "rdoConnectionType",
                "DriverManager"
            ),
            when(
                text("txtUrl", "JDBC URL"),
                "rdoConnectionType",
                "DriverManager"
            ),
            when(
                text("txtUser", "사용자"),
                "rdoConnectionType",
                "DriverManager"
            ),
            when(
                text("txtPasswd", "비밀번호"),
                "rdoConnectionType",
                "DriverManager"
            ),
            when(
                text("txtConnectionFactoryClass", "팩토리 클래스"),
                "rdoConnectionType",
                "ConnectionFactory"
            ),
            when(
                text("txtConnectionFactoryMethod", "팩토리 메서드"),
                "rdoConnectionType",
                "ConnectionFactory"
            ),
        ],
    },
    Kind {
        category: "Property",
        label: "프로퍼티 서비스",
        description: "EgovPropertyService: 키·값을 직접 넣거나 외부 프로퍼티 파일을 읽는다.",
        folder: "property",
        template: "property",
        formats: XML_JAVA,
        xml_name: "context-properties",
        java_name: "EgovConfigProperties",
        inputs: inputs![
            text("txtPropertyServiceName", "서비스 이름"),
            choice("rdoType", "방식", &["Internal Properties", "External File"]),
            when(text("txtKey", "키"), "rdoType", "Internal Properties"),
            when(text("txtValue", "값"), "rdoType", "Internal Properties"),
            when(
                text("txtPropertyFile", "프로퍼티 파일"),
                "rdoType",
                "External File"
            ),
            when(
                choice("cboEncoding", "인코딩", &["UTF-8", "EUC-KR", "MS949"]),
                "rdoType",
                "External File"
            ),
        ],
    },
    Kind {
        category: "Scheduling",
        label: "Job(QuartzJobBean 클래스)",
        description: "JobDetailFactoryBean: QuartzJobBean을 상속한 클래스를 Job으로 만든다.",
        folder: "scheduling",
        template: "beanJob",
        formats: XML_JAVA,
        xml_name: "context-scheduling",
        java_name: "EgovConfigScheduling",
        inputs: inputs![
            text("txtJobName", "Job 이름"),
            text("txtServiceClass", "Job 클래스"),
            check("chkProperty", "Job 데이터 넘기기"),
            when(
                text("txtPropertyName", "데이터 이름"),
                "chkProperty",
                "true"
            ),
            when(text("txtPropertyValue", "데이터 값"), "chkProperty", "true"),
        ],
    },
    Kind {
        category: "Scheduling",
        label: "Job(빈의 메서드 호출)",
        description: "MethodInvokingJobDetailFactoryBean: 빈의 메서드를 Job으로 부른다.",
        folder: "scheduling",
        template: "methodJob",
        formats: XML_JAVA,
        xml_name: "context-scheduling",
        java_name: "EgovConfigScheduling",
        inputs: inputs![
            text("txtJobName", "Job 이름"),
            text("txtServiceName", "대상 빈 이름"),
            text("txtServiceMethod", "메서드"),
            choice("cboConcurrent", "동시 실행", &["false", "true"]),
        ],
    },
    Kind {
        category: "Scheduling",
        label: "Simple 트리거",
        description: "SimpleTriggerFactoryBean: 정한 간격마다 Job을 실행한다.",
        folder: "scheduling",
        template: "simpleTrigger",
        formats: XML_JAVA,
        xml_name: "context-scheduling",
        java_name: "EgovConfigSchedulingTrigger",
        inputs: inputs![
            text("txtTriggerName", "트리거 이름"),
            text("txtJobName", "Job 이름"),
            text("txtStartDelay", "시작 지연(ms)"),
            text("txtRepeatInterval", "반복 간격(ms)"),
        ],
    },
    Kind {
        category: "Scheduling",
        label: "Cron 트리거",
        description: "CronTriggerFactoryBean: cron 식에 맞춰 Job을 실행한다.",
        folder: "scheduling",
        template: "cronTrigger",
        formats: XML_JAVA,
        xml_name: "context-scheduling",
        java_name: "EgovConfigSchedulingTrigger",
        inputs: inputs![
            text("txtTriggerName", "트리거 이름"),
            text("txtJobName", "Job 이름"),
            text("txtCronExpression", "cron 식"),
        ],
    },
    Kind {
        category: "Scheduling",
        label: "스케줄러",
        description: "SchedulerFactoryBean: 트리거를 등록해 실행한다.",
        folder: "scheduling",
        template: "scheduler",
        formats: XML_JAVA,
        xml_name: "context-scheduling",
        java_name: "EgovConfigScheduler",
        inputs: inputs![
            text("txtSchedulerName", "스케줄러 이름"),
            text("txtTriggerName", "트리거 이름")
        ],
    },
    Kind {
        category: "Cache",
        label: "Ehcache 설정(ehcache.xml)",
        description: "Ehcache 2 캐시 정의(기본 캐시와 캐시 하나).",
        folder: "cache",
        template: "cache",
        formats: &[Format::Xml],
        xml_name: "ehcache",
        java_name: "",
        inputs: inputs![
            text("txtDiskStore", "디스크 저장 위치"),
            text("txtDftHeapEntries", "기본: 힙 최대 개수"),
            choice("txtDftEternal", "기본: 만료 없음", &["false", "true"]),
            text("txtDftIdleTime", "기본: 미사용 만료(초)"),
            text("txtDftLiveTime", "기본: 생존 만료(초)"),
            text("txtCacheName", "캐시 이름"),
            text("txtHeapEntries", "힙 최대 개수"),
            choice("txtEternal", "만료 없음", &["false", "true"]),
            text("txtIdleTime", "미사용 만료(초)"),
            text("txtLiveTime", "생존 만료(초)"),
        ],
    },
    Kind {
        category: "Cache",
        label: "Spring 캐시(Ehcache)",
        description: "@Cacheable을 켜고 ehcache.xml을 읽는 CacheManager를 만든다.",
        folder: "cache",
        template: "ehcacheConfigForSpring",
        formats: XML_JAVA,
        xml_name: "context-cache",
        java_name: "EgovConfigCache",
        inputs: inputs![
            text("txtConfigLocation", "ehcache.xml 위치"),
            text(
                "txtComponentScanBasePackage",
                "컴포넌트 스캔 패키지(없으면 비움)"
            )
        ],
    },
];

/// 입력 칸의 기본값. DB에 따라 드라이버·URL·시퀀스 SQL·방언이 다르다. `placeholders`는 프로젝트의
/// 데이터소스 프로퍼티 키(`Globals.DriverClassName` 등)이고, 있으면 값 대신 `${키}`를 넣는다.
pub fn default_value(key: &str, db: Db, base_package: &str, placeholders: &Placeholders) -> String {
    let placeholder = |name: &Option<String>| name.as_ref().map(|k| format!("${{{k}}}"));
    let value = match key {
        "rdoType" | "rdoIdType" | "rdoConnectionType" => return String::new(),
        "txtDatasourceName" | "txtDataSourceName" => "dataSource",
        "txtDriver" => {
            return placeholder(&placeholders.driver).unwrap_or_else(|| {
                match db {
                    Db::Oracle => "oracle.jdbc.OracleDriver",
                    Db::Tibero => "com.tmax.tibero.jdbc.TbDriver",
                    Db::PostgreSql => "org.postgresql.Driver",
                    Db::MariaDb => "org.mariadb.jdbc.Driver",
                    Db::Cubrid => "cubrid.jdbc.driver.CUBRIDDriver",
                }
                .into()
            });
        }
        "txtUrl" => {
            return placeholder(&placeholders.url).unwrap_or_else(|| {
                match db {
                    Db::Oracle => "jdbc:oracle:thin:@localhost:1521:XE",
                    Db::Tibero => "jdbc:tibero:thin:@localhost:8629:tibero",
                    Db::PostgreSql => "jdbc:postgresql://localhost:5432/egov",
                    Db::MariaDb => "jdbc:mariadb://localhost:3306/egov",
                    Db::Cubrid => "jdbc:cubrid:localhost:33000:egov:::",
                }
                .into()
            });
        }
        "txtUser" => return placeholder(&placeholders.user).unwrap_or_else(|| "egov".into()),
        "txtPasswd" => return placeholder(&placeholders.password).unwrap_or_default(),
        "txtJndiName" => "jdbc/egovDS",
        "txtTransactionName" => "txManager",
        "txtEntityManagerFactory" => "entityManagerFactory",
        "txtPackagesToScan" => return base_package.to_owned(),
        "cmbDialectName" => match db {
            Db::Oracle | Db::Tibero => "org.hibernate.dialect.Oracle10gDialect",
            Db::PostgreSql => "org.hibernate.dialect.PostgreSQL10Dialect",
            Db::MariaDb => "org.hibernate.dialect.MariaDB103Dialect",
            Db::Cubrid => "org.hibernate.dialect.CUBRIDDialect",
        },
        "chkAnnotationTransaction" | "chkAopConfigTransaction" => "true",
        "txtAdviceName" => "txAdvice",
        "txtMethodName" => "*",
        "txtTimeout" => "-1",
        "txtRollbackFor" => "Exception",
        "txtPointCutName" => "requiredTx",
        "txtPointCutExpression" => {
            return format!("execution(* {base_package}..impl.*Impl.*(..))");
        }
        "txtGlobalTimeout" => "60",
        "txtIdServiceName" => "egovIdGnrService",
        "txtQuery" => match db {
            Db::Oracle | Db::Tibero => "SELECT SEQ_ID.NEXTVAL FROM DUAL",
            Db::PostgreSql => "SELECT NEXTVAL('seq_id')",
            Db::MariaDb => "SELECT NEXTVAL(seq_id)",
            Db::Cubrid => "SELECT seq_id.NEXT_VALUE",
        },
        "txtTable" => "IDS",
        "txtTableNameFieldValue" => "TB_SAMPLE",
        "txtBlockSize" => "10",
        "chkStrategy" => "true",
        "txtStrategyName" => "egovIdGnrStrategy",
        "txtPrefix" => "SAMPLE-",
        "txtCipers" => "20",
        "txtFillChar" => "0",
        "txtAddress" => "",
        "txtAppenderName" => "console",
        "txtConversionPattern" => "%d %5p [%c] %m%n",
        "txtLogFileName" => "./logs/egov.log",
        "cboAppend" => "true",
        "txtLogFileNamePattern" => "./logs/egov-%d{yyyy-MM-dd}-%i.log",
        "txtMaxFileSize" => "10 MB",
        "txtMaxIndex" => "10",
        "txtInterval" => "1",
        "cboModulate" => "true",
        "txtTableName" => "TB_LOG",
        "txtConnectionFactoryClass" => return format!("{base_package}.cmm.LogConnectionFactory"),
        "txtConnectionFactoryMethod" => "getConnection",
        "txtPropertyServiceName" => "propertiesService",
        "txtKey" => "pageUnit",
        "txtValue" => "10",
        "txtPropertyFile" => "classpath*:/egovframework/egovProps/conf/config.properties",
        "cboEncoding" => "UTF-8",
        "txtJobName" => "sampleJob",
        "txtServiceClass" => return format!("{base_package}.batch.SampleJob"),
        "txtServiceName" => "sampleService",
        "txtServiceMethod" => "execute",
        "chkProperty" => "false",
        "txtPropertyName" => "message",
        "txtPropertyValue" => "hello",
        "cboConcurrent" => "false",
        "txtTriggerName" => "sampleTrigger",
        "txtStartDelay" => "10000",
        "txtRepeatInterval" => "60000",
        "txtCronExpression" => "0 0/5 * * * ?",
        "txtSchedulerName" => "sampleScheduler",
        "txtDiskStore" => "java.io.tmpdir",
        "txtDftHeapEntries" => "10000",
        "txtDftEternal" | "txtEternal" => "false",
        "txtDftIdleTime" => "120",
        "txtDftLiveTime" => "120",
        "txtCacheName" => "sampleCache",
        "txtHeapEntries" => "1000",
        "txtIdleTime" => "300",
        "txtLiveTime" => "600",
        "txtConfigLocation" => "classpath:/ehcache.xml",
        _ => "",
    };
    value.to_owned()
}

/// 프로젝트의 데이터소스 프로퍼티 키(`Globals.DriverClassName`·`Globals.Url`·`Globals.UserName`·`Globals.Password`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placeholders {
    pub driver: Option<String>,
    pub url: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
}

impl Placeholders {
    /// 프로퍼티 키들에서 같은 접두사로 묶인 데이터소스 키를 찾는다(`Globals.`, `Globals.oracle.` 등).
    pub fn from_keys<'a>(keys: impl IntoIterator<Item = &'a str>) -> Self {
        let keys: Vec<&str> = keys.into_iter().collect();
        let prefixes: Vec<&str> = keys
            .iter()
            .filter_map(|k| k.strip_suffix("DriverClassName"))
            .collect();
        for prefix in prefixes {
            let find = |names: &[&str]| {
                names
                    .iter()
                    .map(|n| format!("{prefix}{n}"))
                    .find(|k| keys.contains(&k.as_str()))
            };
            let url = find(&["Url", "URL", "url"]);
            if url.is_some() {
                return Placeholders {
                    driver: Some(format!("{prefix}DriverClassName")),
                    url,
                    user: find(&["UserName", "Username", "User", "user"]),
                    password: find(&["Password", "password"]),
                };
            }
        }
        Placeholders::default()
    }
}

/// 만들 설정 하나.
#[derive(Clone, Debug)]
pub struct Request {
    pub kind: &'static Kind,
    pub format: Format,
    pub rte: Rte,
    /// 입력 칸 값(체크는 `"true"`·`"false"`).
    pub values: BTreeMap<String, String>,
    /// XML 파일 이름(확장자 제외) 또는 Java 클래스 이름.
    pub name: String,
    /// Java Config 패키지.
    pub package: String,
}

/// 만든 파일이 놓일 폴더(프로젝트 루트 기준).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigLayout {
    /// Spring XML 설정 폴더(`context-*.xml`이 있는 곳).
    pub spring: PathBuf,
    /// 클래스패스 루트(log4j2.xml·ehcache.xml).
    pub resources: PathBuf,
    pub java: PathBuf,
}

impl Default for ConfigLayout {
    fn default() -> Self {
        ConfigLayout {
            spring: "src/main/resources/egovframework/spring".into(),
            resources: "src/main/resources".into(),
            java: "src/main/java".into(),
        }
    }
}

impl Request {
    /// 기본값으로 채운 요청.
    pub fn new(
        kind: &'static Kind,
        rte: Rte,
        db: Db,
        base_package: &str,
        placeholders: &Placeholders,
    ) -> Self {
        let mut values = BTreeMap::new();
        for input in kind.inputs {
            let value = match input.kind {
                InputKind::Choice(options) => options[0].to_owned(),
                _ => default_value(input.key, db, base_package, placeholders),
            };
            values.insert(input.key.to_owned(), value);
        }
        Request {
            kind,
            format: kind.formats[0],
            rte,
            values,
            name: kind.xml_name.to_owned(),
            package: format!("{base_package}.config"),
        }
    }

    /// 이 입력 칸을 보일지(다른 칸 값에 따라).
    pub fn visible(&self, input: &Input) -> bool {
        input
            .when
            .is_none_or(|(key, value)| self.values.get(key).is_some_and(|v| v == value))
    }

    /// 만들 파일 경로(프로젝트 루트 기준).
    pub fn path(&self, layout: &ConfigLayout) -> PathBuf {
        match self.format {
            Format::JavaConfig => layout
                .java
                .join(self.package.replace('.', "/"))
                .join(format!("{}.java", self.name)),
            Format::Yaml => layout.resources.join(format!("{}.yaml", self.name)),
            Format::Properties => layout.resources.join(format!("{}.properties", self.name)),
            Format::Xml if self.kind.folder == "logging" || self.kind.template == "cache" => {
                layout.resources.join(format!("{}.xml", self.name))
            }
            Format::Xml => layout.spring.join(format!("{}.xml", self.name)),
        }
    }

    fn template_name(&self) -> String {
        format!("{}{}.hbs", self.kind.template, self.format.suffix())
    }

    fn context(&self) -> Value {
        let mut map = Map::new();
        for input in self.kind.inputs {
            let value = self.values.get(input.key).cloned().unwrap_or_default();
            let value = match input.kind {
                // 공식 템플릿은 `{{#if chkX}}`로 본다(문자열 "false"는 참이 된다).
                InputKind::Check => json!(value == "true"),
                _ => json!(value.trim()),
            };
            map.insert(input.key.to_owned(), value);
        }
        map.insert("rte".into(), json!(self.rte.prefix()));
        map.insert("rtePath".into(), json!(self.rte.prefix().replace('.', "/")));
        map.insert("txtFileName".into(), json!(self.name));
        map.insert("txtConfigPackage".into(), json!(self.package));
        // 데이터소스 종류는 공식 템플릿의 인라인 조각 이름(`{{> dbcp}}`)으로도 쓴다.
        if let Some(kind) = self.values.get("rdoType") {
            map.insert("datasourceKind".into(), json!(kind.to_lowercase()));
        }
        Value::Object(map)
    }
}

/// 설정 파일을 만든다. 프로젝트 템플릿(`overrides/<폴더>/<파일>`)이 있으면 그것을 쓴다.
pub fn generate(
    request: &Request,
    layout: &ConfigLayout,
    overrides: Option<&Path>,
) -> Result<GeneratedFile, String> {
    validate(request)?;
    let name = request.template_name();
    let custom = overrides
        .map(|dir| dir.join(request.kind.folder).join(&name))
        .and_then(|file| std::fs::read_to_string(file).ok())
        .map(|text| js_compatible(&text));
    let template = match custom {
        Some(text) => text,
        None => builtin(request.kind.folder, &name)
            .ok_or_else(|| format!("{} 형식은 이 종류에 없습니다", request.format.label()))?
            .to_owned(),
    };
    let content = registry()
        .render_template(&template, &request.context())
        .map_err(|e| format!("{name} 템플릿 오류: {e}"))?;
    Ok(GeneratedFile {
        path: request.path(layout),
        content,
    })
}

/// 숫자만 받는 칸(Java 코드에 그대로 들어간다).
const NUMBERS: [&str; 14] = [
    "txtTimeout",
    "txtGlobalTimeout",
    "txtBlockSize",
    "txtCipers",
    "txtStartDelay",
    "txtRepeatInterval",
    "txtMaxIndex",
    "txtInterval",
    "txtDftHeapEntries",
    "txtDftIdleTime",
    "txtDftLiveTime",
    "txtHeapEntries",
    "txtIdleTime",
    "txtLiveTime",
];

/// 이름 칸은 XML 속성·Java 식별자에 들어가므로 식별자 모양만 받는다(설정이 깨지거나 값이 새지 않게).
fn validate(request: &Request) -> Result<(), String> {
    let identifier = |s: &str| {
        s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && s.chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    for input in request.kind.inputs {
        let value = request
            .values
            .get(input.key)
            .map(String::as_str)
            .unwrap_or("");
        // 파일 경로, JNDI 이름(`jdbc/x`), 메서드 이름 패턴(`*`), 방언 클래스는 이름 검사에서 뺀다.
        let name_like = input.key.ends_with("Name")
            && !input.key.contains("File")
            && !matches!(
                input.key,
                "cmbDialectName" | "txtMethodName" | "txtJndiName"
            );
        if name_like
            && request.visible(input)
            && !value.trim().is_empty()
            && !identifier(value.trim())
        {
            return Err(format!(
                "{}: 이름에는 글자·숫자·_·.·-만 쓸 수 있습니다 ({value})",
                input.label
            ));
        }
        // XML 속성값을 깨뜨리는 글자는 받지 않는다.
        if value.contains(['<', '>', '"']) {
            return Err(format!("{}: <, >, \" 는 쓸 수 없습니다", input.label));
        }
        if request.visible(input)
            && NUMBERS.contains(&input.key)
            && value.trim().parse::<i64>().is_err()
        {
            return Err(format!("{}: 정수를 넣으세요 ({value})", input.label));
        }
        if request.visible(input) && input.key == "txtFillChar" && value.chars().count() != 1 {
            return Err(format!("{}: 글자 하나를 넣으세요", input.label));
        }
    }
    if request.format == Format::JavaConfig {
        let java = |s: &str| {
            s.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        if !java(&request.name) {
            return Err(format!(
                "클래스 이름이 Java 식별자가 아닙니다: {}",
                request.name
            ));
        }
        if !request.package.split('.').all(java) {
            return Err(format!(
                "패키지 이름이 올바르지 않습니다: {}",
                request.package
            ));
        }
    } else if request.name.trim().is_empty() || request.name.contains(['/', '\\']) {
        return Err("파일 이름이 올바르지 않습니다".into());
    }
    Ok(())
}

/// Ferrite 내장 템플릿.
fn builtin(folder: &str, name: &str) -> Option<&'static str> {
    macro_rules! table {
        ($($folder:literal / $name:literal),* $(,)?) => {
            match (folder, name) {
                $(($folder, $name) => Some(include_str!(concat!("templates/config/", $folder, "/", $name))),)*
                _ => None,
            }
        };
    }
    table![
        "datasource" / "datasource.hbs",
        "datasource" / "datasource-java.hbs",
        "datasource" / "jndiDatasource.hbs",
        "datasource" / "jndiDatasource-java.hbs",
        "transaction" / "datasource.hbs",
        "transaction" / "datasource-java.hbs",
        "transaction" / "jpa.hbs",
        "transaction" / "jpa-java.hbs",
        "transaction" / "jta.hbs",
        "transaction" / "jta-java.hbs",
        "idGeneration" / "xml-id-gnr-sequence-service.hbs",
        "idGeneration" / "xml-id-gnr-sequence-service-java.hbs",
        "idGeneration" / "xml-id-gnr-table-service.hbs",
        "idGeneration" / "xml-id-gnr-table-service-java.hbs",
        "idGeneration" / "xml-id-gnr-uuid-service.hbs",
        "idGeneration" / "xml-id-gnr-uuid-service-java.hbs",
        "logging" / "console.hbs",
        "logging" / "console-yaml.hbs",
        "logging" / "console-properties.hbs",
        "logging" / "file.hbs",
        "logging" / "file-yaml.hbs",
        "logging" / "file-properties.hbs",
        "logging" / "rollingFile.hbs",
        "logging" / "rollingFile-yaml.hbs",
        "logging" / "rollingFile-properties.hbs",
        "logging" / "timeBasedRollingFile.hbs",
        "logging" / "timeBasedRollingFile-yaml.hbs",
        "logging" / "timeBasedRollingFile-properties.hbs",
        "logging" / "jdbc.hbs",
        "logging" / "jdbc-yaml.hbs",
        "logging" / "jdbc-properties.hbs",
        "property" / "property.hbs",
        "property" / "property-java.hbs",
        "scheduling" / "beanJob.hbs",
        "scheduling" / "beanJob-java.hbs",
        "scheduling" / "methodJob.hbs",
        "scheduling" / "methodJob-java.hbs",
        "scheduling" / "simpleTrigger.hbs",
        "scheduling" / "simpleTrigger-java.hbs",
        "scheduling" / "cronTrigger.hbs",
        "scheduling" / "cronTrigger-java.hbs",
        "scheduling" / "scheduler.hbs",
        "scheduling" / "scheduler-java.hbs",
        "cache" / "cache.hbs",
        "cache" / "ehcacheConfigForSpring.hbs",
        "cache" / "ehcacheConfigForSpring-java.hbs",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 모든_종류와_형식에_템플릿이_있다() {
        for kind in KINDS {
            for format in kind.formats {
                let request = Request {
                    format: *format,
                    name: if *format == Format::JavaConfig {
                        kind.java_name.to_owned()
                    } else {
                        kind.xml_name.to_owned()
                    },
                    ..Request::new(
                        kind,
                        Rte::V4,
                        Db::Oracle,
                        "egovframework.example",
                        &Placeholders::default(),
                    )
                };
                let file = generate(&request, &ConfigLayout::default(), None)
                    .unwrap_or_else(|e| panic!("{} {:?}: {e}", kind.label, format));
                assert!(
                    !file.content.trim().is_empty(),
                    "{} {:?}",
                    kind.label,
                    format
                );
            }
        }
    }

    #[test]
    fn 프로젝트의_데이터소스_키를_플레이스홀더로_쓴다() {
        let keys = [
            "Globals.DbType",
            "Globals.oracle.DriverClassName",
            "Globals.oracle.Url",
            "Globals.oracle.UserName",
            "Globals.oracle.Password",
        ];
        let placeholders = Placeholders::from_keys(keys);
        assert_eq!(
            default_value("txtUrl", Db::Oracle, "x", &placeholders),
            "${Globals.oracle.Url}"
        );
        assert_eq!(
            default_value("txtUser", Db::Oracle, "x", &placeholders),
            "${Globals.oracle.UserName}"
        );
        assert_eq!(
            default_value("txtUrl", Db::PostgreSql, "x", &Placeholders::default()),
            "jdbc:postgresql://localhost:5432/egov"
        );
    }

    #[test]
    fn 이름_칸은_식별자만_받는다() {
        let kind = &KINDS[0];
        let mut request = Request::new(kind, Rte::V3, Db::Oracle, "egov", &Placeholders::default());
        assert!(generate(&request, &ConfigLayout::default(), None).is_ok());
        request
            .values
            .insert("txtDatasourceName".into(), "data\" source".into());
        assert!(generate(&request, &ConfigLayout::default(), None).is_err());
    }
}
