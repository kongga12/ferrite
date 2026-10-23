//! Ferrite가 설치해 줄 수 있는 언어 서버 목록. 버전과 내려받을 파일의 SHA-256을 여기 고정한다.
//! 버전을 올릴 때는 크기와 해시를 함께 바꾼다. 해시는 배포처가 공개한 값을 쓴다.
//! - GitHub: 릴리스 자산의 digest(`gh api repos/<저장소>/releases/tags/<태그>`)
//! - Eclipse(jdt.ls): 파일 옆의 `.sha256`
//! - Maven Central: SHA-1만 있으므로, 그 SHA-1과 맞는 jar에서 SHA-256을 구한다. 목록이 바뀐 Ferrite로 올리면 설치본이 "업데이트 있음"으로 보인다.

pub struct Server {
    /// 설치 폴더 이름이자 화면에 보이는 이름.
    pub name: &'static str,
    /// 이 서버가 맡는 LSP 언어 ID.
    pub languages: &'static [&'static str],
    pub version: &'static str,
    /// 사용자가 직접 설치했을 때 PATH에서 찾을 실행 파일 이름.
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub launch: Launch,
    pub source: Source,
}

/// 설치본을 어떻게 실행하는가.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    /// 설치한 실행 파일을 그대로 실행한다.
    Binary,
    /// 이 런타임으로 실행한다: `<런타임> <파일> <인자>`(`php phpactor.phar …`).
    Runtime(&'static str),
    /// Eclipse jdt.ls: Java 21 이상으로 equinox 런처를 띄운다(`crate::java`).
    Jdtls,
    /// Spring Boot LS: Java 21 이상으로 띄우고, 클라이언트가 jdt.ls와의 메시지를 중계한다.
    SpringBoot,
}

