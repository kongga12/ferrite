# Ferrite 개발 워크플로우

> 모든 Phase는 **Windows / macOS / Linux 3개 OS CI 통과**를 완료 조건에 포함한다.
> 3개 OS 전체 CI는 수동 실행과 주 1회로 돈다. PR은 Ubuntu로 검사하고, Phase를 마치는 PR은 머지 전에 3개 OS를 수동으로 돌린다.

## 1. 공통 개발 규칙

| 항목 | 규칙 |
|---|---|
| 브랜치 | `main`은 보호 브랜치. 작업은 `feature/<phase>-<기능>`에서 하고 PR로 병합 |
| CI 게이트 | PR·main 푸시는 Ubuntu에서 `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test`, 그리고 커버리지를 본다. 3개 OS(+ `cargo build --release`, 실행 파일)는 `ci.yml`의 `full` 작업이 맡고, 수동 실행·주 1회 돈다(오래 걸려 PR마다 돌리지는 않는다). 같은 브랜치의 이전 실행은 자동 취소한다. 공개 저장소라 Actions 분 제한은 없다. 패닉 백트레이스를 로그에 남긴다(`RUST_BACKTRACE=1`). |
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
- [x] 3개 OS 검증(2026-10-26): CI 3개 OS 전체(실행 40257065684), 언어 서버 실제 설치(실행 40250457209) 통과
  - 이 검증에서 macOS로 폴더 없이 시작하면 죽던 버그를 찾아 고쳤다. 첫 프레임 전에 `Context::format_shortcut`을 불렀는데, macOS에서는 이 함수가 글꼴로 ⌘ 기호를 확인한다.
  - 파일 감시 알림 때문에 앱 테스트가 가끔 기본 프레임 상한(4)을 넘겨, 모든 앱 테스트의 상한을 한 곳(`harness_builder`)에서 32로 올렸다.

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
    - 3개 OS(Java 17): `egov 템플릿 mvn package` 워크플로로 Web·Boot Web 모두 통과했다(2026-10-26, 실행 40250455011).
- [x] 2c-4 공통컴포넌트 추가(도구 > 공통컴포넌트 추가)
  - 공식 개발환경(Eclipse)의 공통컴포넌트 플러그인(`egovframework.dev.imp.commngt`)을 그대로 쓴다.
    - 플러그인에는 목록(`components.xml` + `components_ko.properties`)과 컴포넌트별 zip(`<패키지>.Wizard.<버전>.zip`)이 들어 있다.
    - Ferrite는 이 파일들을 싣지 않는다. 사용자의 개발환경에서 캐시(`templates/commngt-<버전>/`)로 가져온다.
    - 가져올 수 있는 것: 개발환경 폴더, 플러그인 폴더·jar, 플러그인 업데이트 사이트 zip(안의 jar를 연다), 개발환경 zip.
    - 그래서 그 개발환경의 버전(3.10·4.2 확인)을 따른다. 프로젝트 실행환경과 주 번호가 같은 목록을 먼저 고른다.
  - 공식 마법사와 같은 것
    - 분류별 목록에서 고르면 의존 컴포넌트가 함께 들어간다(체크된 채로 막힘).
    - 설치 여부는 패키지 폴더로 본다.
    - 고른 zip들의 파일을 프로젝트에 푼다. 컴포넌트들이 같이 싣는 파일(`mapper-config.xml` 등)은 내용이 같아 한 번만 쓴다.
  - 공식 마법사와 다른 것
    - [미리 보기]로 새 파일·같은 파일·다른 파일을 먼저 보여 준다.
    - 내용이 다른 기존 파일은 기본으로 건너뛴다. 덮어쓰기를 고르면 `.bak`(있으면 `.bak.1`…)으로 백업한다. 줄바꿈만 다른 파일은 같다고 본다.
    - `pom.xml`은 덮어쓰지 않고, 공통컴포넌트 pom에 있는데 프로젝트 pom에 없는 의존성(`groupId:artifactId`)을 알린다.
    - 건너뛴 `.properties`는 프로젝트 파일에 없는 키를 알린다(예: `globals.properties`).
    - DB 테이블은 만들지 않는다(DB 연결은 Phase 5). 대신 프로젝트 DB의 스크립트(`src/script/ddl`·`dml/<db>/`)를 돌릴 순서대로(기본 묶음 먼저) 알린다.
    - zip 항목 경로는 프로젝트 밖으로 나가지 못한다.
  - 찾은 것: 4.2 목록은 의존에 zip이 없는 상위 패키지(`egovframework.com.sym.ccm`)를 적어 둔 곳이 있어, 그 아래 패키지 모두로 본다.
  - 프로젝트 인코딩이 MS949이거나 실행환경 주 번호가 다르면(3.x ↔ 4.x) 알린다.
  - 검증
    - 가짜 플러그인(폴더·jar·업데이트 사이트 zip)으로 가져오기, 의존 펼치기, 계획, 쓰기, 덮어쓰기·백업, pom·properties 안내를 확인하는 테스트, 창 테스트(kittest)
    - 로컬: 4.2.1 개발환경 zip(37개 분류, 153종)과 3.10 업데이트 사이트 zip(38개 분류, 154종)에서 가져와, 모든 컴포넌트를 빈 프로젝트에 넣었다(패키지 155개, 파일 약 6천 개). 다시 넣으면 모두 같은 파일이다.
    - 로컬(Java 21, Maven 3.9): 빈 프로젝트에 4.2의 FAQ를 넣으면 의존을 더해 패키지 14개(파일 1,442개)가 들어간다. 그 프로젝트가 `mvn compile`을 통과했다(Java 173개). pom은 기본 묶음(cmm)에 든 공통컴포넌트 pom이다.
  - 미룸: DB에 바로 테이블 만들기(Phase 5 DB 연결), 모바일 공통컴포넌트(`egovframework.mdev.imp.commngt`), pom 의존성 자동 추가
- [x] 3개 OS 검증(2026-10-26): CI 3개 OS 전체(실행 40257065684), 템플릿 `mvn package`(실행 40250455011) 통과

### Phase 3 — 프로젝트 도구

- 통합 터미널: alacritty_terminal(PTY 포함). 여러 탭 지원. Windows ConPTY 검증
- 파일 퍼지 검색(nucleo), 전체 텍스트 검색(ignore + regex, .gitignore 준수, UTF-8·CP949)
- Git: 변경 표시(거터), diff 뷰, 스테이지/커밋/푸시(git CLI), 브랜치 전환
- 도킹 레이아웃(egui_tiles), 레이아웃 저장과 복원

**완료 조건**: 10만 파일 저장소에서 퍼지 검색 결과가 100ms 이내에 나온다.

**진행 단계**:
- [x] 3-1 파일로 이동(Ctrl+P)·파일에서 찾기(Ctrl+Shift+F)
  - 파일 목록: `ignore`로 `.gitignore`·`.ignore`를 따른다(git 저장소가 아니어도). 숨김 파일은 넣고, `.git`·`.svn`·`.hg`·`node_modules`·`target`은 늘 뺀다. 병렬로 훑는다.
  - 파일로 이동
    - nucleo-matcher(경로 모드, 대문자가 있을 때만 대소문자 구분)로 점수를 매겨 50개를 보여 준다. 점수가 같으면 짧은 경로가 먼저다.
    - 파일 이름을 굵게, 폴더를 흐리게, 일치한 글자를 칠한다. ↑↓·Enter·Esc로 고른다.
    - 파일 감시가 열린 폴더만 보므로, 창을 열 때마다 목록을 별도 스레드에서 다시 만든다(그동안 이전 목록으로 찾는다).
  - 파일에서 찾기(아래 패널)
    - 대소문자·단어·정규식, 파일 패턴(`*.java, !*Test.java`)
    - 별도 스레드에서 병렬로 찾고 찾은 파일부터 보여 준다. [멈춤]으로 멈춘다. 일치 2만 개에서 멈추고 알린다.
    - 파일은 UTF-8·CP949로 읽는다. 바이너리·20MB 넘는 파일은 건너뛴다. 그래서 grep-searcher 대신 직접 디코딩한 글자에 `regex`를 쓴다(CP949 파일의 한글을 찾으려고).
    - 결과는 보이는 줄만 그린다. 파일 줄을 누르면 접고 펼치고, 결과 줄을 누르면 그 줄·열로 간다.
  - 검증
    - 단위 테스트(목록 규칙, 퍼지 순서·한글 위치, CP949·CRLF·단어·파일 패턴·대소문자·정규식 오류, 긴 줄 자르기), 창 테스트 2개(kittest)
    - 완료 조건(릴리스): 경로 10만 개에서 퍼지 검색 4~16ms
    - 로컬 D:\works(파일 75,042개): 목록 0.43초, 퍼지 검색 4~7ms, 텍스트 검색 파일 45,494개 4.7초(처음 읽기)
