# Ferrite 개발 워크플로우

> 모든 Phase는 **Windows / macOS / Linux 3개 OS CI 통과**를 완료 조건에 포함한다.

## 1. 공통 개발 규칙

| 항목 | 규칙 |
|---|---|
| 브랜치 | `main`은 보호 브랜치. 작업은 `feature/<phase>-<기능>`에서 하고 PR로 병합 |
| CI 게이트 | `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test` → `cargo build --release`, 3개 OS 매트릭스 |
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
- [ ] 2-5b Spring Boot LS 중계: 한 언어에 서버 둘(jdt.ls + Spring Boot LS)을 붙이고 jdt.ls와의 메시지를 중계
- [ ] 2-6 서버 크래시 자동 재시작(백오프), 3개 OS 최종 검증

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