pub enum Source {
    /// 배포 파일. 이 OS·CPU에 맞는 것을 모두 같은 폴더에 설치한다. 그중 첫 파일의 `bin`이 실행
    /// 대상이다. 맞는 것이 하나도 없으면 설치할 수 없다.
    Release(&'static [Asset]),
    /// `go install <모듈>@<버전>`. 무결성은 Go 체크섬 DB(sum.golang.org)가 검증한다.
    Go(&'static str),
    /// 전용 가상환경에 `pip install <패키지>==<버전>`.
    // ponytail: 버전만 고정하고 해시는 고정하지 않는다(의존 휠이 플랫폼마다 달라서).
    // 필요하면 --require-hashes와 플랫폼별 해시 목록으로.
    Pip(&'static str),
    /// Ferrite 실행 파일에 들어 있다. `ferrite <플래그>`로 띄우므로 설치하지 않는다.
    Builtin(&'static str),
}

/// Ferrite 실행 파일을 egov 언어 서버로 띄우는 플래그.
pub const EGOV_LS_FLAG: &str = "--egov-ls";

pub struct Asset {
    /// `std::env::consts::OS` 값. 빈 문자열이면 모든 OS.
    pub os: &'static str,
    /// `std::env::consts::ARCH` 값. 빈 문자열이면 모든 CPU(유니버설 바이너리, 스크립트).
    pub arch: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub archive: Archive,
    /// 설치 폴더 기준 실행 파일(또는 런타임에 넘길 파일) 경로. 파일 이름에 `*`를 하나 쓸 수 있다
    /// (압축 안 jar 이름에 빌드 번호가 붙는 경우). 설치할 때 실제 이름으로 바꿔 기록한다.
    pub bin: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Archive {
    /// 실행 파일 하나를 gzip으로 압축한 것. 풀어서 `bin` 이름으로 둔다.
    Gz,
    Zip,
    TarGz,
    /// 압축하지 않은 파일. 그대로 `bin` 이름으로 둔다.
    Plain,
}

pub const SERVERS: &[Server] = &[
    Server {
        name: "rust-analyzer",
        languages: &["rust"],
        version: "2026-10-19",
        program: "rust-analyzer",
        args: &[],
        launch: Launch::Binary,
        source: Source::Release(&[
            Asset {
                os: "windows",
                arch: "x86_64",
                url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-19/rust-analyzer-x86_64-pc-windows-msvc.zip",
                sha256: "7df6e1a2595d1906e8441cb83fd58170b47d32eb09bc52c1b2184538cfd49a71",
                size: 17_750_891,
                archive: Archive::Zip,
                bin: "rust-analyzer.exe",
            },
            Asset {
                os: "windows",
                arch: "aarch64",
                url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-19/rust-analyzer-aarch64-pc-windows-msvc.zip",
                sha256: "dc2b6c35e2be0bf13f4ae2a6c69adca5730c91d011bfac31371279a4e55bcb2b",
                size: 15_884_787,
                archive: Archive::Zip,
                bin: "rust-analyzer.exe",
            },
            Asset {
                os: "macos",
                arch: "x86_64",
                url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-19/rust-analyzer-x86_64-apple-darwin.gz",
                sha256: "bdfd46e82731a8d3502d852d28b6462c1f1c13eed0b3e76bbcf1ba839028249c",
                size: 14_805_239,
                archive: Archive::Gz,
                bin: "rust-analyzer",
            },
            Asset {
                os: "macos",
                arch: "aarch64",
                url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-19/rust-analyzer-aarch64-apple-darwin.gz",
                sha256: "eedad08a96f7bf92afd0890463787337d17591ea2e3df216033c6bc3ccc9d802",
                size: 14_071_674,
                archive: Archive::Gz,
                bin: "rust-analyzer",
            },
            Asset {
                os: "linux",
                arch: "x86_64",
                url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-19/rust-analyzer-x86_64-unknown-linux-gnu.gz",
                sha256: "8fde1c9dae81459f98e99eedae3654846a9166abea686306c26e57773be558da",
                size: 15_066_130,
                archive: Archive::Gz,
                bin: "rust-analyzer",
            },
            Asset {
                os: "linux",
                arch: "aarch64",
                url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-19/rust-analyzer-aarch64-unknown-linux-gnu.gz",
                sha256: "d31ee384ab788b9abd8e7aa8e1322f23e6cd4310471d373e9a79ce4656e21896",
                size: 14_480_382,
                archive: Archive::Gz,
                bin: "rust-analyzer",
            },
        ]),
    },
    Server {
        name: "clangd",
        languages: &["c", "cpp"],
        version: "23.1.0",
        program: "clangd",
        args: &[],
        launch: Launch::Binary,
        // 압축 안의 lib/clang(내장 헤더)을 실행 파일 기준 상대 경로로 찾으므로 폴더 구조를 유지한다.
        source: Source::Release(&[
            Asset {
                os: "windows",
                arch: "x86_64",
                url: "https://github.com/clangd/clangd/releases/download/23.1.0/clangd-windows-23.1.0.zip",
                sha256: "23412a240756a162e7b98a282f36aa2a23a88db5ce16a0cbc4fef7253768c810",
                size: 29_690_805,
                archive: Archive::Zip,
                bin: "clangd_23.1.0/bin/clangd.exe",
            },
            Asset {
                // x86_64·arm64 유니버설 바이너리.
                os: "macos",
                arch: "",
                url: "https://github.com/clangd/clangd/releases/download/23.1.0/clangd-mac-23.1.0.zip",
                sha256: "1082e6638223b785ca2daf0939f13afcd0bb95c84ee9a4bbaff4745365159253",
                size: 100_060_151,
                archive: Archive::Zip,
                bin: "clangd_23.1.0/bin/clangd",
            },
            Asset {
                os: "linux",
                arch: "x86_64",
                url: "https://github.com/clangd/clangd/releases/download/23.1.0/clangd-linux-23.1.0.zip",
                sha256: "e53b1a96196095faedb7642cf64964f7fb9ad4a0c1f00dd2c172a3d9dcbafdfd",
                size: 117_949_007,
                archive: Archive::Zip,
                bin: "clangd_23.1.0/bin/clangd",
            },
        ]),
    },
    Server {
        // XML(MyBatis 매퍼 등). vscode-xml 릴리스에 올라오는 네이티브 빌드.
        name: "lemminx",
        languages: &["xml"],
        version: "0.29.3",
        program: "lemminx",
        args: &[],
        launch: Launch::Binary,
        source: Source::Release(&[
            Asset {
                os: "windows",
                arch: "x86_64",
                url: "https://github.com/redhat-developer/vscode-xml/releases/download/0.29.3/lemminx-win32.zip",
                sha256: "7eaefaac68253b0ec8e0ad1f1c0f2d0755423d4e99e52497428b52f80df28eb7",
                size: 16_578_930,
                archive: Archive::Zip,
                bin: "lemminx-win32.exe",
            },
            Asset {
                os: "macos",
                arch: "x86_64",
                url: "https://github.com/redhat-developer/vscode-xml/releases/download/0.29.3/lemminx-osx-x86_64.zip",
                sha256: "9e56123194b5c1fdc7d87f1bd21c83cc996ba7be913def3a2836a13305bcd273",
                size: 16_483_151,
                archive: Archive::Zip,
                bin: "lemminx-osx-x86_64",
            },
            Asset {
                os: "macos",
                arch: "aarch64",
                url: "https://github.com/redhat-developer/vscode-xml/releases/download/0.29.3/lemminx-osx-aarch_64.zip",
                sha256: "185db5630ce85be43ea0fab034e7841b1327c2793db05ab481e029cf493d86ce",
                size: 16_271_050,
                archive: Archive::Zip,
                bin: "lemminx-osx-aarch_64",
            },
            Asset {
                os: "linux",
                arch: "x86_64",
                url: "https://github.com/redhat-developer/vscode-xml/releases/download/0.29.3/lemminx-linux-x86_64.zip",
                sha256: "1acc44e24201c1d2f5ccb4e43e7426ed0df6909207a81ff199810e2808104d89",
                size: 16_903_991,
                archive: Archive::Zip,
                bin: "lemminx-linux-x86_64",
            },
            Asset {
                os: "linux",
                arch: "aarch64",
                url: "https://github.com/redhat-developer/vscode-xml/releases/download/0.29.3/lemminx-linux-aarch_64.zip",
                sha256: "0b88a49e83c2611282672f5123ce33abf1371c8e3c55cad6ee45ea00626ffa1b",
                size: 16_296_527,
                archive: Archive::Zip,
                bin: "lemminx-linux-aarch_64",
            },
        ]),
    },
    Server {
        name: "phpactor",
        languages: &["php"],
        version: "2026.10.07.0",
        program: "phpactor",
        args: &["language-server"],
        launch: Launch::Runtime("php"),
        source: Source::Release(&[Asset {
            os: "",
            arch: "",
            url: "https://github.com/phpactor/phpactor/releases/download/2026.10.07.0/phpactor.phar",
            sha256: "b28bf6166e016d539fc2f2aec18ef918b12590e436a6f1ccbdf30b5bbdbdb16b",
            size: 4_815_663,
            archive: Archive::Plain,
            bin: "phpactor.phar",
        }]),
    },
    Server {
        name: "gopls",
        languages: &["go"],
        version: "v0.24.0",
        program: "gopls",
        args: &[],
        launch: Launch::Binary,
        source: Source::Go("golang.org/x/tools/gopls"),
    },
    Server {
        name: "basedpyright",
        languages: &["python"],
        version: "1.40.2",
        program: "basedpyright-langserver",
        args: &["--stdio"],
        launch: Launch::Binary,
        source: Source::Pip("basedpyright"),
    },
    Server {
        name: "jdtls",
        languages: &["java"],
        // 1.61.0이 Java 21로 도는 마지막 판이다. 1.62부터는 Eclipse 4.42 플랫폼(2026-09-12에
        // 필요 Java를 25로 올림)을 써서 Java 21로 띄우면 핵심 모듈이 로드되지 않고 바로 끝난다.
        // 올릴 때는 `java::MIN_JAVA`도 함께 바꾼다(공식 jdtls.py는 아직 21로 검사해 믿을 수 없다).
        version: "1.61.0",
        program: "jdtls",
        args: &[],
        launch: Launch::Jdtls,
        // Java로 짜여 있어 모든 OS가 같은 파일을 쓴다(OS별 설정 폴더가 압축 안에 들어 있다).
        source: Source::Release(&[
            Asset {
                os: "",
                arch: "",
                url: "https://download.eclipse.org/jdtls/milestones/1.61.0/jdt-language-server-1.61.0-202609031315.tar.gz",
                sha256: "338e7e73d61836651ba2453919a0d34fa763eb4e7c03342092309bffb8934c64",
                size: 51_037_522,
                archive: Archive::TarGz,
                bin: "plugins/org.eclipse.equinox.launcher_*.jar",
            },
            // jdt.ls JVM에 에이전트로 붙여 Lombok이 만드는 getter 등을 인식시킨다. Maven Central은
            // SHA-1만 주므로, Central의 SHA-1과 일치하는 jar에서 SHA-256을 구해 고정했다.
            // 1.18.46은 최신 jdt.ls의 "Document does not match the AST" 오류를 고친 판이다.
            Asset {
                os: "",
                arch: "",
                url: "https://repo1.maven.org/maven2/org/projectlombok/lombok/1.18.46/lombok-1.18.46.jar",
                sha256: "01f7b1a015e33e2b62d5f5f37053306357ab1415fd181fcba7794f5d198c1126",
                size: 2_045_364,
                archive: Archive::Plain,
                bin: crate::java::LOMBOK_JAR,
            },
        ]),
    },
    Server {
        // Spring Tools의 언어 서버. Java 파일은 jdt.ls가 주 서버이고 이것은 함께 붙는 보조 서버다
        // (Spring 진단·@Query JPQL 검증 등). application*.properties·yml은 이것이 주 서버다.
        // jdt.ls 쪽 기능(클래스패스, 타입 조회)은 jdt.ls에 실은 확장 번들을 거쳐 클라이언트가 중계한다.
        name: "spring-boot",
        languages: &[
            "java",
            "spring-boot-properties",
            "spring-boot-properties-yaml",
        ],
        version: "2.4.1",
        program: "spring-boot-language-server",
        args: &[],
        launch: Launch::SpringBoot,
        // VS Code 확장(vsix = zip). Open VSX가 공개한 SHA-256으로 고정했다. 정식판만 고른다
        // (Open VSX의 latest는 날짜가 붙은 프리릴리스일 수 있다).
        source: Source::Release(&[Asset {
            os: "",
            arch: "",
            url: "https://open-vsx.org/api/VMware/vscode-spring-boot/2.4.1/file/VMware.vscode-spring-boot-2.4.1.vsix",
            sha256: "6971f2bbc513a944e7f907e48ec68f64848d689630d5616bf54ef0803349f6ed",
            size: 83_290_020,
            archive: Archive::Zip,
            bin: "extension/language-server/spring-boot-language-server*.jar",
        }]),
    },
    Server {
        // egov 매퍼 진단(SQL Injection, 참조, 중복 id). XML의 주 서버는 lemminx이고 이것은 함께
        // 붙는 보조 서버다. 다른 에디터는 같은 서버를 단독 실행 파일 `ferrite-egov-ls`로 쓴다.
        name: "ferrite-egov",
        languages: &["xml"],
        version: env!("CARGO_PKG_VERSION"),
        program: "ferrite-egov-ls",
        args: &[],
        launch: Launch::Binary,
        source: Source::Builtin(EGOV_LS_FLAG),
    },
];

/// 이 언어(LSP 언어 ID)를 맡는 주 서버. 기능 요청(자동완성, 정의 이동 등)은 이 서버로 보낸다.
pub fn for_language(language_id: &str) -> Option<&'static Server> {
    servers_for(language_id).next()
}

/// 이 언어를 맡는 서버 모두. 주 서버가 먼저다(목록 순서). 문서는 모두에게 보내고 진단은 합친다.
pub fn servers_for(language_id: &str) -> impl Iterator<Item = &'static Server> {
    SERVERS
        .iter()
        .filter(move |s| s.languages.contains(&language_id))
}

/// 런처 종류로 찾는다(jdt.ls ↔ Spring Boot LS처럼 서로를 아는 서버용).
pub fn by_launch(launch: Launch) -> Option<&'static Server> {
    SERVERS.iter().find(|s| s.launch == launch)
}
