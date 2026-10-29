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
  - 남은 것: JSON 스키마 검사(`package.json` 등 스키마를 서버에 알리기), egov 화면의 jQuery 자동완성(Phase 완료 조건에서 확인)
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
  - 남은 것: egov 서버는 디스크에서 바뀐 Java 파일을 다시 색인하지 않는다(열고 고친 것과 시작할 때만). 엔드포인트 창은 [새로 고침]으로 다시 묻는다.
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
  - 남은 것: 응답 저장·요청 기록, 멀티파트·파일 본문(`< ./file`), 응답 처리 스크립트(`> {% %}`), `.http` 구문 강조
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
  - 남은 것: 화면 스크립트의 줄을 JSP 파일 줄로 거꾸로 맞추기, 소스 맵(번들된 TS 등)은 js-debug가 하는 대로 두고 따로 확인하지 않았다
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
  - Maven은 전이 의존을 `mvn dependency:list`로 푼다. egov는 전이로 들어오는 로깅·Spring 라이브러리가 많다.
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
  - 남은 것: Gradle 실제 실행은 CI에서 돌리지 않았다(형식 표본으로 확인), Kotlin·Scala 스택은 같은 꼴이라 읽지만 확인하지 않았다
- [ ] 9-2 SARIF 결과 가져오기 — 작음
- [ ] 9-3 의존성 취약점 검사(OSV, 오프라인 DB) — 중간
- [ ] 3개 OS 검증

### 백로그 — Phase 미정

작거나 급하지 않아 Phase에 넣지 않은 것이다. 필요해지면 가까운 Phase의 조각으로 옮긴다.

| 기능 | 내용 | 크기 |
|---|---|---|
| EditorConfig | `.editorconfig`의 들여쓰기·줄바꿈·끝 공백·마지막 줄바꿈을 따른다. `charset`은 latin1·UTF-8·UTF-16만 정의되어 있어, CP949 감지·보존은 지금대로 둔다. | 작음 |
| 단축키 설정·Eclipse 배치 | `settings.toml`에서 단축키를 바꾼다. egov 개발환경(Eclipse)에서 넘어온 사용자를 위해 Eclipse 단축키 배치를 고를 수 있게 한다. | 중간 |
| 사용자 정의 DAP 어댑터 | `debug.toml`에 어댑터 실행 명령과 연결(표준 입출력·TCP)을 직접 적어 목록 밖 어댑터(장비 제조사 디버거 등)를 붙인다. 제조사가 DAP 어댑터를 함께 내주는 경우가 늘고 있다. | 작음 |
| 저장 시 작업 | 저장한 파일이 패턴에 맞으면 명령(포매터, Sass 컴파일 등)을 돌린다. `.ferrite/run.toml`과 같은 틀이다. | 작음 |
| TODO 목록 | `TODO`·`FIXME` 주석을 파일에서 찾기(3-1)로 모아 창에 보인다. | 작음 |
| Handlebars 강조 | egov 코드 생성 템플릿(`.ferrite/templates/code/`)을 고칠 때 쓴다. | 작음 |
| AI 에이전트 연동 | 개방 표준 둘. MCP 서버: 열린 파일·진단·디버거 상태를 외부 코딩 에이전트에 도구로 내준다. ACP(Agent Client Protocol) 클라이언트: 에이전트 프로세스를 띄워 패널에서 대화하고, 제안한 편집을 diff로 받는다. | 큼 |

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
