//! 공식 egov 프로젝트 템플릿(egovframe-vscode-initializr)으로 새 프로젝트를 만든다.
//! Ferrite는 템플릿을 싣지 않는다. 크기·SHA-256을 고정한 파일을 GitHub에서 받거나(앱이
//! `ferrite_lsp::install::download`로) 공식 확장 vsix에서 꺼내 캐시에 두고, 공식 확장과 같이
//! zip을 풀고 pom 틀의 `###…###`를 채운다.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 고정한 공식 확장의 태그. 이 템플릿들은 egov 5.0.0(Spring 6, jakarta, Java 17)이다.
pub const RELEASE: &str = "v5.0.7";
const REPO: &str = "eGovFramework/egovframe-vscode-initializr";

/// 크기·해시를 고정한 파일 하나.
pub struct Pinned {
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub struct Template {
    pub category: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// 공식 확장의 기본 프로젝트 이름.
    pub project_name: &'static str,
    pub zip: Pinned,
    /// pom 틀. 없으면(프런트엔드, 여러 프로젝트를 담은 MSA) zip을 그대로 푼다.
    pub pom: Option<Pinned>,
}

impl Template {
    /// 받을 파일과 주소. zip은 Git LFS라 media 주소로, pom 틀은 raw 주소로 받는다.
    pub fn files(&self) -> Vec<(&Pinned, String)> {
        let mut files = vec![(
            &self.zip,
            format!(
                "https://media.githubusercontent.com/media/{REPO}/{RELEASE}/templates/projects/examples/{}",
                self.zip.file
            ),
        )];
        if let Some(pom) = &self.pom {
            files.push((
                pom,
                format!(
                    "https://raw.githubusercontent.com/{REPO}/{RELEASE}/templates/projects/pom/{}",
                    pom.file
                ),
            ));
        }
        files
    }

    /// 받을 크기(바이트).
    pub fn size(&self) -> u64 {
        self.files().iter().map(|(p, _)| p.size).sum()
    }

