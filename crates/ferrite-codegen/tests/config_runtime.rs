//! 만든 설정을 실제 Spring·log4j2로 읽어 본다(로컬 Maven 저장소와 JDK가 필요하다).
//! `cargo test -p ferrite-codegen --test config_runtime -- --ignored --nocapture`
//!
//! - Spring XML: 컨테이너를 띄워 빈을 모두 만든다(데이터소스는 H2 메모리 DB). JNDI·JTA는 WAS가 있어야
//!   하므로 XSD 검증과 빈 클래스 확인까지만 한다.
//! - Java Config: 컴파일하고 같은 방식으로 띄운다.
//! - log4j2: XML·properties는 log4j2로 읽어 어펜더가 생기는지, YAML은 문법만 본다.
//!
//! egov 3.x(3.10, Spring 4.3, Java 8 대상)와 4.x(4.3, Spring 5.3)를 각각 확인한다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ferrite_codegen::config::{self, ConfigLayout, Format, KINDS, Kind, Placeholders, Request};
use ferrite_codegen::crud::{Db, Rte};

const PACKAGE: &str = "ferrite.check";
const H2_URL: &str = "jdbc:h2:mem:ferrite;DB_CLOSE_DELAY=-1";

/// 검사 프로그램(설정을 읽어 빈을 만든다).
const CHECK: &str = r#"package ferrite.check;

import java.io.File;
import java.io.FileInputStream;
import java.util.Arrays;

import org.springframework.beans.factory.config.BeanDefinition;
import org.springframework.beans.factory.support.DefaultListableBeanFactory;
import org.springframework.beans.factory.xml.XmlBeanDefinitionReader;
import org.springframework.context.annotation.AnnotationConfigApplicationContext;
import org.springframework.context.support.GenericXmlApplicationContext;
import org.springframework.core.io.FileSystemResource;

public class Check {
	public static void main(String[] args) throws Exception {
		String mode = args[0];
		String[] rest = Arrays.copyOfRange(args, 1, args.length);
		if (mode.equals("xml")) {
			GenericXmlApplicationContext context = new GenericXmlApplicationContext();
			for (String file : rest) {
				context.load(new FileSystemResource(file));
			}
			context.refresh();
			System.out.println("OK beans " + Arrays.toString(context.getBeanDefinitionNames()));
			context.close();
		} else if (mode.equals("defs")) {
			DefaultListableBeanFactory factory = new DefaultListableBeanFactory();
			XmlBeanDefinitionReader reader = new XmlBeanDefinitionReader(factory);
			for (String file : rest) {
				reader.loadBeanDefinitions(new FileSystemResource(file));
			}
			for (String name : factory.getBeanDefinitionNames()) {
				BeanDefinition definition = factory.getBeanDefinition(name);
				if (definition.getBeanClassName() != null) {
					Class.forName(definition.getBeanClassName());
				}
			}
			System.out.println("OK defs " + Arrays.toString(factory.getBeanDefinitionNames()));
		} else if (mode.equals("java")) {
			Class<?>[] classes = new Class<?>[rest.length];
			for (int i = 0; i < rest.length; i++) {
				classes[i] = Class.forName(rest[i]);
			}
			AnnotationConfigApplicationContext context = new AnnotationConfigApplicationContext(classes);
			System.out.println("OK beans " + Arrays.toString(context.getBeanDefinitionNames()));
			context.close();
		} else if (mode.equals("log4j2")) {
			org.apache.logging.log4j.core.LoggerContext context =
				org.apache.logging.log4j.core.config.Configurator.initialize("check", null, new File(rest[0]).toURI());
			if (context.getConfiguration().getAppender(rest[1]) == null) {
				throw new IllegalStateException("어펜더가 없다: " + rest[1] + " / " + context.getConfiguration());
			}
			System.out.println("OK log4j2 " + context.getConfiguration().getAppenders().keySet());
			org.apache.logging.log4j.core.config.Configurator.shutdown(context);
		} else if (mode.equals("yaml")) {
			Object data = new org.yaml.snakeyaml.Yaml().load(new FileInputStream(rest[0]));
			if (!(data instanceof java.util.Map) || !((java.util.Map<?, ?>) data).containsKey("Configuration")) {
				throw new IllegalStateException("Configuration이 없다");
			}
			System.out.println("OK yaml");
		}
	}
}
"#;

