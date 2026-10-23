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
- [ ] GitHub 저장소에 올려 CI 3개 OS 통과 확인 (macOS·Linux 빌드는 아직 검증 전)

**완료 조건**: CI 3개 OS 통과. 한글 CP949 파일을 열어 수정하고 저장했을 때 인코딩과 줄바꿈이 보존된다.

### Phase 1 — 에디터 코어

**목표**: 대용량 파일과 코드 편집 품질을 확보한다. 가장 큰 기술 리스크가 있는 구간이다.

- 크레이트 분리: `ferrite-core`(UI 비의존), `ferrite-app`
- ropey 기반 버퍼, undo/redo(편집 그룹 단위)
- **가상화 렌더링 에디터 위젯**: 보이는 줄만 레이아웃하고, 줄 번호와 현재 줄 강조
- tree-sitter 하이라이팅: Rust, Python, Go, C/C++, PHP, Java, SQL, TOML/JSON/YAML
- 찾기/바꾸기(정규식), 줄 이동, 주석 토글, 들여쓰기 감지
- 파일 외부 변경 감지(notify) 후 다시 불러오기 제안
- 설정 파일(TOML): 폰트 크기, 테마, 탭 폭

**완료 조건**: 100MB 텍스트 파일에서 스크롤과 편집 모두 60fps를 유지한다. 코어 단위 테스트 커버리지 70% 이상.

### Phase 2 — LSP 통합

**목표**: 언어 지능을 붙인다.

- JSON-RPC over stdio 클라이언트. 요청/응답 상관관계, 취소, 타임아웃 처리
- 기능: 진단(밑줄 + 문제 패널), 자동완성, 정의/참조 이동, 호버, 포맷, 이름 변경, 코드 액션
- **언어 서버 매니저**:
  - OS·아키텍처별 바이너리를 다운로드하고 SHA-256을 검증한 뒤 설치 경로에 둔다.
  - 버전을 고정하고 업데이트를 알린다.
- 대상 서버: rust-analyzer, gopls, basedpyright, ruff, clangd, phpactor, jdt.ls
- 서버 크래시 시 재시작(지수 백오프)하고 상태바에 표시

**완료 조건**: 7개 언어에서 완성, 정의 이동, 진단이 3개 OS 모두 동작한다.

### Phase 2b — egov 지원 (JSP · MyBatis)

**목표**: egov 프로젝트의 JSP와 MyBatis 매퍼 XML을 IDE 수준으로 다룬다. 오픈소스 언어 서버가 없어 직접 만드는 영역이다.

- `ferrite-egov-ls` 언어 서버 크레이트(Rust, LSP)
- MyBatis:
  - Mapper 메서드 ↔ XML 문장 이동
  - 참조 진단(refid, resultMap, namespace)
  - `${}` SQL Injection 경고
  - `#{}` 속성 완성
- JSP:
  - HTML·Java·EL 하이라이팅
  - taglib 완성
  - 이스케이프 없는 EL 출력의 XSS 경고
  - 뷰 이름 → JSP 이동
- 테스트 코퍼스: 실제 egov 3.x·4.x 프로젝트의 JSP·매퍼 XML 표본. 고객사 소스는 저장소에 넣지 않고 로컬 경로로만 참조한다.

**완료 조건**:
- egov 3.x·4.x 표본 프로젝트에서 이동과 진단이 동작한다.
- 코퍼스 기준으로 `${}` 사용과 이스케이프 없는 EL 출력을 빠짐없이 탐지한다.

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
Phase 0 ─► Phase 1 ─► Phase 2 ─┬─► Phase 2b (egov 지원)
                               ├─► Phase 4
                               ├─► Phase 5 (Phase 1 에디터만 필요, 2와 병행 가능)
                               └─► Phase 3 ─► Phase 6 ─► Phase 7
```

- Phase 2b(egov 지원)는 egov 개발에 바로 쓰려면 가장 먼저 필요한 기능이다. Phase 2의 LSP 클라이언트가 준비되면 Phase 3·4보다 앞당긴다.

- Phase 5(DB)의 JDBC 사이드카는 Java 작업이라 Phase 2와 병행할 수 있다.
- Phase 7의 패키징 스크립트는 언제든 앞당길 수 있다. 사내 배포가 필요해지는 시점에 진행한다.
