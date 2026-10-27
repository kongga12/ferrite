# Ferrite

Rust로 작성한 크로스플랫폼(Windows / macOS / Linux) 경량 IDE.
언어 지능은 LSP와 DAP 같은 오픈 표준과 오픈소스 언어 서버에 위임한다.

- 개발 워크플로우와 로드맵: [docs/WORKFLOW.md](docs/WORKFLOW.md)

## 현재 상태: Phase 2·2b·2c·3·4·5 완료 (LSP 연동, egov 언어 지원·개발도구, 프로젝트 도구, 실행·디버그, DB 도구, 3개 OS 검증), Phase 6 진행 중 (데이터 사이언스)

- 폴더 열기, 지연 로딩 프로젝트 트리(파일 감시로 자동 갱신), 탭 에디터
- 실행(`Ctrl+F5`): Maven(egov `tomcat7:run`·`spring-boot:run`)·Gradle·cargo·go·CMake·Python·Composer 구성을 자동으로 찾고, `.ferrite/run.toml`에 직접 적은 구성과 함께 터미널 탭에서 돌린다.
- 디버그(`F5`): 거터를 눌러 중단점(오른쪽 단추로 조건), 한 줄씩(`F10`)·들어가기(`F11`)·나오기(`Shift+F11`), 호출 스택·변수 트리·디버그 출력. 어댑터(debugpy·codelldb·delve·php-debug·java-debug)는 `도구 > 언어 서버`에서 설치한다. Python·Rust(빌드 후)·Go·PHP·Java는 열린 파일로 구성을 만든다. egov 웹 프로젝트는 실행(`Ctrl+F5`)의 "`tomcat7:run` (디버그 5005)"으로 띄운 뒤 디버그의 "5005 포트에 붙기"로 붙는다.
- DB 연결(`도구 > DB 연결`): PostgreSQL·MariaDB·SQLite 연결을 추가·시험한다. 비밀번호는 파일이 아니라 OS 키체인에 둔다. Oracle·Tibero·CUBRID는 JDBC 사이드카(Java)로 연다. JDK가 필요하고, 드라이버 jar는 직접 고르거나 로컬 Maven 저장소·프로젝트의 `WEB-INF/lib`에서 찾은 것을 넣거나 [받기](Oracle 11g용 ojdbc6·orai18n, CUBRID 11.2용)로 받는다. Ferrite는 드라이버를 싣지 않는다.
- 스키마 브라우저(`도구 > 스키마 브라우저`): 고른 연결의 표·뷰와 열(형식)을 보고, 표·열 이름으로 찾고, [SQL 콘솔에서 조회]로 그 표를 조회한다. 콘솔 연결과 따로 연결해 읽는다. Oracle·Tibero·CUBRID는 사용자 동의어와 PUBLIC 동의어(시스템 계정 표를 가리키는 것은 뺀다)도 가리키는 표의 열과 함께 보인다. 스키마를 읽으면 매퍼 XML의 SQL에서 표·열 이름을 자동 완성하고(FROM 뒤에는 표, `별칭.` 뒤에는 그 표의 열), 스키마에 없는 표와 표에 없는 열을 경고한다.
- SQL 콘솔(`도구 > SQL 콘솔`): 고른 글이나 커서가 있는 문장을 실행(`Ctrl+Enter`)하고, 실행 계획·멈추기·자동 커밋·커밋·롤백을 쓴다. 결과 그리드는 스크롤하면 500행씩 더 받고, 10만 행을 넘으면 앞쪽을 버려 100만 행도 메모리가 늘지 않는다. CSV로 내보내기(UTF-8 BOM)는 조회를 다시 실행해 파일로 흘려 쓴다. 한 표에서 기본 키 열을 모두 넣어 조회하면 칸을 두 번 눌러 고친다(기본 키로 찾는 UPDATE, 값은 파라미터로 묶는다). Oracle·Tibero·CUBRID는 드라이버가 결과 열의 출처를 알려 주지 않아 SQL에서 표를 찾으므로, 조인 없는 한 표 조회에서 열 이름을 그대로 쓴 항목만 고친다. Tibero 6은 멈춤 요청 뒤에도 문장을 1분 가까이 돌려, 권한이 있으면 세션을 끊고 다시 연결해 멈춘다.
- 노트북(`.ipynb`): 셀(코드·마크다운·원본)을 고치고 더하고 옮기고 지우며, 저장된 출력(글, PNG 그림, 오류, HTML 요약)을 본다. Jupyter가 쓴 모양 그대로 저장해 고치지 않은 셀은 바이트까지 그대로 남는다. [JSON 보기]로 원래 글을 고친다. 코드 셀은 Jupyter 커널(ipykernel 등)로 실행한다(Shift+Enter, 모두 실행, 멈춤, 커널 다시 시작). Python은 노트북 위 폴더의 `.venv`·`venv`나 PATH에서 찾는다. [DataFrame 보기]로 커널의 pandas DataFrame을 그리드로 본다(보이는 행만 페이지로 받고, 머리글을 눌러 정렬).
- 도킹 창: 프로젝트·에디터·터미널·문제·검색·참조를 탭으로 묶는다. 탭을 끌어 옆·위아래로 나눈다. 배치는 다음 실행 때 되살아난다(`보기 > 창 배치 초기화`).
- Git(`Ctrl+Shift+G`): 거터에 변경 줄 표시(저장 전 편집 포함), 상태바에 브랜치, 소스 제어 창에서 diff·스테이지·커밋·푸시·브랜치 전환(git CLI)
- 통합 터미널(`` Ctrl+` ``): 탭 여러 개, Windows는 ConPTY(PowerShell), 한글 입출력, 256색·트루컬러, 복사(고른 글자)·붙여넣기
- 파일로 이동(`Ctrl+P`): 파일 이름 퍼지 검색. 파일에서 찾기(`Ctrl+Shift+F`): 프로젝트 전체 검색(대소문자·단어·정규식·파일 패턴, UTF-8·CP949 파일). 둘 다 `.gitignore`를 따른다.
- 트리 우클릭: 새 파일·새 폴더·이름 바꾸기·휴지통으로 삭제
- 가상화 코드 에디터: 100MB 파일도 스크롤·입력 1ms 미만, 줄 번호, 한글 IME 조합 표시
- 언어 서버(LSP): rust-analyzer로 검증했다. gopls·clangd·basedpyright·phpactor·lemminx(XML)·jdt.ls(Java)도 연결한다.
  - Java: jdt.ls를 Lombok 에이전트와 함께 띄운다. 서버 실행에 Java 21 이상이 필요하다(JAVA_HOME 또는 PATH). 프로젝트는 Java 8·11이어도 된다.
  - Spring: Spring Boot LS를 설치하면 jdt.ls와 함께 붙는다. Java 진단을 보태고, `application.properties`·`yml`을 검사한다.
  - egov(내장, 설치 불필요): MyBatis·iBatis 매퍼에서 SQL Injection 위험(`${}`, `$prop$`), 정의 안 된 refid·resultMap, 중복 id를 진단한다.
    - Mapper 인터페이스·DAO 문자열 id ↔ XML 문장을 정의로 이동(F12)하고, XML 문장이 없는 Mapper 메서드를 경고한다.
    - 자동완성: `#{}`(iBatis `#…#`) 안의 VO 필드·`@Param` 이름, resultType·parameterType 클래스 이름(별칭 포함), resultMap의 property.
    - JSP 자동완성: taglib의 태그 이름(`<c:`), 사용자 정의 태그의 속성(필수 먼저, 이미 쓴 것 제외), EL 함수(`${fn:`). TLD는 프로젝트, `WEB-INF/lib`, 로컬 Maven 저장소(JSTL·Spring·egov `ui:`·Tiles)에서 읽는다.
    - JSP 이동(F12): include 경로(`<%@ include file>`, `<jsp:include page>`, `<c:import url>`) → 그 파일, 컨트롤러의 뷰 이름(`return "sample/list"`) → 뷰 리졸버 접두사·접미사로 찾은 JSP.
    - JSP: 이스케이프 없이 출력하는 EL(`${…}`)·`<%= %>`를 알린다.
    - Spring XML 설정: 없는 빈 참조·프로젝트 클래스·property를 경고하고, 빈·class·property·`${키}`(→ `.properties`)·import로 이동하며, class·빈 이름·property·프로퍼티 키를 완성한다.
    - JPA: `@Query` JPQL 안의 엔티티·필드 완성, IDENTITY를 지원하지 않는 Oracle 방언에서 `@GeneratedValue(IDENTITY)` 경고. 요청 값(`param`·`header`·`cookie`, `request.getParameter`)은 경고, 그 밖의 값은 정보다. `<c:out>`·`fn:escapeXml`로 감싸면 사라진다.
  - 언어 서버 매니저(`도구 > 언어 서버`): 버전·SHA-256을 고정한 배포본을 내려받아 설치·업데이트·삭제한다.
  - 서버가 없으면 상태바의 "설치" 버튼으로 바로 설치할 수 있다.
  - 인터넷이 막힌 PC는 다른 PC에서 받은 파일로 설치한다("파일에서…").
  - 진단: 물결 밑줄, 말풍선, 문제 목록(상태바의 "오류 N · 경고 M" 클릭)
  - 자동완성 팝업, 호버 정보
  - 정의로 이동, 참조 목록
  - 문서 포맷, 기호 이름 바꾸기(여러 파일), 코드 액션(빠른 수정·리팩터링)
- egov CRUD 코드 생성(`도구 > egov CRUD 코드 생성`): DDL을 붙여 넣으면 VO·Mapper(인터페이스·XML)·Service·ServiceImpl·Controller·JSP·테스트를 만든다.
  - egov 3.x·4.x, Oracle·Tibero·PostgreSQL·MariaDB·CUBRID SQL. 매퍼 위치·이름, JSP 폴더, 인코딩(UTF-8·MS949)은 프로젝트에서 찾는다.
  - 공식 VS Code 확장과 같은 템플릿 문맥이라, 공식·직접 고친 Handlebars 템플릿을 `.ferrite/templates/code/`에 두면 그것으로 만든다.
- egov 설정 파일 생성(`도구 > egov 설정 파일 생성`): Datasource·Transaction·ID 생성·Logging(log4j2)·Scheduling(Quartz)·Cache(Ehcache)·Property 21종을 XML·Java Config·YAML·properties로 만든다.
  - 만든 설정은 실제 Spring(egov 3.x·4.x)에서 띄워 확인했다. DB별 기본값과 프로젝트의 `${Globals.…}` 키를 쓴다.
- 새 egov 프로젝트(`도구 > 새 egov 프로젝트`): 공식 VS Code 확장 v5.0.7의 템플릿 22종(웹·템플릿·Boot·MSA·모바일·배치·AI RAG, egov 5)으로 프로젝트를 만든다.
  - 템플릿은 처음 한 번 GitHub에서 받는다(크기·SHA-256 고정). 인터넷이 막힌 PC는 공식 vsix나 받아 온 파일로 만든다.
  - 만든 프로젝트를 빌드하려면 Java 17 이상과 Maven이 필요하다.
- 공통컴포넌트 추가(`도구 > 공통컴포넌트 추가`): 공식 개발환경(eGovFrameDev 3.x·4.x)의 공통컴포넌트 플러그인에서 목록을 가져와, 고른 컴포넌트를 의존 컴포넌트와 함께 프로젝트에 넣는다.
  - 미리 보기로 바뀔 파일을 먼저 보여 준다. 내용이 다른 기존 파일은 건너뛰거나 `.bak`으로 백업하고 바꾼다. `pom.xml`은 덮어쓰지 않고 빠진 의존성을 알린다.
  - 프로젝트 DB에 돌릴 테이블 생성·기초 자료 스크립트를 순서대로 알린다.
- 구문 강조(tree-sitter): Rust, Python, Go, C/C++, PHP, Java, SQL, TOML, JSON, YAML, XML, HTML, JSP(HTML + 스크립틀릿 Java + EL), JavaScript, CSS, Properties
- UTF-8(BOM 포함)·CP949 인코딩과 CRLF 감지·보존, 들여쓰기 방식 감지
- 원자적 저장, 미저장 변경 확인(탭 닫기, 창 종료), 외부 변경 시 다시 불러오기·충돌 안내
- 명령줄: `ferrite <폴더>` 또는 `ferrite <파일>`

### 단축키 (macOS는 Ctrl 대신 Cmd)

| 동작 | 키 |
|---|---|
| 새 파일 / 폴더 열기 / 탭 닫기 | `Ctrl+N` / `Ctrl+O` / `Ctrl+W` |
| 저장 / 다른 이름으로 저장 | `Ctrl+S` / `Ctrl+Shift+S` |
| 찾기 / 바꾸기 / 다음·이전 | `Ctrl+F` / `Ctrl+H` / `F3`·`Shift+F3` |
| 파일로 이동 / 파일에서 찾기 | `Ctrl+P` / `Ctrl+Shift+F` |
| 터미널 열기·숨기기 | `` Ctrl+` `` (macOS도 Ctrl) |
| 소스 제어(Git) | `Ctrl+Shift+G` |
| 실행 취소 / 다시 실행 | `Ctrl+Z` / `Ctrl+Y`, `Ctrl+Shift+Z` |
| 줄 주석 토글 | `Ctrl+/` |
| 줄 위·아래로 이동 | `Alt+↑` / `Alt+↓` |
| 들여쓰기 / 내어쓰기 | `Tab` / `Shift+Tab` |
| 단어 단위 이동·삭제 | `Ctrl+←/→`, `Ctrl+Backspace` (macOS는 `Option`) |
| 자동완성 열기 | `Ctrl+Space` (macOS도 Ctrl) |
| 정의로 이동 / 참조 찾기 | `F12`, `Ctrl+클릭` / `Shift+F12` |
| 문서 포맷 / 기호 이름 바꾸기 | `Shift+Alt+F` / `F2` |
| 코드 액션 | `Ctrl+.` 또는 `Alt+Enter` |

### 설정

`파일 > 설정 열기`를 누르면 `settings.toml`이 만들어지고 에디터로 열립니다. 저장하면 바로 적용됩니다.
- 위치: Windows `%APPDATA%\ferrite`, macOS `~/Library/Application Support/ferrite`, Linux `~/.config/ferrite`
- 항목: 글꼴 크기, 테마(system/dark/light), 탭 크기, 기본 들여쓰기

언어 서버 설치 위치: Windows `%LOCALAPPDATA%\ferrite\servers`, macOS `~/Library/Application Support/ferrite/servers`, Linux `~/.local/share/ferrite/servers`

## 빌드

공통으로 [rustup](https://rustup.rs)으로 stable 툴체인을 설치한다.

| OS | 추가 준비 |
|---|---|
| Windows | Visual Studio Build Tools(C++ 워크로드) |
| macOS | `xcode-select --install` |
| Linux (Debian/Ubuntu) | `sudo apt-get install libgtk-3-dev libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`<br>한글 폰트가 없으면 `fonts-nanum` 또는 `fonts-noto-cjk`도 설치(없으면 한글이 네모로 보임) |

```bash
cargo run --release -- .
```

```bash
cargo test --workspace
```

100MB 파일 성능 측정(릴리스 빌드):

```bash
cargo test --release -p ferrite-app -- --ignored --nocapture 대용량
```

## 라이선스

[MIT](LICENSE-MIT) 또는 [Apache-2.0](LICENSE-APACHE) 중 원하는 쪽을 따른다.