- [x] 3-2 통합 터미널(Ctrl+`, 도구 > 터미널)
  - 새 크레이트 `ferrite-term`(UI와 떨어져 단독으로 시험한다)
    - Alacritty의 터미널 엔진 `alacritty_terminal`을 쓴다: VT 해석·화면 상태·스크롤백(1만 줄)·선택, 그리고 그 안의 PTY(Windows ConPTY, Unix pty)와 읽기·쓰기 스레드(`EventLoop`). portable-pty는 같은 일을 겹쳐 하므로 쓰지 않았다.
    - 셸의 질의(커서 위치 등)에 답을 돌려준다(ConPTY는 시작할 때 커서 위치를 묻는다). 클립보드 읽기·쓰기 요청(OSC 52)은 받지 않는다.
    - `TERM=xterm-256color`, `COLORTERM=truecolor`. 기본 셸은 Windows PowerShell, Unix 사용자 셸, 시작 폴더는 프로젝트 루트.
    - 화면 스냅샷: 색을 RGB로 풀고(xterm 256색, 셸이 바꾼 색 반영) 기본 앞·뒤 색은 UI 테마를 따른다. 반전·밑줄·넓은 글자를 표시한다.
    - 키는 xterm 방식으로 보낸다(방향키 응용 프로그램 모드, 수식 키 조합, F1~F12, Ctrl+글자 제어 문자, Alt는 ESC 접두).
    - 붙여넣기: 줄바꿈을 Enter로 바꾸고, 셸이 bracketed paste를 켰으면 감싼다. 이때 붙여 넣는 글자 안의 ESC는 뺀다(감싸기를 끝내고 명령을 실행하지 못하게).
  - 패널: 아래 패널에 탭 여러 개(셸이 정한 제목, `+`, `×`로 셸 끝내기, 숨겨도 셸은 산다)
    - 칸 단위로 그린다. ASCII는 같은 색끼리 이어 그리고, 한글 등 넓은 글자는 두 칸 가운데에 하나씩 그린다.
    - 입력: 글자·IME 확정·붙여넣기, 키. Windows·Linux의 Ctrl+C는 고른 글자가 있으면 복사, 없으면 인터럽트다. Tab·방향키·Esc도 터미널이 받는다. 앱 단축키(Ctrl+S 등)는 앱이 먼저 받는다.
    - 끌어서 고르기, 마우스 휠 스크롤(입력하면 맨 아래로), IME 후보 창 위치를 커서에 맞춤
    - 화면 읽기 프로그램용으로 터미널 역할(`Role::Terminal`)과 화면 글자를 알린다.
    - 셸이 끝나면 종료 코드를 보여 준다.
  - 찾은 것: ConPTY는 곧바로 끝나는 프로세스(`cmd /c …`)의 마지막 화면을 다 내보내지 않을 때가 있다. 셸이 계속 떠 있는 통합 터미널에서는 문제가 없고, 시험만 끝나기 전에 1초 쉰다.
  - 검증
    - 단위 테스트(키 변환, 256색), 실제 셸 테스트(Windows는 ConPTY + cmd, 그 밖에는 sh): 출력·종료 코드, 입력한 명령, 한글 입출력과 두 칸 표시, 크기 변경, 선택 글자
    - 창 테스트(kittest, 실제 기본 셸): Ctrl+`로 열고 명령 실행, 탭 추가·닫기, 숨기기
    - 로컬: 실제 앱에서 PowerShell을 띄워 한글 색 출력과 `dir` 결과를 화면으로 확인했다.
- [x] 3-3 Git(소스 제어 창 Ctrl+Shift+G, 도구 > 소스 제어)
  - `ferrite-core::git`: git CLI를 부른다(`-c core.quotepath=false`로 한글 경로, `GIT_TERMINAL_PROMPT=0`으로 터미널 질문 막기, Windows에서 콘솔 창 숨김).
    - 상태: `status --porcelain=v2 --branch -z`를 읽는다(스테이지·작업 폴더 변경, 이름 바꿈, 충돌, 새 파일, 브랜치·추적 브랜치·앞섬/뒤처짐).
    - 스테이지·내리기(`add`, `restore --staged`), 커밋(메시지는 표준 입력), 브랜치 목록·전환(`switch`)
    - 푸시: 추적 브랜치가 없으면 `push --set-upstream origin <브랜치>`. 인증은 git 자격 증명 도우미에 맡긴다.
    - diff: 스테이지된 쪽·작업 폴더 쪽. 새 파일은 `--no-index`로 전체를 추가로 보인다. UTF-8·CP949로 읽는다.
  - 거터 변경 표시: 인덱스(스테이지된) 내용과 편집 중인 버퍼를 줄 단위로 견준다(`similar`, 50ms 상한, 4MB 넘으면 끔). 저장하지 않은 편집도 바로 보인다. 추가는 초록, 수정은 파랑, 지운 자리는 빨간 짧은 막대다.
  - 상태바에 브랜치와 변경 수, 누르면 소스 제어 창. 상태와 열린 문서의 인덱스 내용은 5초마다(파일을 열거나 저장하거나 git 작업을 하면 바로) 별도 스레드에서 다시 읽는다.
  - 소스 제어 창: 브랜치 전환, 새로 고침, 푸시(별도 스레드), 스테이지됨·변경 사항 목록(하나씩·모두 올리기/내리기), 고른 파일의 diff(줄 머리글자로 칠함), 커밋 메시지와 커밋
  - 검증
    - 단위 테스트: 줄 변경(추가·수정·삭제·끝 삭제·마지막 줄바꿈), 상태 파싱(한글·공백 경로, 이름 바꿈, 충돌, 분리된 HEAD)
    - 실제 저장소 테스트: 새 파일 diff, 스테이지·커밋(한글·따옴표·여러 줄 메시지)·내리기, 브랜치 전환, origin이 없을 때의 오류, 로컬 원격으로 푸시해 추적 브랜치 만들기
    - 창 테스트(kittest): 상태바 브랜치, 저장 전 편집의 거터 표시, diff, 스테이지, 커밋, 커밋 뒤 거터 표시가 사라짐
    - 실제 앱 화면: 바꾼 줄에 파란 막대, 상태바 "Git: main · 변경 2", 소스 제어 창
- [x] 3-4 도킹 레이아웃(보기 > 창 배치 초기화)
  - `egui_tiles` 0.18(MIT/Apache-2.0)을 쓴다. 계획했던 `egui_dock`은 최신 0.21이 egui 0.36까지만 지원해 지금 쓰는 egui 0.37과 맞지 않는다.
  - 창: 프로젝트·에디터(늘 있다), 터미널·문제·검색·참조(닫을 수 있다)
    - 창마다 탭 머리가 있다. 탭을 끌어 다른 묶음의 탭으로 넣거나, 옆·위아래로 나눈다. 경계를 끌어 크기를 바꾼다.
    - 기본 배치는 왼쪽 프로젝트, 오른쪽 에디터다. 도구 창은 처음 열 때 에디터 아래 한 묶음에 탭으로 모인다.
    - 단축키·메뉴·상태바로 연 창은 그 탭이 앞으로 온다. `` Ctrl+` ``과 상태바의 문제 개수는 창이 앞에 있으면 닫고, 다른 탭 뒤에 있으면 앞으로 가져온다.
  - 탭을 닫을 때
    - 검색: 진행 중인 찾기를 멈춘다.
    - 참조: 결과를 버린다.
    - 터미널: 셸은 살려 둔다. 다시 열면 그대로 보인다. 3-2의 "숨기기" 단추는 탭 닫기로 바꿨다.
  - 저장과 복원
    - 앱을 닫을 때 설정 폴더(`settings.toml`과 같은 곳)의 `layout.json`에 쓴다.
    - 시작할 때 읽는다. 파일이 깨졌거나, 같은 창이 둘이거나, 프로젝트·에디터가 빠졌으면 기본 배치로 연다.
    - 참조 결과는 저장하지 않으므로 참조 창은 빼고 연다.
    - 터미널 창이 열린 채로 복원되면 처음 그릴 때 셸을 띄운다.
  - 검증
    - 단위 테스트
      - 도구 창이 에디터 아래 한 묶음에 모인다.
      - 토글: 뒤에 있으면 앞으로, 앞이면 닫는다.
      - 저장·복원. 깨진 파일과 에디터가 빠진 배치는 기본 배치로 연다.
    - 창 테스트(kittest)
      - 검색·문제 창이 같은 묶음의 탭이 된다. 탭 누르기, 탭 닫기 단추, 창 배치 초기화를 확인했다.
      - 기존 창 테스트(터미널·검색·문제·참조)가 도킹 창에서 그대로 통과한다.
    - 실제 앱(Windows)에서 다음 순서로 확인했다.
      1. 터미널·검색 탭을 열고 정상 종료했다.
      2. 다시 실행하니 두 탭이 그대로 열렸다.
      3. 터미널 탭을 에디터 오른쪽으로 끌어 세로로 나눴다.
      4. 다시 실행해도 그 배치가 그대로였다.
  - 찾은 것: 도킹 뒤 터미널 폭이 에디터 열 폭으로 줄었다. 그래서 macOS CI의 긴 프롬프트(호스트 이름) 뒤에서 입력한 명령 줄이 접혔다. 창 테스트는 명령의 출력이 한 줄로 따로 찍혔는지로 확인한다.
- [x] 3개 OS 검증(2026-10-26): CI 3개 OS 전체(실행 40266722545) 통과. 완료 조건(경로 10만 개 퍼지 검색 100ms 이내)은 3-1에서 4~16ms로 확인했다.

### Phase 4 — 실행·디버그

- 실행 구성: 프로젝트 유형을 자동 감지(cargo, go, python, cmake, maven, gradle, composer)하고 사용자 정의 구성을 지원
- DAP 클라이언트: 중단점, 단계 실행, 변수와 호출 스택, 조건부 중단점
- 어댑터: codelldb(Rust, C/C++), delve(Go), debugpy(Python), java-debug(Java), vscode-php-debug(PHP)

**완료 조건**: 5개 어댑터로 중단점 정지와 변수 확인이 3개 OS 모두 동작한다.

**진행 단계**:
- [x] 4-1 실행 구성(`` Ctrl+F5 ``, 실행 메뉴)
  - `ferrite-core::run`: 프로젝트 루트의 표지 파일로 구성을 만든다. 하위 폴더는 보지 않는다.
    - Maven: `tomcat7-maven-plugin`이 있으면 `tomcat7:run`, `spring-boot-maven-plugin`이 있으면 `spring-boot:run`, `jetty-maven-plugin`이 있으면 `jetty:run`, 그리고 `package`·`test`
      - 로컬 egov 프로젝트 pom 27개 중 16개가 tomcat7, 6개가 spring-boot 플러그인을 쓴다.
    - Gradle: `bootRun`(Spring Boot), `run`(application 플러그인), `build`·`test`
    - 그 밖: cargo(run·test·build), go(run·test·build), CMake(구성·빌드·ctest), Python(`main.py`·`app.py`·`manage.py runserver`·pytest), Composer(install·스크립트, `pre-`·`post-`는 뺌, Laravel `artisan serve`)
    - 래퍼(`mvnw`, `gradlew`)가 있으면 그것을 쓴다.
  - 사용자 구성: `.ferrite/run.toml`의 `[[run]]`(name, command, cwd, env). 모르는 키나 빈 name·command는 오류로 알린다. [구성 편집…]은 파일이 없으면 주석뿐인 견본으로 만들어 연다.
  - 실행: 명령 줄을 플랫폼 셸이 실행한다.
    - Windows는 `cmd /d /s /c "줄"`이다. 줄을 이스케이프하지 않고 넘긴다. cmd는 C 런타임과 따옴표 규칙이 다르고, 이렇게 해야 `mvn.cmd`·`gradle.bat`도 PATH에서 찾는다.
    - 그 밖은 사용자 셸을 로그인 셸(`-l -c`)로 띄운다. GUI로 띄운 앱도 프로필이 정한 PATH를 쓰게 하려는 것이다.
  - 고르기 창: 사용자 구성 → 자동 감지 순서. 다시 열면 마지막 구성이 골라져 있어 Enter로 다시 돌린다. 창에는 글자 칸이 없으므로, 열 때 키보드 포커스를 거두고 고르는 키를 다른 창보다 먼저 받는다.
  - 터미널 창의 탭에서 돌린다. 입력도 받는다(Ctrl+C로 멈춤).
    - 탭 이름은 구성 이름이다.
    - 같은 구성을 다시 돌리면 이전 탭을 닫고(돌고 있으면 끝내고) 새로 띄운다.
    - 끝나면 종료 코드를 보인다.
  - 검증
    - 단위 테스트: 감지(egov Maven·래퍼, 여러 종류를 함께, 빈 폴더), 사용자 구성(정상·견본·잘못된 키·빈 값)
    - 실제 셸 테스트: 따옴표가 든 명령 줄, 환경 변수, 종료 코드(Windows cmd, 그 밖은 사용자 셸)
    - 창 테스트(kittest): 고르기 창의 순서, Enter로 실행하면 출력이 보임, 다시 실행하면 같은 탭을 바꿈, 구성 편집이 파일을 엶
    - 실제 앱(Windows): pom(tomcat7)과 사용자 구성이 있는 프로젝트에서 고르기 창을 열고, `mvn -v` 구성을 돌려 출력과 종료 코드 0을 확인했다.
- [x] 4-2 DAP 클라이언트(새 크레이트 `ferrite-dap`)
  - 언어 서버 클라이언트처럼 비동기 런타임 없이 스레드와 채널만 쓴다. 메시지 틀(Content-Length)은 LSP와 같아 `ferrite-lsp`의 것을 쓴다.
  - 어댑터에 닿는 방법: 표준 입출력 프로세스(debugpy·codelldb·vscode-php-debug) 또는 TCP(delve `dlv dap`·java-debug)
  - 시작 순서는 `poll`이 진행한다: initialize → launch/attach → `initialized` 이벤트 → 파일별 setBreakpoints → (필터가 있으면) setExceptionBreakpoints → configurationDone
    - launch 응답을 configurationDone 뒤에 주는 어댑터(debugpy)가 있어, launch 응답을 기다리지 않고 진행한다.
    - 시작한 뒤 바꾼 중단점은 바로 보낸다.
  - 중단점: 줄(1부터)과 조건식. 어댑터가 확인한 결과(확인됨·옮긴 줄·거부 사유)를 돌려준다.
  - 요청: threads·stackTrace·scopes·variables·evaluate(결과는 요청 번호로), 제어: continue·next·stepIn·stepOut·pause·disconnect
    - 실행을 이어 가는 명령은 보내는 즉시 실행 중으로 본다. `continued` 이벤트를 보내지 않는 어댑터가 있다.
  - 이벤트: 시작, 멈춤(사유·스레드), 이어 감, 출력(stdout·stderr·console), 종료 코드, 세션 끝, 연결 끊김(어댑터 stderr 마지막 줄)
  - 어댑터가 거꾸로 보내는 요청(runInTerminal 등)은 지원한다고 알리지 않았으므로 거절 응답을 보낸다. 세션을 버리면 어댑터 프로세스도 끝낸다.
  - 검증
    - 가짜 어댑터(TCP, 각본대로 응답): 시작 순서, 조건부 중단점과 확인·거부 결과, 거꾸로 온 요청 거절, 멈춤 → 호출 스택 → 범위 → 변수, 이어 가기, 한글 출력, 종료 코드, 연결 끊김, 시작 실패(`body.error.format`), 시작 뒤 중단점 변경, 어댑터를 띄우지 못할 때
    - 실제 debugpy 1.8.22: `return total` 줄의 조건부 중단점(`a == 41`)에서 두 번째 호출에만 멈추고, 변수 a=41·b=1·total=42를 읽은 뒤 이어 가서 출력 "결과 42"와 종료 코드 0을 받는다.
      - CI는 3개 OS 모두 debugpy를 설치해 돌린다. 로컬에 없으면 건너뛰고, `pip install --target`으로 따로 둔 것은 `FERRITE_DEBUGPY_PATH`로 알린다.
    - 찾은 것: `print`는 출력을 여러 조각("결과", " 42")으로 보낸다. 출력 창은 조각을 이어 붙여 보여야 한다.
- [x] 4-3 디버그 화면(`F5`, 실행 메뉴)
  - 디버그 구성(`ferrite-core::debug_config`)
    - 사용자 구성: `.ferrite/debug.toml`의 `[[debug]]`(name, adapter, request(launch·attach), arguments)
    - 자동 구성: 열린 파일이 `.py`면 "Python: 현재 파일"(debugpy)
    - 인자 안의 `${file}`(열린 파일)·`${workspaceFolder}`(프로젝트 루트)는 시작할 때 바꾼다.
    - 고르기 창은 실행과 같은 것을 쓴다(`picker.rs`, 구성 종류만 다름).
  - 어댑터 고르기(`ferrite-dap::adapters`)
    - 지금은 debugpy만 있다. PATH의 Python으로 `python -m debugpy.adapter`를 띄운다.
    - 따로 둔 debugpy는 `FERRITE_DEBUGPY_PATH`로 알린다.
    - 어댑터가 곧바로 끝나면 stderr로 이유를 짐작한다(예: "debugpy가 없습니다. pip install로 설치").
    - 설치 관리와 다른 언어는 4-4에서 더한다.
  - 거터
    - 줄 번호 왼쪽에 중단점 칸을 두었다. 누르면 켜고 끄고, `F9`는 커서 줄을 켜고 끈다.
    - 오른쪽 단추는 조건 창을 연다. 빈 조건이면 보통 중단점이 된다.
    - 거터에서 누른 것은 커서를 옮기거나 단어를 고르지 않는다.
    - 점 모양: 빨간 점(보통), "=" 표시(조건), 빈 점(어댑터가 걸 수 없다고 함)
    - 멈춘 줄은 노란 배경과 화살표로 보인다. 멈추면 그 파일을 열어 그 줄로 간다. 어댑터가 준 경로는 열린 문서와 같은 꼴(실제 경로)로 맞춘다.
  - "디버그" 창
    - 도구 막대: 계속 `F5`, 한 줄씩 `F10`, 들어가기 `F11`, 나오기 `Shift+F11`, 일시 정지 `F6`, 멈춤 `Shift+F5`
    - 호출 스택: 누르면 그 프레임의 변수와 자리를 보인다.
    - 변수 트리: 비싸지 않은 범위는 바로 불러온다. 나머지는 펼칠 때 불러온다.
    - egui는 덧붙은 Shift를 무시하고 단축키를 맞추므로, Shift 조합을 먼저 확인한다.
  - "디버그 출력" 창
    - 대상의 출력을 조각째 이어 붙인다(1MB 넘으면 앞에서 버림).
    - 멈췄을 때 식을 계산한다.
  - 멈춤은 끝내기 요청 뒤 세션 끝 알림을 3초까지 기다린다. 바로 어댑터를 끝내면 대상이 남을 수 있다.
  - 미룸
    - 편집으로 줄이 밀릴 때 중단점 따라가기, 중단점 저장(다시 실행하면 사라짐)
    - 예외 중단점 고르기, 조사식, 마우스를 올려 값 보기, 여러 스레드 목록
  - 검증
    - 단위 테스트: 디버그 구성(자동 구성, 변수 바꾸기, 열린 파일 없이 `${file}`, 사용자 구성·잘못된 값), 어댑터 고르기
    - 창 테스트(kittest)
      - 거터: 누르기로 켜고 끄기(커서는 그대로), 오른쪽 단추로 조건 달기, F9
      - 실제 debugpy 끝단: F9로 중단점 → F5 → 자동 구성 → 멈춤(`total = 2`, 멈춘 줄, 호출 스택 `add main.py:3`) → F10(호출한 줄로) → F5(두 번째 호출 `total = 42`) → Shift+F5(세션 닫힘)
    - 실제 앱(Windows): 거터를 눌러 건 중단점에서 멈추고, 멈춘 줄 표시·호출 스택·변수를 화면으로 확인했다.
- [x] 4-4a 어댑터 설치와 연결: debugpy·codelldb·delve·vscode-php-debug(도구 > 언어 서버의 "디버그 어댑터")
  - 목록(`ferrite_lsp::catalog::ADAPTERS`): 언어 서버와 같은 구조, 같은 설치 폴더·설치기·설치 창을 쓴다. 이름이 겹치지 않는지 목록 테스트가 함께 본다.
    | 어댑터 | 버전 | 설치 | 연결 |
    |---|---|---|---|
    | debugpy | 1.8.22 | 전용 가상환경에 pip | 표준 입출력(`python -m debugpy.adapter`) |
    | codelldb | 1.12.3 | VSIX(OS·CPU별, SHA-256 고정) | 표준 입출력 |
    | delve | v1.27.2 | `go install`(배포 실행 파일이 없음) | `dlv dap --listen=127.0.0.1:0`이 알린 주소로 TCP |
    | php-debug | 1.40.2 | VSIX(공통, SHA-256 고정) | node로 표준 입출력 |
    - 듣는 주소를 알리는 어댑터(delve)는 표준 출력에서 "listening at: 주소"를 30초까지 기다린다. 빈 포트로 띄우므로 정해 둔 포트와 부딪히지 않는다.
    - Ferrite가 설치하지 않았으면 PATH에서 찾는다. debugpy는 PATH의 python이 있다고 있는 것으로 보지 않는다(설치 창에 "설치 안 됨").
  - 구성 인자의 기본값: debugpy는 디버그 대상을 사용자의 Python으로 띄운다(설치한 가상환경에는 프로젝트 패키지가 없다). php-debug는 PATH의 php.
  - 자동 구성(열린 파일)
    - Rust: Cargo.toml의 패키지 이름으로 `target/debug/<이름>`을 `cargo build` 뒤에 띄운다(codelldb).
    - Go: 그 폴더의 패키지(delve `mode: debug`)
    - PHP: 그 파일(php-debug가 빈 포트로 기다리고 `${port}`를 Xdebug에 알린다)
  - 빌드 명령(`build`): 시작 전에 플랫폼 셸로 별도 스레드에서 돌린다. 출력(UTF-8·CP949)을 디버그 출력 창에 보인다. 실패하면 시작하지 않는다. 빌드 중 멈춤은 시작만 취소한다.
  - 찾은 것
    - Windows(PDB)에서 Rust 함수의 마지막 식 줄에는 코드가 없어 다음 줄로 옮겨 걸린다.
    - `println!` 줄처럼 한 줄에 걸 자리가 여럿이면, 이어 가도 다시 멈출 수 있다.
    - 설치 창의 표(grid) 안에서 `add_space`를 쓰면 디버그 빌드가 패닉한다(창 테스트로 찾음).
    - macOS: 가상환경의 python은 시스템 Python으로 가는 링크라 권한을 바꿀 수 없다. 설치기는 실행 권한이 빠진 경우에만 준다.
    - macOS: 임시 폴더(/var)는 /private/var로 가는 링크다. go는 실제 경로로 모듈을 찾으므로 링크 경로를 주면 "모듈 밖 폴더"로 빌드가 실패한다(앱은 실제 경로를 쓴다).
  - 검증
    - 단위 테스트: 목록(https·고정 해시·이름 중복), 듣는 주소 읽기, 어댑터 고르기·기본값 채우기, 자동 구성(Rust 패키지 이름, Go 폴더, PHP `${port}` 보존)
    - 창 테스트: 빌드 실패 시 시작하지 않고 한글 빌드 출력을 보임, 설치 창의 어댑터 구역
    - 실제 설치·디버그(`crates/ferrite-dap/tests/adapters.rs`, 무시 표시): 예제 `add(41, 1)`의 중단점에서 a=41·b=1·total=42를 읽고 끝까지 이어 간다.
      - 로컬(Windows): debugpy(가상환경 pip)·codelldb(VSIX) 통과. Go와 Xdebug 3을 쓸 PHP가 없어 delve·php-debug는 건너뛰었다.
      - 3개 OS: `.github/workflows/adapters.yml`(어댑터 코드를 바꾼 PR·수동·주 1회)이 Python·Go·PHP+Xdebug를 갖춰 넷 모두 돌린다.
- [x] 4-4b java-debug(jdt.ls 번들, 메인 클래스·클래스패스 찾기, egov 톰캣에 붙기)
  - java-debug 0.53.1(Maven Central, EPL-1.0)을 어댑터 목록에 "jdt.ls 번들"(`Launch::JdtlsBundle`)로 넣었다.
    - Maven Central은 SHA-1만 준다. 그 SHA-1과 맞는 jar에서 SHA-256을 구해 고정했다.
    - 설치되어 있으면 jdt.ls를 띄울 때 Spring Boot LS 번들과 함께 싣는다. 설치·삭제하면 jdt.ls를 다시 띄운다.
    - 따로 띄우는 프로그램이 아니므로 어댑터 고르기는 "따로 띄우지 않는다"고 알린다.
  - 시작 준비(`ferrite-dap::java::JavaStart`, 응답 해석과 순서만 맡는 상태 기계)
    - jdt.ls에 `workspace/executeCommand`로 차례로 묻는다: 메인 클래스(`vscode.java.resolveMainClass`, 열린 파일의 것을 먼저) → 클래스패스 → Java 실행 파일 → 작업 공간 빌드 → 디버그 세션 포트(`vscode.java.startDebugSession`)
    - launch에서 사용자가 적은 값(mainClass·classPaths·javaExec)은 묻지 않는다. attach는 포트만 받는다.
    - 받은 포트로 DAP를 TCP로 붙는다.
    - jdt.ls가 명령을 모르면(번들이 안 실림) "java-debug를 설치하세요"로 알린다. 빌드 실패(0)·취소(3)면 시작하지 않는다(컴파일 오류가 있어도 끝난 빌드(2)는 띄운다).
  - 앱
    - 허브에 이름으로 고른 서버로 명령을 보내는 길(`execute_command`)을 냈다.
    - jdt.ls 응답은 기다리는 요청 번호로 가로채 준비를 이어 간다(문서 위치 요청의 재시도 목록과 따로 둔다).
    - jdt.ls는 Java 파일을 열면 뜬다. 준비되지 않았으면 그렇게 알린다.
  - 자동 구성
    - Java 파일: "Java: 현재 파일(main)", "Java: 5005 포트에 붙기"(attach)
    - 실행(Maven): "`tomcat7:run` (디버그 5005)"(Maven JVM에 `MAVEN_OPTS`로 JDWP. 사용자 MAVEN_OPTS 뒤에 붙임), "`spring-boot:run` (디버그 5005)"(띄운 앱 JVM에 `jvmArguments`). 보통 실행 바로 뒤에 둔다.
    - JDWP는 `suspend=n,address=5005`: 멈추지 않고 뜨고, Java 9 이상에서는 이 컴퓨터에서만 붙을 수 있다.
    - egov 웹 프로젝트는 실행의 "디버그 5005"로 톰캣을 띄우고, 디버그의 "5005 포트에 붙기"로 붙는다.
  - 검증
    - 단위 테스트: 준비 순서(launch 전체, 적은 값은 건너뜀, attach는 포트만), 실패 안내(메인 클래스 없음, 번들 없음, 빌드 실패), 자동 구성(Java 둘, Maven 디버그 실행 위치와 MAVEN_OPTS)
    - 실제 jdt.ls 1.61 + java-debug(어댑터 시험, 무시 표시)
      - Eclipse 프로젝트(.project·.classpath)를 jdt.ls가 읽고 빌드하면, 위 순서로 포트를 받아 띄운다.
      - `return total;` 줄에서 멈춰 a=41·b=1·total=42를 읽는다. 로컬 Windows에서 통과했다(35초).
      - 3개 OS는 어댑터 워크플로가 Java 21을 갖춰 함께 돌린다.
    - 앱 화면에서 java-debug로 디버그하는 것은 확인하지 못했다. 앱은 사용자 데이터 폴더의 jdt.ls·java-debug를 쓰는데, 이번에는 임시 폴더에만 받았다.
- [x] 3개 OS 검증(2026-10-26): CI 3개 OS 전체(실행 40284841958) 통과. 완료 조건(5개 어댑터로 중단점 정지와 변수 확인, 3개 OS)은 어댑터 워크플로(실행 40284842805)에서 5종 × 3개 OS 시험 15개가 모두 통과해 확인했다.

### Phase 5 — DB 도구

- 연결 관리: 비밀번호는 OS 키체인(keyring)에 저장하고 설정 파일에는 평문으로 저장하지 않음
- 드라이버: PostgreSQL(postgres), MariaDB(mysql), SQLite(rusqlite). 처음 계획한 sqlx는 비동기 전용이라, 비동기 런타임 없이 스레드와 채널로 짠 다른 도구(LSP·DAP·터미널)와 맞추려고 동기 드라이버를 쓴다.
- JDBC 사이드카(Java): Oracle 11g, Tibero, CUBRID
  - 표준 입출력으로 한 줄에 JSON 요청·응답 하나씩 주고받는다.
  - 값은 반드시 PreparedStatement 파라미터 바인딩으로 넣는다.
- 스키마 브라우저, SQL 콘솔(실행 계획 포함), 결과 그리드(가상 스크롤, CSV 내보내기)
- Phase 2b와 연결: 매퍼 XML SQL의 테이블·컬럼 완성과 존재 여부 검증
- 쿼리 취소, 대용량 결과 페이징, 트랜잭션 모드(수동/자동 커밋) 표시

**완료 조건**: 5종 DB에 연결해 조회, 편집, 커밋, 롤백이 동작한다. 100만 행 결과를 스크롤할 때 메모리가 상한을 넘지 않는다.

**진행 단계**:
- [x] 5-1 DB 코어·드라이버·연결 관리(도구 > DB 연결)
  - 새 크레이트 `ferrite-db`
    | 대상 | 크레이트 | 라이선스 |
    |---|---|---|
    | SQLite | rusqlite 0.41(SQLite 내장 빌드) | MIT |
    | PostgreSQL | postgres 0.19(TLS 없음) | MIT/Apache |
    | MariaDB·MySQL | mysql 28(순수 Rust, TLS 없음) | MIT/Apache |
    | 비밀번호 | keyring 4.2 | MIT/Apache |
  - 연결 하나에 작업 스레드 하나. 명령(실행·다음 페이지·결과 닫기·커밋·롤백·자동 커밋)과 결과(열·페이지·끝·오류·트랜잭션)를 채널로 주고받는다.
  - 결과는 500행씩 끌어온다. 작업 스레드가 커서를 쥔 채 다음 명령을 기다리므로, 받은 페이지만큼만 메모리를 쓴다. 읽는 중에 다른 문장을 실행하면 읽던 결과를 닫고 실행한다.
  - 드라이버별
    - SQLite: 타입 있는 값 그대로. 멈춤은 인터럽트.
    - PostgreSQL: 조회는 커서(`DECLARE`·`FETCH`)의 글자 결과로 읽어 어떤 타입이든 서버가 보이는 그대로 보인다. 자동 커밋이면 잠깐 트랜잭션을 열고, 트랜잭션 안이면 세이브포인트를 둔다(조회가 실패·취소돼도 사용자의 트랜잭션이 "중단"되지 않게). 커서로 열 수 없는 조회(데이터를 바꾸는 WITH)는 그대로 실행한다. 멈춤은 취소 요청.
    - MariaDB: 글자 프로토콜로 한 행씩 흘려 읽는다. 멈춤은 다른 연결을 잠깐 열어 `KILL QUERY`.
  - 자동 커밋을 끄면 커밋·롤백 뒤 새 트랜잭션을 연다. 다시 켜면 열린 트랜잭션을 커밋한다(JDBC와 같다).
  - 연결 목록: 설정 폴더의 `connections.toml`(임시 파일에 쓴 뒤 바꿈). 비밀번호는 파일에 쓰지 않고 OS 키체인(Windows 자격 증명 관리자, macOS 키체인, Linux Secret Service)에 연결 고유 값으로 둔다. 시험은 메모리 저장소를 쓴다.
  - DB 연결 창: 새 연결(PostgreSQL·MariaDB·SQLite), 고치기(저장된 비밀번호는 그대로 두거나 바꾸거나 지움), 지우기(한 번 더 묻고 키체인 항목도 지움), 연결 시험(서버 버전 표시)
  - 검증
    - 공통 시나리오(세 드라이버): 1,234행 넣기, 500·500·234행 세 페이지, 읽다 말고 다음 문장, 롤백·커밋, 오류 뒤 계속 쓰기, 실행 중 멈춤
    - SQLite: 로컬·CI 모두
    - PostgreSQL: PATH(또는 `PGBIN`, 흔한 설치 위치)의 initdb·pg_ctl로 임시 클러스터를 띄워 돌린다(로컬 Windows 통과). 트랜잭션 안의 실패 뒤에도 트랜잭션이 살아 있는지, 데이터를 바꾸는 WITH도 본다.
    - MariaDB: `FERRITE_TEST_MARIADB`가 있을 때만. CI의 Linux 작업이 MariaDB 11.4 서비스 컨테이너를 띄워 알려 준다(로컬에는 MariaDB가 없다).
    - 창 테스트: SQLite 연결 추가·시험, PostgreSQL의 빠진 값 알림, 비밀번호는 목록이 아니라 키체인(메모리 저장소)에 있음
    - 실제 앱: 메뉴에서 DB 연결 창을 열고 새 PostgreSQL 연결 양식을 화면으로 확인했다. 사용자 키체인에 항목이 생기지 않게 저장은 하지 않았다.
  - 찾은 것
    - Windows에서 `pg_ctl start`의 출력을 파이프로 받으면, 띄운 서버가 파이프를 물려받아 닫지 않아 끝나지 않는다. 출력을 버리고 서버 로그는 파일로 남긴다.
    - Ubuntu 러너는 기본 소켓 폴더(`/var/run/postgresql`)에 쓸 권한이 없다. 임시 클러스터의 소켓은 임시 폴더에 둔다(`-k`).
    - macOS 러너에는 PostgreSQL이 없다. CI가 Homebrew로 설치한다(`postgresql@17`).
- [x] 5-2 SQL 콘솔과 결과 그리드(도구 > SQL 콘솔, DB 연결 창의 [SQL 콘솔])
  - 도킹 창 "SQL": 연결 고르기, 실행(`Ctrl+Enter`), 실행 계획, 멈추기, 자동 커밋·커밋·롤백, "트랜잭션 중" 표시, CSV로 내보내기
  - 실행할 글: 고른 글이 있으면 그 안의 문장을 차례로(오류가 나면 멈춤), 없으면 커서가 있는 문장 하나. 문장 나누기는 따옴표·주석·PostgreSQL 달러 따옴표(`$$`, `$tag$`) 안의 세미콜론을 건너뛰고, MariaDB는 따옴표 안의 역슬래시도 안다.
  - 실행 계획: SQLite `EXPLAIN QUERY PLAN`, PostgreSQL·MariaDB `EXPLAIN`
  - 결과 그리드: 보이는 행만 그린다. 끝에서 한 페이지(500행) 안쪽이 보이면 다음 페이지를 받는다. 받은 행이 10만 행을 넘으면 앞쪽을 버리고 행 번호는 이어 센다(커서는 앞으로만 읽으므로 버린 행을 보려면 다시 실행). 칸 너비는 첫 페이지로 정하고(40자 상한, 넘으면 줄임표와 마우스 올리면 전체), NULL은 기울여 흐리게.
  - 결과를 읽는 중에 다음 문장을 실행하거나 커밋·롤백하면 남은 행을 닫고 실행한다. 자동 커밋을 끈 동안은 연결을 바꾸지 못한다(열린 트랜잭션이 말없이 롤백되지 않게).
  - CSV: 마지막 조회를 따로 연 연결에서 다시 실행해 한 페이지씩 파일에 흘려 쓴다(메모리는 한 페이지만큼). UTF-8 BOM(Excel의 한글), 필요할 때만 따옴표, NULL은 빈칸. `.csv.part`에 쓰고 다 쓰면 이름을 바꾸며, 멈추거나 실패하면 지운다. 따로 연 연결이므로 커밋하지 않은 변경은 보이지 않는다.
  - 값 고치기는 SQL 문장(UPDATE·DELETE)으로 한다. 그리드에서 바로 고치기는 5-2b로 나눴다.
  - 검증
    - 단위: 문장 나누기(따옴표·주석·달러 따옴표·역슬래시·커서 위치), 결과 버퍼(상한·버린 수·다음 페이지 요청), CSV 필드
    - 100만 행: SQLite 재귀 조회 100만 행을 끝까지 받는 동안 버퍼가 늘 10만 행 이하이고, 마지막 행이 1,000,000번인지 본다.
    - CSV: 1,203행을 내보내 BOM·따옴표·NULL을 보고, 실패하면 파일이 남지 않는지 본다.
    - 창 테스트(SQLite): 연결 창의 [SQL 콘솔]로 붙기, 세 문장을 골라 실행, 그리드의 한글·NULL, CSV 내보내기, 자동 커밋 끄기 → 지우기 → 롤백 → 행이 돌아옴, 실행 계획
    - 실제 앱(Windows): 30만 행 SQLite 파일에 붙어 조회하고 그리드가 첫 두 페이지를 받는 것을 화면으로 확인했다. 임시 연결 목록은 확인 뒤 지웠다.
    - 확인하지 않은 것: 10만 행을 넘겨 앞쪽을 버린 뒤 스크롤 위치를 같은 행으로 되돌리는 동작(시험으로는 버퍼만 본다)
- [x] 5-2b 그리드에서 값 고치기(기본 키로 찾은 행을 파라미터 바인딩한 UPDATE로 고친다)
  - 고칠 수 있는 결과: 결과 열이 모두 한 표에서 왔고 그 표의 기본 키 열이 모두 결과에 있을 때. 식·별칭 식 열은 못 고친다(열 이름이 아니라 드라이버가 알려 주는 원래 표·열로 판단하므로 `SELECT b AS a`도 b를 고친다). 그리드 위에 "표 · 칸을 두 번 눌러 고칩니다" 또는 "읽기 전용"을 보인다.
  - 드라이버마다 원래 표·열을 아는 법
    - SQLite: `column_metadata`(rusqlite 기능)로 열의 DB·표·원래 열, 기본 키는 `PRAGMA table_info`
    - PostgreSQL: 커서를 연 뒤 문장을 준비만 해(실행하지 않음) 열의 표 oid·열 번호를 얻고, 카탈로그에서 열 이름·형식(`format_type`)·기본 키를 읽는다. 보통 표와 나눈 표만.
    - MariaDB: 결과 열의 원래 스키마·표·열(`org_table`, `org_name`), 기본 키는 `information_schema.KEY_COLUMN_USAGE`. 결과를 흘려 읽는 동안 첫 연결은 다른 문장을 못 쓰므로 둘째 연결을 열어 둔다(끊겼으면 한 번 다시 연다).
  - 고치기: 칸을 두 번 눌러 위쪽 줄에서 새 값을 적고 Enter(또는 [저장]), Esc는 그만두기. NULL 칸 고르기. 바이트 값은 못 고친다.
  - 실행: `UPDATE 표 SET 열 = ? WHERE 기본키 = ? …`. 표·열 이름은 따옴표로 감싸고(SQLite·PostgreSQL `"`, MariaDB `` ` ``), 값은 모두 파라미터로 묶는다(SQL 글에 붙이지 않는다). PostgreSQL은 파라미터를 글자로 보내고 `CAST($n AS 열 형식)`으로 바꾼다.
  - 결과: 1행이면 그리드 칸을 바꾸고 "N행 값을 고쳤습니다"(트랜잭션 안이면 "커밋해야 저장됩니다"). 0행이면 "다른 곳에서 바뀌었거나 지워졌을 수 있음". MariaDB는 `CLIENT_FOUND_ROWS`로 바뀐 행이 아니라 찾은 행을 센다(같은 값으로 고쳐도 1행, JDBC 기본과 같다).
  - PostgreSQL 트랜잭션 안에서는 세이브포인트를 둔다. 틀린 값으로 실패해도 사용자의 트랜잭션이 "중단"되지 않는다.
  - 결과를 읽는 중에 고치면 남은 행을 닫고 고친다(커서는 앞으로만 읽고 MariaDB는 흘려 읽는 중 연결을 못 쓴다). 기록에 "남은 행은 닫았습니다"를 남긴다.
  - 검증
    - 단위: UPDATE 문장(복합 기본 키, 종류별 따옴표·파라미터, 이름 속 따옴표), 기본 키가 없거나 빠지면 고칠 수 없음, 결과 버퍼의 칸 바꾸기
    - 공통 시나리오(세 드라이버): 복합 기본 키 표에서 식 열은 못 고침, 따옴표·세미콜론이 든 값, 글자로 적은 수, NULL, 같은 값(1행), 지워진 행(0행), 트랜잭션 안에서 틀린 값 뒤에도 계속 쓰고 롤백, 기본 키 없음·빠짐·두 표는 읽기 전용. SQLite·PostgreSQL은 로컬 통과, MariaDB는 CI.
    - 창 테스트(SQLite): 두 번 눌러 이름 고치기(Enter), NULL 칸을 값으로([저장]), 트랜잭션 안에서 NULL로 고친 뒤 롤백하면 다시 실행했을 때 앞 값, 기본 키가 빠진 조회는 읽기 전용
    - 실제 앱(Windows): 2,000행 SQLite 표에서 칸을 두 번 눌러 고치고, 파일에 바뀐 값이 들어간 것을 확인했다. 임시 연결 목록은 확인 뒤 지웠다.
  - 찾은 것: kittest는 기본으로 한 프레임에 0.25초를 흘려 두 번 누르기가 egui의 더블 클릭 간격(0.3초)을 넘는다. 콘솔 창 시험은 한 프레임을 1/60초로 둔다. 또 0.3초 안에 다른 곳을 누른 뒤의 두 번 누르기는 egui가 세 번 누르기로 센다.
- [x] 5-3 JDBC 사이드카(Oracle 11g·Tibero·CUBRID)
  - 사이드카: `crates/ferrite-db/sidecar/FerriteJdbc.java` 한 파일(Java 8 문법, 의존 없음). 연결마다 `java` 프로세스 하나를 띄워 표준 입출력으로 한 줄에 JSON 하나씩 주고받는다(요청 `{"method", "params"}`, 응답 `{"result"}`·`{"error"}`). 멈춤(`cancel`)만 응답 없이 읽는 스레드에서 바로 `Statement.cancel()`한다. 드라이버가 표준 출력에 쓰는 기록은 표준 오류로 돌리고, 사이드카가 끝나면 표준 오류의 마지막 줄을 오류에 붙인다.
  - 컴파일: 처음 쓸 때 사용자 JDK의 javac(JAVA_HOME, PATH)로 `--release 8`로 컴파일해 사용자 데이터 폴더의 `ferrite/jdbc/<소스 해시>/`에 둔다(javac 8이면 그대로). 소스가 바뀌면 다른 폴더라 다시 컴파일한다. JRE만 있으면 "JDK가 필요합니다".
  - 드라이버 jar: Ferrite는 싣지 않는다. 연결 창에서 (1) [jar 고르기…], (2) 로컬 Maven 저장소(`~/.m2/repository`)와 열린 프로젝트의 `WEB-INF/lib`·`lib`에서 이름으로 찾은 jar를 [추가], (3) [받기]로 받는다. 클래스는 종류마다 정해 싣는다(`oracle.jdbc.OracleDriver`, `com.tmax.tibero.jdbc.TbDriver`, `cubrid.jdbc.driver.CUBRIDDriver`).
    | 받기 | 저장소 | 라이선스 |
    |---|---|---|
    | ojdbc6 11.2.0.4, orai18n 11.2.0.4(한글 문자셋 DB) | Maven Central | Oracle Free Use Terms |
    | cubrid-jdbc 11.2.0.0035 | maven.cubrid.org | BSD-3-Clause |
    - 크기·SHA-256을 고정한다. 저장소의 SHA-1·크기와 맞는 jar에서 SHA-256을 구했다. Tibero는 공개 저장소가 없어 직접 고른다.
  - 연결: 호스트·포트·데이터베이스로 URL을 만든다(Oracle `jdbc:oracle:thin:@//호스트:포트/서비스`, Tibero `jdbc:tibero:thin:@호스트:포트:DB`, CUBRID `jdbc:cubrid:호스트:포트:DB:::`). [JDBC URL]을 적으면 그것을 쓴다(Oracle SID 등).
  - 실행: `Statement.execute`. 결과 집합이면 500행씩 보내고 커서를 쥔 채 기다린다(다른 드라이버와 같다). 값은 드라이버가 보이는 글자 그대로(`getString`), 이진 값(RAW·BLOB 등)은 바이트, CLOB은 글자. 끝의 `;`는 뗀다(Oracle ORA-00911). 값 바인딩이 필요한 문장은 PreparedStatement 파라미터로만 실행한다.
  - 실행 계획: Oracle은 `EXPLAIN PLAN FOR` 뒤 `DBMS_XPLAN.DISPLAY()`를 본다. CUBRID(계획을 문장이 아니라 드라이버 API로만 준다)는 [실행 계획]을 끈다. Tibero는 5-3c에서 켰다.
  - 이 단계의 결과 그리드는 읽기 전용이었다. Oracle 드라이버는 결과 열의 표를 알려 주지 않고, CUBRID는 별칭을 열 이름으로 알려 줘 엉뚱한 열·행을 고칠 수 있다. 그리드 편집은 5-3b에서 SQL을 읽어 표·열을 찾는 방식으로 했다.
  - 찾은 것
    - Oracle 드라이버는 문장을 멈출 때 TCP 긴급 데이터(OOB)를 보내는데 Docker(그리고 흔한 NAT·방화벽)를 거치면 닿지 않아 멈추지 않는다. `oracle.net.disableOob=true`로 연결 안의 멈춤 신호를 쓰게 하니 1.2초 만에 ORA-01013으로 멈췄다.
    - Oracle 드라이버는 트랜잭션 중에 연결을 닫으면 커밋한다. 사이드카는 닫기 전에 롤백한다(다른 DB처럼 커밋하지 않은 변경은 버린다).
    - 문장이 도는 중에 Ferrite가 끝나면 사이드카의 입력이 끝난다. 사이드카는 실행 중인 문장을 멈추고, 10초 안에 끝나지 않으면 스스로 끝난다(서버가 끊긴 연결을 롤백한다). 처음에는 멈춤이 듣지 않은 문장 때문에 사이드카가 남았다.
    - ojdbc6(JDBC 4.0)과 CUBRID 드라이버는 `Connection.getSchema()`가 없거나 지원하지 않는다. 사이드카는 쓰지 않는다.
  - 검증
    - 단위: URL 만들기·빠진 값(드라이버 jar, 서비스 이름, `jdbc:`), 드라이버 목록 저장, jar 이름으로 찾기(소스·문서 jar 빼기), JSON 칸·파라미터, 받을 드라이버 고정 값
    - 공통 시나리오(Oracle XE 11g, CUBRID 11.2, 로컬 Docker): 1,234행을 세 페이지로, 읽다 말고 다음 문장, 자동 커밋 끄고 롤백·커밋, 오류 뒤 계속 쓰기, 실행 중 멈춤(데카르트 곱 조회)
    - JDBC 시나리오: 따옴표·세미콜론·한글 값과 NULL을 파라미터로 넣기, 결과는 읽기 전용, 이진 값은 바이트, 트랜잭션 중에 닫으면 롤백(새 연결의 UPDATE가 앞 연결의 잠금이 풀리길 기다렸다가 2행을 바꾼다). Oracle은 실행 계획과 CLOB도 본다.
    - 드라이버 싣기: 세 종류 jar로 닫힌 포트에 연결해 드라이버의 연결 오류가 나오는지(Tibero는 로컬 프로젝트의 tibero6 jar, 저장소에 넣지 않는다), jar가 없을 때, 다른 DB의 jar일 때
    - 창 테스트: Oracle 연결에서 jar가 없으면 알리고, 프로젝트 `WEB-INF/lib`에서 찾은 jar를 [추가]해 저장한다. 시험은 받을 곳이 없어 [받기]가 꺼져 있다.
    - CI(Linux): Oracle XE 11g·CUBRID 11.2 서비스 컨테이너, 러너의 JDK, `mvn dependency:get`으로 받은 드라이버
    - 실제 앱(Windows): CUBRID 연결 시험(처음 쓸 때 사이드카 컴파일), SQL 콘솔에서 2,000행 표를 조회해 한글·NULL·날짜와 "1000행+ · 읽기 전용", 꺼진 [실행 계획]을 화면으로 확인했다. 임시 연결 목록과 사이드카 폴더는 확인 뒤 지웠다.
    - 확인하지 않은 것: Tibero 서버 시나리오(5-3c에서 개발 서버로 했다), 실제 앱에서 [받기] 누르기(고정 값은 저장소의 SHA-1·크기와 맞춰 확인했다)
- [x] 5-3b JDBC 결과 그리드 편집(SQL을 읽어 한 표에서 온 열과 기본 키를 찾는다)
  - 드라이버가 결과 열의 출처를 알려 주지 않으므로(Oracle은 표 없음, CUBRID는 `v AS n`을 n 열로 알림) SQL에서 찾는다(`ferrite-db/src/origin.rs`). 모르는 모양이면 읽기 전용이다(엉뚱한 열·행을 고치는 것보다 낫다).
    - 받는 모양: `SELECT [ALL] 항목들 FROM [스키마.]표 [[AS] 별칭] [WHERE …] [ORDER BY …] [FOR UPDATE] [OFFSET/FETCH/LIMIT]`. WHERE 안의 부분 조회는 괜찮다.
    - 받지 않는 모양: 조인(쉼표, JOIN), GROUP BY·HAVING, UNION·INTERSECT·MINUS, DISTINCT, CONNECT BY, WITH, FROM 안의 부분 조회, DB 링크(`@`), 플래시백(`AS OF`) 등
    - 고칠 수 있는 항목: `열`, `별칭.열`, `*`, `별칭.*`만. 별칭을 붙인 열(`name AS id`)과 식은 못 고친다. 항목 이름은 드라이버가 준 결과 열 이름과 같아야 한다(다르면 SQL을 잘못 읽은 것으로 본다).
  - 표의 열·기본 키: 사이드카가 `DatabaseMetaData.getColumns`·`getPrimaryKeys`로 알려 준다(결과를 읽는 중에도 묻는다). 스키마를 적지 않았으면 사용자 스키마의 표만 찾는다(동의어나 다른 스키마의 같은 이름 표를 고치지 않게). 없거나 둘이면 읽기 전용. 따옴표 없는 이름은 DB가 두는 대소문자로 바꿔 맞춘다(Oracle 대문자, CUBRID 소문자). CUBRID 11.2는 스키마 칸이 비고 표 이름이 `소유자.표`로 와서 나눈다.
  - Oracle 날짜: 드라이버는 DATE를 `2024-01-02 00:00:00.0`처럼 보인다. 세션 날짜 형식(NLS)에 기대지 않게 `CAST(TO_TIMESTAMP(?, 'YYYY-MM-DD HH24:MI:SS.FF') AS DATE)`(TIMESTAMP는 `TO_TIMESTAMP(…)`)로 넣는다. 날짜만 적어도 된다. CUBRID는 보인 글자를 그대로 넣어도 바뀐다.
  - Tibero는 5-3c에서 켰다.
  - 검증
    - 단위: 한 표 조회와 항목(별칭.열, 따옴표 식별자, 별, 주석·글자 안 FROM), 받지 않는 모양 14가지, 별칭 열·다른 별칭·이름이 다른 열은 고칠 수 없음, 대소문자 규칙, Oracle 날짜 UPDATE 문장
    - Oracle XE 11g·CUBRID 11.2(로컬 Docker): 5-2b 공통 편집 시나리오(복합 기본 키, 식 열, 따옴표·세미콜론 값, 글자로 적은 수, NULL, 같은 값 1행, 지워진 행 0행, 트랜잭션 안 틀린 값 뒤 롤백, 기본 키 없음·빠짐·두 표는 읽기 전용), `SELECT *`, 기본 키 이름을 별칭으로 붙인 열은 못 고치고 조건은 진짜 기본 키 값, Oracle DATE·TIMESTAMP를 보인 글자·날짜만으로 고치기
    - 실제 앱(Windows): CUBRID 2,000행 표에서 "dba.demo_emp · 칸을 두 번 눌러 고칩니다"를 보고 2행 이름을 두 번 눌러 고쳐, DB에서 그 행만 바뀐 것을 확인했다. 임시 연결 목록과 사이드카 폴더는 지웠다.
  - 찾은 것: 처음에는 ORDER BY 목록의 쉼표를 표를 더하는 쉼표로 읽어 `ORDER BY k1, k2` 조회를 읽기 전용으로 봤다(실서버 시나리오에서 찾음). 표를 더하는 쉼표는 표 이름 바로 뒤에서만 막는다.
- [x] 5-3c Tibero 실서버 시험: 멈춤, 그리드 편집, 실행 계획
  - 개발용 Tibero 6 서버(사용자가 준 접속 정보, 저장소 밖 `ref/.env`)와 로컬 프로젝트의 tibero6 드라이버(6.0, 저장소에 넣지 않는다)로 시험했다. CI에는 Tibero 이미지가 없어 돌지 않는다.
  - 멈춤: Tibero 6은 `Statement.cancel()`·`setQueryTimeout` 모두 문장 종류와 상관없이 약 60초 뒤에야 멈춘다(JDBC-12040). `ALTER SYSTEM CANCEL SESSION`은 없다(구문 오류). 그래서 사이드카는 cancel 뒤 3초 안에 멈추지 않으면 다른 연결에서 이 연결의 세션(`V$MYSTAT`의 SID와 `V$SESSION`의 SERIAL#, 숫자만)을 `ALTER SYSTEM KILL SESSION`으로 끊고, 다시 연결해 자동 커밋 상태를 되살린 뒤 "세션을 끊고 다시 연결했습니다(커밋하지 않은 변경은 롤백됐습니다)"로 알린다. `V$` 뷰나 ALTER SYSTEM 권한이 없으면 끊지 않는다(약 60초 걸린다).
    - 찾은 것: 처음에는 끊기 판단 직후 문장이 먼저 끝나 이미 쉬는 연결을 끊었고, 다음 문장이 실패했다. 끊기 판단과 문장 끝내기를 한 잠금으로 묶고, 끊는 중이면 끝날 때까지 기다린 뒤 다시 연결한다.
  - 그리드 편집: Oracle과 같이 SQL에서 표를 찾는다(`getTableName`은 비어 있다). DATE는 `2024-01-02 00:00:00`으로 보이고 `TO_TIMESTAMP(?, 'YYYY-MM-DD HH24:MI:SS.FF')`로 되돌린다.
  - 실행 계획: `DBMS_XPLAN`이 없어 `PLAN_TABLE`을 쓴다(STATEMENT_ID `ferrite`를 지우고, `EXPLAIN PLAN SET STATEMENT_ID`, 깊이대로 들여 OPERATION·OBJECT_NAME·COST). 자동 커밋을 끈 동안에는 이 DELETE·EXPLAIN이 사용자의 트랜잭션 안에서 돈다(롤백하면 계획 행도 사라지고, 커밋하면 남는다).
  - 연결 속성 `program_name=Ferrite`로 DB 관리자가 `V$SESSION`에서 Ferrite 연결을 알아본다.
  - 검증(Tibero 6 개발 서버, 두 번 연속 통과): 공통 시나리오(페이지, 읽다 말고 다음 문장, 롤백·커밋, 오류 뒤 계속, 멈춤 3초대), JDBC 시나리오(파라미터, 별칭 열, 이진 값, 닫을 때 롤백), 5-2b 공통 편집 시나리오, 스키마 시나리오, DATE·TIMESTAMP 고치기, 실행 계획
- [x] 5-4 스키마 브라우저(도구 > 스키마 브라우저)
  - 연결의 기본 자리에 있는 표·뷰와 열(이름, DB가 보이는 형식)을 한 번에 읽는다(`Connection::load_schema` → `Event::Schema`). 콘솔 연결의 결과를 닫지 않게 따로 연결해 읽고 닫는다.
    | 종류 | 읽는 곳 | 범위 |
    |---|---|---|
    | SQLite | `sqlite_master`, `PRAGMA table_info` | main |
    | PostgreSQL | `pg_class`·`pg_attribute`, `format_type` | `pg_*`·`information_schema` 밖의 모든 스키마(`public`은 스키마 이름 없이 먼저) |
    | MariaDB | `information_schema.COLUMNS`·`TABLES` | 현재 DB(없으면 시스템 DB 밖의 모든 DB) |
    | Oracle·Tibero·CUBRID | `DatabaseMetaData.getTables`·`getColumns`(스키마 전체를 한 번에) | 사용자 스키마. Oracle 휴지통(`BIN$…`)은 뺀다 |
  - 창: 연결 고르기(고르면 바로 읽는다), [새로 고침], 표·열 이름으로 찾기(열 이름으로 찾으면 그 표를 펼친다), "표 N개 · 뷰 M개", 표마다 열과 형식, [SQL 콘솔에서 조회](`SELECT * FROM 표`. 따옴표 없이 같은 이름이 아니면 따옴표로 감싼다: Oracle·Tibero 대문자, PostgreSQL 소문자)
  - 기본 키 표시는 하지 않았다. JDBC는 표마다 `getPrimaryKeys`를 물어야 해 표가 많으면 느리다. 필요하면 DB별 사전 조회로 한 번에 읽는다.
  - 검증
    - 단위: 식별자 따옴표(종류별 대소문자, 한글·공백)
    - 공통 시나리오(SQLite·PostgreSQL 로컬, Oracle XE 11g·CUBRID 11.2 로컬 Docker, MariaDB CI): 표와 뷰를 만들어 읽고, 뷰 표시와 열 순서·형식(`char`가 든 이름)을 본다
    - 창 테스트(SQLite): 한글 이름 표·영문 표·뷰를 만들고 연결을 골라 "표 2개 · 뷰 1개", 열 이름(price)으로 찾기, 표를 펼쳐 [SQL 콘솔에서 조회] → `SELECT * FROM "주문"` 2행
    - 실제 앱(Windows): CUBRID 연결을 골라 `demo_emp` 표와 열·형식을 보고, [SQL 콘솔에서 조회]로 SQL 탭이 열려 조회되는 것을 화면으로 확인했다. 임시 연결 목록과 사이드카 폴더는 지웠다.
- [x] 5-4b 매퍼 SQL의 테이블·컬럼 완성·검증
  - 매퍼 기능은 앱이 띄우는 별도 LSP 서버(`ferrite --egov-ls`)라 앱의 DB 연결을 모른다. 그래서 앱에서 붙인다(`ferrite-app/src/mapper_db.rs`). 스키마는 스키마 브라우저가 읽은 것을 쓴다. 읽지 않았으면 아무것도 하지 않는다.
  - 문장 본문(`ferrite_egov::mapper::statement_sql`): 매퍼의 문장·`<sql>` 조각마다 태그·주석을 뺀 글(텍스트·CDATA)을 이어 SQL로 만든다. `#{…}`·`${…}`(iBatis `#…#`·`$…$`)는 같은 길이의 `?`와 빈칸으로 가리고(값이나 실행할 때 정해지는 글), `<where>`·`<set>`·`<trim prefix>`가 넣는 낱말은 넣는다. SQL 자리와 문서 자리를 오간다(태그 안은 SQL이 아니다).
  - SQL 읽기(`ferrite_db::refs`, 낱말 나누기는 `lex`를 `origin`과 함께 쓴다)
    - 표: FROM·JOIN·UPDATE·INTO·USING 뒤의 `[스키마.]표 [[AS] 별칭]`, FROM 목록의 쉼표. 함수 인자 안의 FROM(`EXTRACT(YEAR FROM d)`), 표 함수(`TABLE(…)`), DB 링크(`@`), 부분 조회, 가린 치환은 표로 보지 않는다. WITH 이름은 따로 모은다.
    - 열: `별칭.열`(스키마.표.열이면 뒤의 둘, `패키지.함수(`는 뺀다), INSERT의 열 목록, UPDATE SET의 열
    - 자동 완성 문맥: 표 자리(FROM 등 뒤, FROM 목록의 쉼표 뒤), `별칭.` 뒤(그 표의 열), 그 밖의 식 자리(SELECT·WHERE·ON·SET·연산자·`(`·쉼표 뒤: 문장 표들의 열). 별칭을 치는 자리, 글자 값·주석 안, 함수 인자의 FROM 뒤는 하지 않는다.
  - 자동 완성: 언어 서버를 기다리지 않고 바로 연다(서버 응답이 오면 합친다). XML 언어 서버가 없어도 스키마를 읽었으면 입력 중에 연다. 표는 "표 · 열 N개", 열은 형식(문장 표들의 열이면 표 이름도).
  - 검증: 문서나 스키마가 바뀌면 열린 XML을 다시 본다. 스키마에 없는 표("표 X이(가) 연결 '이름'의 스키마에 없습니다"), 표에 없는 열을 경고로(`ferrite-db` 출처, 문제 창·물결선). WITH 이름, Oracle `DUAL`, 읽지 않은 스키마를 적은 표, 스키마에 없는 표의 열은 보지 않는다(잘못된 경고를 줄인다). 대소문자는 무시한다.
  - 검증
    - 단위: 낱말 나누기(범위, 닫히지 않은 따옴표·주석), 표·별칭(함수 인자 FROM, 표 함수, DB 링크, 부분 조회, WITH), 별칭 열·INSERT·UPDATE 열, 커서 문맥 10가지, 문장 SQL 꺼내기(태그·주석 빼기, 치환 가리기, `<where>` 낱말, 자리 오가기, iBatis), 매퍼 경고 3가지와 자동 완성 4가지
    - 창 테스트(SQLite): 스키마를 읽고 매퍼를 열면 없는 표 두 개를 알리고, `FROM em` 뒤에서 자동 완성(언어 서버 없이)으로 `emp`를 넣으면 다시 맞춰 "열 nope이(가) 표 emp에 없습니다"로 바뀐다
    - 실제 앱(Windows): CUBRID 스키마를 읽고 매퍼를 열어 없는 열 둘·없는 표 하나에 물결선과 "경고 3"을, `FROM demo_` 뒤 자동 완성에서 "demo_emp 표 · 열 4개"를 화면으로 확인했다(XML 언어 서버 없음). 임시 연결 목록과 사이드카 폴더는 지웠다.
  - 미룸: `<include refid>`로 넣는 조각을 펼쳐 읽기(조각과 문장을 따로 본다: 조각 안 `별칭.열`의 별칭은 문장에 있어 확인하지 않는다), 별칭 없는 열 이름의 검증(SELECT 목록의 별칭·함수와 가리기 어렵다), 프로젝트 전체 매퍼 검사(열린 파일만 본다)
- [x] 3개 OS 검증(2026-10-27): CI 3개 OS 전체(실행 40461616152, main `1eb6c0d`) 통과
  - 완료 조건 "DB에 연결해 조회·편집·커밋·롤백"(PostgreSQL·MariaDB·SQLite·Oracle·Tibero·CUBRID 6종)
    - SQLite·PostgreSQL: 3개 OS 모두에서 공통·편집·스키마 시나리오가 돌았다. PostgreSQL은 러너의 initdb로 띄운 임시 클러스터를 쓰고, 2.8~7.3초 걸렸다(건너뛰지 않았다).
    - MariaDB 11.4·Oracle XE 11g·CUBRID 11.2: Linux 작업의 서비스 컨테이너로 돌았다(MariaDB 0.9초, JDBC 7.6초). Windows·macOS 러너는 서비스 컨테이너를 못 띄워 건너뛴다.
    - Tibero 6: CI에 이미지가 없다. 5-3c에서 개발 서버로 두 번 연속 통과했다.
  - 완료 조건 "100만 행 결과를 스크롤할 때 메모리가 상한을 넘지 않는다": 3개 OS 모두에서 SQLite 100만 행을 끝까지 받는 동안 결과 버퍼가 늘 10만 행 이하이고, 마지막 행이 1,000,000번인 것을 확인했다. 확인하지 않은 것: 앞쪽을 버린 뒤 화면의 스크롤 위치를 같은 행으로 되돌리는 동작(5-2부터 남은 항목).
  - 확인하지 않은 것: Tibero 멈춤의 KILL SESSION 대비책은 ALTER SYSTEM 권한이 있는 계정에서만 확인했다(권한이 없으면 약 60초 걸린다).
- [x] 5-5 동의어 지원: 스키마 브라우저와 매퍼 SQL 자동 완성·검증이 동의어로 쓴 표를 안다(3개 OS 검증 뒤에 더한 단계)
  - 왜: 사용자 스키마의 표만 읽으면, 다른 스키마의 표를 동의어로 쓰는 매퍼(Oracle·Tibero egov 프로젝트에 흔하다)에서 "없는 표" 경고가 잘못 뜨고 자동 완성·스키마 브라우저에도 그 표가 없다.
  - 읽기: 사이드카의 스키마 요청에 종류마다 동의어 조회를 함께 보낸다. 사용자 동의어와 `PUBLIC` 동의어 가운데 표·뷰를 가리키는 것을, 그 표의 열과 함께 한 번에 읽는다(`ALL_TAB_COLUMNS`·`db_attribute`와 잇는다).
    | 종류 | 사전 | 가리키는 표 |
    |---|---|---|
    | Oracle | `ALL_SYNONYMS`(DB 링크는 뺀다) | `TABLE_OWNER`·`TABLE_NAME` |
    | Tibero | `ALL_SYNONYMS` | `ORG_OBJECT_OWNER`·`ORG_OBJECT_NAME`(TiberoStudio 분석으로 확인) |
    | CUBRID 11.2 | `db_synonym`(주인 이름은 대문자, 표·열은 소문자) | `target_owner_name`·`target_name` |
    - `PUBLIC` 동의어는 거의 다 DB 자체의 사전 뷰를 가리킨다(Tibero 개발 서버 939개 모두 SYS·SYSCAT·SYSGIS, Oracle XE 3,244개 모두 SYS·SYSTEM·APPQOSSYS). 그래서 시스템 계정(Oracle SYS·SYSTEM·XDB 등, Tibero SYS·SYSCAT·SYSGIS·OUTLN)의 표를 가리키는 것은 뺀다. 빼지 않으면 Tibero 개발 서버에서만 열이 7,621줄 붙는다.
    - 이름이 겹치면 사용자 표, 사용자 동의어, `PUBLIC` 동의어 순으로 이긴다(DB가 이름을 찾는 순서). 동의어 사전을 못 읽으면(권한 등) 동의어 없이 표만 보인다.
    - PostgreSQL·MariaDB·SQLite에는 동의어가 없어 바뀌지 않는다.
  - 스키마 브라우저: "표 N개 · 뷰 M개 · 동의어 K개", `이름 (동의어 → 주인.표)`. [SQL 콘솔에서 조회]는 동의어 이름으로 조회한다.
  - 매퍼 SQL: 동의어도 표로 본다. 자동 완성에 "동의어 → 주인.표 · 열 N개"로 보이고, 동의어로 쓴 표는 없는 표로 알리지 않으며, `별칭.열`은 가리키는 표의 열로 확인한다.
  - 그리드 편집(5-3b)은 그대로 사용자 스키마의 표만 고친다(동의어로 조회한 결과는 읽기 전용).
  - 검증
    - 단위: 동의어로 쓴 표는 경고하지 않고 그 열은 확인한다, 자동 완성 항목(종류·설명)
    - 동의어 시나리오: 시험 표와 그것을 가리키는 사용자 동의어를 만들어, 스키마에 동의어가 가리키는 표의 열과 함께 보이고 시스템 계정 표를 가리키는 동의어는 없는지 본다. Oracle XE 11g·CUBRID 11.2(로컬 Docker, CI Linux), Tibero 6 개발 서버(사용자 허락 뒤, 시험 객체는 지웠다)
    - 실제 앱(Windows): CUBRID에 동의어를 만들어 스키마 브라우저의 "동의어 1개"와 `demo_syn (동의어 → DBA.demo_emp)`을 화면으로 확인했다. 확인용 동의어와 임시 연결 목록은 지웠다.
  - 확인하지 않은 것: 다른 스키마의 표를 가리키는 동의어(시험 계정이 다른 계정의 표를 만들 수 없어 자기 표를 가리키는 동의어로 했다. 읽는 조회는 주인을 가리지 않는다)

### Phase 6 — 데이터 사이언스

- `.ipynb` 열기, 편집, 저장(nbformat v4)
- Jupyter 커널 연결(ZeroMQ 메시징): 셀 실행, 출력 렌더링(텍스트, 이미지, HTML 요약)
- DataFrame 뷰어: 커널의 DataFrame을 페이지로 받아 그리드로 보기(처음 계획은 Arrow IPC였으나 6-3에서 JSON 페이지로 정했다)

**완료 조건**: Jupyter로 만든 노트북을 열어 고쳐 저장하면 고친 셀만 달라진다. ipykernel로 셀을 실행해 글·그림·오류 출력을 본다. 3개 OS.

**진행 단계**:
- [x] 6-1 노트북 열기·고치기·저장, 저장된 출력 보기
  - 새 크레이트 `ferrite-notebook`: 노트북 JSON을 값 그대로 들고 있다가 고친 곳만 바꾼다. 쓸 때는 nbformat과 같은 모양(키 정렬, 들여쓰기 한 칸, 한글 그대로, 끝 줄바꿈. Python `json.dumps(sort_keys=True, indent=1, ensure_ascii=False)`)으로 써서, Jupyter가 쓴 파일은 고치지 않은 곳이 바이트까지 그대로 남는다. 다른 도구가 다른 모양으로 쓴 파일은 처음 저장할 때 한 번 이 모양으로 바뀐다.
    - 셀: 코드·마크다운·원본. 글 고치기(같은 글이면 그대로 둔다. 바꾸면 nbformat처럼 줄 목록), 넣기(nbformat 4.5부터는 8자리 셀 id), 위·아래로 옮기기, 지우기, 종류 바꾸기(코드가 되면 실행 번호·출력 칸을 더하고, 아니게 되면 뺀다)
    - 출력: 스트림(stdout·stderr), 실행 결과·표시 데이터(MIME별 값), 오류(이름·값·추적). 모르는 형식은 이름만.
    - nbformat 4만 연다(3 이하는 까닭을 알린다).
  - 노트북 보기: `.ipynb`를 열면 버퍼(노트북 JSON) 위에 셀을 보인다. 셀을 고치면 노트북을 다시 써서 버퍼를 바꾸므로 저장·변경 표시·외부 변경 감지는 다른 파일과 같다. [JSON 보기]·[노트북 보기]로 바꾼다. JSON이 깨졌으면 까닭과 [JSON 보기로 고치기]를 보인다.
    - 도구: [+ 코드 셀]·[+ 마크다운 셀](고른 셀 아래), [위로]·[아래로]·[셀 지우기], 종류 고르기, 노트북 언어
    - 셀 머리글("코드 [n]", "마크다운", "원본")을 셀 입력칸의 이름으로 잇는다(화면 읽기 프로그램).
    - 출력: 그림(PNG, base64를 풀어 텍스처로, 한 번만 만든다) > 글(text/plain) > HTML 요약(태그를 빼고 20줄) 순으로 고른다. 오류 추적은 터미널 색 코드를 뺀다. stderr는 경고 색.
  - 의존: `image`(PNG만)·`base64`는 이미 잠금 파일에 있던 판이라 새로 받은 패키지는 없다.
  - 미룸: 마크다운 셀을 서식으로 보이기(지금은 글로 고친다), 코드 셀 문법 강조, JPEG·SVG 그림 출력
  - 검증
    - 단위: 읽고 그대로 쓰기, 고치기·넣기·옮기기·지우기·종류 바꾸기, 노트북이 아닐 때, 색 코드 빼기, HTML 요약
    - 왕복: Python으로 nbformat 모양 그대로 만든 노트북(한글, 스트림·실행 결과·PNG·오류 출력, 원본 셀, 4.5 셀 id)을 읽고 쓰면 바이트까지 같고, 셀 하나를 고치면 Python이 같은 고치기로 쓴 파일과 같다
    - 창 테스트: 노트북을 열면 셀·실행 번호·글·오류 출력이 보이고, 둘째 셀을 키보드로 고쳐 저장하면 기대 파일과 바이트까지 같다. [+ 마크다운 셀]과 [JSON 보기]
    - 실제 앱(Windows): 마크다운·코드 셀, 표준 출력, 320×160 PNG 그림, 오류 출력을 화면으로 확인했다. 앱이 남긴 배치 파일은 지웠다.
- [x] 6-2 Jupyter 커널 연결: 커널 찾기·띄우기, 셀 실행과 출력 받기, 멈춤·다시 시작
  - 방식: Python 사이드카(`crates/ferrite-notebook/sidecar/ferrite_kernel.py`)가 공식 `jupyter_client`로 커널을 띄우고 ZeroMQ로 말한다. Ferrite와 사이드카는 표준 입출력의 한 줄 JSON으로 주고받는다(JDBC 사이드카와 같은 틀). ZeroMQ·메시지 서명·Windows 커널 멈춤을 공식 라이브러리가 맡아 Rust 의존이 늘지 않는다. ipykernel을 깔면 jupyter_client도 함께 깔리므로 따로 요구하는 것은 없다.
    - 사이드카는 사용자의 Python으로 돈다(Python 3.8 문법). 소스는 지역 문자 집합과 상관없게 base64로 넘기고, 노트북 폴더의 같은 이름 모듈을 읽지 않게 `sys.path`에서 현재 폴더를 먼저 뺀다. JSON은 원래 표준 출력의 복제본(자식에게 물려주지 않는다)에만 쓰고, print·로그·커널 프로세스의 출력은 표준 오류로 보낸다.
    - 요청: start(커널 이름, 폴더), execute(id, 코드), interrupt. 알림: started, output(nbformat 출력), clear, done(실행 번호, ok·error·aborted·dead), error. 실행은 하나씩 하고, 실행 중에 온 멈춤은 곧바로 커널에 보낸다. 오류가 나면 줄 서 있던 실행은 aborted로 끝낸다(Jupyter와 같다). `clear_output(wait=True)`는 다음 출력이 올 때 지운다. 입력이 닫히면 커널을 끄고 끝난다.
    - Rust `ferrite_notebook::kernel::Kernel`: 곧바로 돌아오고 결과는 이벤트로 온다(시작 전에 보낸 실행은 커널이 뜬 뒤 돈다). 버리면 입력을 닫고 따로 거둔다(UI가 기다리지 않는다. 15초 넘게 끝나지 않으면 끝낸다). 사이드카가 끝나면 까닭(jupyter_client가 없음, 커널 없음, 표준 오류 끝 20줄)을 알린다.
  - 커널 고르기: 노트북 메타데이터의 kernelspec 이름. 이 컴퓨터에 없으면 사이드카 Python의 python3 커널로 띄우고 알린다. Python은 노트북 위 폴더의 가상 환경(`.venv`, `venv`), 없으면 PATH에서 찾는다. 커널은 노트북 폴더에서 돈다.
  - 노트북 보기: [▶ 실행](고른 셀을 실행하고 다음 셀로, Shift+Enter. 제자리는 Ctrl+Enter), [모두 실행], [멈춤], [커널 다시 시작](이전 커널은 실행 중이어도 끄고 새로 띄운다), 커널 상태(꺼짐·띄우는 중·이름·실행 중·끝난 까닭). 실행을 줄 세우면 그 셀의 출력과 번호를 비우고 "코드 [*]"로 보인다. 출력은 오는 대로 노트북에 쓰므로 변경 표시가 켜지고 저장하면 남는다(Jupyter와 같다). 셀마다 번호를 붙여 실행 중에 셀을 넣고 옮기고 지워도 출력이 제 셀로 간다. 노트북을 닫으면 커널을 끈다.
  - 출력 쓰기(`Notebook::push_output`): nbformat처럼 글(text/*, SVG, 자바스크립트)은 줄 목록으로 쓰고 그림 base64는 그대로 둔다. 바로 앞과 같은 스트림은 이어 붙인다.
  - 의존: `base64`는 이미 잠금 파일에 있던 판이다. 시험용으로 ipykernel 7.4.0을 스크래치 가상 환경에 받았다(사용자 허락, 저장소 밖). CI는 debugpy와 함께 pip로 깐다.
  - 알려진 한계: Windows의 ipykernel은 멈춤을 다음 Python 줄에서 일으키므로 긴 C 호출(`time.sleep(60)` 등)은 그 호출이 끝나야 멈춘다(Jupyter도 같다).
  - 미룸: 커널 고르기 화면(지금은 노트북의 커널 또는 python3), `input()`(표준 입력 요청은 받지 않아 오류가 난다), `update_display_data`, `\r`로 줄을 덮어쓰는 진행 막대, 마크다운 셀 서식 보기
  - 검증
    - 단위: 사이드카 알림 읽기, 오류 까닭 글, 없는 Python, nbformat 모양 출력 쓰기(스트림 잇기, 줄 나누기, 그림은 그대로), 가상 환경 Python 찾기
    - 실제 커널(`tests/kernel.rs`, ipykernel 7.4.0): 없는 커널 이름이면 python3로 띄우고 알린다. stdout·stderr·실행 결과와 실행 번호, 커널 폴더, `display_data`와 `clear_output(wait=True)`, 오류 뒤 줄 서 있던 실행은 aborted, 멈추면 KeyboardInterrupt로 끝나고 상태는 남는다. 시험 뒤 남은 Python 프로세스가 없다.
    - 창 테스트: [모두 실행]으로 셋째 셀이 오류 뒤라 돌지 않는 것("코드 [1]"·"코드 [2]"·"코드 [ ]", 출력, 변경 표시), 첫 셀에서 Shift+Enter로 다시 실행(줄바꿈이 들어가지 않는다), 저장한 파일의 실행 번호, [커널 다시 시작] 뒤 실행 번호가 1부터 다시 센다
    - 실제 앱(Windows): 노트북 폴더의 `.venv` Python으로 커널이 뜨고 노트북 폴더에서 도는 것, PNG 그림 출력, 실행 중 "코드 [*]"·실시간 출력·[멈춤], 멈추면 KeyboardInterrupt와 다음 셀이 돌지 않는 것, 저장한 파일을 화면과 파일로 확인했다. 앱을 강제로 끝내도 커널·사이드카가 남지 않았다. 앱이 남긴 배치 파일은 지웠다.
- [x] 6-3 DataFrame 뷰어
  - 전송: 보이는 행 근처만 200행씩 pandas `to_json`(값 배열, ISO 날짜, 실수 15자리, NaN은 null)으로 받는다. 처음 계획한 Arrow IPC 대신 JSON 페이지로 정했다(사용자 결정): 이미 있는 커널 연결과 serde_json만 써서 새 Rust 의존이 없고, 사용자 환경에 pyarrow가 없어도 된다. 페이지만 오가므로 행이 많아도 받는 양은 같다.
  - 커널 쪽: 사이드카의 `data` 요청이 도우미(`_ferrite`, 사용자 이름공간에 이 이름 하나만 둔다)를 조용한 실행(실행 번호·출력·기록 없음)으로 정의하고 `user_expressions`로 부른다. 값은 파이썬 글자의 repr로 오므로 사이드카가 `ast.literal_eval`로 푼다. pandas를 가져오지 않은 커널에서는 가져오지 않는다. 실행 중인 셀이 있으면 그 뒤에 답한다. 실패해도 사이드카는 끝나지 않고 오류로 답한다.
    - 목록: 이름이 밑줄로 시작하지 않는 `pandas.DataFrame`(이름, 행·열 수)
    - 페이지: 행 범위, 정렬(열 자리, 오름·내림. 열 이름·색인이 겹쳐도 되게 자리로 정렬하고, 빈 값은 끝, 같은 값은 원래 순서. 섞인 형식이면 글자로 비교)
    - Rust `Kernel::frames`·`Kernel::page` → `Event::Frames`·`Event::Page`(`FrameInfo`, `Page`)
  - 화면: 노트북 도구의 [DataFrame 보기]가 아래 판(크기 조절)을 연다. DataFrame 고르기(이름 · 행 × 열), [새로 고침], 그리드(색인, 열 머리글에 형식 풍선 도움말, NaN은 흐린 기울임). 보이는 행만 그리고, 받지 않은 페이지는 "…"로 보이며 받는다. 머리글을 누르면 오름차순 → 내림차순 → 정렬 없음. 셀 실행이 다 끝나면 목록과 보던 DataFrame을 다시 받고, 늦게 온 이전 답은 버린다. 커널이 꺼졌거나 Python이 아니면 까닭을 보인다.
  - 의존: 새 Rust 의존 없음. 시험용으로 pandas 3.1.0(numpy 2.5.4 포함)을 스크래치 가상 환경에 받았다(사용자 허락, 저장소 밖). CI는 pip로 깐다.
  - 미룸: Series·polars, 거르기, 칸 값 복사, 날짜를 pandas 표시 모양으로, 정렬 순서 캐시(지금은 페이지마다 커널에서 다시 정렬한다. 수백만 행에서 느리면 넣는다)
  - 검증
    - 단위: 데이터 답 읽기(목록, 오류), 늦게 온 페이지 버리기·실행 뒤 다시 받기·사라진 DataFrame 닫기, 칸 글
    - 실제 커널(pandas 3.1.0): pandas를 가져오지 않았으면 빈 목록, 밑줄 이름·Series는 빼고 DataFrame만, 행 범위·열·형식·색인, NaN은 null, 날짜, 내림·오름 정렬(빈 값은 끝), 실수 정밀도, 없는 이름은 오류, 조용한 실행이라 실행 번호가 늘지 않는다
    - 창 테스트: [DataFrame 보기]에서 커널이 꺼졌다는 안내, 실행 뒤 목록에서 고르기, 첫 페이지, 머리글로 오름·내림 정렬
    - 실제 앱(Windows): 1000행(한글 열, 정수·실수·날짜, NaN) DataFrame의 첫 페이지, 휠로 내려 250행대 페이지 받기, 내림차순 정렬을 화면으로 확인했다. 앱을 끝낸 뒤 몇 초 안에 커널이 꺼졌다. 앱이 남긴 배치 파일은 지웠다.
- [x] 3개 OS 검증(2026-10-28): CI 3개 OS 전체(실행 40606779664, `e077810`) 통과
  - 완료 조건을 3개 OS에서 시험으로 확인했다. 노트북 시험·커널 시험·DataFrame 시험이 세 OS 모두에서 건너뛰지 않고 돌았다(ipykernel 7.4.0, pandas 3.1.0).
    - 노트북 왕복: Jupyter가 쓴 노트북을 바이트까지 그대로 쓰고, 셀 하나를 고치면 그 셀만 달라진다.
    - 커널 출력: 셀을 실행해 글(stdout·stderr·실행 결과), 그림(PNG 표시. 이 검증에서 시험에 더했다), 오류 출력을 받는다.
    - 창 시험: 셀 실행·저장·다시 시작, DataFrame 그리드와 정렬
  - 화면으로 직접 본 것은 Windows뿐이다. macOS·Linux는 창 시험(kittest)으로만 확인했다.

### Phase 7 — 확장·배포

- WASM 플러그인 API(wasmi): 명령, 패널, 언어 설정 확장
  - 언어 설정 확장은 코드 없이 파일만 담은 확장도 받는다: TextMate 문법(`.tmLanguage.json`), 언어 설정(주석 기호·괄호 짝), 파일 확장자. tree-sitter 문법이 없는 형식(장비 스크립트, 사내 설정 언어 등)을 이것으로 강조한다. 널리 쓰는 편집기 확장의 문법 묶음과 같은 구조라 그대로 가져올 수 있다.
- 패키징(cargo-packager): Windows MSI/NSIS, macOS dmg, Linux deb/AppImage
- 코드 서명과 공증: Windows Authenticode, macOS notarization
- 자동 업데이트: 서명된 릴리스 매니페스트 검증

**완료 조건**: 3개 OS 설치 파일로 설치해 실행되고, 제거하면 설치한 것이 남지 않는다. 앱이 서명된 매니페스트로 새 판을 알아보고 받아 바꾼다. 예제 WASM 플러그인이 명령과 패널을 더한다. 3개 OS. 코드 서명·공증은 인증서가 있을 때만 한다.

**진행 단계**:
- [x] 7-1 설치 파일 패키징
  - 도구: cargo-packager 0.11.8. 설정은 `crates/ferrite-app/Cargo.toml`의 `[package.metadata.packager]`에 둔다.
    - 제품 정보: 이름 Ferrite, 식별자 `io.github.kongga12.ferrite`, 분류 개발 도구
    - 설명은 영어로 둔다. WiX(MSI)의 기본 코드 페이지(1252)가 한글을 받지 못한다.
    - Windows: NSIS(사용자별 설치, 관리자 권한 없음. 설치 화면은 OS 언어를 따르고, 없으면 한국어)와 MSI(WiX 3.11, 시스템 전체 설치)
    - macOS: .app·.dmg(최소 11.0, 러너 아키텍처 arm64)
    - Linux: .deb·AppImage. 그래픽(wgpu)·키보드(xkbcommon)·파일 대화상자(xdg-portal)는 실행 중에 라이브러리를 불러와 링크로 드러나지 않으므로 deb 의존을 직접 적었다(xkbcommon, Vulkan 또는 EGL, 한글 글꼴).
  - 워크플로 `.github/workflows/package.yml`: 수동 실행, `v*` 태그, 패키징 설정(워크플로·앱 Cargo.toml·아이콘)을 바꾼 PR에서 3개 OS로 돈다. 만든 파일은 Actions 아티팩트로 올리고 릴리스는 만들지 않는다. NSIS·WiX·linuxdeploy는 cargo-packager가 받는다(판과 해시는 cargo-packager가 고정). 러너에 FUSE가 없어 AppImage 도구는 풀어서 돌린다(`APPIMAGE_EXTRACT_AND_RUN`).
  - 앱 아이콘: `crates/ferrite-app/icons/`(PNG 32·128·256·512, Windows `.ico`). 창 아이콘으로도 쓴다. macOS `.icns`는 cargo-packager가 PNG로 만든다.
  - 서명하지 않는다. Windows는 SmartScreen이 경고하고([추가 정보] → [실행]), macOS는 Gatekeeper가 막는다(Finder에서 오른쪽 단추 → [열기]).
  - 알려진 한계
    - Windows 설치 폴더(`%LOCALAPPDATA%\Ferrite`)는 앱 데이터 폴더(`%LOCALAPPDATA%\ferrite`: 언어 서버·템플릿·JDBC)와 같은 폴더다(대소문자를 가리지 않는다). 제거 프로그램은 설치한 파일만 지우고 폴더는 비었을 때만 지우므로 앱 데이터는 남는다.
    - NSIS 템플릿이 제거할 때 `HKCU\Software\<배포자>\Ferrite` 키를 남긴다(그 안의 값 하나만 지운다).
  - 미룸: 실행 파일 자체 아이콘(Windows 리소스. 지금은 시작 메뉴 바로 가기가 기본 아이콘이다), macOS 유니버설(x86_64) 빌드
  - 검증
    - 단위: 창 아이콘 PNG를 읽는다(256×256)
    - CI: 패키징 워크플로(실행 40609803473)가 3개 OS에서 설치 파일 6개를 만들었다: NSIS `ferrite_0.1.0_x64-setup.exe`(11.3MB), MSI `ferrite_0.1.0_x64_en-US.msi`(14.5MB), `Ferrite_0.1.0_aarch64.dmg`, `ferrite_0.1.0_amd64.deb`, `ferrite_0.1.0_x86_64.AppImage`
    - 실제 설치(Windows): CI가 만든 NSIS 설치 파일을 받아 조용히 설치(`/S /NS`)했다. 설치 폴더(`ferrite.exe`·`uninstall.exe`), 제거 등록 정보(이름·판·배포자·아이콘), 시작 메뉴 바로 가기를 확인했다. 설치한 앱을 실행해 창과 제목 표시줄 아이콘을 화면으로 확인했다. 조용히 제거하면 설치 폴더·제거 등록 정보·시작 메뉴가 지워지고, 위의 레지스트리 키만 남아 손으로 지웠다.
    - 확인하지 못한 것: MSI 설치(관리자 권한), dmg·deb·AppImage 설치와 실행
- [ ] 7-2 코드 서명·공증(인증서가 생기면)
  - cargo-packager가 읽는 것: macOS는 환경 변수 `APPLE_CERTIFICATE`(p12를 base64로)·`APPLE_CERTIFICATE_PASSWORD`와 설정 `macos.signing-identity`. Windows는 설정 `windows.certificate-thumbprint`(인증서 저장소에 넣은 인증서) 또는 `windows.sign-command`(직접 서명 명령).
  - 인증서와 비밀번호는 사용자가 GitHub 비밀 값에 넣는다. 인증서 없이 시험할 수 없는 CI 단계는 미리 넣지 않았다.
- [x] 7-3 자동 업데이트(서명된 릴리스 매니페스트)
  - 방식(사용자 결정): 직접 작게 만든다. 공식 업데이터 크레이트(cargo-packager-updater)는 reqwest·tokio를 끌고 오고, 이미 쓰는 ureq와 HTTP 클라이언트가 둘이 된다.
    - 새 크레이트 `ferrite-update`. 매니페스트(`latest.json`)와 서명 형식은 cargo-packager 업데이터와 같다.
    - 서명 검증은 minisign-verify(의존 없는 작은 크레이트, 새로 받음), 받기는 언어 서버 받기와 같은 ureq 설정(OS 인증서 저장소)이다.
    - 매니페스트: `version`, `notes`, 플랫폼(`windows-x86_64`, `linux-x86_64`, `macos-aarch64`)마다 `url`·`signature`(`.sig` 내용)·`format`
    - 판 비교: `x.y.z` 숫자, 앞의 `v`와 `-beta` 같은 꼬리는 무시한다.
    - 검증: 공개 키와 서명은 minisign 글을 base64로 감싼 것이다(cargo-packager 형식). 서명이 맞지 않으면 파일을 쓰지 않는다. 매니페스트 자체에는 서명이 없다. 바꾼 사람은 서명된 다른 판을 가리키게 할 수 있을 뿐이다.
    - 설치: NSIS로 설치한 Windows(실행 파일 옆에 `uninstall.exe`)는 받아 둔 설치 파일을 앱이 끝날 때 수동 모드(`/P /R`)로 돌린다. 진행 창만 보이고, 끝나면 다시 띄운다. AppImage(`APPIMAGE`)는 같은 폴더에 쓴 뒤 이름을 바꿔 파일을 바꾼다. MSI·dmg·deb·개발 빌드는 내려받기 페이지를 연다.
  - 앱
    - 시작할 때 따로 스레드에서 한 번 묻는다(설정 `update_check`, 기본 켬). 새 판이 있을 때만 상태 표시줄에 [새 판 x.y.z]가 보인다. 망이 막혀 있으면 조용히 넘어간다. [도구 > 업데이트 확인…]은 창을 열어 최신 판·오류도 보인다.
    - 업데이트 창: 판, 릴리스 노트, [받아 설치](진행 막대) 또는 [내려받기 페이지 열기]. 받으면 NSIS는 "닫으면 설치" → [지금 닫고 설치](저장하지 않은 변경은 평소처럼 묻는다). AppImage는 "다시 시작하면 새 판"이다.
    - 설정 `update_url`: 비우면 공식 릴리스(`releases/latest/download/latest.json`). 사내 미러를 쓸 때만 바꾼다.
    - 공개 키: `crates/ferrite-app/updater.pub`(`cargo packager signer generate`가 만든 `.pub`). 비밀 키와 비밀번호는 사용자가 GitHub 비밀 값 `FERRITE_UPDATER_KEY`·`FERRITE_UPDATER_KEY_PASSWORD`에만 넣는다.
  - 릴리스(사용자 결정: 태그로 GitHub 릴리스)
    - 패키징 워크플로가 `v*` 태그에서 설치 파일마다 `.sig`를 만든다(키가 없으면 실패).
    - 이어서 `release` 작업이 태그와 앱 판이 같은지 보고, `.github/scripts/latest_json.py`로 `latest.json`을 만든다. NSIS·AppImage는 서명이 꼭 있어야 하고, dmg는 링크로만 쓴다.
    - 그다음 GitHub 릴리스(자동 릴리스 노트)를 만들어 모두 올린다. 공개는 태그를 올리는 사용자가 정한다.
  - 검증
    - 단위: 판 비교, 매니페스트 읽기(플랫폼 없음은 링크만), 서명(맞음·바뀐 파일·잘못된 서명·잘못된 키), 설치 방식 고르기, AppImage 바꾸기
    - 시험용 키·서명은 내려받기 없이 만들었다. 순수 Python Ed25519(RFC 8032 참조 구현, 시험 벡터로 확인)와 minisign 형식(BLAKE2b-512 해시 서명과 신뢰 주석 전역 서명)이다(`crates/ferrite-update/tests/fixtures`).
    - 로컬 HTTP 서버: 매니페스트 확인, 받기와 진행, 같은 판이면 없음, 바뀐 파일은 쓰지 않음, 매니페스트 없음은 오류
    - 창 테스트: [업데이트 확인]으로 새 판(노트, MSI라 내려받기 페이지), 상태 표시줄 알림, 최신 판, 404 오류
    - `latest.json` 스크립트: 가짜 설치 파일로 세 플랫폼 항목, NSIS 서명이 없으면 실패
    - 앱에 넣은 공개 키를 읽을 수 있는지(테스트)
    - 업데이트 끝까지(3개 OS 검증에서): 아래 [3개 OS 검증]의 로컬 서버 시험
    - 남은 것: 첫 `v*` 태그 릴리스(실제 키 서명·release 작업)와 AppImage 바꾸기의 실제 확인
- [x] 7-4 WASM 플러그인 API(명령·패널)
  - 결정(사용자): 런타임 wasmi 2.0.0, 범위는 둘로 나눈다(언어 설정 확장은 7-5), 패널은 선언형 위젯, 예제는 Rust.
    - wasmi는 순수 Rust 인터프리터다. 크레이트가 적어 빌드·앱 크기가 거의 늘지 않고, JIT이 없어 macOS 서명 때 JIT 권한이 필요 없다. 명령·패널에는 속도가 충분하다.
  - 새 크레이트 `ferrite-plugin`
    - 플러그인 폴더: 설정 폴더의 `ferrite/plugins/<이름>/`에 `plugin.toml`과 `.wasm`. 이름 순으로 읽고, 읽지 못한 플러그인은 오류로 모으고 나머지는 계속 읽는다. 같은 `id`는 먼저 읽은 것만 쓴다.
    - 매니페스트: `api`(지금 1, 다르면 읽지 않는다), `id`, `name`, `version`, `description`, `wasm`(폴더 안 파일 이름만, 기본 `plugin.wasm`), `[[commands]]`·`[[panels]]`(각 `id`·`title`)
    - 주고받기: 코어 wasm 모듈이 `memory`, `ferrite_alloc(len) -> ptr`, `ferrite_call(ptr, len) -> (답 위치 << 32 | 길이)`를 내보낸다. 요청과 답은 UTF-8 JSON이다. Component Model(WIT)은 wasmi에 없고, 이 방식은 어느 언어로든 만들 수 있다.
    - 요청: 명령 `{type: "command", command, editor}`, 패널 `{type: "panel", panel, event, editor}`. `editor`는 활성 문서의 `path`·`language`·`text`(4MiB 넘으면 null)·`selection`이다. 노트북은 버퍼가 JSON이라 넘기지 않는다.
    - 답: `actions`(`replace_selection`, `message`(상태 표시줄), `show_panel`)와 패널의 `ui`(`heading`, `label`, `code`, `separator`, `button`, `input`). 패널 이벤트는 `click`(버튼)·`change`(입력). 모르는 종류는 버려 새 판 플러그인도 옛 앱에서 돈다.
    - 격리: 앱은 아무것도 주지 않아(가져오기 없음) 플러그인은 파일·네트워크·시계를 못 쓴다. 가져오는 것이 있으면 읽지 않는다.
    - 상한: 호출마다 연료 10억, 메모리 256MiB, 답 16MiB. UI 스레드에서 부르므로 연료가 멈춤의 상한이다. 릴리스 빌드(Windows)에서 끝없는 반복은 0.33초에 연료를 다 쓰고, 예제의 글자 세기는 1MB 27ms·3.7MB 103ms다(연료 3억으로는 3.7MB에서 멈춰 올렸다). 넘기거나 멈추면(trap) 인스턴스를 버리고 다음 호출에 새로 띄운다(플러그인 상태는 처음부터).
  - 앱
    - [플러그인] 메뉴: 플러그인마다 하위 메뉴(명령, `<패널> 창`), [플러그인 창], [플러그인 다시 읽기]
    - 플러그인 창(도구 창 하나): 패널이 둘 이상이면 고르는 상자. 처음 열 때, 이벤트가 있을 때, 활성 문서나 그 판이 바뀔 때만 플러그인에 다시 묻는다. 입력칸은 고친 값을 바로 보이고 플러그인 답으로 바꾼다. 패널이 없으면 플러그인 폴더 경로와 [경로 복사]를 보인다.
    - 플러그인이 시킨 일은 창을 다 그린 뒤 한꺼번에 한다(창 배치를 바꾸므로).
  - 예제 `examples/plugins/text-stats`(Rust, 작업 공간 밖, wasm32-unknown-unknown): 명령 [선택 영역을 대문자로]·[글자 수 알리기], 패널 [문서 통계](줄·낱말·글자 수, 찾을 글의 수, 대문자로 바꾸기 버튼). 맨 아래 주고받기 함수를 베끼고 `handle`만 바꾸면 새 플러그인이 된다.
  - 검증
    - 단위(WAT로 쓴 시험 플러그인): 매니페스트 기본값·잘못된 값(api, 빈 id, 폴더 밖 wasm), 답 읽기와 모르는 종류 버리기, 끝나지 않는 플러그인은 연료가 떨어져 멈추고 다시 불러도 된다, 멈춤(trap)·메모리 밖 답·너무 큰 답·잘못된 JSON, 가져오기·내보내기가 맞지 않으면 읽지 않는다, 폴더 읽기 순서·같은 id·실패 모으기
    - 예제: 시험이 예제를 wasm32로 빌드해 명령 두 개, 패널 그리기·입력 상태가 남는지·버튼 이벤트를 확인한다. wasm32 타깃이 없으면 로컬은 건너뛰고 CI(타깃을 더했다)는 실패한다.
    - 창 테스트: 메뉴로 선택 영역 바꾸기와 상태 표시줄 알림, [문서 통계 창]의 통계, 입력칸에 쳐서 찾은 수, 패널 버튼으로 선택 영역 바꾸기, 문서가 바뀐 뒤 다시 그리기
    - 실제 앱(Windows, 릴리스 빌드): 설정 폴더에 예제를 넣고 [플러그인] 메뉴의 하위 메뉴, 낱말을 골라 [선택 영역을 대문자로], [문서 통계 창]의 통계, 입력칸에 쳐서 찾은 수, [글자 수 알리기]의 상태 표시줄 알림을 화면으로 확인했다. 확인 뒤 설정 폴더를 지웠다.
- [x] 7-5 언어 설정 확장(코드 없는 확장: TextMate 문법·주석 기호·괄호 짝·확장자)
  - 결정(사용자): 토크나이저는 직접 만들고 정규식은 onig 6.5.3(Oniguruma, 문법이 기대하는 그대로), 형식은 널리 쓰는 편집기 확장 폴더 그대로, 언어 설정은 주석과 괄호 짝 강조까지(자동 닫기는 하지 않는다).
    - onig는 bindgen(libclang)이 필요한 기본 기능을 빼고, 함께 든 바인딩으로 C 소스만 cc로 빌드한다(tree-sitter와 같다).
  - 새 크레이트 `ferrite-textmate`
    - 확장 폴더: 플러그인 폴더(`ferrite/plugins/<이름>/`)의 `package.json`. `contributes.languages`(id, aliases, extensions, filenames, configuration)와 `contributes.grammars`(language, scopeName, path)를 읽는다. 확장의 코드(JS)는 쓰지 않는다. VS Code 확장(.vsix를 푼 `extension` 폴더)을 고치지 않고 넣을 수 있다.
    - 언어 설정(`language-configuration.json`, JSONC: 주석·끝 쉼표를 뺀다): `comments.lineComment`(글 또는 `{comment}`), `brackets`의 한 글자 짝
    - 문법은 JSON(`.tmLanguage.json`)만 읽는다. plist(`.tmLanguage`)·YAML은 오류로 알린다. 폴더 밖 경로는 받지 않는다.
    - 같은 언어 id는 먼저 읽은 것에 확장자·파일 이름을 더한다. 파일은 파일 이름, 그다음 가장 긴 확장자로 고른다. tree-sitter 내장 언어가 먼저다(언어 확장은 내장 언어가 없는 파일만).
    - 문법은 그 언어 파일을 처음 열 때 규칙으로 옮긴다. 다른 문법을 부르는 문법(마크다운은 수십 개)은 옮기는 데 시간이 걸린다.
  - 토크나이저(VS Code의 TextMate 엔진과 같은 규칙)
    - match, begin/end, begin/while, include(저장소, `$self`, `$base`, `source.x`, `source.x#이름`), captures(begin·end·while), contentName, applyEndPatternLast, end·while의 뒤 참조(`\1`, begin에서 찾은 글을 이스케이프해 채운다)
    - `\G`는 닻 자리(바로 앞 begin이 끝난 자리, begin이 줄 끝까지 맞았으면 그 안의 줄 처음)에서만 맞는다. 줄 끝에서도 한 번 더 찾는다(줄바꿈까지 맞은 begin 뒤의 빈 begin).
    - 빈 매치가 되풀이되지 않게 막고, 한 줄에서 5000번 넘게 찾으면 그 줄은 그만 센다. 1만 바이트 넘는 줄은 강조하지 않고 쌓임만 넘긴다.
    - 쓰지 않는 것: 주입(injections), 캡처 안의 patterns, `\A`
    - 줄마다 시작할 때의 규칙 쌓임을 기억하고 편집한 줄부터 다시 센다. 보이는 줄 앞을 세는 데 한 번에 30ms까지 쓰고, 넘으면 다음 프레임에 이어 센다(그동안 그 줄은 강조 없음).
    - 범위 이름 → 강조 분류: comment, string, constant·support.constant, keyword(operator 빼고)·storage·variable.language·markup.heading, entity.name.function·support.function, entity.name.type/class·support.type/class, entity.name.tag, entity.other.attribute-name·속성 변수. 앱 테마 색을 그대로 쓴다.
  - 앱: 언어 확장 파일은 TextMate로 강조하고, 상태 표시줄에 언어 이름, 줄 주석(Ctrl+/)은 확장의 기호. [플러그인 다시 읽기]가 언어 확장도 다시 읽고 열린 문서에 다시 붙인다. 읽지 못한 것은 플러그인 창에 보인다.
  - 괄호 짝 강조(모든 언어): 커서 바로 뒤(없으면 앞)의 괄호와 그 짝의 배경을 칠한다. 언어 확장은 설정의 brackets, 그 밖에는 `()[]{}`. 앞뒤 2만 글자까지 찾고, 문자열·주석 안의 괄호도 센다.
  - 검증
    - 단위: 범위 분류, match·캡처·begin/end·저장소, 뒤 참조(정규식 글자 이스케이프 포함), while, applyEndPatternLast, 다른 문법과 돌아 부르는 저장소, `\G` 닻(줄 처음 포함), 빈 매치·잘못된 정규식, 펜스 코드 블록(마크다운 모양), 편집한 줄부터 다시 세기, 긴 줄, 시간 예산을 넘기면 이어 세기, JSONC, 확장 폴더 읽기(언어 고르기, 설정, 같은 id 합치기, 오류)
    - 괄호 짝: 앞·뒤 괄호, 중첩, 짝 없음, 선택 중, 모르는 괄호
    - 창 테스트: 언어 확장 파일을 열어 상태 표시줄 이름과 강조, Ctrl+/의 주석 기호, 확장을 지우고 다시 읽으면 일반 텍스트
    - 실제 문법(로컬 VS Code에 든 내장 확장, 저장소에 넣지 않음): 75개 언어를 125ms에 읽었다(plist 문법 하나만 오류). Batch·셸·Lua·Ruby·PowerShell·Perl·F#·Swift·C#·Docker·Makefile·마크다운(펜스 안 Rust 포함) 예제가 맞게 칠해졌다. 1MB 셸 스크립트를 처음부터 다 세는 데 약 1.2초라 시간 예산으로 나눠 센다.
    - 실제 앱(Windows, 릴리스 빌드): VS Code의 셸 스크립트 확장 폴더를 플러그인 폴더에 그대로 넣고 `.sh`를 열어 강조(주석·키워드·문자열), 상태 표시줄의 "Shell Script", 커서 옆 `(`와 짝 `)`의 강조를 화면으로 확인했다. 확인 뒤 설정 폴더를 지웠다.
- [x] 3개 OS 검증(2026-10-28)
  - 설치 시험(패키징 워크플로, 실행 40650277007): 만든 설치 파일을 그 자리에서 설치 → `ferrite --version`(이번에 더한 플래그) → 제거하고 남은 것을 본다. 태그 빌드에서도 돌아 릴리스 전에 막는다.
    - Windows: NSIS(사용자별, `/S`)는 설치 폴더·프로그램 추가/제거 항목·시작 메뉴·HKCU 키가 모두 없어지는지, MSI(`msiexec /qn`)는 설치 폴더와 항목이 없어지는지
    - macOS: dmg를 열어 .app을 복사해 실행한다(dmg는 그 밖에 설치하는 것이 없다). 태그 빌드에서는 업데이트용 .app.tar.gz도 풀어 실행한다.
    - Linux: deb를 apt로 설치해(적은 의존이 풀리는지) 실행하고 purge한 뒤 파일이 남지 않는지, AppImage 실행
    - 창을 띄우는 것은 러너에 화면이 없어 보지 못한다(창 시험은 CI 3개 OS의 kittest).
  - NSIS 제거가 남기던 HKCU 키(`Software\<제작자>\Ferrite`, 설치 위치)를 제거가 끝나면 지운다(`preinstall-section`의 `un.onUninstSuccess`). 7-1 때 손으로 지우던 것이다.
  - 업데이트 끝까지(Windows, 사용자 결정: 로컬 서버와 시험 키)
    - 저장소 밖 작업 트리에서 버릴 시험 키를 넣어 0.1.0 NSIS를, 판을 올려 0.1.1 NSIS를 그 키로 서명해 만들고(`.sig`), `latest_json.py`로 만든 매니페스트의 주소만 로컬 서버로 바꿨다.
    - 0.1.0을 설치하고 설정 `update_url`을 로컬 서버로 둔 뒤 띄우자 시작 확인이 상태 표시줄에 [새 판 0.1.1]을 띄웠다. [받아 설치] → 서명 확인 → [업데이트 준비됨] → 앱을 닫자 설치 파일이 수동 모드로 돌아 0.1.1을 설치하고 Ferrite를 다시 띄웠다(`--version` 0.1.1, 새 판 알림 없음).
    - 제거 시험과 같은 확인으로 제거 뒤 남은 것이 없었다. 받은 설치 파일이 임시 폴더에 남아 앱이 시작할 때 지우게 했다.
    - 실제 키 서명과 release 작업은 첫 `v*` 태그 때 돈다. AppImage 바꾸기는 단위 시험으로만 확인했다.
  - 기능 시험: CI 3개 OS 전체에서 7-3(업데이트 창·로컬 HTTP), 7-4(예제 플러그인 빌드·명령·패널), 7-5(언어 확장 창 시험·펜스 코드 블록)가 건너뛰지 않고 돌았다.
  - 화면으로 직접 본 것은 Windows뿐이다.
  - 남은 것: 7-2 코드 서명·공증(인증서가 생기면), 첫 공개 릴리스

### Phase 8 — 웹·API 개발 도구

**목표**: egov 화면(JSP + jQuery)의 스크립트를 언어 서버로 고치고, 컨트롤러 URL을 편집기에서 바로 호출해 본다.
- 상용 IDE 둘(웹용·C/C++용)의 번들 기능을 살폈더니(2026-10) 둘 다 HTTP 요청 파일, 엔드포인트 목록, 기호 개요를 기본으로 싣는다. Ferrite에는 아직 없다.

- JS·TS·HTML·CSS·JSON 언어 서버: 언어 서버 매니저에 npm 설치 방식(전용 폴더에 버전 고정 `npm install`)을 더해 등록한다. Node가 필요하다(php-debug와 같다). JSON은 스키마 검사(`package.json` 등)가 붙는다.
- 기호 개요·기호 찾기: LSP `textDocument/documentSymbol`(개요 창, 지금 파일)과 `workspace/symbol`(기호 찾기 창, 프로젝트)
- 엔드포인트 목록: 2b-6에서 미룬 것이다. 기호 찾기 창에 Spring Boot LS의 `@/` 기호를 보이고, Spring Boot LS가 없는 egov 3.x(XML 설정) 프로젝트는 egov 서버가 `@RequestMapping` 계열을 모아 같은 꼴로 낸다. 누르면 컨트롤러 메서드로 간다.
- HTTP 요청 파일(`.http`)
  - 널리 쓰는 형식: `###`로 요청을 나누고, 요청 줄·헤더·본문, `{{변수}}`. 환경 파일로 개발·운영 서버 주소를 바꾼다. 비밀 값은 git에서 뺀 개인 환경 파일에 둔다.
  - 요청 옆 [▶]로 보내고 응답(상태·헤더·본문·걸린 시간)을 창에 보인다. JSON은 들여 쓰고, 본문 문자 집합(UTF-8·EUC-KR)은 응답 헤더대로 푼다.
  - 쿠키를 세션 동안 유지한다. egov 로그인 요청 뒤 `.do` 요청을 이어 보낼 수 있다.
  - 엔드포인트 목록에서 그 URL의 요청 틀을 만든다.
  - 회사망은 언어 서버 매니저와 같이 OS 인증서 저장소와 `HTTPS_PROXY`를 따른다. 개발 서버의 자체 서명 인증서는 그 요청에 명시했을 때만 받는다.
- 브라우저 JS 디버그: 오픈소스 JS 디버그 어댑터(DAP 서버, MIT)를 어댑터 목록에 넣는다. Chromium 계열 브라우저를 띄우거나 이미 뜬 브라우저에 붙어, 화면 URL의 스크립트를 프로젝트 파일(`src/main/webapp` 등)과 맞춰 중단점을 건다.

**완료 조건**: egov 표본 프로젝트에서 (1) jQuery를 쓰는 `.js` 파일의 자동완성·정의 이동이 되고, (2) 엔드포인트 목록에서 고른 URL로 `.http` 요청을 만들어 로그인 쿠키를 유지한 채 응답을 보고, (3) JSP 화면 스크립트의 중단점에서 멈춘다. 3개 OS.

**진행 단계**:
- [x] 8-1 JS·TS·HTML·CSS·JSON 언어 서버 등록(npm 설치 방식) — 작음~중간
  - 결정(사용자): JS·TS는 TypeScript 7 네이티브(Go로 다시 쓴 공식 컴파일러, LSP 내장), `.ts`·`.tsx`는 tree-sitter-typescript 0.23.2로 강조, 실제 앱 확인용 npm 내려받기 허용
  - npm 설치 방식(`Source::Npm`): 서버 폴더의 판 폴더에 `npm install <꾸러미>@<판> --omit=dev --ignore-scripts`. 무결성은 npm이 레지스트리 해시로 검증한다. 실행 대상 경로의 `{platform}`·`{arch}`·`{exe}`는 이 플랫폼의 Node 이름으로 바꾼다.
    - TypeScript 7.0.2: 꾸러미의 `bin/tsc`는 OS별 실행 파일을 자식으로 띄우는 Node 스크립트라(Windows에서 끄면 자식이 남는다) `@typescript/typescript-<플랫폼>-<CPU>/lib/tsc`를 `--lsp --stdio`로 바로 띄운다. 실행할 때 Node가 필요 없다. JS·TS·JSX·TSX를 맡는다.
    - HTML·CSS·JSON: vscode-langservers-extracted 4.10.0(MIT)을 Node로 띄운다. 설치의 대부분(85MB 중 65MB)이 HTML 서버가 쓰는 TypeScript 4라 세 서버가 한 폴더를 같이 쓴다. 서버마다 설치 기록을 따로 두고, 마지막 서버를 지울 때 폴더를 지운다.
    - npm 설치와 공용 폴더 지우기는 하나씩 한다. 같은 판이 이미 있는 공용 폴더에는 npm을 다시 돌리지 않는다. 실제 앱에서 셋을 연달아 눌렀을 때 npm 둘이 같은 폴더에 겹쳐 실패하던 것을 이렇게 고쳤다.
  - 진단을 묻는 방식(LSP 3.17 pull): TypeScript 7은 진단을 보내 주지 않고 묻기(`textDocument/diagnostic`)만 받는다.
    - 클라이언트가 이 방식을 안다고 알리고, `diagnosticProvider`를 알린 서버에만 문서의 LSP 판마다 한 번 묻는다. 준비되기 전에 연 문서는 준비된 뒤 묻는다. `workspace/diagnostic/refresh`를 받으면 다시 묻는다.
    - 물어서 받은 진단과 보내 준 진단은 서버 안에서도 따로 둔다(한 서버가 둘을 섞어 써도 서로 덮지 않는다).
  - `workspace/configuration`에는 항목마다 빈 객체로 답한다(VS Code와 같다). null로 답하면 CSS 서버가 검사하다 죽어 진단이 오지 않았다.
  - 언어 서버 창: JSX 판 언어 ID(`…react`)는 빼고 보이고, 새 서버 이름은 짧게 둔다(`vscode-html` 등). 칸이 넓어져 1000px 창에서 버튼이 밖으로 밀리던 것을 고쳤다.
  - 검증
    - 단위: npm 실행 대상 이름 바꾸기, 공용 폴더 설치 기록·지우기, 물어서 받은 진단 읽기, TypeScript·TSX 강조(TS 규칙 + JS·JSX 규칙)
    - 실제 설치(내려받기 시험, 기본으로는 돌지 않는다. CI 3개 OS 전체 작업이 Node 22로 따로 돌린다): TypeScript 7을 받아 Node 없이 띄우고 물어서 타입 오류를 받는다. HTML·CSS·JSON을 함께 설치해 한 폴더를 같이 쓰고, CSS·JSON 진단을 받고, 하나를 지워도 나머지가 남는다.
    - 실제 앱(Windows, 릴리스 빌드): 언어 서버 창에서 넷을 연달아 설치하고, `.ts`의 타입 오류 물결선과 상태 표시줄(`typescript: 준비됨`, 오류 1), `.js`의 `document.getElem` 자동완성(`getElementById` 등), CSS·JSON 진단을 화면으로 확인했다. 확인 뒤 설치 폴더와 설정 폴더를 지웠다.
  - 남은 것: egov 화면의 jQuery 자동완성(Phase 완료 조건에서 확인). JSON 스키마 검사는 백로그 "JSON 스키마 검사"(2026-11-04)에서 했다.
- [x] 8-2 기호 개요·기호 찾기, 엔드포인트 목록 — 중간
  - 결정(사용자): 엔드포인트는 따로 창(8-3의 요청 만들기를 붙일 자리), 개요는 도킹 창
  - 개요 창(`Ctrl+Shift+O`, [편집 > 개요]): 프로젝트 옆 탭. 지금 문서를 맡은 서버 중 기호를 주는 첫 서버에 `textDocument/documentSymbol`을 묻는다.
    - 나무(접기)로 보이고, 이름·종류·덧붙임(시그니처 등)을 보인다. 거르기 칸에 치면 평평하게 걸러 담은 기호 이름을 붙인다. 누르면 그 기호로 간다.
    - 문서를 바꾸면 바로, 같은 문서를 고치면 0.4초 멈춘 뒤 다시 묻는다. 평평한 응답(`SymbolInformation[]`)도 받는다.
  - 기호로 이동(`Ctrl+T`, [편집 > 기호로 이동…]): 치기를 0.25초 멈추면 떠 있는 준비된 서버 모두에 `workspace/symbol`을 묻고, 서버마다 오는 답을 모아(500개까지) 보인다. 서버마다 위치 단위를 따로 기억한다(UTF-8·UTF-16이 섞인다).
  - 엔드포인트 창([도구 > 엔드포인트]): 떠 있는 서버에 `@/`를 묻고 컨트롤러별로 HTTP 메서드와 URL을 보인다. 거르기(URL·컨트롤러), [새로 고침], 누르면 처리 메서드로 간다.
    - egov 서버(내장)를 창을 열 때 띄운다(Java 파일을 열지 않아도 된다).
    - Spring Boot LS가 엔드포인트를 주면 egov 서버의 것은 감춘다(같은 매핑을 둘 다 준다).
  - egov 서버: 색인한 Java 파일에서 요청 매핑을 모아 `workspace/symbol`로 낸다(`workspaceSymbolProvider`).
    - `@RequestMapping`·`@Get/Post/Put/Delete/PatchMapping`. 경로는 `value`·`path`·위치 인자와 배열, 메서드는 `method = RequestMethod.X`와 배열. 클래스의 공통 경로와 메서드 경로를 모든 짝으로 잇는다.
    - 대상: `@Controller`·`@RestController`, 또는 클래스에 `@RequestMapping`. 상수로 쓴 경로는 글자로 읽을 수 없어 건너뛴다.
    - 이름은 Spring Boot LS와 같은 꼴(`@/board/list.do -- GET,POST`), 위치는 처리 메서드 이름(파싱할 때 UTF-16으로 미리 센다), 담은 것은 컨트롤러 이름이다.
  - 검증
    - 단위: 매핑 해석(공통 경로, 배열, 메서드, 메서드 없는 매핑, 상수 경로, 컨트롤러가 아닌 클래스, 한글 메서드 이름 위치), 기호 응답 읽기(나무·평평·파일만 준 기호), 엔드포인트 이름 나누기, 개요를 묻는 때
    - egov 서버 LSP 시험: 실제 통신으로 `workspace/symbol`의 이름·거르기(대소문자 무시)·컨트롤러 이름·위치
    - 로컬 코퍼스(egov 프로젝트들, 저장소에 넣지 않음): 컨트롤러 646개의 매핑을 엔드포인트 5,139개로 모았고 덜 모은 파일은 없다. 닫지 않은 `/**`로 메서드가 통째로 주석이 된 파일은 단순 검사기도 주석을 빼고 센다.
    - 실제 앱(Windows, 릴리스 빌드): Java 파일을 열지 않고 엔드포인트 창에 6개·컨트롤러 2개, 눌러 처리 메서드로 이동, rust-analyzer로 개요 나무(구조체·필드·impl·함수·메서드와 시그니처), `Ctrl+T`에서 egov 엔드포인트와 rust-analyzer 기호가 함께 나오는 것을 화면으로 확인했다. 확인 뒤 설정 폴더를 지웠다.
  - 뒤에 한 것: 디스크에서 바뀐 Java 파일의 다시 색인은 백로그 "디스크 바뀜 알림"(2026-11-03), 엔드포인트 창의 자동 새로 고침은 "엔드포인트 자동 새로 고침"(2026-11-05)에서 했다.
- [x] 8-3 HTTP 요청 파일 실행과 응답 보기 — 중간
  - 결정(사용자): 환경 파일은 널리 쓰는 꼴(`http-client.env.json` + `http-client.private.env.json`), 비밀 값은 개인 환경 파일에만 둔다(OS 키체인에 쓰지 않는다). 엔드포인트의 요청은 컨트롤러마다 `http/<컨트롤러>.http`에 모은다.
  - `ferrite-http` 크레이트(ureq 3.5, rustls + OS 인증서 검증기)
    - 해석: `###`로 나누고 뒤의 글은 이름, `# @name`이 앞선다. 요청 줄(메서드 생략 시 GET, 끝의 `HTTP/1.1`은 뺀다), `?`·`&`로 시작하는 줄은 질의를 잇고, 빈 줄까지 헤더, 다음 `###`까지 본문. `###` 줄도 아래 요청에 든다.
    - 지시: `# @insecure`(그 요청만 인증서 검증을 끈다. 인증서 오류에 이 방법을 알린다), `# @no-redirect`, `# @no-cookie-jar`.
    - 변수: 파일 변수(`@이름 = 값`, 그 안의 변수도 채운다) → 고른 환경 → `$uuid`·`$timestamp`·`$randomInt`. 채우지 못하면 보내지 않고 이름을 알린다(파일 변수 안에서 못 채운 이름을 알린다).
    - 환경 파일: `.http` 파일의 폴더부터 프로젝트 루트까지 올라가며 처음 찾은 폴더의 두 파일을 읽고, 개인 파일 값이 이긴다. 숫자 값은 글로 바꾼다.
    - 보내기: 리다이렉트를 직접 따라가(10번까지) 거치는 응답의 `Set-Cookie`도 담는다. 303과 POST의 301·302는 GET으로 바꾸고 본문을 뺀다(브라우저와 같다). 307·308은 그대로.
    - 쿠키: 도메인(하위 도메인 포함 여부)·경로·Secure·Max-Age를 따르고, 지난 `Expires`(1970)는 지우기로 본다. 같은 쿠키를 바꿀 때 자리를 지킨다.
    - 응답: 본문 20MB까지. 문자 집합은 `Content-Type`의 charset, 없으면 UTF-8, UTF-8이 아니면 EUC-KR(CP949). JSON은 글을 다시 쓰지 않고 들여 써서 키 차례와 숫자 표기(`1.10`, 큰 정수)가 그대로다.
  - 앱
    - `.http`·`.rest` 파일의 요청 줄 거터에 초록 ▶. 누르거나 `Ctrl+Enter`(커서가 든 요청)로 보낸다. 요청 파일의 거터는 중단점을 걸지 않는다.
    - HTTP 응답 창([도구 > HTTP 응답]): 환경 고르기, 쿠키 수와 [쿠키 지우기], 요청 이름·메서드·채운 URL·환경, 상태(색)·걸린 시간·크기·문자 집합, 거친 리다이렉트, 본문(고르고 복사)·헤더 탭. 이진 본문은 크기와 종류만, 512KB를 넘는 글은 앞만 보인다.
    - 보내기는 따로 스레드에서 한다. 쿠키 저장소를 그 스레드가 가졌다 돌려주므로, 앞 요청을 받는 동안에는 새 요청을 보내지 않는다(받은 쿠키를 잃지 않게).
    - 개인 환경 파일이 있고 git 프로젝트의 `.gitignore`가 빼지 않으면 경고하고 [.gitignore에 더하기]를 보인다.
    - 엔드포인트 창의 [요청]: `http/<컨트롤러>.http` 끝에 `### 컨트롤러 경로`와 요청을 더하고(POST·PUT·PATCH는 폼 `Content-Type`), 이미 있으면 그 줄로 간다. 환경 파일이 없으면 `dev.baseUrl` 견본을 만든다. 더한 요청은 사용자가 저장한다.
  - 검증
    - 단위: 해석(이름·지시·질의 잇기·본문·`###` 줄), 변수 순서·못 채운 이름, 환경 파일 찾기·개인 값·`.gitignore`, 쿠키(도메인·경로·Secure·바꾸기·지우기), 문자 집합, JSON 들여쓰기(키 차례·숫자·문자열 속 기호), 리다이렉트 주소, 요청 블록·파일 이름
    - 로컬 서버: 로그인 POST → 302 + `Set-Cookie` → 쿠키를 붙여 따라가 EUC-KR 화면, 이어 보낸 요청에 쿠키, `@no-cookie-jar`면 401, JSON, `@no-redirect`면 302 그대로, 404, 연결 거절
    - 창 시험(kittest): ▶와 `Ctrl+Enter`(줄을 나누지 않는다), 개인 환경 파일의 값, 쿠키 유지, ▶가 아닌 줄은 보내지 않음, `.gitignore`에 더하기, 엔드포인트로 두 번 만들어도 한 번만 더함
    - 실제 앱(Windows, 릴리스 빌드, 로컬 서버): `### 로그인` 줄에서 `Ctrl+Enter` → 302를 따라 EUC-KR 화면 200, 쿠키 1개와 `.gitignore` 경고, 게시판 JSON의 ▶ → 쿠키로 200과 들여 쓴 JSON(키 차례 그대로), 엔드포인트 창 [요청]으로 `LoginController.http`에 두 요청을 만들고 보내 401(쿠키 없음)을 화면으로 확인했다. 확인 뒤 설정 폴더를 지웠다.
  - 뒤에 한 것: 파일·멀티파트 본문, 응답 저장, 구문 강조는 백로그 "HTTP 요청 파일 보강"(2026-11-04), 요청 기록은 "HTTP 요청 기록"(2026-11-05), 응답 처리 스크립트는 "HTTP 응답 처리 스크립트"(2026-11-06)에서 했다.
- [x] 8-4 브라우저 JS 디버그 — 중간
  - 결정(사용자): vscode-js-debug 1.140.0(MIT, 독립 DAP 서버 `js-debug-dap`, 1.2MB, SHA-256 고정, Node 필요), 띄우기와 붙기 둘 다, JSP 인라인 스크립트는 브라우저가 받은 화면을 읽기 전용으로 열어 중단점을 건다, 화면 주소는 8-3 환경 파일의 `baseUrl`
  - 어댑터: `node js-debug/src/dapDebugServer.js 0 127.0.0.1`로 빈 포트에 띄우고 알린 주소로 붙는다(delve와 같은 방식).
    - js-debug는 임시 폴더에 브라우저 프로필(한 번에 약 6MB)을 만들고 지우지 않는다. 세션마다 Ferrite의 폴더를 임시 폴더(`TEMP`·`TMP`·`TMPDIR`)로 주고 세션이 끝나면 지운다.
    - 디버그 중에 앱을 닫으면 대상을 끝내라고 하고 잠깐(3초) 기다린 뒤 임시 폴더를 지운다. 어댑터만 바로 죽이면 브라우저가 늦게 끝나 폴더를 쥐고 있어 지우지 못했다(실제 앱에서 찾았다).
  - 자식 세션(DAP `startDebugging`): 브라우저 페이지마다 어댑터가 거꾸로 요청한다.
    - `initialize`에서 받는다고 알리고, 요청이 오면 같은 주소로 새로 붙어 시작 순서(initialize → launch → initialized → 중단점 → configurationDone)를 밟는다. 자식 안의 자식도 같다.
    - 화면은 세션 하나만 다룬다. 요청 번호를 부모·자식이 나눠 써서 겹치지 않고, 사용자 요청(스택·변수·계속 등)은 멈춘 자식(없으면 마지막 자식)에게 간다. 중단점은 모든 세션에 건다.
    - 자식의 시작·끝은 세션 전체의 일이 아니므로 버린다. 멈춘 채 끝난 자식은 풀린 것으로 알린다. 자식이 있으면 중단점 확인은 자식의 것을 쓴다(부모는 늘 "아직 못 걸음").
    - 멈춤은 자식부터 끊는다(js-debug는 멈춘 페이지가 있으면 부모의 disconnect에 답하지 않는다). disconnect 응답이 오면 끝난 것으로 본다(js-debug는 연결을 닫지 않고 terminated도 보내지 않는다). 붙은 대상(attach)은 끝내지 않고 떨어지기만 한다.
  - 중단점 확인: js-debug는 처음에 "Unbound"로 답하고 스크립트를 불러올 때 `breakpoint` 이벤트(`changed`)로 알린다. 번호로 찾아 고친다.
  - 화면 스크립트: 로컬 파일이 없는 소스(`sourceReference`)는 `source`로 내용을 받아 임시 폴더에 담아 읽기 전용으로 연다.
    - 그 파일의 중단점은 어댑터의 원래 소스(경로·번호)로 건다. 멈춘 자리가 그 소스면 담은 파일의 그 줄을 보인다.
    - js-debug는 인라인 스크립트를 `<스크립트 태그 줄>:<열>` 이름의 소스로 주고, 내용은 그 스크립트만 화면 줄에 맞춰 앞을 빈 줄로 채운 것이다. 그래서 줄 번호가 화면(브라우저가 받은 HTML)의 줄과 같고, 담은 파일은 `.js`로 강조한다.
    - 디버그 창 도구 막대의 "화면 스크립트"(펼칠 때 `loadedSources`로 묻는다)에서 골라 연다. 호출 스택의 화면 스크립트 프레임을 눌러도 열린다. 창이 낮아도 보이게 펼침 메뉴로 두었다.
    - 읽기 전용 에디터: 글자·지우기·줄 나누기·붙여넣기·잘라내기·되돌리기는 버리고 움직임·고르기·복사는 받는다.
  - 자동 구성(`.js`·`.jsp`·`.html` 등): "Chrome(없으면 Edge)로 주소 열기"와 "9222 포트로 연 브라우저에 붙기".
    - 주소는 루트나 `http/`의 `http-client.env.json`의 `dev`(없으면 첫) 환경 `baseUrl`, 없으면 `http://localhost:8080`.
    - webRoot는 `src/main/webapp`·`WebContent`·`web`(없으면 루트), 주소에 컨텍스트 경로가 있으면 `pathMapping`으로 그 아래 URL을 webRoot에 맞춘다.
  - 검증
    - 단위: 자동 구성(주소·webRoot·컨텍스트 경로·붙기 거르개), 담을 파일 이름·로컬 파일 없는 소스 가리기
    - 가짜 어댑터: 거꾸로 요청한 자식 세션의 시작 순서, 자식의 중단점 확인이 부모의 미확인을 대신함, 자식에서 멈추고 스택·소스 받기, 담은 파일의 중단점을 원래 소스로 걸기, 자식이 끝나도 세션은 이어짐
    - 창 시험(kittest): 읽기 전용 문서에 친 글자·지우기·줄 나누기를 버리고 움직임은 받음
    - 실제 어댑터(내려받기 시험, `adapters.yml`이 3개 OS에서 돌린다): js-debug를 받아 헤드리스 Chrome으로 컨텍스트 경로 `/app` 화면을 열고, 외부 스크립트(`/app/js/app.js` → webapp 파일)의 중단점에서 멈추고, 불러온 스크립트에서 화면의 인라인 스크립트를 받아 그 줄에 건 중단점에서 멈춘 뒤, 끝내고 임시 폴더가 남지 않는다.
    - 실제 앱(Windows, 릴리스 빌드, 로컬 웹 서버): 언어 서버 창에서 js-debug 설치, F5 목록에 자동 구성 둘(주소는 환경 파일의 `baseUrl`), `app.js` 3줄 중단점에서 멈춤(호출 스택에 화면 스크립트 프레임), 그 프레임을 눌러 읽기 전용으로 연 화면 스크립트 8줄에 중단점을 걸고 계속해 그 줄에서 멈춤, 도구 막대 "화면 스크립트"에 인라인 스크립트가 보이는 것을 화면으로 확인했다. 브라우저는 헤드리스로 띄운 사용자 구성을 썼다. 앱을 닫은 뒤 Chrome·node 프로세스와 임시 폴더가 남지 않았다. 확인 뒤 설정·설치 폴더를 지웠다.
  - 남은 것: 소스 맵(번들된 TS 등)은 js-debug가 하는 대로 두고 따로 확인하지 않았다. 화면 스크립트의 줄을 JSP 줄로 맞추기는 백로그 "JSP 화면 스크립트 줄 맞추기"(2026-11-06)에서 했다.
- [x] 3개 OS 검증(2026-10-29)
  - 완료 조건 (1) jQuery를 쓰는 `.js`의 자동완성·정의 이동: 확인해 보니 egov 프로젝트에서는 되지 않았다. 그래서 jsconfig.json 제안을 더했다.
    - 확인(TypeScript 7.0.2, jquery 3.7.1): TS 서버는 열린 파일만 보므로 `<script>`로 따로 불러오는 jQuery(`$`)와 다른 스크립트의 전역 함수를 모른다. jQuery 원본만으로는 타입을 짐작하지 못한다(감싸는 코드 때문).
    - 화면 폴더의 스크립트를 묶는 jsconfig.json이 있으면 TS 서버가 라이브러리 파일 이름(`jquery-*.js`)을 보고 `@types/jquery`를 npm으로 받아(자동 타입 받기, VS Code와 같다) `$.ajax`·`.val` 완성과 `$` 정의 이동(`misc.d.ts`)이 된다.
    - 자동 타입 받기는 사용자 캐시(Windows `%LOCALAPPDATA%\Microsoft\TypeScript\7.0`)에 묻지 않고 받는다. 한 번 받으면 jsconfig.json이 없는 프로젝트에서도 jQuery를 안다. 결정(사용자): 그대로 두고 문서에 알린다. 인터넷이 막힌 PC는 프로젝트의 `node_modules/@types`에 넣는다.
    - 결정(사용자): 만들기를 제안한다. 화면 폴더(`src/main/webapp`·`WebContent`·`web`)의 `.js`를 열었는데 그 폴더부터 루트까지 jsconfig.json·tsconfig.json이 없으면 에디터 위에 알림과 [jsconfig.json 만들기]·[닫기]를 보인다. 만들면 `{"include": ["<화면 폴더>/**/*.js"]}`를 쓰고 JS 언어 서버를 다시 띄운다. 닫으면 그 프로젝트에서는 앱이 켜진 동안 묻지 않는다.
    - TypeScript 7은 `jquery-3.7.1.min.js` 같은 `.min.js` 이름으로는 타입을 받지 않는다(`jquery-3.7.1.js`는 받는다). egov 프로젝트는 보통 `.min.js`를 두므로, 화면 폴더에 `jquery*.js`가 있으면 jsconfig.json에 `"typeAcquisition": {"include": ["jquery"]}`를 적는다. CI의 실제 시험이 처음 실패해(빈 캐시에서 jQuery 완성이 오지 않음) 찾았다.
    - 시험: 단위(제안 조건·내용), 창 시험(닫기·만들기), 실제 TypeScript 7(CI 3개 OS 전체 작업의 npm 시험): jsconfig.json이 없으면 다른 스크립트의 전역 함수를 모르고, 있으면 그 함수와 `$.ajax`를 완성한다.
  - 완료 조건 (2) 엔드포인트로 만든 `.http` 요청에서 로그인 쿠키 유지: 8-3의 로컬 서버 시험(로그인 302 → 쿠키 → 이어 보낸 요청)과 창 시험(▶·`Ctrl+Enter`·엔드포인트로 요청 만들기)이 CI 3개 OS에서 통과했다.
  - 완료 조건 (3) JSP 화면 스크립트의 중단점: 8-4의 실제 js-debug 시험(헤드리스 Chrome, `.do` 화면의 인라인 스크립트와 외부 스크립트의 중단점)이 어댑터 워크플로 3개 OS에서 통과했다(실행 40815830231).
  - CI 3개 OS 전체(실행 40820807183, `8280049`) 통과. 실제 TypeScript 7 jQuery 시험(npm)이 3개 OS에서 빈 캐시로 `@types/jquery`를 받아 통과했다.
  - 확인하지 못한 것: 화면으로 직접 본 것은 Windows뿐이다. macOS·Linux는 시험으로만 확인했다. 실제 egov 표본 프로젝트(전자정부 템플릿)로는 돌려 보지 않았고 그 꼴을 본뜬 폴더로 확인했다.

### Phase 9 — 테스트·보안 점검

**목표**: 테스트 결과, 정적 분석 결과, 취약한 라이브러리를 편집기 안에서 보고 그 줄로 간다. 공공 사업의 보안약점 진단과 취약 라이브러리 점검에 대비한다.

- 테스트 러너: 실행 구성(4-1)의 테스트를 돌린 뒤 보고서를 읽어 결과 트리(통과·실패·건너뜀, 걸린 시간)로 보인다.
  - 공통 형식은 JUnit XML이다: Maven(`target/surefire-reports`), Gradle(`build/test-results`), pytest(`--junitxml`), CTest(`--output-junit`). Go는 `go test -json`을 읽는다.
  - 실패를 누르면 그 테스트와 스택의 줄로 간다. 실패한 것만 다시 돌린다(Maven `-Dtest=`, pytest 노드 id 등).
- 정적 분석 결과 보기(SARIF 2.1.0): 외부 도구(SpotBugs·PMD·Semgrep·clang 정적 분석기 등)가 낸 `.sarif`를 열어 문제 창에 넣는다. 도구를 돌리는 일은 실행 구성에 맡긴다.
- 의존성 취약점 검사: `pom.xml`·`build.gradle`·`package-lock.json`·`requirements.txt`·`Cargo.lock`의 라이브러리 버전을 공개 취약점 DB(OSV)와 견주고, 고친 버전을 알린다.
  - Maven은 전이 의존을 `mvn dependency:tree`로 푼다. egov는 전이로 들어오는 로깅·Spring 라이브러리가 많다.
  - 결과는 문제 창과 그 의존성 줄의 경고로 보인다.
  - 인터넷이 막힌 PC는 생태계별 DB 묶음 파일을 다른 PC에서 받아 와 쓴다(언어 서버의 "파일에서…"와 같다).

**완료 조건**: egov 표본 프로젝트에서 `mvn test` 결과가 트리로 보이고 실패 테스트의 줄로 간다. SpotBugs가 낸 SARIF가 문제 창에 뜬다. 알려진 취약 버전(예: log4j 2.14.1)을 쓰는 pom에 경고가 뜨고, 오프라인 DB로도 같다. 3개 OS.

**진행 단계**:
- [x] 9-1 테스트 러너(JUnit XML·`go test -json` 결과 트리) — 중간
  - 결정(사용자): 테스트 창이 명령을 직접 띄운다(끝남·종료 코드를 알고 `go test -json`의 표준 출력을 읽는다), JUnit XML은 roxmltree 0.21.1(eframe이 이미 쓰는 판), 대상은 계획대로 Maven·Gradle·pytest·CTest·Go, 실패를 누르면 스택의 테스트 줄로
  - 테스트 창([실행 > 테스트]): 실행 구성(자동 감지·`.ferrite/run.toml`) 중 테스트 도구로 알 수 있는 명령을 고른다(`mvn test`·`gradle test`·`python -m pytest`·`ctest`·`go test`).
    - [실행]·[실패만 다시]·[멈춤]. 결과·출력은 위 [결과]·[출력]으로 바꿔 보인다(창이 낮아도 다 보이게). 시작하면 출력, 결과가 오면 결과를 보인다.
    - 결과 트리: 묶음(클래스·pytest 모듈·Go 패키지)마다 통과·실패·건너뜀과 걸린 시간. 실패가 있는 묶음은 펼쳐 둔다. 고르면 오른쪽에 사유·스택(누르면 그 줄로, 테스트 줄은 굵게)·전체 글. 실패를 두 번 누르면 테스트 줄로 간다.
    - 출력은 줄마다 문자 집합을 맞춘다(Windows의 Maven은 CP949로 쓴다). `go test -json`은 사건 안의 출력만 보인다.
    - [멈춤]은 셸이 띄운 자식까지 끝낸다(Windows `taskkill /T`, 그 밖은 프로세스 묶음).
  - 보고서(`ferrite-core` `test_report`)
    - 명령에 보고서 옵션을 붙인다: pytest `--junitxml`, CTest `--output-junit`(앱 임시 폴더), Go `-json`. Maven·Gradle은 원래 남긴다.
    - 이번 실행의 보고서만 읽는다(시작 뒤 고친 파일): Maven `target/surefire-reports`·`failsafe-reports`의 `TEST-*.xml`, Gradle `build/test-results`.
    - JUnit XML: `failure`·`error`는 실패, `skipped`는 건너뜀. 실패의 `system-out`(CTest는 여기에만 출력)을 붙인다.
    - CTest(CMake의 JUnit 쓰기를 따름): `status="disabled"`는 건너뜀, `status="notrun"`은 돌리지 못한 것이라 CTest처럼 실패로 센다(사유는 `<skipped message>`). 다만 `SKIP_RETURN_CODE`·`SKIP_REGULAR_EXPRESSION`으로 건너뛴 것은 건너뜀.
    - 자동 감지한 CTest 구성은 `ctest --test-dir build -C Debug`다. Windows의 Visual Studio 생성기(다중 구성)는 `-C`가 없으면 테스트를 하나도 돌리지 않는다(CI의 실제 시험이 Windows에서만 모두 "건너뜀"으로 나와 찾았다).
    - `go test -json`: 부속 테스트, Go 1.24의 빌드 출력(`build-output`)과 빌드 실패는 "(패키지)" 실패로.
  - 실패만 다시: Maven `-Dtest=클래스#메서드+메서드 -Dsurefire.failIfNoSpecifiedTests=false`, Gradle `--tests 클래스.메서드`, pytest `--lf`, CTest `--rerun-failed`, Go `-run "^(이름들)$" 패키지들`. 받은 결과를 앞 결과에 덮어 쓴다.
  - 줄 찾기: Java는 스택에서 테스트 클래스의 프레임을 골라 패키지 경로(`com/example/CalcTest.java`)로 파일을 찾는다(Gradle의 `app//` 앞붙이, 안쪽 클래스). pytest는 마지막 `파일.py:줄:`, Go는 `파일_test.go:줄:`.
  - 검증
    - 단위: 실제 Surefire 3.5.4 보고서(로컬 Maven으로 오프라인 실행)와 Gradle·pytest(xunit2)·CTest·`go test -json` 형식, 도구 알아내기, 보고서 옵션·실패만 다시 명령, 이번 실행의 보고서 찾기, 줄 찾기
    - 창 시험(kittest): 결과 트리와 실패 사유, 스택의 테스트 줄을 눌러 그 파일 그 줄로
    - 실제 도구(CI 3개 OS 전체 작업): Maven(JUnit 5)·pytest·Go·CTest를 돌려 결과·테스트 줄을 확인하고 실패만 다시 돌려 실패한 것만 도는지 본다.
  - 남은 것: Kotlin·Scala 스택은 같은 꼴이라 읽지만 확인하지 않았다. Gradle 실제 실행은 백로그 "Gradle 실제 실행"(2026-11-05)에서 했다.
- [x] 9-2 SARIF 결과 가져오기 — 작음
  - 결정(사용자): 문제 창 목록과 열린 파일의 물결선 둘 다, 프로젝트의 SARIF를 찾아 고르고 바뀌면 다시 읽기, 실제 SpotBugs로 로컬·CI 확인(로컬에서 spotbugs-maven-plugin 4.10.5.0을 받음, 53개 jar 31MB)
  - 문제 창 위 [이 파일]·[정적 분석 (N)]: 정적 분석은 프로젝트의 SARIF(`*.sarif`·`*.sarif.json`, SpotBugs의 `spotbugsSarif.json`, 최근 것이 먼저)를 골라 [불러오기], 그 밖은 [다른 파일…]. [찾기]·[지우기].
    - 도구·수준별 개수와 파일을 보이고, 결과를 파일별로 묶어 `줄:열 [수준] 규칙: 메시지`로 보인다. 누르면 그 파일 그 줄·열로 간다. 말풍선은 도구와 규칙 설명 주소.
    - 불러온 파일을 1초마다 보고 바뀌면 다시 읽는다(실행 구성으로 분석을 다시 돌린 뒤).
    - 열린 파일에는 언어 서버 진단처럼 물결선·말풍선으로 보이고 상태바 개수에 들어간다. 열이 없으면 들여쓰기를 뺀 그 줄을 칠한다. 같은 자리면 심각한 것을 위에 그린다.
  - SARIF 2.1.0 읽기(`ferrite-core` `sarif`)
    - 수준: 결과의 `level`, 없으면 규칙의 `defaultConfiguration.level`, 그것도 없으면 경고. `note`·`none`은 정보. `kind`가 `pass`·`notApplicable`이면 뺀다.
    - 메시지: 규칙의 메시지 틀(`messageStrings`)에 인자(`arguments`)를 채운 것을 쓴다(SpotBugs는 짧은 `text`와 자세한 틀을 함께 준다). 없으면 `text`.
    - 파일: `uriBaseId` → `originalUriBaseIds`로 잇기, `file:` 주소, SARIF 폴더·프로젝트 기준 상대 경로, 그래도 없으면 경로 끝이 같은 프로젝트 파일. SpotBugs(Maven 플러그인)는 기준 주소 없이 원본 폴더 기준 경로(`com/example/board/Board.java`)만 적어서 마지막 방법이 필요하다. `%XX`를 푼다.
    - 테스트 보고서와 같이 쓰는 프로젝트 파일 찾기를 `files`로 뺐다.
  - 검증
    - 단위: 실제 SpotBugs 4.10.5 SARIF(로컬에서 낸 것), 기준 주소·`file:` 주소·규칙 기본 수준·`kind=pass`·`%20`, 메시지 틀, SARIF 파일 찾기, 결과를 에디터 범위로
    - 창 시험(kittest): 불러오기 → 파일별 목록, 누르면 그 줄로, 물결선·상태바 개수, SARIF가 바뀌면 다시 읽기
    - 실제 SpotBugs(CI 3개 OS 전체 작업): Maven으로 분석해 `spotbugsSarif.json`을 찾고, null 역참조(`NP_ALWAYS_NULL`)가 그 파일 12줄의 오류로, 메시지 틀이 채워져 나온다.
    - 실제 앱(Windows, 릴리스 빌드): SpotBugs가 낸 SARIF를 찾아 불러와 오류 1·경고 2·참고 1, 7·12줄 물결선, 상태바 "오류 1 · 경고 2", 눌러서 12줄로 가는 것을 화면으로 확인했다. 확인 뒤 설정 폴더를 지웠다.
  - 남은 것: clang의 실제 SARIF는 돌려 보지 않았다(SARIF 2.1.0 공통 형식과 기준 주소 처리로 읽는다). PMD·Semgrep은 백로그 "PMD·Semgrep 실제 SARIF"(2026-11-05)에서 했다.
- [x] 9-3 의존성 취약점 검사(OSV, 오프라인 DB) — 중간
  - 결정(사용자): 온라인(api.osv.dev, 이름·버전만 보내고 화면에 알린다)과 오프라인 DB(생태계별 OSV `all.zip`을 [받기] 또는 [파일에서…]) 둘 다, Maven·Gradle은 빌드 도구로 전이 의존까지 풀고 전이 의존은 끌어온 직접 의존의 줄에, 5개 생태계, 결과는 문제 창 [취약점]과 의존성 줄 물결선(SARIF와 같은 틀)
  - 문제 창 [취약점 (N)]: [검사], 온라인·오프라인 DB 고르기. 오프라인은 생태계마다 받은 DB의 크기·받은 때와 [받기](크기 안내)·[파일에서…](인터넷이 막힌 곳).
    - 의존성 개수와 심각도별 취약 개수, 빌드 도구 실패·DB 없는 생태계는 노란 글로 알린다.
    - 의존성 파일별로 `줄 [심각도] CVE · 이름 버전 → 고친 버전 · 끌어온 것`, 누르면 그 줄로 간다. 말풍선은 OSV 이름과 요약.
    - 열린 의존성 파일에는 물결선·말풍선(치명·높음은 오류, 보통·모름은 경고, 낮음은 참고), 상태바 개수에 들어간다.
  - 의존성 읽기(`ferrite-core` `vuln::manifest`): 프로젝트에서 `pom.xml`(맨 위만, 모듈은 함께 풀린다)·`build.gradle(.kts)`·`package-lock.json`·`requirements*.txt`·`Cargo.lock`을 찾는다(빌드 결과 폴더는 뺀다).
    - Maven: `mvn dependency:tree -DoutputFile=… -DappendOutput=true`의 나무를 읽고, 모듈마다 그 pom의 `<dependencies>` 아래 직접 의존 줄을 찾는다(없으면 관리 블록, 부모에서 온 것은 첫 줄).
    - Gradle: `gradle -q dependencies`의 `runtimeClasspath` 나무(`->`로 고른 버전, `(*)`, 하위 프로젝트 `project :lib`). 래퍼(`mvnw`·`gradlew`)는 파일 폴더에서 프로젝트 루트까지 올라가며 찾는다.
    - npm 잠금 v2·v3(`packages`의 `node_modules/…` 경로, 중첩은 끌어온 것)·v1(`dependencies` 나무), PyPI `==` 고정(환경 표시·해시·옵션 줄은 건너뛰고 이름은 PEP 503으로), Cargo는 crates.io에서 온 것만.
  - OSV 대조(`vuln::osv`, `vuln::version`)
    - 영향: `versions` 목록, `ECOSYSTEM`·`SEMVER` 범위의 `introduced`·`fixed`·`last_affected`(끝없는 구간 포함, `GIT`은 뺀다). 고친 버전은 그 버전이 든 구간의 것. 철회한 레코드는 뺀다.
    - 버전 비교: 생태계마다 사전 배포 순서(Maven alpha < beta < milestone < rc < snapshot < 정식 < sp, PyPI dev < a < b < rc < 정식 < post, semver `-` 사전 배포와 `+` 빌드 정보). 끝의 0은 없는 것과 같다.
    - 심각도: `database_specific.severity`(GitHub 권고), 없으면 CVSS v3 벡터로 기본 점수를 계산한다(RUSTSEC·PYSEC).
    - 온라인은 `querybatch`(1000개씩, 같은 이름·버전은 한 번)로 해당 ID를 받고 레코드를 받는다. 오프라인은 묶음의 레코드마다 패키지 이름만 먼저 읽고 맞는 것만 다 읽는다.
    - 한 취약점이 여러 DB의 레코드(GHSA·PYSEC·RUSTSEC)로 있어 ID·CVE가 겹치면 하나로 합친다(심각도는 높은 것, 고친 버전은 있는 것). 합치지 않으면 실제 예제에서 88건(모름 26)이 56건이 된다.
  - 검증
    - 단위: Log4Shell 레코드의 세 구간과 고친 버전, `versions`·`last_affected`·끝없는 구간·`GIT`·철회, CVSS v3 점수(10.0·9.8·5.9·6.1), 결과 정렬·중복·여러 DB의 같은 취약점 합치기, 오프라인 묶음 읽기·들이기, Maven 나무(모듈·관리 블록)·Gradle·npm v3·v1·requirements·Cargo.lock, 래퍼 찾기, 버전 비교
    - 창 시험(kittest): 오프라인 DB로 검사 → 개수·DB 없는 생태계 알림·파일별 목록, 누르면 그 줄로, 물결선·상태바 개수
    - 실제 도구(CI 3개 OS 전체 작업): `mvn dependency:tree`로 log4j-slf4j-impl 2.14.1의 전이 log4j-core 2.14.1을 풀어 직접 의존 줄에 두고, 온라인 OSV와 받은 Maven DB(약 10MB) 둘 다 CVE-2021-44228(고친 버전 2.15.0)을 찾는다.
    - 실제 앱(Windows, 릴리스 빌드): pom.xml(log4j-slf4j-impl 2.14.1)·requirements.txt·npm 잠금·Cargo.lock 예제를 온라인으로 검사해 의존성 8개·취약 56건, pom.xml 7줄 물결선과 상태바 "오류 3 · 경고 6". 오프라인은 [받기]로 Maven DB(10.5MB)를 받아 pom.xml 9건이 온라인과 같고, DB 없는 npm·PyPI·crates.io를 알린다. 확인 뒤 설정·DB 폴더를 지웠다.
  - 남은 것: 오프라인은 검사마다 묶음을 다 푼다(npm 220MB는 오래 걸린다. 느리면 들일 때 패키지별 색인). Gradle 실제 실행은 백로그 "Gradle 실제 실행"(2026-11-05)에서 했다.
- [x] 3개 OS 검증(2026-11-02)
  - 결정(사용자): 실제 egov 템플릿으로 확인한다(웹 템플릿 egovframe-web.zip 15.5MB를 받고, 의존성 이름·버전을 api.osv.dev에 묻고, Maven DB 10.5MB를 받는다). 로컬과 `templates.yml` 3개 OS.
  - 완료 조건 (1) egov 표본 프로젝트의 `mvn test` 결과 트리와 실패 줄: 확인해 보니 egov 템플릿에서는 되지 않았다. 그래서 건너뜀 알림과 켜기 단추를 더했다.
    - 확인(egov 5.0.0 웹 템플릿, surefire 3.5.2): pom의 maven-surefire-plugin 설정에 `<skipTests>true</skipTests>`가 있어 `mvn test`가 "Tests are skipped."로 끝난다. 설정에 직접 적은 값이라 `-DskipTests=false`로도 켜지지 않는다. 테스트 창은 "결과 보고서가 없습니다(컴파일 실패 등)"라고 보여 까닭을 알 수 없었다.
    - 결정(사용자): 알리고 켜기 단추. Maven 구성이 보고서 없이 끝나고 돌린 폴더의 pom.xml에 그 설정이 있으면 "테스트를 건너뛰었습니다: pom.xml의 maven-surefire-plugin 설정이 skipTests=true입니다(egov 템플릿 기본값)"와 [pom.xml에서 테스트 켜기]를 보인다. 누를 때만 그 요소를 지운다(그 줄에 그것만 있으면 줄째, `ferrite-core` `test_report::enable_tests`). Maven 기본으로 돌아가 패키징 때는 `-DskipTests`로 건너뛸 수 있다. 다른 플러그인(failsafe)의 같은 설정은 건드리지 않는다.
    - 시험: 단위(찾기·지우기, CRLF, 한 줄에 같이 있는 것), 실제 egov 웹 템플릿(`templates.yml` 3개 OS, 실행 41386606209): 템플릿 pom이 테스트를 건너뛰게 한 것을 찾아 지우고, 넣은 JUnit 5 테스트를 `mvn test`로 돌려 통과 1·실패 1, 실패 줄이 `SampleTest.java` 15줄이다.
    - 실제 앱(Windows, 릴리스 빌드): 웹 템플릿 프로젝트에서 [실행] → 건너뜀 알림과 단추 → [pom.xml에서 테스트 켜기] → [실행] → 통과 1·실패 1 트리, 실패를 두 번 눌러 15줄로 가는 것을 화면으로 확인했다.
  - 완료 조건 (2) SpotBugs SARIF가 문제 창에 뜬다: 9-2의 실제 SpotBugs 시험이 CI 3개 OS 전체 작업에서 통과했다.
  - 완료 조건 (3) log4j 2.14.1 pom의 경고와 오프라인 DB: 9-3의 실제 시험이 CI 3개 OS 전체 작업에서 통과했다(실행 41367369739). egov 웹 템플릿에서도 의존성(로컬 113개, CI 111개)을 풀어 취약 41건(egov RTE가 끌어온 Spring 6.2.11·log4j 2.25.3 등)을 egov 의존성 줄에 두고, 온라인과 받은 Maven DB의 결과가 같다(로컬, `templates.yml` 3개 OS 실행 41386606209).
  - CI 3개 OS 전체(실행 41386608513, `9076acc`) 통과.
  - 확인하지 못한 것: 화면으로 직접 본 것은 Windows뿐이다. Gradle 실제 실행은 돌리지 않았다(뒤에 백로그 "Gradle 실제 실행"(2026-11-05)에서 했다).

### Phase 10 — AI 에이전트 연동

**목표**: 외부 코딩 에이전트(Claude Code 등)가 편집기 상태를 보고, 편집기 안에서 에이전트와 대화한다. 두 개방 표준을 쓴다.

- MCP 서버(Model Context Protocol): 열린 파일·진단·디버거 상태를 에이전트에 도구로 내준다.
  - 결정(사용자): 이 PC의 HTTP(127.0.0.1, Streamable HTTP)로 듣고 Bearer 토큰을 확인한다. 기본은 꺼져 있다. 읽기만 한다(파일을 고치는 일은 에이전트가 자기 도구로 한다).
  - 에이전트에 넣을 설정(Claude Code 명령, JSON)을 창에서 복사한다.
- ACP 클라이언트(Agent Client Protocol): 에이전트 프로세스를 띄워 패널에서 대화하고, 제안한 편집을 diff로 받는다. 세부 결정은 10-2에서 받는다.

**완료 조건**: 실제 에이전트가 MCP로 열린 파일의 저장 안 한 내용·진단·디버거 멈춘 자리를 읽는다. ACP 에이전트와 패널에서 대화하고 편집을 diff로 받아 적용한다. 3개 OS.

**진행 단계**:
- [x] 10-1 MCP 서버(읽기 도구) — 중간
  - 결정(사용자): 이 PC의 HTTP + 토큰, 읽기만. 실제 에이전트(Claude Code)로 확인.
  - 서버(`ferrite-mcp`): 127.0.0.1의 포트(설정 `mcp_port`, 기본 47711)에서 Streamable HTTP(`POST /mcp`, JSON 응답)로 듣는다. HTTP는 표준 라이브러리로 요청 하나를 받고 닫는다(Content-Length·chunked).
    - `Authorization: Bearer <토큰>`을 글자마다 견주어 확인하고(틀리면 401), 다른 Origin(브라우저 페이지)에서 온 요청은 403으로 막는다(DNS 다시 묶기). 다른 경로 404, GET 405(서버가 먼저 보내는 흐름은 쓰지 않음), DELETE 204.
    - JSON-RPC: `initialize`(클라이언트가 고른 판이 2025-11-25·2025-06-18·2025-03-26이면 그대로, 아니면 가장 새 판. `Mcp-Session-Id`를 준다), `ping`, `tools/list`, `tools/call`. 알림은 202. 예전 판의 묶음 요청도 받는다. 모르는 메서드 -32601, 모르는 도구 -32602.
    - 도구 다섯(모두 `readOnlyHint`): `list_open_files`(열린 파일·언어·저장 안 한 변경·활성·줄 수, 프로젝트 루트), `read_open_file`(저장하지 않은 버퍼 내용, 경로 끝으로 찾기, 줄 범위), `get_active_editor`(경로·커서 줄·열·고른 글), `get_diagnostics`(언어 서버·정적 분석·의존성 취약점, 경로로 거르기), `get_debug_state`(구성·어댑터·상태·호출 스택·고른 프레임의 불러온 변수). 줄·열은 1부터.
  - 앱: `[도구 > AI 에이전트 연결(MCP)…]` 창에서 켜고 끄고, 토큰을 새로 만들고, Claude Code 명령(`claude mcp add --transport http …`)과 JSON 설정을 복사한다. 설정 `mcp_server = true`면 앱을 켤 때 함께 켠다.
    - 토큰은 설정 폴더의 `mcp-token`(무작위 32바이트, 유닉스는 나만 읽기)에 두어 앱을 다시 켜도 에이전트 설정이 맞는다.
    - 앱은 0.2초마다 상태를 스냅샷으로 넣고 서버 스레드는 그것만 읽는다(앱 창이 내려가 있어도 답한다). 문서 내용은 버퍼 버전이 바뀐 것만 다시 옮긴다. 진단은 열린 문서의 언어 서버·정적 분석·취약점과, 열지 않은 파일의 정적 분석·취약점.
  - 검증
    - 단위: Origin·토큰 견주기, 토큰 만들기, 경로 끝 찾기
    - 실제 HTTP(`ferrite-mcp` 시험): 토큰 없음·틀림 401, 다른 Origin 403, localhost Origin 허용, 판 협상, 세션 머리, 알림 202, 도구 다섯과 읽기 전용 표시, 저장 안 한 내용·줄 범위·경로 끝, 열리지 않은 파일은 `isError`, 진단 거르기, 디버거 상태, 모르는 도구·메서드, 404·405, 버리면 더 듣지 않음
    - 창 시험(kittest): MCP를 켜고 에디터에 친 글(저장 안 함)을 에이전트처럼 HTTP로 읽고, 저장 안 한 변경·언어·커서, 끄면 더 듣지 않음
    - 실제 앱(Windows, 릴리스 빌드, `mcp_server = true`): 127.0.0.1:47711에서 initialize 200(세션 머리)·알림 202·도구 다섯·열린 파일·활성 에디터·`read_open_file` 2줄(한글)을 받고 토큰 없이는 401. 이때 응답에 문자 집합이 없어 PowerShell이 한글을 잘못 읽는 것을 보고 `charset=utf-8`을 붙였다.
  - 실제 Claude Code로는 Phase 10 검증에서 확인했다(아래).
- [x] 10-2 ACP 클라이언트(대화 패널·diff) — 큼
  - 결정(사용자): 내장 에이전트(Gemini CLI `gemini --acp`, Claude Code `npx -y @agentclientprotocol/claude-agent-acp`, 예전 이름 `@zed-industries/claude-code-acp`는 더 고치지 않는다)와 settings.toml의 `[[agent]]`(name·command·args·env), 파일 쓰기는 diff를 보고 [받기]로 버퍼에 적용(저장은 사용자가), 터미널 기능은 주지 않는다, 확인은 가짜 에이전트와 Gemini CLI.
  - 클라이언트(`ferrite-acp`): 에이전트를 프로젝트 루트에서 띄워(PATH의 `.cmd`도 찾고 콘솔 창은 숨긴다) 표준 입출력으로 JSON-RPC를 줄마다 하나씩 주고받는다. `initialize`(판 1, `fs.readTextFile`·`fs.writeTextFile`, `terminal: false`) → `session/new`(cwd, MCP 서버) → `session/prompt`, 멈춤은 `session/cancel` 알림.
    - 받는 것: `session/update`(답 글·생각·사용자 글 조각, `tool_call`·`tool_call_update`(diff·글 내용), `plan`), 요청 `session/request_permission`·`fs/read_text_file`·`fs/write_text_file`. 모르는 요청(`terminal/*` 등)은 -32601.
    - `session/new`의 거절은 글에 인증 말이 있을 때만 로그인 안내를 붙인다. 에이전트가 끝나면 표준 오류의 끝 줄을 알린다.
    - Ferrite의 MCP 서버(10-1)가 켜져 있으면 세션에 넘긴다(HTTP MCP를 받는 에이전트만). 에이전트가 Ferrite의 열린 파일·진단·디버거 상태를 읽는다.
  - 에이전트 창(`[도구 > 에이전트]`, 도크 창): 에이전트 고르기·[시작]·[끝내기]·[지우기], 대화(나·답·접힌 생각·도구 호출의 상태와 diff 줄 수·계획), 글 칸(Ctrl+Enter로 보내기)·[멈춤].
    - 권한 요청: 에이전트가 준 선택지 단추와 [물리기]. 멈추면 남은 요청은 물린다.
    - 파일 읽기: 프로젝트 안 파일만, 열린 문서는 버퍼(저장 안 한 내용), 아니면 디스크(UTF-8·CP949). 줄과 줄 수를 따른다.
    - 파일 쓰기: 프로젝트 안 파일만. 지금 내용과의 diff(앞뒤 2줄, `ferrite-core` `git::diff_lines`)를 카드로 보이고 [받기]면 그 파일을 열어 버퍼를 바꾼다(실행 취소할 수 있고 저장은 사용자가). 새 파일은 그 내용으로 만들어 연다. [버리기]면 에이전트에 거절을 알린다. 그동안 에이전트는 기다린다.
    - 받은 쓰기는 디스크에 저장하기 전이라, 에이전트가 자기 셸로 돌리는 명령(빌드·시험)은 저장하기 전 내용을 본다.
  - 검증
    - 단위(가짜 에이전트, 같은 프로세스의 파이프): 시작 순서와 보내는 능력, HTTP MCP를 못 받는 에이전트에는 표준 입출력 서버만, 대화 조각·생각·도구 호출(diff·글)·고침 합치기·계획, 읽기·권한·쓰기 답하기와 거절, 모르는 메서드, 멈춤, 에이전트가 끝남, 인증 거절, 없는 프로그램
    - 창 시험(kittest): 가짜 에이전트와 대화해 읽기 요청에 저장 안 한 버퍼로 답하고, 권한 단추, diff 카드의 지운 줄·더한 줄, [받기]로 버퍼에 적용(저장 전, 디스크 그대로), 프로젝트 밖 쓰기 거절
    - 실제 Gemini CLI 0.42.0(`--acp`): 프로그램 찾기(`.cmd`)·띄우기·`initialize`까지 맞았으나, `session/new`에서 Google이 "This client is no longer supported for Gemini Code Assist for individuals"로 거절해(이 계정 종류의 서비스 정책) 대화는 확인하지 못했다. 이때 로그인 안내를 잘못 붙이는 것을 보고 고쳤다.
  - 실제 에이전트와의 대화·diff는 Phase 10 검증에서 Claude Code로 확인했다(아래).
- [x] 3개 OS 검증(2026-11-03)
  - 결정(사용자): 이 PC의 Claude Code에 로그인해 실제로 확인한다. 사용자 계정으로 MCP 한 번, ACP 몇 번 묻고, ACP 어댑터(npm `@agentclientprotocol/claude-agent-acp` 0.100.0, 풀면 약 390MB)를 npx로 받는다.
  - MCP(Claude Code 2.1.317, 릴리스 빌드, `mcp_server = true`): `claude -p`에 `--mcp-config`(HTTP·Bearer 토큰)로 Ferrite를 붙여 물으니 `list_open_files`·`get_active_editor`·`read_open_file`를 불러 열린 파일(Board.java, Java, 5줄, 저장 안 한 변경 없음)·커서(1줄 1열)·2번째 줄 내용을 맞게 답했다.
  - ACP(같은 Claude Code, 어댑터 0.100.0): `ferrite-acp`의 실제 시험(`tests/gemini.rs`의 `claude_code…`, 기본으로는 돌지 않음)으로 세션이 2초 만에 열리고, 파일을 고쳐 달라고 하면 읽기·편집 도구 호출 → diff 1개가 붙은 권한 요청(allow_once·allow_always·reject_once) → 한 번 허락 → 파일이 고쳐지고 end_turn. 에이전트를 닫으면 0.4초 안에 손자 프로세스(node·claude)까지 끝나 폴더가 풀린다.
    - 알게 된 것: 이 어댑터 판은 `fs/read_text_file`·`fs/write_text_file`를 부르지 않는다. Claude Code가 자기 도구로 디스크를 직접 읽고 쓰고, 편집 diff는 권한 요청에 붙여 보인다. 그래서 Claude Code에서는 diff 카드의 [받기] 대신 권한 카드의 diff를 보고 허락하면 에이전트가 디스크에 쓰고, Ferrite는 파일 감시로 열린 문서를 다시 읽는다(저장 안 한 변경이 있으면 충돌로 알린다). 에이전트가 저장 안 한 버퍼를 보려면 MCP(`read_open_file`)를 쓴다.
    - 알게 된 것: Claude Code의 권한 모드(사용자 설정 `permissions.defaultMode`)가 `auto`·`acceptEdits`면 묻지 않고 고친다. 이 PC의 설정이 `auto`라 처음 시험에서는 권한 요청 없이 고쳐졌다. 시험은 시험 폴더의 `.claude/settings.local.json`에 `default`를 두어 묻게 하고, 두 길(클라이언트에 쓰기를 맡김, 권한 뒤 직접 씀)을 다 받는다.
  - 내장 Claude Code 항목을 바뀐 npm 이름으로 고쳤다.
  - CI 3개 OS 전체(실행 41561435959, `ff7522c`) 통과.
  - 확인하지 못한 것: 실제 에이전트와는 `ferrite-acp`로 대화했고 앱의 에이전트 창으로는 하지 않았다(창은 가짜 에이전트 창 시험으로 확인). MCP의 진단·디버거 상태는 실제 HTTP 시험과 창 시험으로만 확인했다. 실제 에이전트 확인은 Windows뿐이다.

### 백로그 — Phase 미정

작거나 급하지 않아 Phase에 넣지 않은 것이다. 필요해지면 가까운 Phase의 조각으로 옮긴다.

| 기능 | 내용 | 크기 |
|---|---|---|

- [x] 단축키 설정·Eclipse 배치(2026-11-02) — 중간
  - 결정(사용자): 자주 쓰는 편집 명령(줄 지우기·줄 복제·줄로 가기·단축키 목록)도 새로 만든다, 바꾸기는 settings.toml(`keymap`·`[keys]`)과 읽기 전용 단축키 목록 창
  - 배치(`ferrite-app` `keymap`): 명령 35개마다 이름(`file.save`)·보일 이름·기본 배치(VS Code식)·Eclipse 배치의 키(여럿 가능)를 한 표에 둔다. 메뉴의 단축키 글도 이 배치에서 가져온다.
    - Eclipse 배치: 찾기·바꾸기 `Ctrl+F`, 다음·이전 찾기 `Ctrl+K`·`Ctrl+Shift+K`, 줄로 가기 `Ctrl+L`, 줄 지우기 `Ctrl+D`, 줄 복제 `Ctrl+Alt+↓/↑`, 파일에서 찾기 `Ctrl+H`, 파일로 이동 `Ctrl+Shift+R`, 기호로 이동 `Ctrl+Shift+T`, 개요 `Ctrl+O`, 정의로 이동 `F3`, 참조 찾기 `Ctrl+Shift+G`, 문서 포맷 `Ctrl+Shift+F`, 이름 바꾸기 `Alt+Shift+R`, 빠른 수정 `Ctrl+1`, 실행 `Ctrl+F11`, 디버그 `F11`·계속 `F8`, 들어가기 `F5`·한 줄씩 `F6`·나오기 `F7`, 멈춤 `Ctrl+F2`, 중단점 `Ctrl+Shift+B`, 단축키 목록 `Ctrl+Shift+L`, 터미널 `Ctrl+Alt+Shift+T`. Eclipse에 없는 폴더 열기·소스 제어·일시 정지는 메뉴로.
    - 새 명령의 기본 배치: 줄로 가기 `Ctrl+G`, 줄 지우기 `Ctrl+Shift+K`, 줄 복제 `Shift+Alt+↓/↑`, 단축키 목록 `Ctrl+Shift+L`. `[편집]`·`[보기]` 메뉴에도 넣었다.
    - 키 글: `Ctrl+Shift+F`, `Control+Space`, `Alt+Up`, `Ctrl+.`. `Ctrl`은 macOS에서 Cmd, `Control`은 어느 OS에서나 실제 Ctrl. 대소문자를 가리지 않는다.
    - 조합 키가 많은 것부터 맞춘다(egui는 덧붙은 Shift·Alt를 무시하고 맞추므로 `Shift+F5`를 `F5`보다 먼저).
  - 설정(`ferrite-core` `settings`): `keymap = "default" | "eclipse"`, `[keys]`에 명령 이름 → 키 하나·목록(빈 값은 키 없음). 모르는 명령·키는 상태바와 목록 창에 알리고 나머지는 적용한다.
  - 포커스: 줄 명령·줄로 가기는 에디터에 포커스가 있을 때만 받는다. 터미널에 포커스가 있으면 셸이 쓰는 Ctrl+글자(Shift·Alt 없이, Ctrl+D 입력 끝·Ctrl+L 화면 지우기·Ctrl+K·Ctrl+W 등)는 단축키로 가로채지 않는다(기본 배치에서도 바뀐 동작이다. macOS의 Cmd는 상관없다).
  - 단축키 목록 창(`[보기 > 단축키 목록]`): 배치, 명령마다 보일 이름·설정 이름·키, 같은 키를 쓰는 다른 명령(겹침), 설정 오류, 찾기, [설정 열기].
  - 검증
    - 단위: 키 글 읽기, 두 배치 모두 모든 명령을 읽고 키가 겹치지 않음, 설정으로 덮기·빼기·잘못된 것 알림·맞추는 순서·겹침, 터미널 제어 키, 줄 지우기·복제(마지막 줄·여러 줄·실행 취소), 설정 읽기
    - 창 시험(kittest): Eclipse 배치에서 에디터 포커스 없이는 `Ctrl+D`를 받지 않고, 포커스가 있으면 줄 지우기·`Ctrl+Alt+↓` 복제, `Ctrl+L` 줄로 가기(지금 줄을 채움, 범위 밖 오류), `Ctrl+Shift+L` 목록
    - 실제 앱(Windows, 릴리스 빌드, `keymap = "eclipse"`): 단축키 목록 창(배치: Eclipse), 에디터에서 `Ctrl+D` 줄 지우기·`Ctrl+Alt+↓` 줄 복제·`Ctrl+L` 줄로 가기 창, `Ctrl+Alt+Shift+T`로 연 터미널에서 `Ctrl+L`이 셸로 가 화면이 지워지는 것을 화면으로 확인했다. 확인 뒤 설정 폴더를 지웠다.
  - 확인하지 못한 것: macOS의 Cmd 배치는 시험으로만 확인했다.
- [x] EditorConfig(2026-11-02) — 작음
  - 결정(사용자): 규칙대로 기존 파일도 저장할 때 맞춘다(VS Code·Eclipse의 EditorConfig 확장과 같다). 예전 파일을 저장하면 diff가 커질 수 있다.
  - 읽기(`ferrite-core` `editorconfig`): 파일의 폴더부터 위로 `root = true`까지의 `.editorconfig`를 읽고, 위의 것부터 가까운 것으로, 한 파일 안에서는 뒤의 절이 덮는다. 이름·값은 대소문자를 가리지 않고 `unset`은 지운다.
    - 절의 글롭: `/`가 있으면 그 폴더 기준, 없으면 어느 깊이든. `*`·`**`·`?`·`[...]`·`[!...]`·`{a,b}`(globset, `ignore`가 이미 쓰는 판). 조각 앞뒤의 `**`(`**.min.js`)는 globset이 받는 꼴로 바꾼다. 숫자 범위 `{1..3}`과 조각 한가운데의 `**`는 다르게 맞춰질 수 있다.
    - 따르는 것: `indent_style`·`indent_size`(`tab`이면 `tab_width`)·`tab_width`(없으면 `indent_size`)·`end_of_line`(`lf`·`crlf`, `cr`은 저장 형식에 없어 따르지 않음)·`trim_trailing_whitespace`·`insert_final_newline`. `charset`은 따르지 않는다(CP949 감지·보존을 지금대로).
  - 적용: 열 때(다시 불러올 때도) 들여쓰기는 감지한 것보다 EditorConfig가 앞서고(칸 수만 적었으면 공백 들여쓰기의 칸 수만), 줄바꿈은 상태바에 EditorConfig 값으로 두어 저장할 때 바뀐다. 탭 문자 표시 폭도 따른다. 열기만 해서는 고치지 않는다(바뀐 것으로 치지 않음).
    - 저장할 때 끝 공백을 지우고 마지막 줄바꿈을 맞춘다(`false`면 끝의 줄바꿈을 지운다). 버퍼를 고치므로 화면과 디스크가 같고 실행 취소할 수 있다. 노트북(`.ipynb`)은 Jupyter가 쓴 모양을 지키려고 빼고, 저장할 때마다 다시 읽어 다른 이름으로 저장·`.editorconfig` 변경을 따른다.
    - `.editorconfig`를 Ferrite에서 저장하면 열린 파일에 다시 적용한다. 상태바 들여쓰기에 마우스를 올리면 적용한 `.editorconfig`가 보인다.
  - 검증
    - 단위: 가까운 파일·뒤의 절이 덮음과 `root`에서 멈춤, 대소문자, `{a,b}`·`/` 기준 글롭, `**.min.js`·`[!a]`, `tab_width`·`indent_size = tab`·`unset`·범위 밖·`cr`, 끝 공백 지우기·마지막 줄바꿈 넣기·빼기·빈 글
    - 창 시험(kittest): LF·공백 4칸·끝 공백·마지막 줄바꿈 없는 파일이 `.editorconfig`(CRLF·공백 2칸·끝 공백 지우기·마지막 줄바꿈)로 열려 들여쓰기 공백 2·CRLF, 열기만 해서는 고치지 않고, `Ctrl+S`로 CRLF·끝 공백 없음·마지막 줄바꿈으로 저장
    - 실제 앱(Windows, 릴리스 빌드): 같은 예제를 열어 상태바 "공백: 2"·CRLF, 디스크 파일은 그대로인 것을 화면으로 확인했다(저장은 창 시험으로).
- [x] TODO 목록·저장 시 작업(2026-11-02) — 작음 둘
  - 결정(사용자): 저장 시 작업은 실행 구성과 같은 `.ferrite/run.toml`의 `[[on_save]]`에 적고, 창 없이 돌려 결과를 상태바에 알린다.
  - TODO 목록(`[편집 > TODO 목록]`, 명령 `search.todo`, 기본 키 없음): 파일에서 찾기(3-1)를 `\b(TODO|FIXME)\b` 정규식(대소문자 구분, 파일 패턴 없이)으로 돌려 파일별로 보이고 누르면 그 줄로 간다. `.gitignore`를 따른다. 주석인지는 보지 않는다.
  - 저장 시 작업(`ferrite-core` `run::SaveTask`): `name`·`pattern`(루트 기준 글롭, `/`가 없으면 어느 폴더든)·`command`·`cwd`·`env`. 명령의 `${file}`·`${relativeFile}`·`${fileDirname}`·`${fileBasename}`·`${fileBasenameNoExtension}`·`${fileExtname}`·`${workspaceFolder}`를 저장한 파일로 바꾼다(VS Code 작업 변수와 같은 이름).
    - 저장하면 맞는 작업을 적은 차례로 숨겨 돌리고(앞의 작업이 고친 파일을 뒤의 작업이 본다) 상태바에 "저장 시 작업 중"→"완료: 이름들" 또는 "실패: 이름(종료 코드): 출력 끝 3줄". 편집 포커스를 가져가지 않는다. 포매터가 파일을 고치면 열린 문서는 파일 감시로 다시 읽힌다.
    - 셸 명령을 숨겨 돌리는 코드(디버그 전 빌드·의존성 풀기)를 한데 모아(`debug_ui::run_hidden`, 출력은 UTF-8·CP949를 가려 읽는다) 같이 쓴다.
  - 검증
    - 단위: `[[on_save]]` 읽기와 빈 값 오류, 패턴(루트 기준·어느 폴더든·프로젝트 밖), 변수 바꾸기, 작업 폴더
    - 창 시험(kittest): 설정으로 준 키로 TODO 목록 → 파일 2개에서 2개(`TODOS`·소문자 `todo`는 아님), `*.txt` 저장 → 작업이 파일을 만들고 "완료", `*.md` 저장 → `exit 3`이 "실패"
    - 실제 앱(Windows, 릴리스 빌드): `[편집 > TODO 목록]`으로 TODO·FIXME 2건, `*.css` 저장 시 작업이 로그 파일을 만들고 상태바에 "저장 시 작업 완료", 커서는 에디터에 남는 것을 화면으로 확인했다. 확인 뒤 설정 폴더를 지웠다.
- [x] 사용자 정의 DAP 어댑터·Handlebars 강조(2026-11-02) — 작음 둘
  - 사용자 어댑터(`.ferrite/debug.toml`의 `[[adapter]]`, `ferrite-core` `debug_config::CustomAdapter`): `name`·`command`·`args`·`address`·`env`. `[[debug]]`의 `adapter`에 그 이름을 쓰면 목록의 어댑터보다 먼저 고른다.
    - `command`만: 표준 입출력으로 말한다. `address`만(`IP:포트`): 이미 듣는 어댑터에 붙는다. 둘 다: 띄운 뒤 그 주소가 열릴 때까지(30초, 어댑터가 살아 있는 동안) 다시 붙어 본다. 어댑터가 먼저 끝나면 바로 실패를 알린다(출력 끝 몇 줄과 함께).
    - `command`는 경로면 프로젝트 루트 기준, 이름이면 PATH에서 찾는다. `command`·`args`의 `${workspaceFolder}`는 루트로 바꾼다. 작업 폴더는 루트.
  - Handlebars 강조: 언어 확장이 `.hbs`·`.handlebars`를 맡지 않으면 내장 TextMate 문법(이 판에서 쓴 것)으로 칠한다. `{{ }}` 구분자·주석(`{{!-- --}}`)·블록(`#if`·`#each`·`/if`·`else`)·도우미(여는 괄호 바로 뒤의 이름)·문자열·수·`@index`·변수와 HTML·XML 태그. 확장 수에는 세지 않는다.
  - 검증
    - 단위: `[[adapter]]` 읽기와 오류(command·address 없음, 주소 형식), 어댑터 고르기(표준 입출력·붙기·띄우고 붙기, 경로·PATH, 없는 이름은 목록으로), Handlebars 문법(주석·태그·블록·도우미와 인자·문자열·수·변수)
    - 가짜 어댑터(ferrite-dap): 띄운 어댑터가 정해 둔 주소를 늦게 열어도 붙고, 주소를 열지 않고 끝나면 기다리지 않고 실패
    - 실제 앱(Windows, 릴리스 빌드): 코드 생성 템플릿(`jsp_list.hbs`)이 Handlebars로 열려 `{{ }}`·`{{#if}}`·`{{#each}}`·변수·HTML 태그가 칠해지는 것을 화면으로 확인했다.
  - 확인하지 못한 것: 실제 제조사 어댑터로는 돌려 보지 않았다(가짜 어댑터와 시험으로 연결 방식 셋을 확인).
- [x] 에이전트 권한 모드·고르기 설정(2026-11-03) — 작음
  - 까닭: Phase 10 검증에서 Claude Code는 사용자 설정의 권한 모드(`auto`·`acceptEdits`)면 묻지 않고 디스크에 바로 고쳤다. Ferrite에서 에이전트의 모드를 보고 바꾼다.
  - 결정(사용자): settings.toml의 `agent_mode` 하나로 처음 모드를 정하고 창에서 언제든 바꾼다. 창에는 에이전트가 주는 고르기 설정(모드·모델·생각 깊이 등)을 모두 보인다.
  - 클라이언트(`ferrite-acp`): `session/new`의 `configOptions`(고르기만, 묶음은 펼친다. 불린은 받는다고 알리지 않았다)를 읽고 `session/set_config_option`으로 바꾼다. 응답과 `config_option_update` 알림은 설정 전체로 바꾼다. 둘 다 주면 `configOptions`를 쓰고(규격), 예전 판의 `modes`만 주는 에이전트는 "모드" 설정 하나로 보여 `session/set_mode`로 바꾸고 `current_mode_update`를 따른다. 거절되면 "설정을 바꾸지 못했습니다"를 알리고 그대로 둔다.
  - 창: 세션이 열리면 대화 위에 설정마다 이름과 고르기 칸(값의 설명은 마우스를 올리면)을 에이전트가 준 차례대로 보인다.
    - `agent_mode`(기본 빈 글이면 에이전트 설정 그대로): 세션이 열리면 category가 `mode`인 설정을 그 값으로 바꾼다. 그 모드가 없으면 있는 값들을 알리고 그대로 둔다. Claude Code·Gemini CLI(0.42에서 확인)는 `default`가 고치기 전에 묻는 모드다.
    - 권한 카드가 고칠 파일 이름만 보이던 것을 diff 줄(앞뒤 2줄)까지 보이게 했다. 디스크에 직접 쓰는 에이전트(Claude Code)는 여기서만 고칠 내용을 본다.
  - 검증
    - 가짜 에이전트(파이프): 설정 읽기(불린 빼기·묶음 펼치기·설명), 바꾸기 요청과 응답, 에이전트가 보낸 바뀜, 거절, 두 꼴을 다 주면 새 판, 예전 판 모드의 `set_mode`와 `current_mode_update`
    - 창 시험(kittest): `agent_mode = "default"`로 세션을 열면 모드를 바꾸는 요청을 보내고, 고르기 칸으로 모델을 바꾸며, 권한 카드에 diff 줄이 보인다
    - 실제 Claude Code(2.1.317, 어댑터 0.100.0, `ferrite-acp` 실제 시험 `claude_code의…`, 기본으로는 돌지 않음, 대화 없음): 세션을 열면 `configOptions`로 Mode(category `mode`: default·acceptEdits·plan·auto·bypassPermissions)·Model·Effort(`thought_level`)·Fast mode·Ultracode(`model_config`, on·off)·Agent(category 없음, 사용자의 서브에이전트)를 준다. 이 PC 설정대로 `auto`이던 모드를 `default`로 바꾸는 데 2.4초. 받은 직후 첫 실행에서는 닫은 뒤 15초 넘게 프로세스가 남았고, 이후 세 번은 2초 안에 끝났다.
  - 확인하지 못한 것: 실제 에이전트와는 `ferrite-acp`로 확인했고 앱의 에이전트 창으로는 보지 않았다(창은 가짜 에이전트 창 시험으로 확인).

- [x] 디스크 바뀜 알림·egov 서버 다시 색인(2026-11-03) — 중간
  - 까닭: 에이전트(Claude Code는 디스크에 직접 쓴다)·git·다른 도구가 쓴 파일을 언어 서버가 몰랐다. egov 서버는 시작할 때와 열고 고친 것만 색인해 엔드포인트·매퍼 진단이 낡았다.
  - 결정(사용자): LSP 표준(`workspace/didChangeWatchedFiles`, 동적 등록)으로 모든 서버에 알린다.
  - 클라이언트(`ferrite-lsp` `watch`): 능력에 `didChangeWatchedFiles`(dynamicRegistration·relativePatternSupport)를 알리고, 서버가 `client/registerCapability`·`unregisterCapability`로 보낸 감시 패턴을 서버마다 기억한다. 글 패턴은 전체 경로에, 상대 패턴(`baseUri`, 작업 폴더도)은 그 폴더 아래 경로에 맞추고(`*`는 경로 구분자를 넘지 않는다, Windows는 대소문자 무시) 종류(만들기·바꾸기·지우기)도 본다. 맞는 바뀜만 보낸다.
  - 앱: 프로젝트 전체 감시(`watcher::ProjectWatcher`). Windows·macOS는 재귀 감시 하나, Linux는 inotify 감시 수 한도 때문에 폴더마다(새로 생긴 폴더도 더한다). `.git`·`.gradle`·`bin`·`build`·`node_modules`·`out`·`target`은 뺀다. 프레임마다 모은 바뀜(같은 경로는 한 번, 종류는 지금 디스크 상태로)을 떠 있는 서버 모두에 넘긴다.
  - egov 서버: 클라이언트가 받으면 `**/*.java`·`**/*.xml` 감시를 등록하고, 바뀐 파일을 다시 색인한 뒤 열린 문서를 다시 진단한다. 열린 문서(편집 중인 내용이 기준)와 빌드 결과물 폴더(같은 매퍼의 사본)는 건너뛴다. 웹 구조 파일(.properties·.tld·web.xml)과 폴더째 옮기기는 시작할 때만 읽는다.
  - 고친 것(10-1 MCP): 스냅샷은 0.2초 간격으로만 넣는데, 간격 안에 생긴 바뀜은 창이 가만히 있으면 끝내 들어가지 않아 에이전트가 낡은 진단을 읽었다. 남은 시간 뒤에 한 번 더 그려 넣는다. 아래 실제 앱 확인에서 찾았다.
  - 검증
    - 단위: 감시 등록(글·상대 패턴·작업 폴더 기준·종류·등록 풀기·다른 등록 무시), 프로젝트 감시(바뀐 파일·빌드 결과물 빼기·새 폴더 안의 파일·지움)
    - egov 서버 LSP 시험(실제 통신): 디스크에 쓴 컨트롤러가 엔드포인트에 들어오고 지우면 빠지며 `target` 아래 사본은 색인하지 않는다. 닫은 매퍼 XML을 디스크에서 고치면 Mapper 인터페이스의 "XML 문장 없음" 경고가 사라진다.
    - 실제 앱(Windows, 릴리스 빌드, MCP로 진단 읽기): 매퍼 인터페이스를 열어 둔 채 하위 폴더의 매퍼 XML에 디스크에서 문장을 더하면 0.3초 안에 경고가 사라지고, 빼면 다시 뜬다(MCP 고침 전에는 경고가 남은 채로 읽혔다).
  - 확인하지 못한 것: jdt.ls 등 다른 서버가 실제로 무엇을 등록하고 따라잡는지는 보지 않았다(이 PC에 설치된 서버가 없다). 앱 시험 전체를 되풀이해 돌리면 드물게 SQL 콘솔 시험이 멈추거나 MCP 창 시험의 연결이 끊겼다(14번 중 3번, main 코드로도 6번 중 1번 멈춤). 멈춤의 원인은 아래 "드물게 끝나지 않던 앱 시험"에서 찾았다.
- [x] 드물게 끝나지 않던 앱 시험(2026-11-03) — 작음
  - 까닭: `ferrite-app` 시험을 되풀이하면 드물게 시험이 끝나지 않았다(main 코드로도). 시험 안의 60초 제한도 걸리지 않았다.
  - 찾기: 이 PC에 디버거가 없어, 마지막 단계 몇 개를 메모리에 두고 20초 동안 진행이 없으면 파일에 쓰는 임시 기록으로 좁혔다(파일에 바로 쓰는 기록은 타이밍을 바꿔 190번 동안 재현되지 않았다). 시험 실행 파일을 네 개씩 나란히 돌려 수백 번에 한 번 재현했다.
    - `매퍼_sql의_표를…` 시험이 끝에서 시험 폴더를 지울 때, 파일 둘(`EmpMapper.xml`·`콘솔.db`)을 지운 뒤 폴더의 다음 항목 읽기(`FindNextFileW`)가 돌아오지 않았다. 스레드는 CPU를 쓰지 않고 커널 I/O 대기였다.
    - 앱을 버린 바로 뒤라 파일 감시(notify)가 그 폴더의 감시 핸들을 닫고 SQLite 스레드가 DB 파일을 닫는 중이었다. 앱 코드의 교착이 아니라 Windows 파일 시스템 쪽 정지로 보인다.
  - 고침(시험만): `remove_project`가 다른 스레드에서 지우고 10초만 기다린 뒤, 못 지우면 폴더를 남기고 시험을 끝낸다. 앱을 띄운 시험 끝의 폴더 지우기 9곳과 프로젝트 감시 시험(`watcher.rs`)도 이것으로 바꿨다. 없는 폴더는 지운 것으로 친다(전에는 NotFound를 5초 동안 다시 시도해 "새 egov 프로젝트 창" 시험이 5초 늦었다).
  - 검증: 고친 뒤 그 시험을 네 개씩 나란히 800번 돌려 실패·멈춤 0번(이번에는 정지 자체가 일어나지 않아 폴더를 남긴 일도 0번), 앱 시험 전체 3번 통과
  - 확인하지 못한 것: 커널 쪽의 정확한 원인(스택을 못 봤다). 실제 앱의 트리 삭제는 휴지통으로 옮기기(이름 바꾸기)라 같은 상황은 아니지만 확인하지는 않았다. 앞서 본 MCP 창 시험의 연결 끊김(ConnectionReset)은 그 뒤 재현되지 않아 원인을 못 찾았다.
- [x] 곧바로 끝난 명령의 마지막 출력이 터미널에서 사라짐(2026-11-04) — 작음
  - 까닭: `실행_구성을_골라_터미널_탭에서_돌리고_다시_실행하면_그_탭을_바꾼다`가 Ubuntu 전체 CI(실행 41774699255 첫 시도, `full (ubuntu-latest)`)에서 첫 실행을 60초 기다리다 실패했다. 셸은 코드 0으로 끝났는데(`echo run-$FERRITE_RUN`) 화면이 비어 있었다. 그 작업만 다시 돌리니 통과했다.
  - 찾기(alacritty_terminal 0.26.0, 최신 판)
    - PTY 읽기 스레드(`EventLoop::pty_read`)는 읽은 바이트를 화면에 넣으려고 화면 잠금을 `try_lock`한다. UI가 잠금을 쥐고 있으면(매 프레임 `snapshot()` 등) 바이트를 든 채 다시 읽는다.
    - Linux에서는 셸이 끝나 PTY의 slave가 모두 닫히면 그 읽기가 EIO로 실패한다. 그러면 들고 있던 바이트를 파싱하지 않고 오류로 돌아가 마지막 출력이 사라진다. macOS는 그때 0바이트를 읽어 잃지 않고, Windows(ConPTY)는 다른 길이다.
    - 결정적으로 재현하는 시험을 만들었다. 화면을 잠그고, 셸이 `run-ok`를 찍고 끝날 때까지(`/proc`에서 좀비가 될 때까지) 기다린 뒤 잠금을 푼다. 고치기 전 Ubuntu CI에서 20초 동안 `run-ok`가 오지 않아 실패했고(임시 탐색 실행 41780294468), 고친 뒤 통과했다(41780407938).
  - 고침(`ferrite-term`): Unix에서 PTY를 직접 열고(`rustix_openpty::openpty`, alacritty의 `tty::new`와 같다) slave 복사본(CLOEXEC)을 `Terminal`이 살아 있는 동안 쥐어 둔다. 그러면 셸이 끝나도 EIO 대신 WouldBlock이 와서, 읽기 스레드는 잠금을 잡을 때까지 기다렸다가 파싱한다. 셸이 끝난 것은 전처럼 SIGCHLD로 안다. alacritty에는 공개된 `tty::from_fd`로 넘긴다.
  - 검증: 위 시험(Linux)이 고치기 전 실패, 고친 뒤 통과. 기존 `ferrite-term` 시험(종료 코드 받기, 실행 구성 명령)도 Ubuntu에서 통과. 로컬(Windows) `ferrite-term` 시험, 앱의 실행 구성 시험, clippy 통과. 전체 CI(3개 OS, 실행 41780626358) 첫 시도에 통과.
  - 남은 것: alacritty_terminal에 알릴 만하다(`pty_read`가 오류로 돌아가기 전에 든 바이트를 파싱하면 된다).
- [x] TS 7 jQuery 자동완성 시험의 흔들림(2026-11-04) — 작음
  - 까닭: `npm에서_받은_typescript_7은_jsconfig가_있으면_화면_폴더의_전역과_jquery를_안다`가 Windows CI(`full (windows-latest)`의 npm 단계)에서 가끔 실패했다(실행 41577258429·41752144558). 화면 폴더의 전역 함수는 나오고 jQuery(`$.ajax`)만 180초 내내 나오지 않았다. 다른 OS와 다시 돌린 실행은 통과했다.
  - 찾기: 실패 로그만으로는 원인이 보이지 않아, 시험이 서버의 자동 타입 받기(ATA) 기록(`window/logMessage`)을 모아 실패 때 출력하게 하고, Windows에서 그 시험만 6개 나란히 도는 임시 워크플로로 같은 실패를 한 번 잡았다.
    - TS 7 서버가 같은 프로젝트에 ATA 요청 둘(#1·#2)을 같은 순간에 내고, 같은 타입 캐시 폴더에서 `npm install @types/jquery`를 겹쳐 돌렸다. #1이 끝나자 프로그램을 다시 만들었는데, 그때 #2(0.5초 뒤 끝남)는 아직 jQuery 타입을 쓰는 중이었다.
    - TS 7 서버는 바뀜 알림이 오기 전에는 그 파일을 다시 읽지 않는다. 실제 TS 7로 확인했다: 타입 파일을 비운 채 띄우면 `ajax`가 없고, 파일을 되돌려도 알리지 않으면 계속 없고, `workspace/didChangeWatchedFiles`로 알리면 바로 나온다.
    - 처음 의심한 것(함께 도는 다른 시험의 TS 서버가 끝난 뒤 남은 npm과 겹침)은 로컬과 CI에서 일부러 겹쳐도 재현되지 않았다.
  - 고침(시험만): 첫 세션이 60초 안에 jQuery를 모르면 서버를 다시 띄워 본다. 받은 타입은 디스크에 온전히 남고 다음 세션은 npm 없이 그것을 읽는다. 실패하면 ATA 기록을 함께 출력한다.
  - 검증
    - 고치기 전: Windows에서 그 시험만 6개 나란히 돌려 1번 실패(CI와 같은 `[true, false]`, 위 ATA 기록).
    - 고친 뒤: 같은 방식으로 18번(6개 × 3번) 모두 통과. 그중 3번은 첫 세션에서 같은 겹침이 일어나 jQuery를 몰랐고, 다시 띄운 세션이 4~5초 만에 찾았다.
    - 전체 CI(3개 OS, 수동 실행) 2번 통과(41759123599·41762132854). 두 번째의 Windows 작업에서도 첫 세션이 jQuery를 몰랐고 다시 띄운 세션으로 통과했다(고치기 전이면 실패였다).
  - 남은 것: 앱에서도 생길 수 있었다(아래 "프로젝트 밖 감시 폴더"에서 고쳤다).
- [x] 프로젝트 밖 감시 폴더(2026-11-04) — 작음
  - 까닭: 위 jQuery 시험의 원인이 앱에도 있었다. "jsconfig.json 만들기"는 JS 서버를 다시 띄우므로, 그 PC에서 jQuery 타입을 처음 받을 때 TS 서버의 npm 둘이 겹치면 서버를 다시 띄울 때까지 jQuery 완성이 없었다. TS 서버는 프로젝트 밖 타입 캐시를 감시 등록하고 그 바뀜을 알려 주길 기다리는데, 앱은 프로젝트 폴더만 감시했다.
  - TS 7 서버가 등록하는 것(실제로 찍어 봄, 프로젝트 `D:\works\…\project`): 타입 캐시 `…\AppData\Local\Microsoft\TypeScript\7.0`의 `node_modules`, 프로젝트 자신(소문자 경로), 프로젝트의 조상 `D:\works`(위로 package.json 찾기), 서버 설치 폴더의 `lib`. 프로젝트가 임시 폴더(`AppData\Local\Temp`) 아래면 프로젝트 밖 파일들의 공통 부모로 `C:\users\<이름>\appdata` 전체(파일 82만 개)가 등록되기도 했다.
  - 클라이언트(`ferrite-lsp` `watch`): 등록 패턴마다 맞을 수 있는 가장 좁은 폴더(기준 폴더 + 패턴 앞의 글자 그대로인 조각, `C:/…/7.0/**/*` → `C:\…\7.0`)를 등록 때 계산해 `LanguageServer::watched_dirs`로 내준다. 상대 글 패턴(`**/*.java`)과 글롭 없는 정확한 파일 경로(서버마다 수천 개일 수 있다)는 넣지 않는다.
  - 앱(`watcher::ProjectWatcher::watch_outside`): 디스크 바뀜을 넘길 때마다 서버들의 폴더를 모아 프로젝트 밖 것을 재귀로 감시한다(바뀐 것만 다시 건다). 넓은 폴더는 뺀다: 프로젝트 안(이미 본다), 프로젝트를 품은 조상, 홈(홈 밖이면 드라이브)에서 3단계보다 얕은 폴더(`AppData` 같은 것). 다른 것 아래의 폴더는 한 번만 감시한다. Windows는 대소문자를 가리지 않고 비교한다(TS 서버는 소문자 경로도 보낸다). 프로젝트 안처럼 `node_modules` 등을 빼지 않는다(타입 캐시가 그 아래에 있다).
  - 검증
    - 단위: 패턴별 가장 좁은 폴더(절대 글 패턴·상대 패턴·같은 폴더 한 번, 상대 글 패턴과 정확한 경로는 없음), 프로젝트 밖 캐시의 만들기·바꾸기를 보고 조상·프로젝트 안·얕은 폴더·덮인 하위 폴더는 빼며 등록이 사라지면 감시를 푼다
    - 실제 TS 7(시험 코드로): 타입 캐시의 jQuery 타입 파일을 비운 채 띄우면 `$.ajax` 완성이 없고, 파일을 되돌려도 알리지 않으면 계속 없고, 알리면 나온다.
    - 실제 앱(Windows, 릴리스 빌드, MCP로 진단 읽기): `// @ts-check`와 `$.ajaxx()`를 쓴 board.js를 열면 "Property 'ajaxx' does not exist on type 'JQueryStatic'" 오류가 난다. 타입 캐시의 `JQueryStatic.d.ts`에 `ajaxx` 선언을 더하면 1.2초 만에 오류가 사라지고, 되돌리면 1.2초 만에 다시 뜬다. 고치기 전(main) 앱은 선언을 더하고 10초가 지나도 오류가 그대로였다.
    - 뒤에 macOS CI(실행 41801828819)에서 시험이 한 번 실패했다. FSEvents가 감시를 걸기 직전에 만든 `project`·`project/src`의 만듦을 늦게 알렸는데, 시험은 캐시 폴더의 바뀜만 와야 한다고 보았다. 넓은 폴더(`base`)의 바뀜이 오지 않는지만 보도록, 프로젝트 안의 바뀜은 받게 고쳤다.
  - 확인하지 못한 것: TS 서버의 npm이 실제로 겹친 순간을 앱에서 재현하지는 않았다(드물다). 같은 결과(파일이 다 쓰인 뒤의 바뀜 알림)를 위처럼 파일을 바꿔 확인했다. macOS·Linux의 실제 앱은 보지 않았다(단위 시험은 3개 OS CI).
- [x] MCP 시험이 끈 뒤에도 옛 주소로 연결되던 흔들림(2026-11-04) — 작음
  - 까닭: `mcp로_에이전트가_열린_파일의_저장_안_한_내용과_활성_에디터와_디버거_상태를_읽는다`가 macOS CI(실행 41767071753의 첫 시도, `full (macos-latest)`)에서 실패했다. `mcp.stop()` 뒤 옛 주소로 `connect_timeout(500ms)`가 성공했다. 시험이 생긴 11-02 뒤로 이 한 번이고, 다시 돌린 시도는 통과했다.
  - 찾기: 서버는 포트 0에 바인딩하고, `Drop`은 멈춤 표시를 세운 뒤 자기에게 연결해 받기 스레드를 깨우고 join한다. 리스너는 그 스레드가 가지므로 join이 끝나면 이 프로세스에서는 닫혀 있다. 그래서 "듣고 → 닫고 → 옛 주소로 연결"을 되풀이하는 임시 탐색(Rust std만, macOS·Ubuntu 러너)으로 경우를 나눠 쟀다.
    - 처음 의심한 둘은 아니었다: (1) 다른 시험이 같은 임시 포트를 곧바로 받음, (2) TCP self-connect. macOS 러너는 `net.inet.tcp.randomize_ports: 0`이라 임시 포트를 차례대로 준다(49164, 49165, …). 조용할 때와 다른 스레드가 리스너를 열고 닫는 중에 각 8만 7천 번 돌려 0번 연결됐다(Ubuntu는 80만·58만 번 중 0·1번).
    - 다른 스레드가 자식 프로세스를 띄우는 중일 때만 연결됐다. 시험처럼 네 번 답한 뒤 닫으면 macOS 618번 중 292번, Ubuntu 2,678번 중 130번이었다. 자식을 띄우면 fork·posix_spawn부터 exec까지 자식이 부모의 fd를 모두(CLOEXEC 것도) 복사해 쥔다. 그 사이에 부모가 리스너를 닫아도 소켓은 커널에서 LISTEN으로 남아 연결을 받는다.
    - exec 전에 50ms 머무는 자식을 띄워 그 창을 넓혀 확인했다. 닫은 직후 연결된 경우(macOS 19번, Ubuntu 73번) 모두 lsof에서 그 포트의 LISTEN을 다른 pid(exec 전 자식)가 쥐고 있었다. 그 연결로 요청을 보내면 응답 없이 끊겼다(ConnectionReset·EOF). 100ms 뒤 다시 연결하면 0번 연결됐다.
    - 앱 시험은 한 프로세스에서 나란히 돌고, 여러 시험이 자식 프로세스(git, 저장 시 셸 작업 등)를 띄운다. macOS는 이 창이 Linux보다 길어(위 비율) macOS에서 먼저 드러났다. Windows는 fork가 없고 소켓 핸들을 물려주지 않는다.
    - 더 드문 경우(macOS만): Rust std는 macOS에서 `socket()` 뒤에 CLOEXEC를 따로 건다(원자적이지 않다). 그 사이에 띄운 자식은 exec 뒤에도 리스너를 쥔다. 탐색 5만 4천 번 중 2번 lsof에서 `sleep`이 쥐고 있었다(Ubuntu는 `SOCK_CLOEXEC`라 0번).
  - 고침(시험만): 서버 결함이 아니다. 서버는 리스너를 닫고 join하며, 다른 스레드가 띄우는 자식의 fd 복사는 서버가 막을 수 없다. 시험은 TCP 연결 여부 대신 둘을 본다. 하나는 서버가 없어진 것(`address()`가 None)이다. 다른 하나는 켜 있을 때 200으로 답하던 `ping`에 끈 뒤 옛 주소에서 200으로 답하는 것이 없다는 것이다. 잠깐 남은 리스너로 들어간 연결은 아무도 받아 가지 않아 응답 없이 끊긴다(읽기는 5초 제한).
  - 검증: 로컬(Windows) 그 시험과 clippy 통과. 전체 CI(3개 OS, 실행 41774699255) 통과했고, MCP 시험은 3개 OS 모두 첫 시도에 통과했다. 첫 시도의 `full (ubuntu-latest)`는 관계없는 터미널 시험(`실행_구성을_골라_터미널_탭에서_돌리고_다시_실행하면_그_탭을_바꾼다`, 셸이 곧바로 0으로 끝나 화면이 빔)으로 실패해, 그 작업만 다시 돌려 통과했다.
  - 확인하지 못한 것: 실패한 실행에서 그 순간 어느 시험이 자식을 띄웠는지는 로그로 알 수 없다. 앱에서는 끈 직후 잠깐(macOS 1ms쯤, 위 비율과 자식 띄우는 빠르기로 어림) 연결이 되었다가 응답 없이 끊길 수 있다. 위의 더 드문 경우에는 그 자식이 끝날 때까지 포트가 남아, 같은 고정 포트로 다시 켜면 실패할 수 있다. 고치려면 자식을 띄우는 쪽에서 fd를 닫아야 해서 넣지 않았다.

- [x] JSON 스키마 검사(2026-11-04) — 작음~중간
  - 결정(사용자): 내장 스키마와 SchemaStore 목록(온라인, 망이 막히면 내장만), settings.toml의 `[[json_schema]]`로 사용자 연결, 실제 서버(npm, 풀면 약 85MB)로 확인.
  - 내장(`ferrite-lsp` `schemas/`, SchemaStore Apache-2.0, `NOTICE.md`): package.json·tsconfig*.json·jsconfig.json·.eslintrc(.json)·.prettierrc(.json)·.babelrc(.json)·babel.config.json. 합쳐 548KB. 특정 편집기 전용 확장 키(`x-` 일부)는 뺐다(검사 규칙은 그대로).
    - 설정의 `json.schemas`에 내용째 넣고 주소는 SchemaStore 것(`https://www.schemastore.org/<이름>.json`)을 써서, 다른 스키마가 그 주소를 가리켜도(package.json → prettierrc·eslintrc) 망 없이 푼다.
  - SchemaStore 목록(약 1,400종, 파일 패턴 약 3,000개): 앱을 켠 뒤 JSON 서버가 처음 준비되면 한 번 받아(0.5MB) `json/schemaAssociations`로 알린다. 스키마 내용은 서버가 https로 직접 받는다.
    - 연결은 객체 꼴(`{ 패턴: [주소] }`)로 보낸다. 배열 꼴은 서버의 JSON-RPC가 배열 `params`를 인자로 펼쳐 첫 항목만 받아 아무 파일에도 맞지 않았다(실제 서버로 확인).
    - 못 받으면 상태바에 알리고 내장만 쓴다. 이때는 내장 스키마에서 바깥 스키마를 가리키는 `$ref`를 빼고 다시 보낸다: 서버(4.10.0)는 받지 못한 참조가 하나라도 있으면 그 스키마로 검사하지 않고 "Unable to load schema"만 알리는데, package.json 스키마도 stylelintrc 등 바깥 스키마 9개를 가리킨다. 빼면 그 안쪽(다른 도구 설정)만 검사하지 않는다. `json.validate.schemaRequest`로 경고를 끄는 길은 이 판의 서버가 그 설정을 읽지 않아 쓰지 않았다.
  - 사용자 연결(`[[json_schema]]` `files`·`url`): 패턴은 프로젝트 기준(`folderUri`), `url`은 주소(`https://`·`file://`)나 프로젝트 기준 경로. 바뀌면 떠 있는 JSON 서버에 설정을 다시 보낸다.
  - 설정은 JSON 서버가 준비되면 `workspace/didChangeConfiguration`으로 보낸다. 서버는 설정에 `json.validate.enable`이 없으면 검사를 끄므로 늘 켠다(지금까지는 설정을 보내지 않아 서버의 처음 기본값으로 문법만 검사했다).
  - 검증
    - 단위: 내장·사용자 설정(내용째·주소·프로젝트 기준 경로와 패턴, 빈 주소 빼기), 망이 막혔을 때 바깥 참조 빼기(상대 주소 포함, 같은 스키마 안·내장끼리는 둠), 목록을 패턴별 연결로(내장 빼기, 같은 패턴 여러 주소), `[[json_schema]]` 읽기
    - 실제 서버(`download.rs`의 웹 서버 묶음 시험, `servers.yml`): 내장(package.json `"name": 1` → Expected "string"), 사용자 연결(config/app.json → Expected "integer"), SchemaStore 목록(renovate.json `"extends": 1` → Expected "array")
    - 실제 앱(Windows, 릴리스 빌드, MCP로 진단 읽기): 같은 세 경우가 0.5~1초에 뜬다. 닿지 않는 프록시로 망을 막으면 package.json은 내장으로 검사하고(목록 받기 실패 뒤 다시 보낸 설정으로) renovate.json은 진단이 없다.
  - 확인하지 못한 것: 화면으로 직접 본 것은 아니다(MCP로 읽음). 프록시 뒤에서 서버가 스키마를 받는 것은 보지 않았다. 서버도 Ferrite처럼 `HTTPS_PROXY` 환경 변수를 읽는다(아래 "JSON 스키마 받기의 인증서 검증"에서 확인).

- [x] JSON 스키마 받기의 인증서 검증(2026-11-04) — 작음
  - 까닭: "JSON 서버에 프록시 넘기기"를 하려다 서버 코드를 보니, 프록시는 이미 되고 인증서 검증이 꺼져 있었다.
    - 프록시: 서버(request-light)는 `http.proxy` 설정이 없으면 `HTTPS_PROXY`·`https_proxy`·`HTTP_PROXY`·`http_proxy` 환경 변수를 읽는다. 서버는 Ferrite의 환경을 물려받으므로 Ferrite와 같은 프록시를 쓴다. `NO_PROXY`는 읽지 않는다.
    - 검증: 서버는 설정을 받을 때 `!!http.proxyStrictSSL`로 검증 여부를 정한다. Ferrite가 `http`를 보내지 않아 거짓이 되어, SchemaStore 스키마를 TLS 인증서 검증 없이 받고 있었다(4.10.0 코드로 확인).
  - 결정(사용자): 검증을 켜고, Node로 도는 언어 서버가 OS 인증서 저장소도 믿게 한다. Ferrite 자신의 다운로드(OS 검증기)와 같은 기준이다.
  - 고침
    - JSON 설정에 `http.proxyStrictSSL = true`를 넣는다.
    - 언어 서버를 띄울 때 `NODE_USE_SYSTEM_CA=1`을 넘긴다. 사용자가 이미 정했으면 그대로 둔다. Node 22.19·24.6부터 읽는다. 그 전 판과 Node가 아닌 서버는 무시한다. TS 서버가 띄우는 타입 받기 npm에도 물려간다.
    - 회사의 TLS 검사 프록시 뒤에서 Node가 그 전 판이면 SchemaStore 스키마만 받지 못한다(진단에 까닭과 `--use-system-ca` 안내가 뜬다). 내장 스키마는 그대로 된다.
  - 검증
    - 단위: 설정에 `http.proxyStrictSSL`, 서버 명령에 `NODE_USE_SYSTEM_CA`(이미 있으면 두기)
    - 실제 서버(vscode-langservers-extracted 4.10.0, Node 26, 이 PC): 일회용 자체 서명 인증서(openssl)의 로컬 HTTPS 서버가 주는 스키마로 `{"port": "80"}`을 검사했다.
      - 예전 설정(`http` 없음): 스키마를 받아 Expected "integer"(검증 없이 받는다)
      - 새 설정 + `NODE_USE_SYSTEM_CA`: "Unable to load schema … self-signed certificate"
      - 새 설정 + 그 인증서를 믿게(`NODE_EXTRA_CA_CERTS`): Expected "integer"
      - 새 설정 + `NODE_USE_SYSTEM_CA`로 SchemaStore의 renovate 스키마: Expected "array"
    - 실제 서버 시험(`download.rs`, `servers.yml`, 3개 OS)이 같은 설정과 서버 띄우기로 SchemaStore 스키마를 받는다.
  - 확인하지 못한 것: OS 저장소에만 있는 루트 인증서(회사 TLS 검사 프록시)로 받는 것은 보지 않았다. 시험 때문에 OS 저장소를 바꾸지 않았다. 앱 화면으로는 보지 않았다(앱이 보내는 설정을 그대로 서버에 보내 확인했다).

- [x] HTTP 요청 파일 보강(2026-11-04) — 작음~중간
  - 결정(사용자): `.http` 구문 강조, 파일·멀티파트 본문, 응답을 파일로 저장. 요청 기록은 이번에 넣지 않았다. 응답 처리 스크립트는 JS 엔진을 넣어야 해서 뺐다.
  - 파일 본문: 본문의 `< 경로` 줄을 그 파일의 바이트 그대로 넣는다(이미지 등). 경로는 `.http` 파일의 폴더 기준이고 `{{변수}}`를 채운다. 파일 안의 변수는 채우지 않는다. 한 파일 100MB까지(보내는 본문은 메모리에 다 올린다). 읽지 못하면 보내지 않고 경로와 까닭을 알린다.
    - `Content-Type`이 `multipart/`면 본문 줄을 CRLF로 잇는다. 그래야 서버가 파트를 나눈다(`.http` 파일 자체의 줄 끝과 상관없다). 그 밖에는 LF.
  - 응답 저장: 요청 끝의 `>> 경로`는 응답 본문을 받은 바이트 그대로 저장한다. 파일이 있으면 두고 `이름-1.확장자`처럼 번호를 붙인 새 파일에 쓴다(`create_new`로 만들어 그 사이 생긴 파일도 덮지 않는다). `>>! 경로`는 덮어쓴다. 폴더가 없으면 만든다. 응답 창에 쓴 파일이나 쓰지 못한 까닭을 보인다.
  - 응답 처리 스크립트(`> {% … %}`, `> ./a.js`)는 돌리지 않고 본문에서 뺀다. 응답 창에 돌리지 않는다고 알린다. 그전에는 본문으로 보냈다.
  - `>>`·`>`는 빈 줄 없이 헤더 바로 뒤에 와도 받는다.
  - 구문 강조: 내장 TextMate 문법(`ferrite-textmate`의 `http.tmLanguage.json`, Handlebars처럼 이 판에서 쓴 것)을 `.http`·`.rest`에 쓴다. 칠하는 것은 `###`·주석과 지시(`@name` 등)·파일 변수·메서드·주소·`HTTP/1.1`·질의 이어 쓰기·헤더 이름·`{{변수}}`·본문의 JSON(키·글·숫자·참거짓)·`<`·`>>`이다. 돌리지 않는 스크립트는 주석처럼 흐리게 보인다. 줄 주석(`Ctrl+/`)은 `#`.
    - 빈 줄(본문 시작)은 `\A`로 잡는다. 엔진은 줄 끝에 줄바꿈을 붙여 넘기는데, `^`가 그 줄바꿈 뒤(줄 끝)에서도 맞아 모든 줄 끝에서 본문이 시작되기 때문이다.
  - 검증
    - 단위: 해석(파일 줄은 본문에 두기, `>>`·`>>!`, 여러 줄·한 줄·파일 스크립트 빼기, 헤더 바로 뒤의 저장 줄), 보낼 본문(파일 바이트 그대로, 멀티파트 CRLF, 경로의 변수, 멀티파트가 아니면 LF, 없는 파일), 저장(폴더 만들기·번호 붙이기·덮어쓰기·확장자 없는 이름), 강조(위의 칠하는 것 모두, 메서드 없는 요청 줄, 빈 줄 뒤는 헤더가 아님)
    - 로컬 서버(`ferrite-http` 시험): 멀티파트로 PNG 머리처럼 CR·LF·`0xff`가 섞인 파일을 올리면 서버가 받은 본문이 바이트까지 같다.
    - 창 시험(kittest): `>> out/me.json`이 있는 요청을 두 번 보내면 처음은 `me.json`, 다음은 `me-1.json`에 받은 그대로 쓰고 응답 창에 그 경로를 보인다.
      - 처음 CI에서 macOS·Windows만 실패했다. 앱은 연 파일의 정규화한 경로로 저장하는데 시험은 임시 폴더 경로 그대로 견주었다(macOS `/var` → `/private/var`, Windows CI의 짧은 이름 `RUNNER~1`). 시험이 앱이 알린 경로를 받아 끝 부분과 내용을 보게 고쳤다.
    - 실제 앱(Windows, 릴리스 빌드, 로컬 서버): 강조를 화면으로 보고, 요청 줄에서 `Ctrl+Enter`로 사진(PNG 75바이트)과 한글 제목을 멀티파트로 올렸다. 서버는 원본과 같은 바이트를 받았다고 답했고(200), 그 응답이 `out/upload.json`에 저장되어 트리에 나타났다. 응답 창에 스크립트를 돌리지 않는다는 알림과 저장한 경로가 보였다.
  - 확인하지 못한 것: 실제 egov 서버(커먼즈 파일 업로드)로는 올려 보지 않았다(멀티파트 꼴은 표준이다). macOS·Linux는 시험으로만 확인했다.

- [x] Linux CI 디스크 부족(2026-11-05) — 작음
  - 까닭: Linux 작업이 두 번 "No space left on device"로 실패했다. 실행 41770003746은 `ubuntu-latest`가 `cargo test --workspace` 도중 죽었고, 41801828819는 `full (ubuntu-latest)`가 시험을 모두 통과한 뒤 마지막 캐시 저장에서 실패했다.
    - 계측(실행 41935224614, 단계 사이 `df`와 10초마다 기록): 러너 이미지가 처음부터 59GB를 쓴다(서비스 컨테이너가 뜬 `ubuntu-latest`는 62GB). full 작업은 그 위에 약 13GB(target의 debug 9.2GB·release 1.8GB)를 쓰고, 이어서 캐시 저장이 1.6GB짜리 압축 파일을 만든다. `ubuntu-latest` 작업은 약 10GB를 쓴다.
    - 계측한 러너는 디스크가 145GB(빈 곳 86GB)라 넘치지 않았다. 러너마다 디스크 크기가 다른데, 72GB 디스크라면 빈 곳이 10~13GB라 두 작업 모두 넘친다. 두 실패 모두 이 어림과 맞는다. 실패한 실행의 로그에는 `df`가 없어 그 러너의 크기는 확인하지 못했다.
  - 고침(`ci.yml`): 두 Linux 작업(`ubuntu-latest`·`full (ubuntu-latest)`) 맨 앞에서 이 저장소가 쓰지 않는 도구를 지운다. 지우는 것은 .NET(5.8GB)·Android SDK(12GB)·GHC(3.7GB)·Swift(3.5GB)·CodeQL(1.4GB)과 쓰지 않는 Docker 이미지(2.5GB)다. `df`로 보면 27~28GB가 빈다.
    - 시험이 쓰는 Java·Go·Node·Python·Maven·PostgreSQL과 서비스 컨테이너의 이미지는 둔다. 외부 액션 없이 `rm -rf`와 `docker image prune`만 쓴다.
    - 두 작업 끝에서 `df -h /`로 남은 디스크를 보인다. 두 단계는 YAML 앵커로 한 번만 적었다.
    - target 줄이기(릴리스 빌드 전에 debug 지우기, 디버그 정보 줄이기)는 넣지 않았다. 지우기만으로 여유가 넉넉하다. 또 debug를 지우면 캐시에 debug 의존성이 빠져 다음 실행에서 다시 빌드한다.
  - 검증
    - 전체 CI(3개 OS, 실행 41937924258)가 통과했다. Linux의 남은 디스크는 정리 뒤 110~114GB, 작업 끝 100~101GB다(145GB 러너).
    - 72GB 러너 흉내(실행 41937919988): 지운 뒤 72GB 디스크에서 남을 만큼만 비도록 큰 파일을 채우고 돌렸다. `ubuntu-latest`는 빈 곳 37.5GB로 시작해 가장 적을 때 27.2GB였다. full은 41.4GB로 시작해 28.0GB였고, 캐시 저장(1.6GB)까지 성공했다. 고치기 전이라면 시작 때 빈 곳은 10GB·13GB다.
  - 비용: 지우기에 62~131초가 든다. PR마다 도는 `ubuntu-latest` 작업은 이것을 넣고 5분쯤 걸린다. 더 줄이려면 지우기를 뒤에서 돌리거나 Android SDK만 지운다.
  - 확인하지 못한 것: 실제로 디스크가 작은 러너에 걸린 실행은 보지 않았다(큰 파일로 흉내만 냈다). 러너 이미지가 바뀌어 지울 경로가 없어지거나 다른 큰 도구가 생기면 효과가 달라진다(`rm -rf`는 없는 경로에서 실패하지 않는다). `ubuntu-latest`가 Ubuntu 26으로 옮겨 가는 중이라는 알림이 있다.

- [x] HTTP 요청 기록(2026-11-05) — 작음
  - 결정(사용자): 기록은 앱이 켜진 동안만 메모리에 둔다. 채운 URL·응답 헤더·본문에 개인 환경 파일의 토큰이나 세션 값이 들 수 있어 디스크에 남기지 않는다.
  - HTTP 응답 창 위의 [기록] 목록: 최근 30개, 새것이 위.
    - 한 줄은 `#차례 상태 메서드 이름 · 보낸 때`다(예 `#3 404 GET 없는 주소 · 방금`). 같은 요청을 여러 번 보내도 차례로 가린다. 보낸 때는 시각 대신 지난 시간(방금·N분 전·N시간 전)이다. 항목에 마우스를 올리면 채운 URL과 파일:줄이 보인다.
    - 고르면 그때의 응답(상태·시간·크기·헤더·본문, 저장한 경로, 오류)을 다시 본다.
    - [다시 보내기]: 그 요청을 지금 파일에서 다시 보낸다. 이름(`###` 제목·`@name`)이 있으면 이름으로 찾아 위에 줄을 더해도 따라가고, 없으면 보낸 때의 줄이다. 지금 파일 내용과 고른 환경으로 다시 채운다(보낸 그대로 되풀이하지 않는다). 파일을 닫았으면 연다.
    - [요청으로 가기]는 그 요청 줄로 간다. [기록 지우기]는 목록을 비운다. 받는 중에는 다시 보내기와 지우기를 막는다.
  - 메모리: 기록에는 보일 본문 글(512KB까지)만 두고 받은 바이트는 버린다(응답은 20MB까지 받는다). 전에는 하나만 두던 응답에서도 본문을 바이트와 글로 두 벌 들고 있었다.
  - 검증
    - 단위: 이름으로 다시 찾기(줄이 밀려도, 이름이 없거나 못 찾으면 그 줄), 지난 시간 글
    - 창 시험(kittest): 세 번 보낸 기록, 목록에서 #1(401)을 고르면 그 응답, 파일 맨 위에 줄을 더한 뒤 [다시 보내기]가 이름으로 찾아 200(쿠키), [요청으로 가기]가 밀린 줄로 감, [기록 지우기]
    - 실제 앱(Windows, 릴리스 빌드, 부를 때마다 횟수를 늘리는 로컬 서버): `Ctrl+Enter`로 횟수 요청 둘과 404를 보낸 뒤 목록에서 #1을 고르면 그때의 응답(n 7, 마지막은 n 8)이 보였다. 다른 실행에서 #1을 고르고 [다시 보내기]를 누르면 #4(n 11)가 목록 맨 위에 생겼다.
    - `ferrite-app` 시험 전체를 돌렸을 때 한 번 MCP 시험(`mcp로_에이전트가_열린_파일의_…`)이 MCP 서버 연결에서 실패했다. 그 시험만 두 번, 전체를 한 번 더 돌리니 통과했다. 이 변경은 MCP를 건드리지 않는다.
  - 확인하지 못한 것: macOS·Linux는 시험으로만 확인했다.

- [x] 엔드포인트 자동 새로 고침(2026-11-05) — 작음
  - 엔드포인트 창은 Java 파일이 바뀌면 [새로 고침]을 누르지 않아도 다시 묻는다.
    - 바뀜은 언어 서버 묶음(`lsp_hub`)이 센다. 편집한 Java 문서의 내용을 서버에 보냈을 때(저장하지 않아도, 입력은 150ms 디바운스)와 디스크에서 바뀐 `.java`를 서버에 알렸을 때(에이전트·git·다른 도구)다.
    - egov 서버는 메시지를 차례로 처리하며 바뀐 내용을 그 자리에서 다시 색인한다. 그래서 알린 뒤에 보낸 물음은 바뀐 목록을 받는다.
    - 바뀐 뒤 500ms 동안 더 바뀌지 않으면 묻는다. Java 파일에 입력하는 동안 큰 목록(egov 프로젝트는 수천 개)을 거듭 받지 않게 하려는 것이다. 창이 가려져 있으면 묻지 않고, 다시 보일 때 묻는다.
    - 다시 묻는 동안 지난 목록을 그대로 보이고, 답이 오면 바꾼다. 전에는 물을 때 목록을 비웠다.
  - 하지 않은 것: 저장하지 않고 닫은 Java 문서는 세지 않는다(서버는 디스크 내용으로 되돌린다). 이때는 [새로 고침]을 누른다.
  - 검증
    - 단위: 처음 열면 바로 묻는다. 바뀐 것이 없으면 묻지 않고, 바뀌면 500ms 뒤에 묻는다. 다시 묻는 동안 지난 목록을 두고, 지난 물음의 답은 버리고, 답이 오면 바꾼다. [새로 고침]은 바로 묻는다.
    - 창 시험(kittest)으로는 보지 않았다. 내장 egov 서버는 `ferrite` 실행 파일의 플래그로 뜨는데, 시험 실행 파일은 그 플래그를 모른다.
    - 실제 앱(Windows, 릴리스 빌드, 컨트롤러 하나가 든 폴더): 엔드포인트 창에 `/board/list.do` 하나가 보였다. 에디터에서 `"/list.do"`를 `"/list2.do"`로 고치고(저장하지 않음) 다른 컨트롤러 파일을 폴더에 복사했다. [새로 고침]을 누르지 않았는데 `/board/list2.do`와 `/notice/list.do`(GET)가 컨트롤러 둘로 나뉘어 보였다.
  - 확인하지 못한 것: Spring Boot LS가 함께 답할 때와 egov 프로젝트처럼 엔드포인트가 수천 개일 때 다시 묻는 부담은 보지 않았다. macOS·Linux는 시험으로만 확인했다.

- [x] Gradle 실제 실행(2026-11-05) — 작음
  - 9-1 테스트 러너와 9-3 취약점 검사의 Gradle 처리를 형식 표본으로만 확인했다. 실제 Gradle 9.2.0을 `gradle init`이 만드는 꼴(루트에는 `settings.gradle.kts`만, 하위 프로젝트에 Kotlin DSL 빌드 파일)로 돌려 보고 틀린 셋을 고쳤다.
  - 고친 것
    - 실행 구성: 루트에 빌드 파일이 없고 설정 파일만 있으면 Gradle 프로젝트로 알지 못해 `gradle test`·`gradle build`가 없었다(테스트 창에도 구성이 없었다). 이제 설정 파일도 보고, 루트와 한 단계 아래 폴더의 빌드 파일에서 `application`·Spring Boot를 찾는다. 루트에서 돌린 작업은 하위 프로젝트에서 다 돈다.
    - 취약점 검사: 플랫폼(BOM) 아래의 `(c)` 줄은 버전 제약일 뿐 의존이 아닌데 의존으로 읽어, 같은 라이브러리가 BOM 줄에 겹쳐 보일 수 있었다. 이제 뺀다.
    - 취약점 검사: 저장소에서 받지 못한 의존(닫힌 망 등)은 `1.0 FAILED`로 나오고 Gradle은 성공으로 끝난다. 버전에 `FAILED`가 붙어 대조되지 않던 것을 적은 버전(`1.0`)으로 본다.
  - 확인한 Gradle 동작(로컬, 받기 없이 `--offline`): 루트에서 `gradle dependencies`는 루트 프로젝트만 낸다(빌드 파일이 없으면 "No configurations"). 그래서 하위 프로젝트 빌드 파일마다 그 폴더에서 돌리는 지금 방식이 맞다.
  - CI: 3개 OS 전체 작업에 `gradle/actions/setup-gradle@v6`(Gradle 9.2.0, 캐시 끔)을 더했다.
  - 검증
    - 단위: 실행 구성(설정 파일과 `app/build.gradle.kts`의 `application` → `gradle run`·`build`·`test`), 의존성 나무의 `(c)`·`FAILED`
    - 실제 도구(`test_tools`, CI 3개 OS 전체 작업)
      - 테스트: 루트에서 `gradle test` → `app/build/test-results`의 보고서에서 통과 1·실패 2·건너뜀 1(이름 `adds()` 꼴), 실패 스택의 줄을 `app/src/test/java/…/CalcTest.java` 16줄로 찾고, `--tests`로 실패한 둘만 다시 돈다. 실행 구성이 이 명령을 만드는지도 본다.
      - 의존성: 하위 프로젝트 `app`·`lib`. log4j-slf4j-impl이 실행 때 끌어오는 log4j-core를 그 직접 의존 줄에, slf4j-api 1.7.x → 2.0.9(Gradle이 고른 판), `lib`가 끌어온 것은 `project(":lib")` 줄에, BOM이 정한 판(`-> 5.11.4`), BOM 제약은 빼고, 테스트 의존은 없음.
      - 로컬: 테스트 시험은 통과했다. 의존성 시험은 하위 프로젝트로 바꾸기 전의 단일 프로젝트 판이 통과했고, 바꾼 판은 CI에서 본다(로컬 Gradle 캐시에 log4j POM이 없어 받아야 한다).
    - 실제 앱(Windows, 릴리스 빌드, `gradle init` 꼴 프로젝트): [실행 > 테스트]에 `gradle test` 구성이 보이고, [실행] → "통과 1 · 실패 2 · 건너뜀 1" 트리, [실패만 다시] → 실패한 둘만 걸린 시간이 바뀜, `addsWrong()`를 두 번 눌러 `CalcTest.java` 16줄로 감을 화면으로 확인했다. 확인 뒤 설정 폴더와 예제 프로젝트를 지웠다.
  - 확인하지 못한 것: Gradle 래퍼(`gradlew`)로는 돌리지 않았다(래퍼를 고르는 것은 단위 시험으로 본다). 취약점 창을 실제 앱에서 Gradle 프로젝트로 돌려 보지는 않았다.

- [x] PMD·Semgrep 실제 SARIF(2026-11-05) — 작음
  - 9-2 SARIF 가져오기는 SpotBugs만 실제로 돌렸다. PMD와 Semgrep을 실제로 돌려 낸 SARIF를 읽었다. 경로에 한글과 빈칸을 넣었다(한국어 Windows에서 흔한 `게시판 프로젝트` 꼴).
  - 읽기는 고칠 것이 없었다. 두 도구의 실제 출력은 이렇다.
    - PMD(maven-pmd-plugin 3.28.0, PMD 7.17.0)
      - SARIF를 짧은 이름으로 고르지 못한다. 렌더러 클래스로 고른다: `mvn compile pmd:pmd -Dformat=net.sourceforge.pmd.renderers.SarifRenderer` → `target/pmd.sarif.json`.
      - 파일 주소는 절대 `file:` 주소다. 빈칸은 `%20`, 한글은 그대로 적는다.
      - 수준은 우선순위에 따라 `note`·`warning`을 결과마다 적는다.
    - Semgrep(1.183.0)
      - 파일 주소는 돌린 폴더 기준 상대 주소(`uriBaseId: %SRCROOT%`, 그 경로는 주지 않는다)다. Windows에서는 역슬래시(`src\main\…`)로 적는다.
      - 결과에 수준이 없고 규칙의 `defaultConfiguration`에만 있다.
      - Python이라 규칙 파일을 시스템 로캘 인코딩으로 읽는다. 한국어 Windows(cp949)에서는 한글 메시지가 든 UTF-8 규칙을 읽다 멈춘다. `PYTHONUTF8=1`로 돌리면 된다(Python 3.15부터는 UTF-8 모드가 기본이다). Semgrep 휠은 Python 3.14까지라 3.15에는 설치되지 않는다.
  - 정적 분석 탭의 안내에 SpotBugs·PMD·Semgrep으로 SARIF를 내는 명령을 적었다(PMD의 렌더러 클래스, Semgrep의 `PYTHONUTF8`).
  - CI: 3개 OS 전체 작업에서 `semgrep==1.183.0`을 pip로 설치한다.
  - 검증
    - 실제 도구(`test_tools`, CI 3개 OS 전체 작업, 로컬 Windows 통과)
      - PMD: 프로젝트에서 `target/pmd.sarif.json`을 찾고, UnnecessaryImport(3줄, 참고)·UnusedPrivateField(6줄, 경고)·EmptyCatchBlock(11~12줄, 경고)를 한글·빈칸 경로의 원본 파일로 찾는다.
      - Semgrep: 한글 메시지 규칙 둘로 `reports/semgrep.sarif`를 내고, empty-catch(9~12줄, 오류)·parse-int(10줄, 참고)와 메시지(`숫자 변환 s` 등)를 원본 파일로 찾는다.
    - 실제 앱(Windows, 릴리스 빌드, `게시판 프로젝트` 폴더)
      - PMD: 정적 분석 탭에서 `target/pmd.sarif.json`을 불러오면 "PMD · 오류 0 · 경고 2 · 참고 1", 파일별 목록, 3·6·11~12줄 물결선, 상태 표시줄 "경고 2"가 보인다.
      - Semgrep: `reports/semgrep.sarif`를 불러오면 "Semgrep OSS · 오류 1 · 경고 0 · 참고 1", 한글 메시지, 물결선이 보이고, 결과를 눌러 10줄 20열로 간다.
      - 확인 뒤 설정 폴더와 예제 프로젝트, Semgrep을 설치한 venv를 지웠다. PMD jar는 Maven 로컬 저장소에 남는다.
  - 확인하지 못한 것: clang 정적 분석기의 SARIF, egov 표준 점검 규칙(PMD 규칙 묶음)으로 돌린 결과

- [x] JSP 화면 스크립트 줄 맞추기(2026-11-06) — 중간
  - JSP 파일에 바로 중단점을 걸고, 멈춘 자리도 JSP의 줄로 보인다. 전에는 디버그 창의 "화면 스크립트"에서 브라우저가 받은 화면을 읽기 전용으로 열어 걸어야 했다.
  - 왜 따로 해야 하나: js-debug는 화면 주소(`/board/list.do`)를 만든 JSP를 모른다. 그래서 JSP 경로로 건 중단점은 영영 걸리지 않고, 인라인 스크립트는 브라우저가 받은 화면의 줄로만 준다.
  - 줄 맞추기(`ferrite_dap::page_lines`)
    - 화면 스크립트 내용과 프로젝트의 JSP(`.jsp`·`.jspf`·`.jspx`·`.tag`, UTF-8·CP949)를 줄마다 견줘 가장 긴 공통 줄 맞춤(LCS)을 찾는다.
    - JSP 줄의 바뀌는 부분(EL `${…}`, 스크립틀릿 `<% %>`, `<c:url …/>` 같은 접두사 태그, Thymeleaf `[[…]]`)은 아무 글이나 맞고, 그 앞뒤 글자로 맞춘다. 태그만 있는 줄(`<c:if>`)과 지시자는 화면에 없으므로 건너뛴다.
    - 의미 있는 줄(글자·숫자 셋 이상)을 절반 넘게 맞춘 JSP 중 가장 많이 맞춘 것을 고른다. 같은 스크립트가 여러 JSP에 있으면 화면 주소와 낱말이 많이 겹치는 경로를 고른다.
    - 맞춘 줄이 아닌 화면 줄(빈 줄)은 앞의 맞춘 줄부터 세되 다음 맞춘 줄은 넘지 않는다.
  - 디버거(js-debug 세션만)
    - JSP 중단점은 js-debug에 보내지 않고 둔다. 화면을 불러오면 js-debug가 인라인 스크립트를 `loadedSource`로 알리고(DAP 이벤트를 새로 받는다), 앱은 내용을 받아 JSP를 찾고 그 JSP의 중단점을 화면 줄로 바꿔 그 스크립트에 건다. 화면에 그대로 나온 줄만 걸 수 있고, 그렇지 않은 줄(지시자·`<c:if>` 안)은 빈 점으로 남는다.
    - 멈춘 프레임이 그 스크립트면 JSP와 그 줄을 보인다(호출 스택 `Window.tick  BoardList.jsp:13`). 디버그 창의 "화면 스크립트" 목록에도 맞춘 JSP 이름을 붙인다(`… → BoardList.jsp`).
    - 담은 파일에 직접 건 중단점도 그대로 쓴다(직접 건 것 뒤에 JSP에서 온 것을 붙여 보낸다).
  - js-debug의 동작(실제 어댑터로 확인)
    - 인라인 스크립트의 중단점은 화면 주소로 건다(`setBreakpointByUrl`). 그래서 화면을 다시 불러도 걸려 있고, 화면을 불러오는 중에 바로 도는 줄(맨 위 코드)에서도 멈춘다.
    - 처음 화면에서는 스크립트를 알려 준 뒤에 걸 수 있으므로, 이미 지나간 맨 위 코드와 `$(function(){…})`(문서 준비)는 화면을 한 번 다시 불러야 멈춘다. 버튼·타이머·Ajax 콜백 안은 처음부터 멈춘다.
  - 검증
    - 단위: egov 목록 JSP(지시자·taglib·`<c:url>`·EL·거짓 `<c:if>`)와 그 화면 스크립트의 줄 대응·중단점 줄, 맞지 않는 화면, 같은 스크립트가 여러 JSP에 있을 때 주소로 고르기, 바뀌는 부분 맞추기, JSP 찾기
    - 가짜 어댑터(앱의 디버거): CP949 JSP에 건 중단점이 `loadedSource` → 내용 받기 → 화면 15줄로 걸리고 JSP는 어댑터에 가지 않는다. 멈춘 자리가 JSP 5줄, 호출 스택은 JSP 이름이다. JSP 중단점을 옮기면 다시 걸고, 지시자 줄은 빈 점이다.
    - 실제 어댑터(`adapters.yml`, 3개 OS, 로컬 Windows 5번 연속 통과): 헤드리스 Chrome에서 `loadedSource`를 받고, 실제 화면 스크립트로 JSP와 줄(JSP 10줄 → 화면 8줄)을 찾아 멈춘 자리를 JSP 줄로 맞추고, 맨 위 줄에 건 중단점이 화면을 다시 불렀을 때 멈춘다.
      - 시험을 만들다 찾은 것: 멈춘 채 보낸 식(`evaluate`)과 이어 가기(`continue`)를 js-debug는 함께 처리해, 답을 기다리지 않으면 이어 가기가 먼저 브라우저에 가서 식이 돌지 않았다(시험만 고쳤다. 앱은 사용자가 차례로 누른다).
      - CI의 Windows에서 찾은 것: 시험용 웹 서버가 연결을 차례로 받아, 브라우저가 요청 없이 미리 열어 둔 연결을 읽다 다시 부른 화면에 답하지 못했다. 연결마다 따로 받게 고쳤다.
    - 실제 앱(Windows, 릴리스 빌드, 로컬 웹 서버): 언어 서버 창에서 js-debug를 설치하고, CP949 JSP(`BoardList.jsp`, 화면에서는 지시자·`<c:set>`이 빠져 세 줄 위)의 13줄에 중단점을 걸고 헤드리스 구성으로 F5를 눌렀다. JSP 13줄에서 멈추고 호출 스택에 `Window.tick  BoardList.jsp:13`, `setInterval`을 부른 줄은 EL이 든 `BoardList.jsp:16`으로 보였다. 앱을 닫은 뒤 node·Chrome 프로세스가 남지 않았다. 확인 뒤 설정·설치 폴더와 예제를 지웠다.
  - 확인하지 못한 것: `<jsp:include>`·Tiles처럼 여러 JSP가 합쳐진 화면(스크립트마다 그것을 만든 JSP를 찾으므로 될 것으로 본다), 스크립트 한 줄을 여러 줄로 만드는 반복(`<c:forEach>` 안의 스크립트, 첫 줄에만 걸린다), 처음 화면의 맨 위 코드(다시 불러야 한다)

- [x] HTTP 응답 처리 스크립트(2026-11-06) — 중간
  - `.http` 요청 뒤의 `> {% … %}`(또는 `> ./check.js`)를 응답을 받은 뒤 돌린다. 로그인 응답의 토큰을 변수로 두고 다음 요청의 `{{token}}`에 쓰거나, `client.test`로 응답을 검사한다.
  - 결정(사용자): JS 엔진은 boa(0.22, 순수 Rust)를 쓴다. C 컴파일러가 필요 없다. 기본 기능(Temporal 등)은 뺐다.
    - 실행 파일이 약 12.5MB 커졌다(Windows 릴리스 51.6MB → 64.1MB). 새 크레이트 약 40개.
  - API(`ferrite_http::script`): 널리 쓰는 API 중 흔히 쓰는 것
    - `client.global.set·get·isEmpty·clear·clearAll`: 정한 값은 글로 둔다(객체는 JSON).
    - `client.test(이름, 함수)`(던지면 실패), `client.assert(조건, 메시지)`, `client.log(…)`(`console.log`도 같다), `client.exit()`
    - `response.status`, `response.body`(`Content-Type`이 JSON이면 객체, 아니면 글), `response.headers.valueOf·valuesOf`(대소문자 무시), `response.contentType.mimeType·charset`
    - 없는 것: `request` 객체, `jsonPath`, 요청 전 스크립트(`< {% %}`) 실행
  - 변수: 찾는 순서는 파일 변수 → 스크립트가 정한 변수 → 고른 환경 → 동적 변수다. 스크립트가 정한 값이 환경의 같은 이름보다 앞선다(받은 토큰이 견본 값을 이긴다).
  - 해석
    - 한 줄(`> {% … %}`)·여러 줄 스크립트와 스크립트 파일(요청 파일 폴더 기준, UTF-8 또는 CP949)을 쓴 차례대로 모은다. 닫지 않은 스크립트도 두고 돌리면 문법 오류로 알린다.
    - 요청 줄 앞의 요청 전 스크립트(`< {% %}`, `< ./pre.js`)는 건너뛴다. 전에는 그 줄을 요청 줄로 읽어 엉뚱한 주소로 보냈다. 돌리지 않고 응답 창에 알린다.
  - 돌리기
    - 보내는 스레드에서 응답을 받은 뒤 스크립트들을 한 JS 환경에서 차례로 돌린다. 오류가 나면 거기서 멈추고, 그 앞에서 정한 변수는 남는다. `client.exit()`는 오류가 아니다.
    - 응답과 변수는 JSON으로 넘긴다(U+2028·2029는 이스케이프한다).
    - 요청 파일 안의 스크립트는 앞을 빈 줄로 채워, 오류에 붙는 줄(`(응답 스크립트:6:3)`)이 요청 파일의 줄과 같다.
    - 끝없는 반복은 한 함수 안의 반복 1,000만 번에서 멈춘다(응답 창이 다음 요청을 기다리지 않게). 반복 안에서 반복하는 함수를 부르면 더 오래 돌 수 있다(코드에 `ponytail:`로 적었다).
  - 앱(HTTP 응답 창)
    - 변수는 쿠키처럼 앱이 켜진 동안 두고 보내는 스레드가 가졌다 돌려준다. [변수 N개 지우기](걸면 이름들)
    - 상태 아래 "테스트 2개 통과"(초록)·"테스트 3개 중 1개 실패"·"응답 스크립트 오류"(빨강)
    - [스크립트] 탭: 오류, 테스트마다 통과·실패와 까닭, 로그
  - 칠하기: `> {% %}` 안을 JS(키워드·글·숫자·주석·`client`·`response`·함수 이름)로 칠한다. 전에는 돌리지 않으므로 주석처럼 흐렸다. 요청 전 스크립트도 요청 줄로 칠하지 않는다.
  - 검증
    - 단위(스크립트): JSON 본문에서 값 꺼내기·숫자와 객체 값·지우기, 테스트 통과·실패·`TypeError` 까닭, 헤더 대소문자·여러 값, 글 본문, 오류에서 멈추고 앞의 값은 남음, `exit`, 끝없는 반복 멈춤, 문법 오류
    - 단위(해석·채우기): 여러 줄·한 줄·닫지 않은 스크립트와 파일, 요청 전 스크립트를 건너뜀, 변수 순서(파일 > 스크립트 > 환경), CP949 스크립트 파일, 오류의 줄이 요청 파일의 줄(6줄), 없는 스크립트 파일
    - 로컬 서버: 토큰 POST → 스크립트가 토큰을 변수로 두고 테스트 통과 → `Authorization: Bearer {{token}}`으로 보낸 요청이 200. 받기 전에는 채우지 못한다고 알린다.
    - 창 시험(kittest): 정하기 전에는 "채우지 못한 변수", 로그인 뒤 "테스트 2개 중 1개 실패"와 변수 둘, [스크립트] 탭의 실패 까닭과 로그, 다음 요청이 그 변수로 200, 요청 전 스크립트 알림, [변수 2개 지우기]
    - 칠하기: 스크립트 안의 `client`·함수·글·숫자·주석, 요청 전 스크립트 줄은 요청 줄이 아님
    - 실제 앱(Windows, 릴리스 빌드, 로컬 토큰 서버)
      - 로그인 요청을 `Ctrl+Enter`로 보내 200, "테스트 2개 중 1개 실패"(일부러 실패하게 쓴 만료 검사), [변수 1개 지우기]. 스크립트는 JS로 칠해졌다.
      - 이어 게시판 요청(`Authorization: Bearer {{token}}`)을 보내니 받은 토큰으로 200을 받고, 스크립트 파일(`> ./check-board.js`)의 테스트가 "테스트 1개 통과"로 나왔다.
      - 확인하지 못한 것: [스크립트] 탭을 자동 클릭으로 누르는 것. 두 번 탭이 바뀌지 않았는데, 그동안 탐색기가 포커스를 가져가고 화면 배치가 바뀌어(창이 다른 크기로 떴다) 더 하지 않았다. 창 시험에서 포인터 이벤트로 눌러 탭 내용을 확인했다.
      - 확인 뒤 예제·서버·설정 폴더를 지웠다.
  - 시험을 돌리다 본 것: `ferrite-app` 시험 전체에서 MCP 시험(`mcp로_에이전트가_열린_파일의_…`)이 `ConnectionReset`(10054)으로 자주 실패한다. 이 변경을 빼고 main 코드로 돌려도 같다. 혼자 돌리면 통과한다.

- 살폈지만 넣지 않은 것: 임베디드(MCU 플래시·시리얼 모니터·장비 설정 파일), 프레임워크별 지원(React·Vue·Angular 등, JS·TS 언어 서버로 기본은 된다), 린터 개별 연동(언어 서버나 저장 시 작업으로 된다), 원격 개발·컨테이너·SFTP 배포, 로컬 AI 완성 모델, 계정 기반 설정 동기화. egov 개발과 거리가 멀거나 크기에 비해 쓰임이 적다.

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
- Phase 8은 Phase 2(언어 서버)·2b(egov 색인)·4(DAP)에, Phase 9는 Phase 4(실행 구성)에 기댄다. Phase 6·7과는 상관없어 앞당길 수 있다.
