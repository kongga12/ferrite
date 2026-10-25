# Ferrite 개발 워크플로우

> 모든 Phase는 **Windows / macOS / Linux 3개 OS CI 통과**를 완료 조건에 포함한다.
> 3개 OS 전체 CI는 수동 실행과 주 1회로 돈다. PR은 Ubuntu로 검사하고, Phase를 마치는 PR은 머지 전에 3개 OS를 수동으로 돌린다.

## 1. 공통 개발 규칙

| 항목 | 규칙 |
|---|---|
| 브랜치 | `main`은 보호 브랜치. 작업은 `feature/<phase>-<기능>`에서 하고 PR로 병합 |
| CI 게이트 | PR·main 푸시는 Ubuntu에서 `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test`, 그리고 커버리지를 본다. 3개 OS(+ `cargo build --release`, 실행 파일)는 `ci.yml`의 `full` 작업이 맡고, 수동 실행·주 1회 돈다. 같은 브랜치의 이전 실행은 자동 취소한다. 비공개 저장소는 Actions 분을 Windows ×2, macOS ×10으로 세기 때문이다(2026-10에 무료 2,000분의 95% 사용). |
| 3개 OS·실제 서버 수동 실행 | `gh workflow run ci.yml --ref <브랜치>`, `gh workflow run servers.yml --ref <브랜치>` |
| 주석·커밋 메시지 | 한국어 |
| 테스트 | 코어 로직(인코딩, 버퍼, 프로토콜 파싱)은 단위 테스트 필수. UI는 수동 체크리스트로 검증 |
| 플랫폼 분기 | `#[cfg(target_os = "...")]`는 한 함수 안에 모으고, 호출부에는 분기를 흩뿌리지 않음 |
| 에러 처리 | 사용자 데이터를 다루는 경로(저장, 종료, DB 쓰기)에서는 실패를 삼키지 않음. 상태바나 대화상자로 노출 |
| 의존성 | 새 크레이트는 라이선스(허용적 라이선스만)와 3-OS 빌드 가능 여부를 확인한 뒤 추가 |

## 2. Phase 로드맵

### Phase 0 — 스켈레톤 (현재 단계)

**목표**: 3개 OS에서 실행되는 최소 편집기 루프를 만든다.

- [x] eframe 창, 메뉴바, 상태바
- [x] 폴더 열기(네이티브 대화상자), 지연 로딩 프로젝트 트리
- [x] 탭 에디터(열기, 편집, 저장, 닫기)
- [x] 인코딩 감지·보존: UTF-8(BOM 포함), CP949(MS949). CRLF 보존
- [x] 원자적 저장: 임시 파일에 쓴 뒤 rename. 심볼릭 링크와 권한 보존
- [x] 미저장 변경 보호: 탭 닫기나 창 종료 시 확인
- [x] OS별 한글 폰트 로딩
- [x] 명령줄로 폴더 열기 (`ferrite <폴더>`)
- [x] 테스트: 인코딩·저장 단위 테스트 5개, 헤드리스 UI 테스트 1개(egui_kittest, 접근성 검사 포함)
- [x] 3-OS CI 워크플로 작성 (`.github/workflows/ci.yml`)
- [x] GitHub 저장소에 올려 CI 3개 OS 통과 확인 (2026-10-23, 실행 39885760694)

**완료 조건**: CI 3개 OS 통과. 한글 CP949 파일을 열어 수정하고 저장했을 때 인코딩과 줄바꿈이 보존된다.

### Phase 1 — 에디터 코어

**목표**: 대용량 파일과 코드 편집 품질을 확보한다. 가장 큰 기술 리스크가 있는 구간이다.

- [x] 크레이트 분리: `ferrite-core`(UI 비의존), `ferrite-app`
- [x] ropey 기반 버퍼, undo/redo(편집 그룹 단위)
- [x] **가상화 렌더링 에디터 위젯**: 보이는 줄만 레이아웃하고, 줄 번호와 현재 줄 강조, IME 조합 표시
- [x] tree-sitter 하이라이팅: Rust, Python, Go, C/C++, PHP, Java, SQL, TOML/JSON/YAML
  - egov용으로 XML, HTML, JavaScript, CSS를 추가했다. JSP는 HTML로만 강조하고 나머지는 Phase 2b에서 다룬다.
- [x] 찾기/바꾸기(정규식), 줄 이동, 주석 토글, 들여쓰기 감지
- [x] 파일 외부 변경 감지(notify) 후 다시 불러오기 제안
- [x] 설정 파일(TOML): 폰트 크기, 테마, 탭 폭, 기본 들여쓰기
- [x] (추가) 파일 작업
  - 트리 우클릭 메뉴: 새 파일·새 폴더·이름 바꾸기·휴지통 삭제
  - `Ctrl+N` 새 파일, `Ctrl+Shift+S` 다른 이름으로 저장
  - 이름 규칙은 3개 OS 모두 Windows 기준으로 검사한다.

**완료 조건**: 100MB 텍스트 파일에서 스크롤과 편집 모두 60fps를 유지한다. 코어 단위 테스트 커버리지 70% 이상.

**검증 결과** (2026-10-23, Windows 11, 릴리스 빌드, `perf.rs`):

| 항목 | 결과 |
|---|---|
| 파일 | 100MB, 134만 줄 |
| 열기 | 201ms |
| 스크롤 | 평균 0.36ms, p95 0.54ms |
| 입력 | 평균 0.18ms, p95 0.31ms |
| 줄바꿈 | p95 0.32ms |

- 모두 16.7ms 예산 안이다. 단, GPU 렌더링을 뺀 CPU 측 프레임 시간이다. 보이는 줄만 그리므로 GPU 비용은 파일 크기와 무관하다.
- 커버리지는 CI의 `coverage` 작업(`cargo llvm-cov --fail-under-lines 70`)이 게이트로 검사한다.

**알려진 한계** (코드에 `ponytail:` 주석으로 표시):
- 5MB 초과 파일은 구문 강조를 끈다. UI 스레드에서 동기 파싱하기 때문이며, 백그라운드 파싱을 도입하면 상향한다.
- 한 줄 10,000자 초과분은 그리지 않는다(압축된 한 줄짜리 파일 대비).
- 위아래 이동은 문자 열 기준이다. 한글처럼 폭이 넓은 문자가 섞이면 화면 열과 어긋날 수 있다.
- 찾기는 검색할 때마다 문서 전체를 문자열로 만든다(100MB 기준 수백 ms).
- 다중 커서는 없다.
- 한글 IME: Windows에서 한영 전환 동작은 사용자가 수동 확인했다(2026-10-23). 조합 입력 표시와 macOS·Linux IME는 아직 확인하지 않았다.

### Phase 2 — LSP 통합

