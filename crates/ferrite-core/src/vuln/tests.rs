use std::path::{Path, PathBuf};

use serde_json::json;

use super::manifest::{self, Kind, Manifest};
use super::osv::{self, Advisory};
use super::*;

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ferrite-vuln-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn write(root: &Path, path: &str, text: &str) -> PathBuf {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, text).unwrap();
    file
}

fn dep(ecosystem: Ecosystem, name: &str, version: &str) -> Dependency {
    Dependency {
        ecosystem,
        name: name.to_owned(),
        version: version.to_owned(),
        file: PathBuf::from("pom.xml"),
        line: 0,
        via: None,
    }
}

/// GHSA-jfh8-c2jp-5v3q(Log4Shell)의 구조를 줄였다. 범위가 셋이고 구간마다 고친 버전이 다르다.
fn log4shell() -> Advisory {
    Advisory::parse(&json!({
        "id": "GHSA-jfh8-c2jp-5v3q",
        "summary": "Remote code injection in Log4j",
        "aliases": ["CVE-2021-44228"],
        "database_specific": {"severity": "CRITICAL"},
        "affected": [{
            "package": {"ecosystem": "Maven", "name": "org.apache.logging.log4j:log4j-core"},
            "ranges": [
                {"type": "ECOSYSTEM", "events": [{"introduced": "2.13.0"}, {"fixed": "2.15.0"}]},
                {"type": "ECOSYSTEM", "events": [{"introduced": "2.0-beta9"}, {"fixed": "2.3.1"}]},
                {"type": "ECOSYSTEM", "events": [{"introduced": "2.4"}, {"fixed": "2.12.2"}]}
            ]
        }]
    }))
    .unwrap()
}

#[test]
fn 레코드가_버전에_해당하면_그_구간을_고친_버전을_준다() {
    let advisory = log4shell();
    let core = |v| {
        advisory.affects(&dep(
            Ecosystem::Maven,
            "org.apache.logging.log4j:log4j-core",
            v,
        ))
    };
    assert_eq!(core("2.14.1"), Some(Some("2.15.0".into())));
    assert_eq!(core("2.11.0"), Some(Some("2.12.2".into())));
    assert_eq!(core("2.0-beta9"), Some(Some("2.3.1".into())));
    assert_eq!(core("2.15.0"), None);
    assert_eq!(core("2.12.2"), None);
    assert_eq!(core("1.2.17"), None);
    // 다른 패키지·생태계는 보지 않는다.
    assert_eq!(
        advisory.affects(&dep(
            Ecosystem::Maven,
            "org.apache.logging.log4j:log4j-api",
            "2.14.1"
        )),
        None
    );
    assert_eq!(
        advisory.affects(&dep(
            Ecosystem::Npm,
            "org.apache.logging.log4j:log4j-core",
            "2.14.1"
        )),
        None
    );
}

#[test]
fn 목록_끝없는_구간_last_affected와_철회를_따른다() {
    let advisory = Advisory::parse(&json!({
        "id": "PYSEC-1",
        "affected": [{
            "package": {"ecosystem": "PyPI", "name": "PyYAML"},
            "versions": ["4.1"],
            "ranges": [
                {"type": "GIT", "repo": "https://example.invalid", "events": [{"introduced": "0"}, {"fixed": "abc123"}]},
                {"type": "ECOSYSTEM", "events": [{"introduced": "0"}, {"last_affected": "5.3"}]},
                {"type": "ECOSYSTEM", "events": [{"introduced": "6.0b1"}]}
            ]
        }]
    }))
    .unwrap();
    let yaml = |v| advisory.affects(&dep(Ecosystem::PyPI, "pyyaml", v));
    assert_eq!(yaml("4.1"), Some(None));
    assert_eq!(yaml("5.3"), Some(None));
    assert_eq!(yaml("5.4"), None);
    assert_eq!(yaml("6.0"), Some(None));
    assert_eq!(advisory.severity, Severity::Unknown);
    assert!(Advisory::parse(&json!({"id": "X", "withdrawn": "2024-01-01T00:00:00Z"})).is_none());
}

