# Ferrite

Rust로 작성한 크로스플랫폼(Windows / macOS / Linux) 경량 IDE.
언어 지능은 LSP와 DAP 같은 오픈 표준과 오픈소스 언어 서버에 위임한다.

- 개발 워크플로우와 로드맵: [docs/WORKFLOW.md](docs/WORKFLOW.md)

## 현재 상태: Phase 2 완료 (LSP 연동), Phase 2b 기능 완료 (3개 OS 검증 남음), Phase 2c 진행 중 (egov 개발도구)

- 폴더 열기, 지연 로딩 프로젝트 트리(파일 감시로 자동 갱신), 탭 에디터
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

MIT OR Apache-2.0