    /// 캐시에 크기가 맞는 파일이 다 있는가. 화면 표시용이라 해시는 만들 때 본다.
    pub fn cached(&self, cache: &Path) -> bool {
        self.files()
            .iter()
            .all(|(p, _)| size_matches(&cache.join(p.file), p))
    }
}

/// 공식 확장 v5.0.7의 `templates-projects.json` 순서.
pub static TEMPLATES: &[Template] = &[
    Template {
        category: "웹",
        name: "웹 프로젝트",
        description: "기본 egov 웹 애플리케이션",
        project_name: "egov-web",
        zip: Pinned {
            file: "egovframe-web.zip",
            size: 15547491,
            sha256: "13674f6f76e74e6d70478b7e9ad0689b0f84b5ffc9828a49f8b511dc2b3d6b69",
        },
        pom: Some(Pinned {
            file: "egovframe-web-pom.xml",
            size: 5794,
            sha256: "f5a79a0bc70a22d9d2ff366d624bfb9d5cb1f2126868c783b2da8d37ff8a76f2",
        }),
    },
    Template {
        category: "템플릿",
        name: "심플 홈페이지",
        description: "기본 기능을 갖춘 간단한 홈페이지",
        project_name: "egov-template-simple",
        zip: Pinned {
            file: "egovframe-template-simple.zip",
            size: 1595816,
            sha256: "c34a8597260ac22f941cf5d4c6987fdc24dcbe167ea56ad6ac29bc93034dee59",
        },
        pom: Some(Pinned {
            file: "egovframe-template-simple-pom.xml",
            size: 8728,
            sha256: "4ad65ea11417a76c740369c19393eabded539d28b695e770a5ebda32effa1d26",
        }),
    },
    Template {
        category: "템플릿",
        name: "포털 사이트",
        description: "기업용 포털 사이트",
        project_name: "egov-template-portal",
        zip: Pinned {
            file: "egovframe-template-portal.zip",
            size: 4125985,
            sha256: "75c0a63062fcd5385c3d3e9c44f739d20d98d824aac65afea9691c3c9f1744ce",
        },
        pom: Some(Pinned {
            file: "egovframe-template-portal-pom.xml",
            size: 8796,
            sha256: "051479c94f49f1c96e0c1939c70cdc9eb0b962454b813c2e654def01be89112d",
        }),
    },
    Template {
        category: "템플릿",
        name: "기업 업무",
        description: "기업 업무 애플리케이션",
        project_name: "egov-template-enterprise",
        zip: Pinned {
            file: "egovframe-template-enterprise.zip",
            size: 1491778,
            sha256: "34a7895b4fef3c622ffd8e0174e586bb01e209320ac2ebdbc33dcc9c00a6aaf1",
        },
        pom: Some(Pinned {
            file: "egovframe-template-enterprise-pom.xml",
            size: 9443,
            sha256: "7fa2a6dd453eff21269b18bb79a62eb24761328666cad2cdb3e4054039255c69",
        }),
    },
    Template {
        category: "템플릿",
        name: "공통컴포넌트",
        description: "공통컴포넌트를 모두 담은 프로젝트",
        project_name: "egov-template-common-components",
        zip: Pinned {
            file: "egovframe-template-common-components.zip",
            size: 23464037,
            sha256: "8ccac2da943aa25cb533ef288709d26fbd76609106d1b6112bd8d881b6900bf0",
        },
        pom: Some(Pinned {
            file: "egovframe-template-common-components-pom.xml",
            size: 17926,
            sha256: "b897390b7e08975bf687954c77a22704478674a92f99c4c49a49e2ecf84bc2f9",
        }),
    },
    Template {
        category: "Boot",
        name: "Boot 웹 프로젝트",
        description: "Spring Boot 기반 웹 프로젝트",
        project_name: "egov-boot-web",
        zip: Pinned {
            file: "egovframe-boot-web.zip",
            size: 15551650,
            sha256: "1284019f991f0676683dc1b19e9133318e50f18092505e0c1635ef1336bd1f6c",
        },
        pom: Some(Pinned {
            file: "egovframe-boot-web-pom.xml",
            size: 4310,
            sha256: "0e7185bf748e2639614022ec1cdfa1fac5b7b2b00b93f73708033d2a8103ef41",
        }),
    },
    Template {
        category: "Boot",
        name: "심플 홈페이지(백엔드)",
        description: "Spring Boot 심플 홈페이지 백엔드",
        project_name: "egov-boot-simple-backend",
        zip: Pinned {
            file: "egovframe-boot-simple-backend.zip",
            size: 389689,
            sha256: "0fcf2be8369bc787164ffe1a5deb90c5d57781a3573812900305b5aecafdbd95",
        },
        pom: Some(Pinned {
            file: "egovframe-boot-simple-backend-pom.xml",
            size: 11622,
            sha256: "1008ef270decff318357259d789ccdb745b2ca99f36ad5b4e48001323a7e3842",
        }),
    },
    Template {
        category: "Boot",
        name: "심플 홈페이지(프런트엔드)",
        description: "React 기반 심플 홈페이지 프런트엔드",
        project_name: "egov-boot-simple-frontend",
        zip: Pinned {
            file: "egovframe-boot-simple-frontend.zip",
            size: 3713472,
            sha256: "caaeff023271b53a4f7908ddf79d66b284ab919eedec22f3806dae517da0307e",
        },
        pom: None,
    },
    Template {
        category: "MSA",
        name: "공통컴포넌트(KRDS)",
        description: "KRDS를 적용한 MSA 공통컴포넌트",
        project_name: "egov-msa-boot-common-components",
        zip: Pinned {
            file: "egovframe-msa-common-components.zip",
            size: 25569472,
            sha256: "472787489f98ff7f1195f7d8ac585868dc539e6bcb25b8965874b4d77925afc2",
        },
        pom: None,
    },
    Template {
        category: "MSA",
        name: "포털(백엔드)",
        description: "MSA 포털 백엔드 서비스",
        project_name: "egov-msa-portal-backend",
        zip: Pinned {
            file: "egovframe-msa-portal-backend.zip",
            size: 1101491,
            sha256: "dd8e7563c6eb981c4ad6641f68bf9473badccac10164f51c8357b217c64fdd1d",
        },
        pom: None,
    },
    Template {
        category: "MSA",
        name: "포털(프런트엔드)",
        description: "MSA 포털 프런트엔드",
        project_name: "egov-msa-portal-frontend",
        zip: Pinned {
            file: "egovframe-msa-portal-frontend.zip",
            size: 6562922,
            sha256: "022b3615d7140f05361f09ce54024284b3be072a0f6f355ecd6738c71bb9e8e0",
        },
        pom: None,
    },
    Template {
        category: "모바일",
        name: "모바일 웹 프로젝트",
        description: "모바일 웹 애플리케이션",
        project_name: "egov-mobile-web",
        zip: Pinned {
            file: "egovframe-mobile-web.zip",
            size: 359099,
            sha256: "b1ebcf172a775fada2418509a61b51914709eeb803ca6d03ef1bba766c9bc57b",
        },
        pom: Some(Pinned {
            file: "egovframe-mobile-web-pom.xml",
            size: 5631,
            sha256: "ad7242c46ccb6db02fab615a67077dcce08dfda394deccafefac989f634a630e",
        }),
    },
    Template {
        category: "모바일",
        name: "모바일 공통컴포넌트",
        description: "하이브리드 앱을 지원하는 모바일 공통컴포넌트",
        project_name: "egov-mobile-common-components",
        zip: Pinned {
            file: "egovframe-mobile-common-components.zip",
            size: 10696748,
            sha256: "0c0abe6922042e940221ea05f94868dac293ba4d2315d17fdc16d26ef23b42c0",
        },
        pom: Some(Pinned {
            file: "egovframe-mobile-common-components-pom.xml",
            size: 12674,
            sha256: "7869aa6dba3e7719cffb9adc45f7b1bf6d30dbc8a400593dbb8a301d1dea1fcf",
        }),
    },
    Template {
        category: "모바일",
        name: "DeviceAPI 웹 프로젝트",
        description: "디바이스 API를 연동하는 웹 프로젝트",
        project_name: "egov-mobile-deviceapi",
        zip: Pinned {
            file: "egovframe-mobile-deviceapi.zip",
            size: 176604,
            sha256: "f66bf9d235e400fb2a6525565db432754e25be9d96f6a5a7592c82d1ce0cc45e",
        },
        pom: Some(Pinned {
            file: "egovframe-mobile-deviceapi-pom.xml",
            size: 6942,
            sha256: "31f9a7ed3f392ead36b37da3905106475f9157dc0568c916d623285e4a1811da",
        }),
    },
    Template {
        category: "배치",
        name: "스케줄러(파일)",
        description: "파일 기반 배치 스케줄러",
        project_name: "egov-boot-batch-scheduler-file",
        zip: Pinned {
            file: "egovframe-boot-batch-file-scheduler.zip",
            size: 714016,
            sha256: "407b09f3c678f8bd96c5da7aa38fe574622a1b507a676d3937c1a8cafafb95d1",
        },
        pom: Some(Pinned {
            file: "egovframe-boot-batch-file-scheduler-pom.xml",
            size: 6649,
            sha256: "d820f3f9f8417be93b70030382a9983c8445d9b1ac859c0c57ee7fdbbf60f108",
        }),
    },
    Template {
        category: "배치",
        name: "명령줄(파일)",
        description: "파일 기반 명령줄 배치",
        project_name: "egov-boot-batch-commandline-file",
        zip: Pinned {
            file: "egovframe-boot-batch-file-commandline.zip",
            size: 713252,
            sha256: "c51352a2b4ce41ed64f0a0f543c5319164dcc16ac32f853bac44f1ee166535cc",
        },
        pom: Some(Pinned {
            file: "egovframe-boot-batch-file-commandline-pom.xml",
            size: 6649,
            sha256: "d820f3f9f8417be93b70030382a9983c8445d9b1ac859c0c57ee7fdbbf60f108",
        }),
    },
    Template {
        category: "배치",
        name: "웹(파일)",
        description: "파일 기반 웹 배치",
        project_name: "egov-boot-batch-web-file",
        zip: Pinned {
            file: "egovframe-boot-batch-file-web.zip",
            size: 1465409,
            sha256: "15b75e445cfd9dd0162dbf23034937acb6cac4cd25af45cc162b55ef91a7a1c8",
        },
        pom: Some(Pinned {
            file: "egovframe-boot-batch-file-web-pom.xml",
            size: 7834,
            sha256: "33203862f947e2d9103a7ea9c59cdf1c7e7278f138e77c8453231827a4941df5",
        }),
    },
    Template {
        category: "배치",
        name: "스케줄러(DB)",
        description: "DB 기반 배치 스케줄러",
        project_name: "egov-boot-batch-scheduler-db",
        zip: Pinned {
            file: "egovframe-boot-batch-db-scheduler.zip",
            size: 712628,
            sha256: "b0cb160b252325eb6c29c93d978a5026913d377339e15c5551f615ca0cd2b7e7",
        },
        pom: Some(Pinned {
            file: "egovframe-boot-batch-db-scheduler-pom.xml",
            size: 6649,
            sha256: "d820f3f9f8417be93b70030382a9983c8445d9b1ac859c0c57ee7fdbbf60f108",
        }),
    },
    Template {
        category: "배치",
        name: "명령줄(DB)",
        description: "DB 기반 명령줄 배치",
        project_name: "egov-boot-batch-commandline-db",
        zip: Pinned {
            file: "egovframe-boot-batch-db-commandline.zip",
            size: 1440218,
            sha256: "0de452a317f939f37867f66d293d8e6879fd868aa89c3ad1cc7a07c2b1c0ffd9",
        },
        pom: Some(Pinned {
            file: "egovframe-boot-batch-db-commandline-pom.xml",
            size: 6649,
            sha256: "d820f3f9f8417be93b70030382a9983c8445d9b1ac859c0c57ee7fdbbf60f108",
        }),
    },
    Template {
        category: "배치",
        name: "웹(DB)",
        description: "DB 기반 웹 배치",
        project_name: "egov-boot-batch-web-db",
        zip: Pinned {
            file: "egovframe-boot-batch-db-web.zip",
            size: 1470738,
            sha256: "e229849102dee90fe7605107d9a4122425b67ddf2aa1bbc41b01af42bbc9e0c9",
        },
        pom: Some(Pinned {
            file: "egovframe-boot-batch-db-web-pom.xml",
            size: 7417,
            sha256: "e983552b34bd870629e3e8fb0f8e71810a9ced7ff4e8e2b89a8d7066b924ab17",
        }),
    },
    Template {
        category: "AI",
        name: "RAG(Spring AI)",
        description: "Spring AI와 Redis Stack을 쓰는 RAG",
        project_name: "egov-ai-rag-springai",
        zip: Pinned {
            file: "egovframe-ai-rag-springai.zip",
            size: 95943,
            sha256: "f30cbeb0c71c7cf41142efc451aeb57e657b17da4aa50f800f590782dbfa3073",
        },
        pom: Some(Pinned {
            file: "egovframe-ai-rag-springai-pom.xml",
            size: 5555,
            sha256: "8ad4193e29bd8045f8a54ff0a212fef9ab3fe85b05568c6d5e8a914cb69eac3d",
        }),
    },
    Template {
        category: "AI",
        name: "RAG(LangChain4j)",
        description: "LangChain4j와 PostgreSQL을 쓰는 RAG",
        project_name: "egov-ai-rag-langchain4j",
        zip: Pinned {
            file: "egovframe-ai-rag-langchain4j.zip",
            size: 97636,
            sha256: "6c3d129dcdbf0f6a9759443d36bca881ac4b9f30bc689caca9380f457e784614",
        },
        pom: Some(Pinned {
            file: "egovframe-ai-rag-langchain4j-pom.xml",
            size: 6783,
            sha256: "20e82a88bd48c5a3ffb719fd91241b822f5f5f0ed14726a0dd49408f18474026",
        }),
    },
];

/// 새 프로젝트 입력값.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// 프로젝트 폴더 이름.
    pub name: String,
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    /// 프로젝트 폴더를 만들 곳.
    pub output: PathBuf,
}