**목표**: 언어 지능을 붙인다.

**진행 단계** (한 번에 끝내기엔 커서 조각별로 PR):
- [x] 2-1 LSP 클라이언트 기반(`ferrite-lsp`), 문서 동기화, 진단 표시
  - 진단 표시: 물결 밑줄, 말풍선, 문제 목록, 상태바
  - 실제 rust-analyzer로 검증
- [x] 2-2 자동완성, 호버, 정의·참조 이동
  - 자동완성 팝업: `Ctrl+Space`, 트리거 문자, 단어 입력 시 자동으로 열림. 거르기, `↑↓`·`Enter`/`Tab`·`Esc`, 자동 import 반영
  - 호버: 마우스가 0.4초 머물면 표시
  - 정의로 이동: `F12`, `Ctrl+클릭`. 다른 파일이면 그 파일을 연다.
  - 참조 목록: `Shift+F12`
  - 편집 직후 서버가 버린 요청(ContentModified)은 최대 3회 자동 재시도
  - 아직 없음: 스니펫 펼치기, 이동 기록(뒤로 가기)
- [x] 2-3 포맷, 이름 바꾸기, 코드 액션
  - 문서 포맷: `Shift+Alt+F`
  - 기호 이름 바꾸기: `F2`. 다른 파일까지 바꾸면 탭으로 열어 버퍼에만 반영하고 저장하지 않는다(검토 후 저장).
  - 코드 액션: `Ctrl+.` 또는 `Alt+Enter`. 빠른 수정이 먼저 오고, 편집을 나중에 받는 resolve 방식과 서버 명령 실행·`workspace/applyEdit`을 지원한다.
  - "코드" 메뉴에 언어 기능을 모았다.
  - 오래된 편집 거부: 요청 뒤 문서가 바뀌었거나, 서버가 편집에 붙인 문서 버전이 지금과 다르면 적용하지 않고 다시 시도하라고 알린다. 실제로 겹친 응답이 코드를 깨뜨린 문제를 막은 것이다.
  - 아직 없음: 파일 만들기·이름 바꾸기·지우기 같은 리소스 작업, 저장 시 자동 포맷
- [x] 2-4 언어 서버 매니저(다운로드, 체크섬, 버전 고정)와 나머지 서버 등록
  - 목록(`ferrite-lsp::catalog`)에 버전·URL·크기·SHA-256을 고정했다.
    - 배포 파일: rust-analyzer, clangd, lemminx(XML, MyBatis 매퍼용), phpactor(.phar, 실행에 php 필요)
    - 도구로 설치: gopls(`go install`), basedpyright(전용 가상환경에 `pip install`)
  - 설치 위치는 `<데이터 폴더>/ferrite/servers/<이름>/<버전>/`이다.
    - 내려받은 파일은 크기·SHA-256이 목록과 같을 때만 푼다. zip 경로는 폴더 밖으로 못 나간다.
    - 마지막에 설치 기록(`.ferrite-bin`)을 쓴다. 기록이 없는 폴더는 중간에 멈춘 설치로 보고 쓰지 않는다.
  - 서버를 찾는 순서: Ferrite가 설치한 것 → PATH → 흔한 설치 위치.
  - 화면
    - "도구 > 언어 서버" 창: 상태(설치됨, 업데이트 있음, 직접 설치됨, 배포본 없음), 설치·업데이트·삭제
    - 상태바: 서버가 없으면 "<서버> 설치" 버튼을 띄운다. 크기는 말풍선에 보인다.
    - 시작할 때 목록보다 예전 버전이 설치되어 있으면 알린다.
  - 인터넷이 막힌 환경: "주소 복사"로 받을 파일을 알려 주고, 다른 PC에서 받아 온 파일을 "파일에서…"로 설치한다. 해시가 맞아야 한다.
  - 회사망 TLS 검사 프록시를 위해 OS 인증서 저장소로 검증한다. 프록시는 `HTTPS_PROXY` 환경 변수를 따른다.
  - 한계
    - pip는 버전만 고정하고 해시는 고정하지 않는다. 의존 휠이 플랫폼마다 달라서다.
    - ruff는 등록하지 않았다. 언어마다 서버를 하나만 띄우는 구조라서다.
    - jdt.ls는 2-5에서 등록한다.
- [x] 2-5 Java: jdt.ls + Lombok
  - 언어 서버 매니저로 jdt.ls 1.61.0(Eclipse 배포본, 공개 SHA-256)과 Lombok 1.18.46을 함께 설치한다.
    - 1.61.0이 Java 21로 도는 마지막 판이라 이것으로 고정했다. 1.62부터는 Eclipse 4.42 플랫폼이라 Java 25가 필요하다.
    - 공식 `jdtls.py`는 아직 21로 검사해서, 1.63을 Java 21로 띄우면 핵심 모듈이 로드되지 않고 바로 끝난다(실제로 확인).
    - 실제 내려받기 확인(Windows): 53MB를 17초에 받고, 서버가 12초에 준비됐다. Lombok `@Getter`의 getter 호출을 오류 없이 인식했다.
    - Lombok jar: Maven Central은 SHA-1만 준다. Central의 SHA-1과 맞는 jar로 SHA-256을 구해 고정했다.
    - 한 서버가 여러 파일을 가질 수 있게 했다. 오프라인 설치는 파일 여러 개를 골라 해시로 짝을 찾는다.
  - 실행: 공식 스크립트(`bin/jdtls.py`)와 같은 JVM 인자를 쓴다.
    - 설정 폴더는 읽기 전용으로 공유한다. `-data`는 프로젝트마다 `servers/.workspaces/jdtls/<폴더이름-해시>`로 둔다.
    - Lombok은 `-javaagent`로 붙인다.
  - Java: jdt.ls는 Java 21 이상으로 띄워야 한다. JAVA_HOME, PATH 순으로 21 이상을 찾는다.
    - 없으면 찾은 버전과 함께 이유를 상태바에 보인다.
    - 프로젝트는 Java 8·11이어도 된다.
  - jdt.ls 코드 액션의 `java.apply.workspaceEdit` 명령은 서버로 보내지 않고 클라이언트가 직접 적용한다.
  - 한계
    - 힙은 vscode-java 기본값(`-Xms100m -Xmx2G`)이다.
    - JRE를 대신 설치해 주지는 않는다(vscode-java는 JRE를 동봉).