/// QuartzJobBean을 상속한 Job(테스트용).
const SAMPLE_JOB: &str = r#"package ferrite.check;

import org.quartz.JobExecutionContext;
import org.springframework.scheduling.quartz.QuartzJobBean;

public class SampleJob extends QuartzJobBean {
	@Override
	protected void executeInternal(JobExecutionContext context) {
	}
}
"#;

/// 메서드 Job의 대상 빈과 egov 서비스가 쓰는 messageSource(테스트용). `{rte}`는 실행환경 경로.
const SUPPORT: &str = r#"package ferrite.check;

import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
import org.springframework.context.support.ReloadableResourceBundleMessageSource;

@Configuration
public class Support {
	@Bean(name = "sampleService")
	public StringBuilder sampleService() {
		return new StringBuilder();
	}

	@Bean(name = "messageSource")
	public ReloadableResourceBundleMessageSource messageSource() {
		ReloadableResourceBundleMessageSource source = new ReloadableResourceBundleMessageSource();
		source.setBasenames("classpath:/{rte}/fdl/property/messages/properties", "classpath:/{rte}/fdl/idgnr/messages/idgnr");
		return source;
	}
}
"#;

fn jar(repo: &Path, group: &str, artifact: &str, version: &str) -> PathBuf {
    let path = repo
        .join(group)
        .join(artifact)
        .join(version)
        .join(format!("{artifact}-{version}.jar"));
    assert!(
        path.is_file(),
        "로컬 Maven 저장소에 없음: {}",
        path.display()
    );
    path
}

fn kind(template: &str, folder: &str) -> &'static Kind {
    KINDS
        .iter()
        .find(|k| k.template == template && k.folder == folder)
        .unwrap()
}

/// 검사용 값으로 채운 요청(빈 이름이 겹치지 않게, 데이터소스는 H2).
fn request(kind: &'static Kind, format: Format, rte: Rte, values: &[(&str, &str)]) -> Request {
    let mut request = Request::new(kind, rte, Db::Oracle, PACKAGE, &Placeholders::default());
    request.format = format;
    request.package = PACKAGE.into();
    let overrides: BTreeMap<&str, &str> = [
        ("txtDriver", "org.h2.Driver"),
        ("txtUrl", H2_URL),
        ("txtUser", "sa"),
        ("txtPasswd", ""),
        ("txtQuery", "SELECT SEQ_ID.NEXTVAL"),
        ("txtServiceClass", "ferrite.check.SampleJob"),
        ("txtServiceMethod", "toString"),
        ("chkProperty", "true"),
        ("txtPackagesToScan", PACKAGE),
        ("cmbDialectName", "org.hibernate.dialect.H2Dialect"),
    ]
    .into_iter()
    .chain(values.iter().copied())
    .collect();
    for (key, value) in overrides {
        if request.values.contains_key(key) {
            request.values.insert(key.into(), value.into());
        }
    }
    request
}

struct Run {
    dir: PathBuf,
    classpath: String,
    separator: &'static str,
}

impl Run {
    fn write(&self, request: &Request, name: &str) -> PathBuf {
        let mut request = request.clone();
        request.name = name.into();
        let file = config::generate(&request, &ConfigLayout::default(), None)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let path = self.dir.join(&file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, file.content).unwrap();
        path
    }

    fn java(&self, args: &[&str]) {
        let classes = self.dir.join("classes");
        let output = Command::new("java")
            // Spring 4.3의 CGLIB가 JDK 17+에서 클래스를 정의하려면 java.lang을 열어야 한다.
            .args(["--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp"])
            .arg(format!(
                "{}{}{}",
                classes.display(),
                self.separator,
                self.classpath
            ))
            .arg("ferrite.check.Check")
            .args(args)
            .output()
            .expect("java를 실행할 수 없다");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("OK"),
            "{args:?} 실패:\n{stdout}\n{stderr}"
        );
        eprintln!(
            "  {}",
            stdout.lines().find(|l| l.starts_with("OK")).unwrap_or("")
        );
    }
}