impl Request {
    pub fn new(template: &Template, output: PathBuf) -> Self {
        Self {
            name: template.project_name.to_owned(),
            group_id: "egovframework.example".to_owned(),
            artifact_id: template.project_name.to_owned(),
            version: "1.0.0".to_owned(),
            output,
        }
    }

    pub fn target(&self) -> PathBuf {
        self.output.join(&self.name)
    }

    /// 잘못된 칸마다 한 줄. 이름·groupId·artifactId는 공식 확장과 같은 규칙이다. 모두
    /// XML에 안전한 글자만 받으므로 pom에 그대로 넣는다.
    pub fn problems(&self, template: &Template) -> Vec<String> {
        let mut problems = Vec::new();
        let name_segment = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        };
        if !self.name.split('.').all(name_segment) {
            problems.push(
                "프로젝트 이름: 영문·숫자·-·_만 쓰고, 점은 낱말 사이에 하나씩만 둡니다".into(),
            );
        }
        if template.pom.is_some() {
            let lower_start = |s: &str| s.starts_with(|c: char| c.is_ascii_lowercase());
            let group_segment = |s: &str| {
                lower_start(s)
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            };
            if !self.group_id.split('.').all(group_segment) {
                problems.push(
                    "groupId: 점으로 나눈 낱말마다 영문 소문자로 시작하고 소문자·숫자만 씁니다(예: com.example)"
                        .into(),
                );
            }
            if !lower_start(&self.artifact_id)
                || !self
                    .artifact_id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                problems.push("artifactId: 영문 소문자로 시작하고 소문자·숫자·-만 씁니다".into());
            }
            if self.version.is_empty()
                || !self
                    .version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
            {
                problems.push("버전: 영문·숫자·. - _ +만 씁니다(예: 1.0.0-SNAPSHOT)".into());
            }
        }
        if self.output.as_os_str().is_empty() {
            problems.push("만들 위치를 고르세요".into());
        }
        problems
    }
}