- [x] 2-5b Spring Boot LS 중계: 한 언어에 서버 둘(jdt.ls + Spring Boot LS)을 붙이고 jdt.ls와의 메시지를 중계
  - 실제 확인(Windows, Spring Boot 3.5.6 Maven 프로젝트)
    - Java 파일만 열면 12.5초 뒤 jdt.ls가 Spring 프로젝트를 알리고 Spring Boot LS가 뜬다.
    - `application.properties`에서 `server.port`는 문제가 없고, 없는 키에만 "unknown property" 경고가 6.1초에 뜬다. 클래스패스를 jdt.ls 중계로 받았다는 증거다.
    - 처음에는 Spring Boot LS가 초기화하다 죽었다. 클라이언트가 `workspace.executeCommand` 기능을 알리지 않으면 Spring Boot LS가 명령 목록을 만들지 않아 NullPointerException이 난다. 클라이언트 기능에 추가해 고쳤다.
  - Spring Boot LS 2.4.1: Open VSX의 VS Code 확장(vsix 83MB, 공개 SHA-256)을 설치한다. 정식판만 고른다.
  - 허브가 한 언어에 서버 여럿을 붙인다.
    - 문서는 모두에게 보내고, 진단은 서버별로 모아 합친다.
    - 기능 요청은 주 서버(목록 순서)에만 보낸다. Java의 주 서버는 jdt.ls다.
  - 중계: VS Code 확장(`commons-vscode`)과 같은 규칙(`ferrite-lsp::spring`)
    - jdt.ls를 띄울 때 Spring 확장 번들 5개를 `initializationOptions.bundles`로 싣는다.
    - Spring Boot LS의 `sts/*` 요청 12종은 jdt.ls `sts.java.*` 명령으로 보내고 응답을 돌려준다.
    - jdt.ls의 `workspace/executeClientCommand` 콜백(클래스패스 변경)은 Spring Boot LS 명령으로 넘긴다.
    - Spring Boot LS가 준비되면 설정을 보내고 클래스패스 연동을 켠다.
  - 파일
    - `application*`·`bootstrap*`의 `.properties`·`.yml`은 Spring Boot LS가 주 서버다.
    - `.properties` 구문 강조를 추가했다(tree-sitter-properties). egov의 `globals.properties`에도 붙는다.
  - 언제 띄우나: Java 파일만으로는 띄우지 않는다(JVM 힙 1GB). jdt.ls 확장이 Spring 프로젝트를 찾아 알리거나 Spring 설정 파일을 열면 띄운다(VS Code와 같게).
  - Spring XML 설정 지원(`support-spring-xml-config`)을 켰다(egov의 `context-*.xml`).
  - 한계
    - Spring Boot LS의 자동완성·호버는 Java 파일에서는 아직 쓰지 않는다(주 서버 jdt.ls만 요청).
    - XML 파일은 Spring Boot LS에 보내지 않는다.
