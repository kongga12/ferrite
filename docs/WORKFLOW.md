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
  - 실행 계획: Oracle은 `EXPLAIN PLAN FOR` 뒤 `DBMS_XPLAN.DISPLAY()`를 본다. Tibero(시험할 서버가 없다)·CUBRID(계획을 문장이 아니라 드라이버 API로만 준다)는 [실행 계획]을 끈다.
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
    - 확인하지 않은 것: Tibero 서버 시나리오(공개 이미지가 없다. 드라이버 싣기와 연결 오류까지만), 실제 앱에서 [받기] 누르기(고정 값은 저장소의 SHA-1·크기와 맞춰 확인했다)
- [x] 5-3b JDBC 결과 그리드 편집(SQL을 읽어 한 표에서 온 열과 기본 키를 찾는다)
  - 드라이버가 결과 열의 출처를 알려 주지 않으므로(Oracle은 표 없음, CUBRID는 `v AS n`을 n 열로 알림) SQL에서 찾는다(`ferrite-db/src/origin.rs`). 모르는 모양이면 읽기 전용이다(엉뚱한 열·행을 고치는 것보다 낫다).
    - 받는 모양: `SELECT [ALL] 항목들 FROM [스키마.]표 [[AS] 별칭] [WHERE …] [ORDER BY …] [FOR UPDATE] [OFFSET/FETCH/LIMIT]`. WHERE 안의 부분 조회는 괜찮다.
    - 받지 않는 모양: 조인(쉼표, JOIN), GROUP BY·HAVING, UNION·INTERSECT·MINUS, DISTINCT, CONNECT BY, WITH, FROM 안의 부분 조회, DB 링크(`@`), 플래시백(`AS OF`) 등
    - 고칠 수 있는 항목: `열`, `별칭.열`, `*`, `별칭.*`만. 별칭을 붙인 열(`name AS id`)과 식은 못 고친다. 항목 이름은 드라이버가 준 결과 열 이름과 같아야 한다(다르면 SQL을 잘못 읽은 것으로 본다).
  - 표의 열·기본 키: 사이드카가 `DatabaseMetaData.getColumns`·`getPrimaryKeys`로 알려 준다(결과를 읽는 중에도 묻는다). 스키마를 적지 않았으면 사용자 스키마의 표만 찾는다(동의어나 다른 스키마의 같은 이름 표를 고치지 않게). 없거나 둘이면 읽기 전용. 따옴표 없는 이름은 DB가 두는 대소문자로 바꿔 맞춘다(Oracle 대문자, CUBRID 소문자). CUBRID 11.2는 스키마 칸이 비고 표 이름이 `소유자.표`로 와서 나눈다.
  - Oracle 날짜: 드라이버는 DATE를 `2024-01-02 00:00:00.0`처럼 보인다. 세션 날짜 형식(NLS)에 기대지 않게 `CAST(TO_TIMESTAMP(?, 'YYYY-MM-DD HH24:MI:SS.FF') AS DATE)`(TIMESTAMP는 `TO_TIMESTAMP(…)`)로 넣는다. 날짜만 적어도 된다. CUBRID는 보인 글자를 그대로 넣어도 바뀐다.
  - Tibero는 시험할 서버가 없어 아직 읽기 전용이다.
  - 검증
    - 단위: 한 표 조회와 항목(별칭.열, 따옴표 식별자, 별, 주석·글자 안 FROM), 받지 않는 모양 14가지, 별칭 열·다른 별칭·이름이 다른 열은 고칠 수 없음, 대소문자 규칙, Oracle 날짜 UPDATE 문장
    - Oracle XE 11g·CUBRID 11.2(로컬 Docker): 5-2b 공통 편집 시나리오(복합 기본 키, 식 열, 따옴표·세미콜론 값, 글자로 적은 수, NULL, 같은 값 1행, 지워진 행 0행, 트랜잭션 안 틀린 값 뒤 롤백, 기본 키 없음·빠짐·두 표는 읽기 전용), `SELECT *`, 기본 키 이름을 별칭으로 붙인 열은 못 고치고 조건은 진짜 기본 키 값, Oracle DATE·TIMESTAMP를 보인 글자·날짜만으로 고치기
    - 실제 앱(Windows): CUBRID 2,000행 표에서 "dba.demo_emp · 칸을 두 번 눌러 고칩니다"를 보고 2행 이름을 두 번 눌러 고쳐, DB에서 그 행만 바뀐 것을 확인했다. 임시 연결 목록과 사이드카 폴더는 지웠다.
  - 찾은 것: 처음에는 ORDER BY 목록의 쉼표를 표를 더하는 쉼표로 읽어 `ORDER BY k1, k2` 조회를 읽기 전용으로 봤다(실서버 시나리오에서 찾음). 표를 더하는 쉼표는 표 이름 바로 뒤에서만 막는다.
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
- [ ] 3개 OS 검증

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