/// 받아 둔 템플릿으로 프로젝트를 만들고 그 폴더를 돌려준다. 이미 있는 폴더에는 만들지 않고,
/// 도중에 실패하면 만들던 폴더를 지운다.
pub fn create(template: &Template, request: &Request, cache: &Path) -> Result<PathBuf, String> {
    let problems = request.problems(template);
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    if !request.output.is_dir() {
        return Err(format!(
            "만들 위치가 없습니다: {}",
            request.output.display()
        ));
    }
    for (pinned, _) in template.files() {
        verify(&cache.join(pinned.file), pinned)?;
    }
    let target = request.target();
    // create_dir은 이미 있으면 실패한다. 그래서 아래에서 지우는 폴더는 늘 여기서 만든 것이다.
    fs::create_dir(&target).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => format!("이미 있습니다: {}", target.display()),
        _ => format!("{}: {e}", target.display()),
    })?;
    let result = fill(template, request, cache, &target);
    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result.map(|()| target)
}

fn fill(template: &Template, request: &Request, cache: &Path, dir: &Path) -> Result<(), String> {
    let zip = cache.join(template.zip.file);
    // extract는 폴더 밖으로 나가는 항목 경로를 막는다.
    File::open(&zip)
        .map_err(zip::result::ZipError::Io)
        .and_then(zip::ZipArchive::new)
        .and_then(|mut archive| archive.extract(dir))
        .map_err(|e| format!("{}: {e}", zip.display()))?;
    if let Some(pom) = &template.pom {
        let path = cache.join(pom.file);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = [
            ("###NAME###", request.name.as_str()),
            ("###ARTIFACT_ID###", &request.artifact_id),
            ("###GROUP_ID###", &request.group_id),
            ("###VERSION###", &request.version),
            ("###URL###", "https://www.egovframe.go.kr"),
        ]
        .iter()
        .fold(text, |text, (token, value)| text.replace(token, value));
        let pom_path = dir.join("pom.xml");
        fs::write(&pom_path, text).map_err(|e| format!("{}: {e}", pom_path.display()))?;
    }
    Ok(())
}