fn check(rte: Rte, release: &str, jars: Vec<PathBuf>) {
    eprintln!("== {rte:?}");
    let dir = std::env::temp_dir().join(format!(
        "ferrite-config-check-{rte:?}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let separator = if cfg!(windows) { ";" } else { ":" };
    let run = Run {
        dir: dir.clone(),
        classpath: jars
            .iter()
            .map(|j| j.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(separator),
        separator,
    };
    let path = |p: &PathBuf| p.to_string_lossy().into_owned();
    let property_file = dir.join("check.properties");
    std::fs::write(&property_file, "pageUnit=10\n").unwrap();
    let property_url = format!(
        "file:{}",
        property_file.to_string_lossy().replace('\\', "/")
    );
    let ehcache_url = format!(
        "file:{}",
        dir.join("src/main/resources/ehcache.xml")
            .to_string_lossy()
            .replace('\\', "/")
    );

    // ---- 만들기(XML·Java)
    let mut java_classes: Vec<(String, &str)> = Vec::new();
    let xml = |template: &str, folder: &str, name: &str, values: &[(&str, &str)]| {
        run.write(
            &request(kind(template, folder), Format::Xml, rte, values),
            name,
        )
    };
    let jdbc = xml(
        "datasource",
        "datasource",
        "context-ds-jdbc",
        &[("rdoType", "JDBC")],
    );
    let dbcp = xml(
        "datasource",
        "datasource",
        "context-ds-dbcp",
        &[("rdoType", "DBCP")],
    );
    let c3p0 = xml(
        "datasource",
        "datasource",
        "context-ds-c3p0",
        &[("rdoType", "C3P0")],
    );
    let jndi = xml("jndiDatasource", "datasource", "context-ds-jndi", &[]);
    let tx = xml("datasource", "transaction", "context-tx", &[]);
    let jta = xml("jta", "transaction", "context-jta", &[]);
    let jpa = xml(
        "jpa",
        "transaction",
        "context-jpa",
        &[
            ("txtTransactionName", "jpaTxManager"),
            ("txtAdviceName", "jpaTxAdvice"),
            ("txtPointCutName", "jpaTx"),
            ("chkAnnotationTransaction", "false"),
        ],
    );
    let sequence = xml(
        "xml-id-gnr-sequence-service",
        "idGeneration",
        "context-id-seq",
        &[
            ("txtIdServiceName", "seqIdGnr"),
            ("rdoIdType", "BigDecimal"),
        ],
    );
    let table = xml(
        "xml-id-gnr-table-service",
        "idGeneration",
        "context-id-table",
        &[("txtIdServiceName", "tableIdGnr")],
    );
    let uuid = xml(
        "xml-id-gnr-uuid-service",
        "idGeneration",
        "context-id-uuid",
        &[
            ("txtIdServiceName", "uuidIdGnr"),
            ("rdoIdType", "Address"),
            ("txtAddress", "12:34:56:78:9A:AB"),
        ],
    );
    let internal = xml("property", "property", "context-prop", &[]);
    let external = xml(
        "property",
        "property",
        "context-prop-file",
        &[
            ("rdoType", "External File"),
            ("txtPropertyFile", &property_url),
        ],
    );
    let bean_job = xml(
        "beanJob",
        "scheduling",
        "context-job-bean",
        &[("txtJobName", "beanJob")],
    );
    let method_job = xml(
        "methodJob",
        "scheduling",
        "context-job-method",
        &[("txtJobName", "methodJob")],
    );
    let simple = xml(
        "simpleTrigger",
        "scheduling",
        "context-trigger-simple",
        &[
            ("txtTriggerName", "simpleTrigger"),
            ("txtJobName", "methodJob"),
        ],
    );
    let cron = xml(
        "cronTrigger",
        "scheduling",
        "context-trigger-cron",
        &[("txtTriggerName", "cronTrigger"), ("txtJobName", "beanJob")],
    );
    let scheduler = xml(
        "scheduler",
        "scheduling",
        "context-scheduler",
        &[("txtTriggerName", "cronTrigger")],
    );
    xml("cache", "cache", "ehcache", &[]);
    let cache = xml(
        "ehcacheConfigForSpring",
        "cache",
        "context-cache",
        &[("txtConfigLocation", &ehcache_url)],
    );

    let mut java = |template: &str, folder: &str, name: &'static str, values: &[(&str, &str)]| {
        run.write(
            &request(kind(template, folder), Format::JavaConfig, rte, values),
            name,
        );
        java_classes.push((format!("{PACKAGE}.{name}"), name));
    };
    java("datasource", "datasource", "DsJdbc", &[("rdoType", "JDBC")]);
    java("datasource", "datasource", "DsDbcp", &[("rdoType", "DBCP")]);
    java("datasource", "datasource", "DsC3p0", &[("rdoType", "C3P0")]);
    java("jndiDatasource", "datasource", "DsJndi", &[]);
    java("datasource", "transaction", "Tx", &[]);
    java("jta", "transaction", "Jta", &[]);
    java(
        "jpa",
        "transaction",
        "Jpa",
        &[
            ("txtTransactionName", "jpaTxManager"),
            ("txtAdviceName", "jpaTxAdvice"),
            ("txtPointCutName", "jpaTx"),
            ("chkAnnotationTransaction", "false"),
        ],
    );
    java(
        "xml-id-gnr-sequence-service",
        "idGeneration",
        "IdSeq",
        &[
            ("txtIdServiceName", "seqIdGnr"),
            ("rdoIdType", "BigDecimal"),
        ],
    );
    java(
        "xml-id-gnr-table-service",
        "idGeneration",
        "IdTable",
        &[("txtIdServiceName", "tableIdGnr")],
    );
    java(
        "xml-id-gnr-uuid-service",
        "idGeneration",
        "IdUuid",
        &[
            ("txtIdServiceName", "uuidIdGnr"),
            ("rdoIdType", "Address"),
            ("txtAddress", "12:34:56:78:9A:AB"),
        ],
    );
    java("property", "property", "Prop", &[]);
    java(
        "property",
        "property",
        "PropFile",
        &[
            ("rdoType", "External File"),
            ("txtPropertyFile", &property_url),
            ("txtPropertyServiceName", "filePropertiesService"),
        ],
    );
    java(
        "beanJob",
        "scheduling",
        "JobBean",
        &[("txtJobName", "beanJob")],
    );
    java(
        "methodJob",
        "scheduling",
        "JobMethod",
        &[("txtJobName", "methodJob")],
    );
    java(
        "simpleTrigger",
        "scheduling",
        "TriggerSimple",
        &[
            ("txtTriggerName", "simpleTrigger"),
            ("txtJobName", "methodJob"),
        ],
    );
    java(
        "cronTrigger",
        "scheduling",
        "TriggerCron",
        &[("txtTriggerName", "cronTrigger"), ("txtJobName", "beanJob")],
    );
    java(
        "scheduler",
        "scheduling",
        "Scheduler",
        &[("txtTriggerName", "cronTrigger")],
    );
    java(
        "ehcacheConfigForSpring",
        "cache",
        "Cache",
        &[("txtConfigLocation", &ehcache_url)],
    );

    // ---- 컴파일(검사 프로그램·지원 클래스·만든 Java Config)
    let src = dir.join("check-src/ferrite/check");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("Check.java"), CHECK).unwrap();
    let rte_path = rte.prefix().replace('.', "/");
    std::fs::write(
        src.join("Support.java"),
        SUPPORT.replace("{rte}", &rte_path),
    )
    .unwrap();
    std::fs::write(src.join("SampleJob.java"), SAMPLE_JOB).unwrap();
    let mut sources: Vec<PathBuf> = vec![
        src.join("Check.java"),
        src.join("Support.java"),
        src.join("SampleJob.java"),
    ];
    let java_dir = dir.join("src/main/java/ferrite/check");
    sources.extend(
        std::fs::read_dir(&java_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path()),
    );
    let output = Command::new("javac")
        .args([
            "-encoding",
            "UTF-8",
            "--release",
            release,
            "-Xlint:none",
            "-proc:none",
            "-d",
        ])
        .arg(dir.join("classes"))
        .arg("-cp")
        .arg(&run.classpath)
        .args(&sources)
        .output()
        .expect("javac를 실행할 수 없다");
    assert!(
        output.status.success(),
        "{rte:?} Java Config 컴파일 실패:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("  Java Config {}개 컴파일", java_classes.len());

    // ---- XML: 컨테이너를 띄운다.
    let support_xml = dir.join("context-support.xml");
    std::fs::write(
        &support_xml,
        r#"<beans xmlns="http://www.springframework.org/schema/beans" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.springframework.org/schema/beans http://www.springframework.org/schema/beans/spring-beans.xsd"><bean id="sampleService" class="java.lang.StringBuilder"/><bean id="messageSource" class="org.springframework.context.support.ReloadableResourceBundleMessageSource"><property name="basenames"><list><value>classpath:/{rte}/fdl/property/messages/properties</value><value>classpath:/{rte}/fdl/idgnr/messages/idgnr</value></list></property></bean></beans>"#
            .replace("{rte}", &rte_path),
    )
    .unwrap();
    let mut main: Vec<String> = vec!["xml".into()];
    main.extend(
        [
            &jdbc,
            &tx,
            &sequence,
            &table,
            &uuid,
            &internal,
            &bean_job,
            &method_job,
            &simple,
            &cron,
            &scheduler,
            &cache,
            &support_xml,
        ]
        .iter()
        .map(|p| path(p)),
    );
    run.java(&main.iter().map(String::as_str).collect::<Vec<_>>());
    run.java(&["xml", &path(&dbcp)]);
    run.java(&["xml", &path(&c3p0)]);
    run.java(&["xml", &path(&external), &path(&support_xml)]);
    if rte == Rte::V4 {
        run.java(&["xml", &path(&jdbc), &path(&jpa)]);
    } else {
        // Spring 4.3은 Hibernate 5.6과 맞지 않아 정의만 본다.
        run.java(&["defs", &path(&jpa)]);
    }
    run.java(&["defs", &path(&jndi), &path(&jta)]);

    // ---- Java Config: 같은 묶음으로 띄운다.
    let class = |name: &str| format!("{PACKAGE}.{name}");
    let group: Vec<String> = [
        "DsJdbc",
        "Tx",
        "IdSeq",
        "IdTable",
        "IdUuid",
        "Prop",
        "PropFile",
        "JobBean",
        "JobMethod",
        "TriggerSimple",
        "TriggerCron",
        "Scheduler",
        "Cache",
        "Support",
    ]
    .iter()
    .map(|n| class(n))
    .collect();
    let mut args: Vec<&str> = vec!["java"];
    args.extend(group.iter().map(String::as_str));
    run.java(&args);
    run.java(&["java", &class("DsDbcp")]);
    run.java(&["java", &class("PropFile"), &class("Support")]);
    run.java(&["java", &class("DsC3p0")]);
    if rte == Rte::V4 {
        run.java(&["java", &class("DsJdbc"), &class("Jpa")]);
    }

    // ---- log4j2
    for template in [
        "console",
        "file",
        "rollingFile",
        "timeBasedRollingFile",
        "jdbc",
    ] {
        let logging = kind(template, "logging");
        // 로그 파일은 검사 폴더 안에 쓴다(작업 폴더를 어지르지 않게).
        let log_file = path(&dir.join("logs/check.log"));
        let log_pattern = path(&dir.join("logs/check-%d{yyyy-MM-dd}-%i.log"));
        let mut values = vec![
            ("txtAppenderName", "out"),
            ("txtLogFileName", log_file.as_str()),
            ("txtLogFileNamePattern", log_pattern.as_str()),
        ];
        if template == "jdbc" {
            values.push(("rdoConnectionType", "DriverManager"));
        }
        for format in [Format::Xml, Format::Properties, Format::Yaml] {
            let file = run.write(
                &request(logging, format, rte, &values),
                &format!("log4j2-{template}"),
            );
            match format {
                Format::Yaml => run.java(&["yaml", &path(&file)]),
                _ => run.java(&["log4j2", &path(&file), "out"]),
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "로컬 Maven 저장소(egov·Spring jar)와 JDK가 필요하다"]
fn 만든_설정이_실제_spring에서_읽힌다() {
    let repo = ferrite_egov::taglib::maven_repository().expect("로컬 Maven 저장소");
    let spring = |version: &str| {
        [
            "spring-core",
            "spring-beans",
            "spring-context",
            "spring-aop",
            "spring-expression",
            "spring-tx",
            "spring-jdbc",
            "spring-orm",
            "spring-context-support",
        ]
        .into_iter()
        .map(|a| jar(&repo, "org/springframework", a, version))
        .collect::<Vec<_>>()
    };
    let common = |repo: &Path| {
        vec![
            jar(repo, "org/aspectj", "aspectjweaver", "1.9.22"),
            jar(repo, "org/apache/commons", "commons-dbcp2", "2.9.0"),
            jar(repo, "org/apache/commons", "commons-pool2", "2.10.0"),
            jar(repo, "c3p0", "c3p0", "0.9.1.1"),
            jar(repo, "com/h2database", "h2", "1.4.180"),
            jar(repo, "org/quartz-scheduler", "quartz", "2.1.7"),
            jar(repo, "net/sf/ehcache", "ehcache", "2.10.9.2"),
            jar(repo, "org/apache/logging/log4j", "log4j-core", "2.17.1"),
            jar(repo, "org/apache/logging/log4j", "log4j-api", "2.17.1"),
            jar(repo, "org/yaml", "snakeyaml", "2.4"),
            jar(repo, "javax/transaction", "jta", "1.1"),
            jar(repo, "commons-logging", "commons-logging", "1.2"),
            jar(repo, "javax/persistence", "javax.persistence-api", "2.2"),
        ]
    };
    let mut v3 = spring("4.3.25.RELEASE");
    v3.extend(common(&repo));
    v3.extend([
        jar(
            &repo,
            "egovframework/rte",
            "egovframework.rte.fdl.idgnr",
            "3.10.0",
        ),
        jar(
            &repo,
            "egovframework/rte",
            "egovframework.rte.fdl.property",
            "3.10.0",
        ),
        jar(
            &repo,
            "egovframework/rte",
            "egovframework.rte.fdl.cmmn",
            "3.10.0",
        ),
        jar(&repo, "org/slf4j", "slf4j-api", "1.7.36"),
        // egov 3.x 프로퍼티 서비스가 쓴다(ExtendedProperties).
        jar(&repo, "commons-collections", "commons-collections", "3.2.2"),
    ]);
    check(Rte::V3, "8", v3);

    let mut v4 = spring("5.3.37");
    v4.push(jar(&repo, "org/springframework", "spring-jcl", "5.3.37"));
    v4.extend(common(&repo));
    v4.extend([
        jar(
            &repo,
            "org/egovframe/rte",
            "org.egovframe.rte.fdl.idgnr",
            "4.3.0",
        ),
        jar(
            &repo,
            "org/egovframe/rte",
            "org.egovframe.rte.fdl.property",
            "4.3.0",
        ),
        jar(
            &repo,
            "org/egovframe/rte",
            "org.egovframe.rte.fdl.cmmn",
            "4.3.0",
        ),
        jar(&repo, "org/slf4j", "slf4j-api", "2.0.17"),
        jar(&repo, "org/hibernate", "hibernate-core", "5.6.15.Final"),
        // Hibernate 5.6이 실행할 때 쓰는 것.
        jar(&repo, "org/jboss/logging", "jboss-logging", "3.4.3.Final"),
        jar(&repo, "net/bytebuddy", "byte-buddy", "1.12.18"),
        jar(&repo, "antlr", "antlr", "2.7.7"),
        jar(
            &repo,
            "org/jboss/spec/javax/transaction",
            "jboss-transaction-api_1.2_spec",
            "1.1.1.Final",
        ),
        jar(&repo, "org/jboss", "jandex", "2.4.2.Final"),
        jar(&repo, "com/fasterxml", "classmate", "1.5.1"),
        jar(&repo, "javax/activation", "javax.activation-api", "1.2.0"),
        jar(
            &repo,
            "org/hibernate/common",
            "hibernate-commons-annotations",
            "5.1.2.Final",
        ),
        jar(&repo, "javax/xml/bind", "jaxb-api", "2.3.1"),
        jar(&repo, "org/glassfish/jaxb", "jaxb-runtime", "2.3.1"),
        // egov 4.x 프로퍼티 서비스가 쓴다.
        jar(
            &repo,
            "org/apache/commons",
            "commons-configuration2",
            "2.11.0",
        ),
        jar(&repo, "org/apache/commons", "commons-text", "1.12.0"),
        jar(&repo, "org/apache/commons", "commons-lang3", "3.17.0"),
    ]);
    check(Rte::V4, "11", v4);
}