#[test]
fn cvss_v3_벡터로_심각도를_낸다() {
    assert_eq!(
        osv::cvss3("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"),
        Some(10.0)
    );
    assert_eq!(
        osv::cvss3("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
        Some(9.8)
    );
    assert_eq!(
        osv::cvss3("CVSS:3.1/AV:N/AC:H/PR:N/UI:N/S:U/C:H/I:N/A:N"),
        Some(5.9)
    );
    assert_eq!(
        osv::cvss3("CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N"),
        Some(6.1)
    );
    assert_eq!(
        osv::cvss3("CVSS:3.1/AV:L/AC:L/PR:L/UI:N/S:U/C:N/I:N/A:N"),
        Some(0.0)
    );
    let scored = |vector: &str| {
        Advisory::parse(
            &json!({"id": "RUSTSEC-1", "severity": [{"type": "CVSS_V3", "score": vector}]}),
        )
        .unwrap()
        .severity
    };
    assert_eq!(
        scored("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
        Severity::Critical
    );
    assert_eq!(
        scored("CVSS:3.1/AV:N/AC:H/PR:N/UI:N/S:U/C:H/I:N/A:N"),
        Severity::Moderate
    );
}

#[test]
fn 결과는_심각한_것부터_같은_것은_한_번() {
    let low = Advisory::parse(&json!({
        "id": "GHSA-low",
        "database_specific": {"severity": "LOW"},
        "affected": [{"package": {"ecosystem": "Maven", "name": "org.apache.logging.log4j:log4j-core"},
            "ranges": [{"type": "ECOSYSTEM", "events": [{"introduced": "0"}]}]}]
    }))
    .unwrap();
    let core = dep(
        Ecosystem::Maven,
        "org.apache.logging.log4j:log4j-core",
        "2.14.1",
    );
    let found = osv::findings(&[low, log4shell()], &[core.clone(), core]);
    let titles: Vec<_> = found
        .iter()
        .map(|f| (f.title(), f.severity, f.fixed.clone()))
        .collect();
    assert_eq!(
        titles,
        [
            ("CVE-2021-44228", Severity::Critical, Some("2.15.0".into())),
            ("GHSA-low", Severity::Low, None),
        ]
    );
}

#[test]
fn 오프라인_묶음에서_해당_패키지의_레코드만_읽는다() {
    let root = temp_root("offline");
    let zip_path = root.join("all.zip");
    {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let other = json!({"id": "GHSA-other", "affected": [{"package": {"ecosystem": "Maven", "name": "a:b"},
            "ranges": [{"type": "ECOSYSTEM", "events": [{"introduced": "0"}]}]}]});
        for (name, record) in [
            ("GHSA-jfh8-c2jp-5v3q.json", log4shell_json()),
            ("GHSA-other.json", other),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(record.to_string().as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    let core = dep(
        Ecosystem::Maven,
        "org.apache.logging.log4j:log4j-core",
        "2.14.1",
    );
    let advisories = osv::query_offline(&zip_path, std::slice::from_ref(&core)).unwrap();
    assert_eq!(
        advisories.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["GHSA-jfh8-c2jp-5v3q"]
    );
    // 들이기는 zip만 받는다.
    let dest = root.join("db/Maven/all.zip");
    osv::import_database(&zip_path, &dest).unwrap();
    assert!(dest.is_file());
    let not_zip = write(&root, "notes.txt", "x");
    assert!(osv::import_database(&not_zip, &dest).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

fn log4shell_json() -> serde_json::Value {
    json!({
        "id": "GHSA-jfh8-c2jp-5v3q",
        "aliases": ["CVE-2021-44228"],
        "affected": [{
            "package": {"ecosystem": "Maven", "name": "org.apache.logging.log4j:log4j-core"},
            "ranges": [{"type": "ECOSYSTEM", "events": [{"introduced": "2.13.0"}, {"fixed": "2.15.0"}]}]
        }]
    })
}

const POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.example</groupId>
  <artifactId>demo</artifactId>
  <version>1.0</version>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.apache.logging.log4j</groupId>
        <artifactId>log4j-slf4j-impl</artifactId>
        <version>2.14.1</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.apache.logging.log4j</groupId>
      <artifactId>log4j-slf4j-impl</artifactId>
    </dependency>
    <dependency>
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>4.13.2</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
</project>
"#;

#[test]
fn maven_나무의_전이_의존은_끌어온_직접_의존의_줄에_둔다() {
    let root = temp_root("maven");
    let pom = write(&root, "pom.xml", POM);
    let tree = "com.example:demo:jar:1.0\n\
                +- org.apache.logging.log4j:log4j-slf4j-impl:jar:2.14.1:compile\n\
                |  +- org.slf4j:slf4j-api:jar:1.7.25:compile\n\
                |  \\- org.apache.logging.log4j:log4j-core:jar:2.14.1:runtime\n\
                \\- junit:junit:jar:tests:4.13.2:test\n";
    let deps = Manifest {
        kind: Kind::Maven,
        path: pom.clone(),
    }
    .read(Some(tree))
    .unwrap();
    let found: Vec<_> = deps
        .iter()
        .map(|d| {
            (
                d.name.as_str(),
                d.version.as_str(),
                d.line,
                d.via.as_deref(),
            )
        })
        .collect();
    // `<dependencies>` 아래 것(관리 블록이 아니라)의 줄.
    assert_eq!(
        found,
        [
            (
                "org.apache.logging.log4j:log4j-slf4j-impl",
                "2.14.1",
                16,
                None
            ),
            (
                "org.slf4j:slf4j-api",
                "1.7.25",
                16,
                Some("org.apache.logging.log4j:log4j-slf4j-impl")
            ),
            (
                "org.apache.logging.log4j:log4j-core",
                "2.14.1",
                16,
                Some("org.apache.logging.log4j:log4j-slf4j-impl")
            ),
            ("junit:junit", "4.13.2", 20, None),
        ]
    );
    assert!(deps.iter().all(|d| d.file == pom));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn maven_모듈마다_그_pom에_둔다() {
    let root = temp_root("maven-modules");
    write(
        &root,
        "pom.xml",
        &POM.replace(
            "<artifactId>demo</artifactId>",
            "<artifactId>parent</artifactId>",
        ),
    );
    let web = write(
        &root,
        "web/pom.xml",
        POM.replace(
            "<artifactId>demo</artifactId>",
            "<artifactId>web</artifactId>",
        )
        .as_str(),
    );
    write(&root, "target/pom.xml", POM);
    let tree = "com.example:parent:pom:1.0\n\
                com.example:web:war:1.0\n\
                \\- junit:junit:jar:4.13.2:test\n";
    let deps = Manifest {
        kind: Kind::Maven,
        path: root.join("pom.xml"),
    }
    .read(Some(tree))
    .unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!((deps[0].file.clone(), deps[0].line), (web, 20));
    // 찾기는 맨 위 pom만, 빌드 결과는 뺀다.
    write(&root, "web/node_modules/x/package-lock.json", "{}");
    write(&root, "app/package-lock.json", "{}");
    let found: Vec<_> = manifest::find(&root)
        .into_iter()
        .map(|m| (m.kind, m.path.strip_prefix(&root).unwrap().to_path_buf()))
        .collect();
    assert_eq!(
        found,
        [
            (Kind::Npm, PathBuf::from("app").join("package-lock.json")),
            (Kind::Maven, PathBuf::from("pom.xml")),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn 빌드_도구_명령은_가까운_래퍼를_쓴다() {
    let root = temp_root("wrapper");
    let build = write(&root, "lib/build.gradle", "");
    let gradle = Manifest {
        kind: Kind::Gradle,
        path: build,
    };
    let out = root.join("tree.txt");
    assert_eq!(
        gradle.command(&root, &out).unwrap(),
        "gradle -q --console=plain dependencies"
    );
    let wrapper = write(
        &root,
        if cfg!(windows) {
            "gradlew.bat"
        } else {
            "gradlew"
        },
        "",
    );
    assert_eq!(
        gradle.command(&root, &out).unwrap(),
        format!("\"{}\" -q --console=plain dependencies", wrapper.display())
    );
    let maven = Manifest {
        kind: Kind::Maven,
        path: write(&root, "pom.xml", POM),
    };
    assert_eq!(
        maven.command(&root, &out).unwrap(),
        format!(
            "mvn -B -q dependency:tree -DoutputFile=\"{}\" -DappendOutput=true",
            out.display()
        )
    );
    assert_eq!(
        Manifest {
            kind: Kind::Npm,
            path: root.join("package-lock.json")
        }
        .command(&root, &out),
        None
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gradle_runtime_classpath를_읽는다() {
    let root = temp_root("gradle");
    let build = write(
        &root,
        "build.gradle",
        "plugins { id 'java' }\n\
         dependencies {\n    \
             implementation 'org.apache.logging.log4j:log4j-core:2.14.1'\n    \
             implementation(\"com.google.guava:guava:31.0-jre\")\n    \
             implementation project(':lib')\n    \
             implementation platform('org.junit:junit-bom:5.11.4')\n    \
             implementation 'com.example:missing:1.0'\n\
         }\n",
    );
    let output = "\n\
------------------------------------------------------------\n\
Root project 'demo'\n\
------------------------------------------------------------\n\
\n\
compileClasspath - Compile classpath for source set 'main'.\n\
\\--- junit:junit:4.12\n\
\n\
runtimeClasspath - Runtime classpath of source set 'main'.\n\
+--- org.apache.logging.log4j:log4j-core:2.14.1\n\
|    \\--- org.apache.logging.log4j:log4j-api:2.14.1\n\
+--- com.google.guava:guava:31.0-jre -> 32.1.0-jre\n\
|    \\--- com.google.guava:failureaccess:1.0.1\n\
+--- org.junit:junit-bom:5.11.4\n\
|    \\--- org.junit.jupiter:junit-jupiter-api:5.11.4 (c)\n\
+--- com.example:missing:1.0 FAILED\n\
\\--- project :lib\n     \
     \\--- org.yaml:snakeyaml -> 1.33 (*)\n\
\n\
(*) - Indicates repeated occurrences of a transitive dependency subtree.\n";
    let deps = Manifest {
        kind: Kind::Gradle,
        path: build,
    }
    .read(Some(output))
    .unwrap();
    let found: Vec<_> = deps
        .iter()
        .map(|d| {
            (
                d.name.as_str(),
                d.version.as_str(),
                d.line,
                d.via.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("org.apache.logging.log4j:log4j-core", "2.14.1", 2, None),
            (
                "org.apache.logging.log4j:log4j-api",
                "2.14.1",
                2,
                Some("org.apache.logging.log4j:log4j-core")
            ),
            ("com.google.guava:guava", "32.1.0-jre", 3, None),
            (
                "com.google.guava:failureaccess",
                "1.0.1",
                3,
                Some("com.google.guava:guava")
            ),
            // BOM 아래 `(c)`(버전 제약)는 의존이 아니다.
            ("org.junit:junit-bom", "5.11.4", 5, None),
            // 받지 못한 것(`FAILED`)은 적은 버전으로.
            ("com.example:missing", "1.0", 6, None),
            ("org.yaml:snakeyaml", "1.33", 4, Some("project :lib")),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn npm_잠금_파일_v3와_v1을_읽는다() {
    let root = temp_root("npm");
    let v3 = write(
        &root,
        "package-lock.json",
        r#"{
  "name": "web",
  "lockfileVersion": 3,
  "packages": {
    "": {
      "dependencies": { "lodash": "^4.17.20" }
    },
    "node_modules/lodash": {
      "version": "4.17.20"
    },
    "node_modules/@babel/core": {
      "version": "7.0.0"
    },
    "node_modules/@babel/core/node_modules/semver": {
      "version": "5.7.1"
    },
    "node_modules/local": {
      "resolved": "packages/local",
      "link": true
    },
    "packages/local": {
      "version": "1.0.0"
    }
  }
}"#,
    );
    let deps = Manifest {
        kind: Kind::Npm,
        path: v3,
    }
    .read(None)
    .unwrap();
    let mut found: Vec<_> = deps
        .iter()
        .map(|d| {
            (
                d.name.as_str(),
                d.version.as_str(),
                d.line,
                d.via.as_deref(),
            )
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            ("@babel/core", "7.0.0", 10, None),
            ("lodash", "4.17.20", 7, None),
            ("semver", "5.7.1", 13, Some("@babel/core")),
        ]
    );
    let v1 = write(
        &root,
        "old/package-lock.json",
        r#"{
  "lockfileVersion": 1,
  "dependencies": {
    "minimist": {
      "version": "0.0.8"
    },
    "mkdirp": {
      "version": "0.5.1",
      "dependencies": {
        "minimist": {
          "version": "1.2.0"
        }
      }
    },
    "mine": {
      "version": "github:me/mine#abc"
    }
  }
}"#,
    );
    let deps = Manifest {
        kind: Kind::Npm,
        path: v1,
    }
    .read(None)
    .unwrap();
    let found: Vec<_> = deps
        .iter()
        .map(|d| {
            (
                d.name.as_str(),
                d.version.as_str(),
                d.line,
                d.via.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("minimist", "0.0.8", 3, None),
            ("mkdirp", "0.5.1", 6, None),
            ("minimist", "1.2.0", 3, Some("mkdirp")),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn requirements는_고정한_버전만_cargo_lock은_crates_io만() {
    let root = temp_root("pip-cargo");
    let requirements = write(
        &root,
        "requirements.txt",
        "# 웹\n\
         Django==3.2.0  # LTS\n\
         -r base.txt\n\
         requests[security]==2.19.1 ; python_version >= \"3.6\"\n\
         PyYAML==5.3 \\\n    --hash=sha256:abc\n\
         flask>=2.0\n\
         numpy==1.*\n",
    );
    let deps = Manifest {
        kind: Kind::Pip,
        path: requirements,
    }
    .read(None)
    .unwrap();
    let found: Vec<_> = deps
        .iter()
        .map(|d| (d.name.as_str(), d.version.as_str(), d.line))
        .collect();
    assert_eq!(
        found,
        [
            ("django", "3.2.0", 1),
            ("requests", "2.19.1", 3),
            ("pyyaml", "5.3", 4)
        ]
    );
    let lock = write(
        &root,
        "Cargo.lock",
        "version = 4\n\n\
         [[package]]\n\
         name = \"app\"\n\
         version = \"0.1.0\"\n\
         dependencies = [\n \"smallvec\",\n]\n\n\
         [[package]]\n\
         name = \"smallvec\"\n\
         version = \"1.6.0\"\n\
         source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\
         checksum = \"abc\"\n\n\
         [[package]]\n\
         name = \"forked\"\n\
         version = \"1.0.0\"\n\
         source = \"git+https://example.invalid/forked#abc\"\n",
    );
    let deps = Manifest {
        kind: Kind::Cargo,
        path: lock,
    }
    .read(None)
    .unwrap();
    let found: Vec<_> = deps
        .iter()
        .map(|d| (d.name.as_str(), d.version.as_str(), d.line))
        .collect();
    assert_eq!(found, [("smallvec", "1.6.0", 10)]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn 여러_db에_있는_같은_취약점은_하나로_합친다() {
    let record = |id: &str, aliases: &[&str], severity: Option<&str>, fixed: Option<&str>| {
        let mut events = vec![json!({"introduced": "0"})];
        events.extend(fixed.map(|f| json!({"fixed": f})));
        Advisory::parse(&json!({
            "id": id,
            "aliases": aliases,
            "summary": if severity.is_some() { "" } else { "Buffer overflow in SmallVec::insert_many" },
            "database_specific": {"severity": severity},
            "affected": [{"package": {"ecosystem": "crates.io", "name": "smallvec"},
                "ranges": [{"type": "SEMVER", "events": events}]}]
        }))
        .unwrap()
    };
    // RUSTSEC는 심각도가 없고, GHSA는 고친 버전이 없다고 해 본다.
    let rustsec = record(
        "RUSTSEC-2021-0003",
        &["CVE-2021-25900", "GHSA-43w2-9j62-hq99"],
        None,
        Some("1.6.1"),
    );
    let ghsa = record(
        "GHSA-43w2-9j62-hq99",
        &["CVE-2021-25900"],
        Some("CRITICAL"),
        None,
    );
    let other = record(
        "RUSTSEC-2018-0003",
        &["CVE-2018-25023"],
        Some("HIGH"),
        Some("0.6.3"),
    );
    let smallvec = dep(Ecosystem::Crates, "smallvec", "0.6.0");
    let found = osv::findings(&[rustsec, ghsa, other], std::slice::from_ref(&smallvec));
    assert_eq!(found.len(), 2);
    let merged = found
        .iter()
        .find(|f| f.title() == "CVE-2021-25900")
        .unwrap();
    assert_eq!(merged.severity, Severity::Critical);
    assert_eq!(merged.fixed.as_deref(), Some("1.6.1"));
    assert_eq!(merged.summary, "Buffer overflow in SmallVec::insert_many");
    assert!(merged.aliases.contains(&"GHSA-43w2-9j62-hq99".to_owned()));
}