/// 인터넷이 막힌 PC: 다른 곳에서 가져온 파일로 캐시를 채운다. 템플릿 zip·pom 틀을 고르거나
/// 공식 확장 vsix(모든 템플릿이 들어 있다)를 고른다. 이름은 보지 않고 크기·해시로 맞춘다.
pub fn import(template: &Template, files: &[PathBuf], cache: &Path) -> Result<(), String> {
    fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let mut missing = Vec::new();
    for (pinned, _) in template.files() {
        let dest = cache.join(pinned.file);
        if verify(&dest, pinned).is_ok() {
            continue;
        }
        let direct = files
            .iter()
            .find(|f| size_matches(f, pinned) && verify(f, pinned).is_ok());
        let placed = match direct {
            Some(file) => place(&dest, pinned, |partial| fs::copy(file, partial).map(drop)).is_ok(),
            None => files.iter().any(|f| from_archive(f, pinned, &dest).is_ok()),
        };
        if !placed {
            missing.push(pinned.file);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "고른 파일 중 맞는 것이 없습니다: {} (공식 확장 {RELEASE}의 vsix나 이 파일을 고르세요)",
            missing.join(", ")
        ))
    }
}

/// vsix(zip) 안에서 이름·크기가 같은 항목을 꺼낸다. 항목 경로는 쓰지 않고 `dest`에만 쓴다.
fn from_archive(archive: &Path, pinned: &Pinned, dest: &Path) -> Result<(), String> {
    let mut zip = File::open(archive)
        .map_err(zip::result::ZipError::Io)
        .and_then(zip::ZipArchive::new)
        .map_err(|e| e.to_string())?;
    let suffix = format!("/{}", pinned.file);
    let index = (0..zip.len())
        .find(|&i| {
            zip.by_index(i).is_ok_and(|e| {
                e.size() == pinned.size && e.name().is_ok_and(|n| n.ends_with(&suffix))
            })
        })
        .ok_or_else(|| format!("{}에 {}가 없습니다", archive.display(), pinned.file))?;
    place(dest, pinned, |partial| {
        let mut entry = zip.by_index(index).map_err(io::Error::other)?;
        io::copy(&mut entry, &mut File::create(partial)?).map(drop)
    })
}