- [x] 2-6 서버 크래시 자동 재시작(백오프), 3개 OS 최종 검증
  - 3개 OS 결과(PR #8): Windows·macOS·Ubuntu 모두 통과했다. 실제 서버 테스트는 lemminx, jdt.ls + Lombok getter 인식, Spring Boot LS ↔ jdt.ls 중계다.
  - 자동 재시작
    - 실행 중이던 서버가 죽으면 1·2·4·8·16초 기다렸다 다시 띄운다. 그 사이 상태바는 "다시 시작하는 중…"이다.
    - 다섯 번 연달아 죽으면 멈추고 "오류 (눌러서 다시 시작)"으로 남긴다.
    - 1분 넘게 돌다 죽었으면 횟수를 처음부터 센다. 수동 재시작·설치·삭제도 횟수를 지운다.
    - 띄우기 전 설정 오류(Java가 낮음, 런타임 없음 등)는 다시 띄워도 안 되므로 재시작하지 않는다.
    - jdt.ls가 죽으면 Spring Boot LS도 함께 다시 띄운다(클래스패스 연동이 끊기므로).
  - 3개 OS 검증: `.github/workflows/servers.yml`
    - 실제로 내려받아 설치·실행하는 테스트(lemminx, jdt.ls + Lombok, Spring Boot LS 중계)를 Windows·macOS·Linux에서 돌린다.
    - 서버 관련 코드가 바뀐 PR, 수동 실행, 매주 한 번(고정한 배포 파일이 사라지지 않았는지) 돈다.

- JSON-RPC over stdio 클라이언트. 요청/응답 상관관계, 취소, 타임아웃 처리
- 기능: 진단(밑줄 + 문제 패널), 자동완성, 정의/참조 이동, 호버, 포맷, 이름 변경, 코드 액션
- **언어 서버 매니저**:
  - OS·아키텍처별 바이너리를 다운로드하고 SHA-256을 검증한 뒤 설치 경로에 둔다.
  - 버전을 고정하고 업데이트를 알린다.
- 대상 서버: rust-analyzer, gopls, basedpyright, ruff, clangd, phpactor, jdt.ls, Spring Boot LS
- Java:
  - jdt.ls를 Lombok 에이전트와 함께 띄운다.
  - Spring Boot LS ↔ jdt.ls 중계를 클라이언트에 구현한다.
- 서버 크래시 시 재시작(지수 백오프)하고 상태바에 표시

**완료 조건**: 7개 언어에서 완성, 정의 이동, 진단이 3개 OS 모두 동작한다.

### Phase 2b — egov 지원 (JSP · MyBatis · Spring · JPA)

**목표**: egov 프로젝트의 JSP, 매퍼 XML, Spring 설정, JPA를 IDE 수준으로 다룬다.
- 오픈소스 언어 서버가 없거나 부족해 직접 만드는 영역이다.

- `ferrite-egov-ls` 언어 서버 크레이트(Rust, LSP)
- MyBatis(iBatis 2 sqlMap 포함):
  - Mapper 메서드 ↔ XML 문장 이동
  - 참조 진단(refid, resultMap, namespace)
  - `${}`·`$prop$` SQL Injection 경고
  - `#{}` 속성 완성
- JSP:
  - HTML·Java·EL 하이라이팅
  - taglib 완성
  - 이스케이프 없는 EL 출력의 XSS 경고
  - 뷰 이름 → JSP 이동
- Spring:
  - XML 설정의 빈 class·ref·property 검증과 이동
  - `${}` 플레이스홀더 → 프로퍼티 값 이동
  - 엔드포인트 목록
- JPA:
  - `@Query` 안 엔티티·필드 완성
  - IDENTITY 미지원 DB(Oracle 11g 등)에서 `@GeneratedValue(IDENTITY)` 경고
- 테스트 코퍼스: 실제 egov 3.x·4.x 프로젝트 표본. 고객사 소스는 저장소에 넣지 않고 로컬 경로로만 참조한다.

**완료 조건**:
- egov 3.x·4.x 표본 프로젝트에서 이동과 진단이 동작한다.
- 코퍼스 기준으로 `${}`·`$prop$` 사용과 이스케이프 없는 EL 출력을 빠짐없이 탐지한다.

**진행 단계**:
- [x] 2b-1 `ferrite-egov` 언어 서버와 MyBatis·iBatis 매퍼 진단
  - 크레이트 `ferrite-egov`: tree-sitter-xml로 매퍼를 분석하고 LSP로 진단을 보낸다.
    - Ferrite에는 내장된다(`ferrite --egov-ls`, 설치 불필요). 다른 에디터는 단독 실행 파일 `ferrite-egov-ls`를 쓴다.
    - XML 문서의 보조 서버로 붙는다. 주 서버는 lemminx다.
  - 진단
    - SQL Injection: MyBatis `${x}`, iBatis `$x$`(경고). SQL 본문(텍스트·CDATA)에서만 찾고 주석과 속성값(`<if test>`)은 뺀다.
    - 정의 안 된 참조(경고): `<include refid>`, `resultMap`·`extends`, `select`. 같은 파일 → `namespace.id` → (iBatis) 전체 순으로 찾는다.
    - 같은 파일의 중복 id(오류). 문장·`<sql>`·resultMap 종류마다 따로 본다.
  - 프로젝트 색인: 열 때 프로젝트의 매퍼를 모은다. 빌드 결과물 폴더(target, bin, build 등)는 뺀다. EUC-KR(CP949) 파일도 읽는다.
  - 코퍼스 확인(로컬, 저장소 밖): 매퍼 1,462개(MyBatis 676, iBatis 786)를 실패 없이 분석했다.
    - 문자열 치환 967곳을 찾았다. 따로 짠 단순 검사기와 파일마다 일치해 누락이 0이다.
    - 테스트: `FERRITE_EGOV_CORPUS=<경로> cargo test -p ferrite-egov --test corpus -- --ignored`
- [x] 2b-2 이동: Mapper 인터페이스·DAO 문자열 id ↔ XML 문장, refid·resultMap 정의로 이동
  - 정의로 이동(F12·Ctrl+클릭)
    - XML: `refid`·`resultMap`·`extends`·`select` → 정의(다른 파일 포함), MyBatis 문장 id → Mapper 인터페이스 메서드
    - Java: Mapper 메서드 선언 → XML 문장
    - Java 호출: `sampleMapper.selectX(vo)` → XML 문장. 받는 필드·변수의 선언 타입을 import → 같은 패키지 순으로 찾는다(타입 추론 없음).
    - Java 문자열: DAO 방식 `selectList("ns.id", vo)` → XML 문장. iBatis는 id 그대로도 찾는다.
  - 진단: XML 문장이 없는 Mapper 메서드(경고, 실행하면 BindingException)
    - 매퍼 XML의 namespace와 이어진 인터페이스만 본다.
    - `@Select` 등 SQL 애너테이션 메서드와 default 메서드는 뺀다.
  - 허브: 요청을 그 기능을 지원한다고 알린 준비된 서버에만 보낸다(lemminx가 없어도 XML 이동이 된다).
    - 정의 요청은 지원하는 서버 모두에 보내 결과를 합친다. jdt.ls와 egov를 함께 묻는다.
    - 합친 결과가 여럿이면 목록으로 보여 준다. 커서가 있던 선언 자체는 뺀다.
  - egov 서버는 위치를 UTF-16으로 주고받는다. 다른 서버의 결과와 합치므로 같은 단위를 쓴다.
  - 코퍼스 확인(로컬): XML과 이어진 Mapper 인터페이스는 271개, XML이 필요한 메서드는 3,623개다.
    - 그중 125개(3.5%)는 문장이 없다.
    - 표본을 직접 확인했더니 모두 XML에 id가 없는 진짜 누락이었다. 일부는 실제로 호출되어 실행하면 오류가 날 코드였다.
  - 미룸: namespace ↔ 인터페이스 불일치 진단. 문자열 id DAO 방식은 namespace가 임의 문자열이라 오탐이 많다.
- [x] 2b-3 `#{}` 속성 완성, resultType·parameterType 클래스 완성
  - `#{}`(iBatis `#…#`) 안: VO 필드 완성. 상속 필드를 포함하고 static은 뺀다. `#{user.`처럼 점으로 하위 필드로 내려간다.
    - 타입은 문장의 parameterType(parameterClass)이다. 없으면 namespace 인터페이스 메서드의 매개변수에서 찾고, `@Param`이 있으면 그 이름이 첫 조각이다.
    - Map 계열(map·hashmap·EgovMap)은 속성을 알 수 없어 후보가 없다.
  - 별칭: iBatis sqlMap의 `typeAlias`, MyBatis 설정의 `typeAlias`·`package` 스캔.
    - 그다음 프로젝트에서 단순 이름이 같은 클래스를 찾는다(대소문자 무시).
  - 클래스 이름 속성값(resultType·parameterType·type·ofType·javaType·resultClass·parameterClass·class)
    - 프로젝트 클래스·별칭·MyBatis 기본 별칭을 보여 준다. 고르면 속성값 전체를 FQN으로 바꾼다. 200개를 넘으면 미완성 목록이다.
  - resultMap 안 `property`: resultMap 타입의 필드
  - 문맥은 커서 앞 텍스트로 판단한다(입력 중인 XML은 따옴표가 닫히지 않아 트리가 깨지기 쉽다).
  - Java 색인을 클래스까지 넓혔다: 필드, 상위 클래스, 메서드 매개변수, `@Param`.
  - 허브: 자동완성도 지원하는 서버 모두에 묻고 합친다(XML은 lemminx + egov). 트리거 문자도 합친다.
  - 코퍼스(로컬): parameterType 10,634건을 풀었다.
    - 프로젝트 VO로 풀린 것 7,411건, Map 1,060건, 프로젝트 밖 클래스 1,991건이다.
    - 못 푼 172건은 대부분 String·long 같은 기본 타입 별칭이다. 실제로 못 찾은 VO 이름은 `juVO` 6건이다.
- [x] 2b-4 JSP 하이라이팅(HTML + 스크립틀릿 Java + EL), 이스케이프 없는 EL 출력 XSS 경고
  - JSP 구간 찾기(`ferrite-core::jsp`): 주석 `<%-- --%>`, 지시자, 선언, 스크립틀릿, 표현식 `<%= %>`, EL `${}`·`#{}`
    - 출력 자리도 안다: 본문, 어떤 태그의 속성값, `<script>`·`<style>` 안. `\${`는 글자 그대로다.
    - 구문 강조와 egov 진단이 함께 쓴다.
  - 강조: HTML 층(JSP 구간을 가린 사본)과 Java 층(스크립틀릿 안쪽만 남긴 사본)을 따로 파싱하고, 그 위에 JSP 구간을 덧칠한다.
    - 편집할 때마다 문서 전체를 다시 파싱한다. `<%` 하나로 구간 경계가 편집 밖까지 바뀌기 때문이다.
    - 릴리스 빌드로 평균 1.2ms, 가장 큰 166KB JSP도 26ms다.
  - XSS 진단(egov 서버, JSP는 이 서버뿐)
    - 경고 `xss-request`: 요청 값(`param`·`paramValues`·`header`·`cookie`, `pageContext.request.*`, `request.getParameter` 등)을 그대로 출력
    - 정보 `xss-output`: 그 밖의 값을 이스케이프 없이 출력(저장형 XSS 가능). 상태바의 오류·경고 수에는 들어가지 않는다.
    - `<script>` 안이면 JS 이스케이프가 필요하다고 알린다.
    - 출력이 아닌 것: 사용자 정의 태그의 속성값(`c:if`, `c:forEach` 등, `<c:out escapeXml="false">`만 출력), `#{}`, `isELIgnored="true"` 페이지의 EL
    - 안전하다고 보는 식: `fn:escapeXml`, 결과가 참거짓·숫자인 식(비교·산술·`empty`·`fn:length`), 글자만 내보내는 삼항식, `contextPath`, 대문자 상수, `varStatus`의 `index`·`count` 등
    - 미룸: 같은 파일 `<c:set var>` 값 따라가기, 속성의 이벤트 핸들러(`onclick`)를 JS 문맥으로 보기
  - 코퍼스(로컬): JSP 3,650개, EL 42,165곳
    - 따로 짠 단순 검사기와 파일마다 EL 수가 일치한다(누락 0). 구간 찾기·강조·진단 모두 패닉이 없다.
    - 경고 93건은 모두 `param.*`·`request.getParameter`를 그대로 출력하는 자리다.
    - 정보 24,011건(페이지당 약 6.6건). 대문자 상수·반복 상태를 빼서 28,507건에서 줄였다. 남은 것은 대부분 VO 값 출력이다.
    - 이 검사 중 태그 안 한글에서 글자 중간을 자르는 패닉을 찾아 고쳤다.
- [x] 2b-5 JSP taglib 완성, include·뷰 이름 → JSP 이동
  - taglib 자동완성(egov 서버)
    - 태그 이름: `<c`는 선언된 모든 라이브러리의 `접두사:태그`, `<c:o`는 그 접두사의 태그
    - 속성 이름: 사용자 정의 태그 안(앞이 공백). 필수 속성 먼저, 이미 쓴 속성은 뺀다.
    - EL 함수: `${fn:le` → `length` 등(서명을 함께 보여 준다)
    - 선언은 파일 안과 정적 include(`<%@ include file %>`, 깊이 3)로 읽는 공통 파일에서 모은다.
    - 새 트리거 문자는 없다. 이름 글자를 치면 이미 요청하므로 Java·XML의 동작은 그대로다.
  - TLD를 찾는 곳(앞이 우선)
    - 프로젝트의 `.tld` 파일. uri로도, `web.xml`의 `taglib-uri` 매핑이나 `.tld` 경로로도 찾는다.
    - `WEB-INF/lib`의 jar(`META-INF/**/*.tld`)
    - 로컬 Maven 저장소의 잘 알려진 jar: JSTL(javax·jakarta), spring-webmvc, egov ptl.mvc(`ui:`), Tiles, Spring Security, Spring Modules(`validator:`), SiteMesh
      - 저장소 위치는 `~/.m2/settings.xml`의 `localRepository`(전자정부 개발환경처럼 바꾼 경우), 없으면 `~/.m2/repository`
      - 버전은 pom.xml·build.gradle에 적힌 것을 먼저, 없으면 가장 높은 것
  - 정의로 이동(F12)
    - JSP include 경로(지시자 `file`, `jsp:include`·`jsp:forward`의 `page`, `c:import`의 `url`) → 파일. `/`로 시작하면 웹 루트(`WEB-INF`가 있는 폴더) 기준, 아니면 현재 JSP 기준이다.
    - Java 문자열 뷰 이름(`return "sample/list"`, `new ModelAndView("…")`) → 뷰 리졸버의 접두사·접미사로 찾은 JSP. 파일이 있을 때만 대상이다.
      - 뷰 리졸버: Spring XML의 `*ViewResolver` 빈(`p:prefix`·`property`), Spring Boot `spring.mvc.view.prefix`·`suffix`
  - 구간 찾기: 따옴표 밖의 `<이름`은 새 태그로 본다. 입력 중이라 앞 태그가 닫히지 않아도 뒤가 속성으로 잘못 잡히지 않는다.
  - 코퍼스(로컬, 프로젝트마다 실제 서버를 띄워 확인)
    - 태그 목록이 나오는 파일: `c:` 2,246/2,319, `form:` 1,051/1,071, `spring:` 773/817, `ui:` 358/361, `validator:` 200/200, `tiles:` 220/221
      - 못 푼 것은 대부분 다른 JSP에 include되는 조각 파일이다. taglib을 부모가 선언한다.
    - include 5,854/6,037. 못 푼 것은 `/home.do` 같은 서블릿 URL이다.
    - 뷰 이름 2,402/3,133. 못 푼 것 중 같은 이름의 JSP가 있는 것은 56건뿐이다.
      - 나머지는 JSON·엑셀 빈 뷰, Thymeleaf 프로젝트, 뷰가 아닌 문자열이다.
  - 미룸: tagdir(`.tag` 파일) 태그, 조각 파일을 include하는 부모의 taglib 선언 따라가기, Gradle 캐시의 jar, Java 설정(`@Configuration`)의 뷰 리졸버
- [x] 2b-6 Spring XML 설정·JPA 보완(Spring Boot LS가 다루지 못하는 부분)
  - Spring XML은 egov 서버가 직접 다룬다.
    - Spring Boot LS의 XML 지원은 XML 문서를 그 서버에 보내야 하고, 그러려면 jdt.ls와 JVM 둘이 떠 있어야 한다.
    - 없는 빈·클래스·속성 진단과 플레이스홀더 이동은 Spring Boot LS에 없다.
  - Spring XML 설정(`<beans>`, `beans:beans` 포함)
    - 진단(경고)
      - `unresolved-bean`: 없는 빈 참조(`ref`, `<ref bean>`, `p:x-ref`, `parent`, `depends-on`, `factory-bean`, `transaction-manager`, `*-ref` 등)
        - 빈은 XML의 id·name·alias, 다른 네임스페이스의 id(`tx:advice` 등), `@Component` 계열·egov `@Mapper`·`@Bean` 메서드(이름 규칙은 Spring과 같다)
        - `${}`로 만든 이름은 실행 전에는 몰라 뺀다.
      - `unresolved-class`: 프로젝트 패키지인데 없는 클래스. 라이브러리 클래스는 모르므로 뺀다.
      - `unknown-property`: 프로젝트 클래스에 setter·필드가 없는 property. 상위 클래스가 라이브러리면 모르므로 뺀다.
    - 정의로 이동: 빈 참조 → 빈 정의(XML·Java), class → Java 클래스, property → setter·필드, `${키}` → `.properties`의 키(프로필별 파일 모두), `<import resource>` → 파일(`classpath:`는 `src/main/resources`·`WEB-INF/classes` 기준)
    - 자동완성: class(프로젝트 클래스), 빈 이름, property 이름(그 빈 class의 setter·필드), `${` 뒤 프로퍼티 키
  - JPA
    - `@Query`·`@NamedQuery(query)`·`createQuery("…")` 안 JPQL 자동완성
      - `FROM`·`JOIN`·`UPDATE` 뒤 엔티티 이름(`@Entity(name)`, 없으면 클래스 이름)
      - `별칭.` 뒤 엔티티 필드. `b.writer.`처럼 필드 타입으로 내려간다.
      - 문자열을 `+`로 이어도 쿼리 하나로 본다.
    - `@GeneratedValue(strategy = IDENTITY)` 진단(`jpa-identity`)
      - 경고: Hibernate 방언이 Oracle8i·9i·10g(IDENTITY는 Oracle12cDialect부터)
      - 정보: Oracle인데 버전을 모름(드라이버·JDBC URL·egov `Globals.DbType`만 보임). 11g 이하라면 SEQUENCE를 쓰라고 알린다.
      - DB 단서는 `.properties`, `application.yml`, Spring XML, `persistence.xml`에서 모은다. Tibero는 IDENTITY 지원 여부를 확인하지 못해 방언으로만 판단한다.
  - Java 색인을 넓혔다: 타입·필드 위치, 빈 이름, 엔티티 이름. 애너테이션 인자는 `value =`·`name =` 형태도 읽는다.
  - 코퍼스(로컬, 프로젝트마다 실제 서버를 띄워 확인)
    - Spring XML 설정 313개를 열어 진단 1건이 나왔다. 배포된 Tomcat 사본 안의 설정이라 Java 소스가 없어 실제로 못 찾는 빈이다.
    - 플레이스홀더 127/131 이동. 못 간 것은 `spring.profiles.active` 같은 시스템 프로퍼티다.
    - IDENTITY를 쓰는 엔티티 4개는 모두 PostgreSQL 프로젝트라 알림이 나오지 않는다(맞음).
  - 미룸
    - 엔드포인트 목록(`@RequestMapping` URL 모음): 앱에 기호 검색 화면이 생기면 Spring Boot LS의 `@/` 기호를 재사용한다.
    - 라이브러리 클래스의 property 검증(jar의 class 파일 읽기)
    - `mappedBy` 검증, 경로 조인 별칭(`JOIN b.files f`) 필드 완성

### Phase 2c — egov 개발도구 (프로젝트·설정·코드 생성)

**목표**: 공식 egov 개발환경(Eclipse)과 VS Code 확장이 제공하는 생성 기능을 3개 OS에서 제공한다.
- 언어 서버가 필요 없어 Phase 1 이후 언제든 병행할 수 있다.

- 템플릿 엔진: handlebars-rust. 공식 VS Code 확장의 Handlebars 템플릿과 호환된다.
- 새 egov 프로젝트 마법사:
  - Web, Template, Boot, MSA, Batch, Mobile, AI RAG
  - 공식 템플릿 ZIP은 버전을 고정해 내려받고 체크섬을 검증한다.
- 공통컴포넌트 추가
- 설정 파일 생성: Datasource, Transaction, ID 생성, Logging, Scheduling, Cache, Property. 출력 형식은 XML·Java Config·YAML·properties 중에서 고른다.
- CRUD 코드 생성:
  - VO, Mapper 인터페이스·XML, Service·ServiceImpl, Controller, JSP·Thymeleaf, 테스트
  - 입력은 수동 필드 정의. Phase 5 이후에는 DB 테이블에서 바로 생성한다.
- Apache-2.0 템플릿을 포함하거나 변형하면 NOTICE에 출처를 표기한다.

**완료 조건**:
- Web·Boot Web 템플릿으로 만든 프로젝트가 3개 OS에서 `mvn package`를 통과한다.
- CRUD 생성 결과가 컴파일되고, 생성된 매퍼 XML이 Phase 2b 진단을 경고 없이 통과한다.

**진행 단계**:
- [x] 2c-1 CRUD 코드 생성(`ferrite-codegen`, 도구 > egov CRUD 코드 생성)
  - 입력: `CREATE TABLE` DDL을 붙여 넣거나 필드를 직접 입력한다. 화면에서 열·필드 이름, Java 타입, 키, 필수, 길이, 설명을 고친다.
    - DDL: Oracle·Tibero·PostgreSQL·MariaDB·CUBRID 모양. 표 제약 `PRIMARY KEY (…)`, MySQL `COMMENT`, `COMMENT ON COLUMN`을 읽는다.
    - 타입: 공식 확장의 표를 따르되 `NUMBER(p,s)`는 자릿수로 고르고(≤9 Integer, ≤18 Long, 소수 BigDecimal), CLOB·NVARCHAR2·BLOB 등을 보탰다.
  - 만드는 것: VO·검색 VO, Mapper 인터페이스·XML, Service·ServiceImpl, Controller, JSP 목록·등록(수정), ServiceImpl 테스트(가짜 Mapper)
    - 실행환경 3.x(`egovframework.rte`)·4.x(`org.egovframe.rte`). 4.3도 `@Mapper`를 쓴다(`@EgovMapper`는 5.x).
    - 매퍼 SQL은 DB마다 다르다: Oracle·Tibero `ROWNUM`, PostgreSQL `LIMIT … OFFSET`, MariaDB·CUBRID `LIMIT a, b`, MariaDB `CONCAT` LIKE. 값은 모두 `#{}`이고 `jdbcType`을 붙인다(Oracle의 null 값 오류 방지).
    - Controller는 입력값을 서버에서 다시 검증한다(필수·길이, `BindingResult`). 한 페이지 목록 수에 상한을 둔다.
    - JSP는 출력을 모두 이스케이프한다(`<c:out>`, `fn:escapeXml`, `form:*`의 `htmlEscape`).
    - DDL의 설명은 Java 문자열·Javadoc·HTML에 맞게 이스케이프한다.
  - 템플릿은 Ferrite가 직접 썼다. 문맥 이름과 도우미(`concat`·`setVar`·`eq` 등)는 공식 VS Code 확장과 같다.
    - 그래서 공식 템플릿이나 고친 템플릿을 프로젝트의 `.ferrite/templates/code/`에 두면 그것으로 만든다.
    - JS Handlebars의 `배열.length`는 handlebars-rust가 못 읽어, 읽을 때 `len` 도우미로 바꾼다.
  - 프로젝트에서 기본값을 찾는다.
    - 실행환경 버전, DB(`Globals.DbType`·JDBC URL), JUnit 4·5, 인코딩(pom의 `sourceEncoding`, 없으면 기존 Java 파일), 줄바꿈
    - 기능 패키지: 가장 많은 `….service` 패키지의 부모 + 클래스 이름
    - 매퍼 폴더·파일 이름: MyBatis `mapperLocations`를 따른다(MyBatis가 읽는 곳에 두려고). 예: `mapper/**/*_SQL_${Globals.DbType}.xml` → `mapper/{기능}/{클래스}_SQL_{db}.xml`. 없으면 매퍼가 가장 많은 폴더와 그 이름 모양. `mapperLocations`가 파일을 하나씩 적는 방식이면 목록에 더하라고 알린다.
    - JSP 폴더: 웹 루트 + 뷰 리졸버 접두사. 웹 루트가 여럿이면(배포용 사본) `webapp`·`WebContent`를 먼저 고른다.
    - `WEB-INF/classes`(빌드 결과물)는 보지 않는다.
  - 파일은 프로젝트 인코딩(UTF-8·MS949)과 줄바꿈으로 쓴다. JSP `pageEncoding`, XML 선언도 맞춘다. 이미 있는 파일은 덮어쓰기를 골라야 쓴다.
  - 검증
    - 만든 매퍼 XML을 DB 5종 모두 2b 진단에 돌려 경고 0, JSP XSS 경고 0
    - 로컬: `javac`로 egov 3.10(Spring 4.3, `--release 8`)·4.3(Spring 5.3, `--release 11`) jar에 대고 컴파일 성공. 3.x는 만든 JUnit 테스트도 실행해 통과
    - 로컬: 공식 확장의 CRUD 템플릿을 Ferrite로 렌더링(`setVar`로 만드는 SET 절 확인). 확장 파일은 저장소에 넣지 않는다.
    - 로컬 코퍼스 23개 프로젝트에서 기본값을 확인했다. 이 검사로 한글 글자 중간에서 자르는 패닉과 `WEB-INF/classes`를 고르는 문제를 고쳤다.
  - 미룸: Thymeleaf 화면, 기본 키 생성(ID 생성 서비스·시퀀스. 2c-2 설정 생성과 함께), egov 5.x(`@EgovMapper`·jakarta), DB 테이블에서 바로 생성(Phase 5)
- [x] 2c-2 설정 파일 생성(도구 > egov 설정 파일 생성)
  - 종류 21가지(공식 확장과 같다)
    - Datasource: DBCP2·C3P0·JDBC, JNDI
    - Transaction: DataSource·JPA(Hibernate)·JTA. `@Transactional`과 AOP 포인트컷 적용을 고른다.
    - ID 생성: 시퀀스·테이블(접두어·자릿수 전략)·UUID
    - Logging(log4j2): 콘솔·파일·크기별·날짜별 롤링·JDBC
    - Scheduling(Quartz): QuartzJobBean Job, 메서드 호출 Job, Simple·Cron 트리거, 스케줄러
    - Cache: Ehcache 2 `ehcache.xml`, Spring 캐시. Property: EgovPropertyService
  - 형식: XML, Java Config(로깅 제외), YAML·properties(로깅). 종류마다 표(입력 칸·기본값·형식)로 정의해 화면 하나로 그린다.
  - 공식 확장과 같은 입력 칸 이름·고르는 값이라, 공식 템플릿이나 고친 템플릿을 `.ferrite/templates/config/<폴더>/<파일>`에 두면 그것으로 만든다.
  - 공식 확장과 다르게 고친 것
    - 실행환경 3.x·4.x를 모두 지원한다.
    - Java Config는 `javax.sql.DataSource`를 쓴다(공식 템플릿은 없는 `jakarta.sql.DataSource`).
    - 로깅은 log4j2로만 만든다(공식 Java 로깅은 지원이 끝난 log4j 1).
    - 값을 형식에 맞게 바꾼다(XML `&`, Java 문자열·properties의 `\`, YAML 따옴표). 이름 칸은 식별자만, 숫자 칸은 정수만 받는다.
    - Java Config 값은 `@Value`로 받아 `${…}` 플레이스홀더도 된다.
  - 프로젝트에서 기본값을 찾는다: Spring XML 폴더(`context-*.xml`이 가장 많은 곳), DB별 드라이버·URL·시퀀스 SQL·Hibernate 방언, 데이터소스 프로퍼티 키(`Globals.Url` 등이 있으면 `${Globals.Url}`로 채운다)
  - 파일은 XML·YAML·properties를 UTF-8(선언과 같게)로, Java Config는 프로젝트 인코딩으로 쓴다.
  - 검증(로컬, 실제 jar와 JDK)
    - egov 3.10(Spring 4.3)·4.3(Spring 5.3) 모두 확인했다.
    - XML: 컨테이너를 띄워 빈을 모두 만든다. 데이터소스 3종, AOP 트랜잭션, ID 생성 3종, 프로퍼티(내부·외부 파일), Quartz Job·트리거·스케줄러, Ehcache를 다룬다. JPA는 4.x에서 Hibernate 5.6 + H2로 띄웠다. JNDI·JTA는 WAS가 있어야 하므로 XSD 검증과 빈 클래스 확인까지만 했다.
    - Java Config 18개: 컴파일하고 같은 묶음으로 띄웠다.
    - log4j2: 어펜더 5종의 XML·properties를 log4j2로 읽어 어펜더가 생기는 것을 확인했다(JDBC는 H2). YAML은 문법만 확인했다.
    - 찾은 것: egov 프로퍼티 서비스는 초기화 때 `messageSource`(…/fdl/property/messages/properties)가 있어야 한다. 만든 파일에 주석으로 알린다.
  - 미룸: Ehcache 3(로컬에 jar가 없어 검증 못 함), 공통 설정(context-common의 messageSource 등) 생성
- [x] 2c-3 새 egov 프로젝트 마법사(도구 > 새 egov 프로젝트)
  - 템플릿 22종: 공식 VS Code 확장 v5.0.7의 목록과 같다(웹, 템플릿 4, Boot 3, MSA 3, 모바일 3, 배치 6, AI RAG 2). 모두 egov 5.0.0이다.
  - Ferrite는 템플릿을 싣지 않는다. 처음 한 번 받아 캐시(언어 서버 옆 `templates/egov-v5.0.7/`)에 둔다.
    - 출처: GitHub `eGovFramework/egovframe-vscode-initializr` 태그 v5.0.7. zip은 Git LFS라 `media.githubusercontent.com`, pom 틀은 `raw.githubusercontent.com`에서 받는다.
    - 파일마다 크기·SHA-256을 고정했다(LFS 포인터의 oid와 공식 vsix 안의 파일이 22종 모두 같음을 확인). 맞지 않으면 남기지 않는다.
    - 인터넷이 막힌 PC: [파일에서…]로 공식 vsix(모든 템플릿이 들어 있다)나 받아 온 zip·pom을 고른다. 이름은 보지 않고 크기·해시로 맞춘다. [주소 복사]로 받을 주소를 얻는다.
  - 만드는 방식은 공식 확장과 같다: `<위치>/<이름>`에 zip을 풀고, pom 틀의 `###NAME###`·`###GROUP_ID###`·`###ARTIFACT_ID###`·`###VERSION###`·`###URL###`을 채워 `pom.xml`을 쓴다.
    - 입력 규칙도 같다: 이름 `영문·숫자·-·_`(점은 낱말 사이에 하나씩), groupId `소문자로 시작하는 낱말.낱말`, artifactId `소문자·숫자·-`. 버전은 `영문·숫자·. - _ +`만 받는다. 모두 XML에 안전한 글자라 pom에 그대로 넣는다.
    - 이미 있는 폴더에는 만들지 않는다. 도중에 실패하면 만들던 폴더를 지운다. zip 항목 경로는 폴더 밖으로 나가지 못한다.
    - 받아 둔 파일도 만들 때마다 해시를 다시 본다. 맞지 않으면 지워서 다시 받게 한다.
  - 만든 뒤 [열기]로 Ferrite에서 연다.
  - 검증
    - 가짜 zip으로 만들기·가져오기·실패 정리·입력 검사를 확인하는 단위 테스트, 창 테스트(kittest)
    - 로컬: 공식 vsix에서 22종을 모두 가져와 만들고, pom에 채우지 않은 자리가 없음을 확인했다. 창에서 DeviceAPI 템플릿으로 만들고 연다.
    - Windows(Java 21, Maven 3.9): Web·Boot Web을 실제로 GitHub에서 받아(각 15.5MB) 만들고 `mvn package`(테스트 포함)를 통과했다.
    - 3개 OS: `egov 템플릿 mvn package` 워크플로(수동 실행, Java 17)로 같은 확인을 한다. 아직 돌리지 않았다(Actions 예산).
- [ ] 2c-4 공통컴포넌트 추가

### Phase 3 — 프로젝트 도구

- 통합 터미널: portable-pty + alacritty_terminal. 여러 탭 지원. Windows ConPTY 검증
- 파일 퍼지 검색(nucleo), 전체 텍스트 검색(ignore + grep-searcher, .gitignore 준수)
- Git: 변경 표시(거터), diff 뷰, 스테이지/커밋/푸시(git CLI), 브랜치 전환
- 도킹 레이아웃(egui_dock), 레이아웃 저장과 복원

**완료 조건**: 10만 파일 저장소에서 퍼지 검색 결과가 100ms 이내에 나온다.

### Phase 4 — 실행·디버그

- 실행 구성: 프로젝트 유형을 자동 감지(cargo, go, python, cmake, maven, gradle, composer)하고 사용자 정의 구성을 지원
- DAP 클라이언트: 중단점, 단계 실행, 변수와 호출 스택, 조건부 중단점
- 어댑터: codelldb(Rust, C/C++), delve(Go), debugpy(Python), java-debug(Java), vscode-php-debug(PHP)

**완료 조건**: 5개 어댑터로 중단점 정지와 변수 확인이 3개 OS 모두 동작한다.

### Phase 5 — DB 도구

- 연결 관리: 비밀번호는 OS 키체인(keyring)에 저장하고 설정 파일에는 평문으로 저장하지 않음
- sqlx: PostgreSQL, MariaDB, SQLite
- JDBC 사이드카(Java): Oracle 11g, Tibero, CUBRID
  - stdio JSON-RPC로 통신한다.
  - 쿼리는 반드시 PreparedStatement 파라미터 바인딩으로 실행한다.
- 스키마 브라우저, SQL 콘솔(실행 계획 포함), 결과 그리드(가상 스크롤, CSV 내보내기)
- Phase 2b와 연결: 매퍼 XML SQL의 테이블·컬럼 완성과 존재 여부 검증
- 쿼리 취소, 대용량 결과 페이징, 트랜잭션 모드(수동/자동 커밋) 표시

**완료 조건**: 5종 DB에 연결해 조회, 편집, 커밋, 롤백이 동작한다. 100만 행 결과를 스크롤할 때 메모리가 상한을 넘지 않는다.

### Phase 6 — 데이터 사이언스

- `.ipynb` 열기, 편집, 저장(nbformat v4)
- Jupyter 커널 연결(ZeroMQ 메시징): 셀 실행, 출력 렌더링(텍스트, 이미지, HTML 요약)
- DataFrame 뷰어: Arrow IPC 기반 그리드

### Phase 7 — 확장·배포

- WASM 플러그인 API(wasmtime): 명령, 패널, 언어 설정 확장
- 패키징(cargo-packager): Windows MSI/NSIS, macOS dmg, Linux deb/AppImage
- 코드 서명과 공증: Windows Authenticode, macOS notarization
- 자동 업데이트: 서명된 릴리스 매니페스트 검증

## 3. Phase별 3-OS 검증 체크리스트 (수동)

각 Phase 완료 시 OS마다 확인한다.

- [ ] 실행 직후 한글 UI 정상 표시(깨짐 없음)
- [ ] 단축키: Windows/Linux는 Ctrl, macOS는 Cmd
- [ ] 파일 대화상자 동작
- [ ] HiDPI(150%, 200% 배율) 렌더링
- [ ] 해당 Phase 신규 기능의 핵심 시나리오

## 4. 진행 순서와 의존 관계

```
Phase 0 ─► Phase 1 ─┬─► Phase 2 ─┬─► Phase 2b (egov 언어 지원)
                    │            ├─► Phase 4
                    │            └─► Phase 3 ─► Phase 6 ─► Phase 7
                    ├─► Phase 2c (egov 생성 도구, 언어 서버 불필요)
                    └─► Phase 5 (DB, Phase 1 에디터만 필요)
```

- Phase 2b·2c(egov 지원)는 egov 개발에 바로 쓰려면 가장 먼저 필요한 기능이다. Phase 3·4보다 앞당긴다.
- Phase 2c와 Phase 5(DB)의 JDBC 사이드카는 Phase 2와 병행할 수 있다.
- Phase 7의 패키징 스크립트는 언제든 앞당길 수 있다. 사내 배포가 필요해지는 시점에 진행한다.
