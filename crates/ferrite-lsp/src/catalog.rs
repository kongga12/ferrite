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
    /// jdt.ls에 싣는 확장 번들(java-debug). 따로 띄우지 않는다.
    JdtlsBundle,
}

pub enum Source {
    /// 배포 파일. 이 OS·CPU에 맞는 것을 모두 같은 폴더에 설치한다. 그중 첫 파일의 `bin`이 실행
    /// 대상이다. 맞는 것이 하나도 없으면 설치할 수 없다.
    Release(&'static [Asset]),
    /// `go install <모듈>@<버전>`. 무결성은 Go 체크섬 DB(sum.golang.org)가 검증한다.
    Go(&'static str),
    /// 전용 폴더에 `npm install <패키지>@<버전>`(설치 스크립트는 돌리지 않는다). 무결성은 npm이
    /// 레지스트리의 해시로 검증한다. `bin`은 설치 폴더 기준 실행 대상이고 `{platform}`·`{arch}`·`{exe}`를
    /// 이 플랫폼의 Node 이름(`win32`·`x64`)과 실행 파일 꼬리로 바꾼다. `folder`가 있으면 같은 패키지를 쓰는
    /// 서버들이 그 폴더 하나에 설치한다(HTML·CSS·JSON 서버 묶음).
    Npm {
        package: &'static str,
        bin: &'static str,
        folder: Option<&'static str>,
    },
    /// 전용 가상환경에 `pip install <패키지>==<버전>`.
    // ponytail: 버전만 고정하고 해시는 고정하지 않는다(의존 휠이 플랫폼마다 달라서).
    // 필요하면 --require-hashes와 플랫폼별 해시 목록으로.
    Pip(&'static str),
    /// Ferrite 실행 파일에 들어 있다. `ferrite <플래그>`로 띄우므로 설치하지 않는다.
    Builtin(&'static str),
}

/// HTML·CSS·JSON 언어 서버 묶음(npm 꾸러미 하나, MIT). 셋이 한 설치 폴더를 같이 쓴다. HTML 서버가
/// 쓰는 TypeScript 4가 설치의 대부분(85MB 중 65MB)이라 따로 설치하면 세 배가 된다.
const WEB_SERVERS: &str = "vscode-langservers-extracted";
const WEB_SERVERS_VERSION: &str = "4.10.0";

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
        // TypeScript 7(Go로 다시 쓴 공식 컴파일러)에 든 언어 서버. JS도 맡는다. npm 꾸러미의 bin은 이
        // 실행 파일을 자식으로 띄우는 Node 스크립트라(끌 때 자식이 남는다) 실행 파일을 바로 띄운다.
        name: "typescript",
        languages: &[
            "typescript",
            "typescriptreact",
            "javascript",
            "javascriptreact",
        ],
        version: "7.0.2",
        // 직접 설치했다면 TypeScript 7 미리보기(`@typescript/native-preview`)의 실행 파일.
        program: "tsgo",
        args: &["--lsp", "--stdio"],
        launch: Launch::Binary,
        source: Source::Npm {
            package: "typescript",
            bin: "node_modules/@typescript/typescript-{platform}-{arch}/lib/tsc{exe}",
            folder: None,
        },
    },
    Server {
        name: "vscode-html",
        languages: &["html"],
        version: WEB_SERVERS_VERSION,
        program: "vscode-html-language-server",
        args: &["--stdio"],
        launch: Launch::Runtime("node"),
        source: Source::Npm {
            package: WEB_SERVERS,
            bin: "node_modules/vscode-langservers-extracted/bin/vscode-html-language-server",
            folder: Some(WEB_SERVERS),
        },
    },
    Server {
        name: "vscode-css",
        languages: &["css"],
        version: WEB_SERVERS_VERSION,
        program: "vscode-css-language-server",
        args: &["--stdio"],
        launch: Launch::Runtime("node"),
        source: Source::Npm {
            package: WEB_SERVERS,
            bin: "node_modules/vscode-langservers-extracted/bin/vscode-css-language-server",
            folder: Some(WEB_SERVERS),
        },
    },
    Server {
        name: "vscode-json",
        languages: &["json"],
        version: WEB_SERVERS_VERSION,
        program: "vscode-json-language-server",
        args: &["--stdio"],
        launch: Launch::Runtime("node"),
        source: Source::Npm {
            package: WEB_SERVERS,
            bin: "node_modules/vscode-langservers-extracted/bin/vscode-json-language-server",
            folder: Some(WEB_SERVERS),
        },
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
        // egov 매퍼 진단(SQL Injection, 참조, 중복 id)과 매퍼 XML ↔ Java 이동, JSP 출력의 XSS 진단.
        // XML의 주 서버는 lemminx, Java는 jdt.ls이고 이것은 함께 붙는 보조 서버다(JSP는 이것뿐).
        // 다른 에디터는 같은 서버를 단독 실행 파일 `ferrite-egov-ls`로 쓴다.
        name: "ferrite-egov",
        languages: &["xml", "java", "jsp"],
        version: env!("CARGO_PKG_VERSION"),
        program: "ferrite-egov-ls",
        args: &[],
        launch: Launch::Binary,
        source: Source::Builtin(EGOV_LS_FLAG),
    },
];

/// Ferrite가 설치해 줄 수 있는 디버그 어댑터. 이름은 디버그 구성의 `adapter` 값이고, `languages`는
/// 디버그하는 언어(화면 표시용)다. 언어 서버와 같은 방식으로 설치·업데이트·삭제한다(`crate::install`).
pub const ADAPTERS: &[Server] = &[
    Server {
        // 어댑터와 디버그 대상 모두 Python 모듈로 돈다. 대상은 사용자의 Python으로 띄운다(가상환경이 아니라).
        name: "debugpy",
        languages: &["python"],
        version: "1.8.22",
        program: "python",
        args: &["-m", "debugpy.adapter"],
        launch: Launch::Binary,
        source: Source::Pip("debugpy"),
    },
    Server {
        // LLDB 기반. VSIX 안의 어댑터와 lldb 라이브러리를 폴더째 쓴다.
        name: "codelldb",
        languages: &["rust", "c", "cpp"],
        version: "1.12.3",
        program: "codelldb",
        args: &[],
        launch: Launch::Binary,
        source: Source::Release(&[
            Asset {
                os: "windows",
                arch: "x86_64",
                url: "https://github.com/vadimcn/codelldb/releases/download/v1.12.3/codelldb-win32-x64.vsix",
                sha256: "a916e509308dac817732f63ca604a8b93ed29cd16f38a2fa9f0b64ed58e8f51a",
                size: 50_943_475,
                archive: Archive::Zip,
                bin: "extension/adapter/codelldb.exe",
            },
            Asset {
                os: "macos",
                arch: "x86_64",
                url: "https://github.com/vadimcn/codelldb/releases/download/v1.12.3/codelldb-darwin-x64.vsix",
                sha256: "e25cc716b94c62c07fec268ff2785d2b797245b160502baef8b9c970a0c4d8e8",
                size: 47_735_596,
                archive: Archive::Zip,
                bin: "extension/adapter/codelldb",
            },
            Asset {
                os: "macos",
                arch: "aarch64",
                url: "https://github.com/vadimcn/codelldb/releases/download/v1.12.3/codelldb-darwin-arm64.vsix",
                sha256: "2f114a990e1b368dd1dbd33c80c0e719767af2d228391ec0df0571c957f9ac91",
                size: 45_390_048,
                archive: Archive::Zip,
                bin: "extension/adapter/codelldb",
            },
            Asset {
                os: "linux",
                arch: "x86_64",
                url: "https://github.com/vadimcn/codelldb/releases/download/v1.12.3/codelldb-linux-x64.vsix",
                sha256: "1cd7f386598022b51a5b93b9ffa23e812b23f519cfe1833384ec4bef4bfd1be1",
                size: 55_571_094,
                archive: Archive::Zip,
                bin: "extension/adapter/codelldb",
            },
            Asset {
                os: "linux",
                arch: "aarch64",
                url: "https://github.com/vadimcn/codelldb/releases/download/v1.12.3/codelldb-linux-arm64.vsix",
                sha256: "0887f67d440554617894266f80706b700907c36b95e6e49d23b95a0e05318101",
                size: 54_544_514,
                archive: Archive::Zip,
                bin: "extension/adapter/codelldb",
            },
        ]),
    },
    Server {
        // 배포 실행 파일이 없어 `go install`로 만든다. Go 디버그에는 어차피 Go가 있어야 한다.
        name: "delve",
        languages: &["go"],
        version: "v1.27.2",
        program: "dlv",
        args: &["dap"],
        launch: Launch::Binary,
        source: Source::Go("github.com/go-delve/delve/cmd/dlv"),
    },
    Server {
        // jdt.ls에 싣는 확장 번들이다. 따로 띄우지 않고, jdt.ls에 디버그 세션을 열게 해 받은 포트로 붙는다.
        // 설치·삭제하면 jdt.ls를 다시 띄워야 실린다.
        name: "java-debug",
        languages: &["java"],
        version: "0.53.1",
        program: "java-debug",
        args: &[],
        launch: Launch::JdtlsBundle,
        source: Source::Release(&[Asset {
            os: "",
            arch: "",
            url: "https://repo1.maven.org/maven2/com/microsoft/java/com.microsoft.java.debug.plugin/0.53.1/com.microsoft.java.debug.plugin-0.53.1.jar",
            sha256: "4f4778d452a6a0665536f43ce4e32403a24be6593336b80dc85a322912859e24",
            size: 2_960_556,
            archive: Archive::Plain,
            bin: "com.microsoft.java.debug.plugin-0.53.1.jar",
        }]),
    },
    Server {
        // Xdebug 3과 말한다. 디버그 대상 PHP에 Xdebug 확장이 있어야 한다.
        name: "php-debug",
        languages: &["php"],
        version: "1.40.2",
        program: "php-debug",
        args: &[],
        launch: Launch::Runtime("node"),
        source: Source::Release(&[Asset {
            os: "",
            arch: "",
            url: "https://github.com/xdebug/vscode-php-debug/releases/download/v1.40.2/php-debug-1.40.2.vsix",
            sha256: "17631993fe800083a2fc89a6782c8bf58a79603ad201ddff60d9b59ba2345d5f",
            size: 1_797_181,
            archive: Archive::Zip,
            bin: "extension/out/phpDebug.js",
        }]),
    },
    Server {
        // 브라우저(Chrome·Edge) 디버그. TCP로 듣는 DAP 서버이고, 페이지마다 자식 세션을 거꾸로
        // 요청한다(startDebugging).
        name: "js-debug",
        languages: &["javascript"],
        version: "1.140.0",
        program: "js-debug",
        args: &[],
        launch: Launch::Runtime("node"),
        source: Source::Release(&[Asset {
            os: "",
            arch: "",
            url: "https://github.com/microsoft/vscode-js-debug/releases/download/v1.140.0/js-debug-dap-v1.140.0.tar.gz",
            sha256: "27dab92937ec1ab35821ae955aac867544fe06a1b6307229049f2d789af10968",
            size: 1_249_709,
            archive: Archive::TarGz,
            bin: "js-debug/src/dapDebugServer.js",
        }]),
    },
];

/// 언어 서버와 디버그 어댑터 모두(설치 창 순서).
pub fn all() -> impl Iterator<Item = &'static Server> {
    SERVERS.iter().chain(ADAPTERS)
}

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