/// 옆의 임시 파일에 쓰고, 해시가 맞을 때만 `dest`로 옮긴다.
fn place(
    dest: &Path,
    pinned: &Pinned,
    write: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), String> {
    let mut partial = dest.as_os_str().to_owned();
    partial.push(".import");
    let partial = PathBuf::from(partial);
    let result = write(&partial)
        .map_err(|e| format!("{}: {e}", dest.display()))
        .and_then(|()| verify(&partial, pinned))
        .and_then(|()| fs::rename(&partial, dest).map_err(|e| format!("{}: {e}", dest.display())));
    let _ = fs::remove_file(&partial);
    result
}

fn size_matches(path: &Path, pinned: &Pinned) -> bool {
    fs::metadata(path).is_ok_and(|m| m.len() == pinned.size)
}

/// 크기·SHA-256이 고정값과 같은가.
pub fn verify(path: &Path, pinned: &Pinned) -> Result<(), String> {
    let (size, sha256) = File::open(path)
        .and_then(digest)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if size == pinned.size && sha256 == pinned.sha256 {
        Ok(())
    } else {
        Err(format!(
            "{}이(가) 고정한 파일과 다릅니다(손상 또는 변조)",
            pinned.file
        ))
    }
}

fn digest(mut file: File) -> io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let mut size = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let hex = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((size, hex))
}
