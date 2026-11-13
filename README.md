# Ferrite

Rust로 작성한 크로스플랫폼(Windows / macOS / Linux) 경량 IDE.
언어 지능은 LSP와 DAP 같은 오픈 표준과 오픈소스 언어 서버에 위임한다.

- 개발 워크플로우와 로드맵: [docs/WORKFLOW.md](docs/WORKFLOW.md)

## 현재 상태: Phase 2·2b·2c·3·4·5·6·7·8 완료 (LSP 연동, egov 언어 지원·개발도구, 프로젝트 도구, 실행·디버그, DB 도구, 데이터 사이언스, 확장·배포, 웹·API 개발 도구, 3개 OS 검증). 코드 서명·공증은 인증서가 생기면 한다

- 설치 파일: 패키징 워크플로(`Package`)가 Windows(NSIS·MSI)·macOS(dmg)·Linux(deb·AppImage) 설치 파일을 만든다. 아직 서명하지 않아 Windows SmartScreen 경고([추가 정보] → [실행])와 macOS Gatekeeper 차단(오른쪽 단추 → [열기])이 뜬다.
- WASM 플러그인: 설정 폴더의 `ferrite/plugins/<이름>/`에 `plugin.toml`과 `.wasm`을 넣으면 [플러그인] 메뉴에 명령이, 플러그인 창에 패널이 생긴다. 플러그인은 앱이 넘긴 문서만 보고 파일·네트워크는 못 쓴다. 예제는 `examples/plugins/text-stats`(Rust).
- 언어 확장: 널리 쓰는 편집기 확장 폴더(`package.json`, `language-configuration.json`, `.tmLanguage.json`)를 플러그인 폴더에 그대로 넣으면 tree-sitter 문법이 없는 형식을 TextMate 문법으로 강조하고, 줄 주석 기호와 괄호 짝을 쓴다. 커서 옆 괄호의 짝은 모든 언어에서 강조한다.

- 폴더 열기, 지연 로딩 프로젝트 트리(파일 감시로 자동 갱신), 탭 에디터
- 실행(`Ctrl+F5`): Maven(egov `tomcat7:run`·`spring-boot:run`)·Gradle(루트에 설정 파일만 있는 하위 프로젝트 빌드 포함)·cargo·go·CMake·Python·Composer 구성을 자동으로 찾고, `.ferrite/run.toml`에 직접 적은 구성과 함께 터미널 탭에서 돌린다.
- 디버그(`F5`): 거터를 눌러 중단점(오른쪽 단추로 조건), 한 줄씩(`F10`)·들어가기(`F11`)·나오기(`Shift+F11`), 호출 스택·변수 트리·디버그 출력(식은 멈췄으면 고른 프레임에서 계산하고, js-debug(화면)·debugpy는 실행 중에도 계산한다). 어댑터(debugpy·codelldb·delve·php-debug·java-debug·js-debug)는 `도구 > 언어 서버`에서 설치한다. 목록 밖 어댑터(장비 제조사 디버거 등)는 `.ferrite/debug.toml`의 `[[adapter]]`에 실행 명령(표준 입출력)이나 TCP 주소(`IP:포트`, 명령과 함께면 띄운 뒤 붙는다)를 적어 쓴다. Python·Rust(빌드 후)·Go·PHP·Java는 열린 파일로 구성을 만든다. egov 웹 프로젝트는 실행(`Ctrl+F5`)의 "`tomcat7:run` (디버그 5005)"으로 띄운 뒤 디버그의 "5005 포트에 붙기"로 붙는다.
  - 브라우저 JS(js-debug, Node 필요): `.js`·`.jsp`·`.html`을 열고 F5. Chrome(없으면 Edge)을 따로 둔 프로필로 띄우거나, `--remote-debugging-port=9222`로 연 브라우저에 붙는다. 화면 주소는 HTTP 요청 환경 파일의 `baseUrl`(없으면 `http://localhost:8080`), 스크립트는 `src/main/webapp`(또는 `WebContent`)의 파일과 맞춘다(컨텍스트 경로 포함). Web Worker 스크립트에 건 중단점에서도 멈추고(화면을 다시 불러 새로 뜬 워커의 맨 위 코드 포함), 멈추지 않았을 때 디버그 출력의 식은 화면에서 돈다.
  - JSP의 인라인 스크립트는 JSP 파일에 바로 중단점을 건다. 화면을 불러오면 브라우저가 받은 스크립트를 내용으로 JSP와 맞춰(EL·`<c:url>` 등으로 바뀐 줄은 앞뒤 글자로) 그 줄에 걸고, 멈춘 자리와 호출 스택도 JSP 줄로 보인다. 화면에 나오지 않는 줄(지시자, 태그만 있는 줄)은 걸리지 않아 빈 점으로 남는다.
    - 여러 JSP로 합친 화면(Tiles 레이아웃, `<jsp:include>`)이나 `<script>`가 여럿인 화면은 스크립트 블록마다 그것을 만든 JSP를 찾는다(호출 스택에 `header.jsp:5`와 `BoardList.jsp:7`이 함께 보인다).
    - 블록은 브라우저가 받은 원래 HTML에서 잘라 맞추므로, jQuery `.load()`·`.html()`로 끼운 조각의 스크립트가 있어도 맞고, `<c:forEach>`로 같은 블록이 여러 번 나오면 나온 곳마다 건다.
    - 화면을 불러오는 중에 바로 도는 줄(맨 위 코드, `$(function(){…})`)은 JSP 중단점을 걸기 전에 지나간다. 그래서 JSP 중단점이 있는 화면은 처음 맞춘 뒤 Ferrite가 화면마다 한 번 다시 불러 거기서도 멈추고, 디버그 출력에 "[화면을 다시 불렀습니다: …]"를 남긴다. 멈춰 있으면 [계속]을 누른 뒤에 부른다. 폼을 보내 연 화면은 폼을 다시 보낼 수 있어 다시 부르지 않고 디버그 출력에 알린다. 버튼·타이머·Ajax 콜백 안의 줄은 처음부터 멈춘다.
    - 프레임: `<iframe>`·`<frame>`(`<frameset>`) 안 화면과 프레임 안의 프레임도 프레임 주소로 블록마다 맞추고, 다시 부를 때는 그 프레임만 처음 주소로 다시 연다(프레임 안의 프레임은 품은 프레임만 다시 열어 함께 연다). 링크나 폼으로 다른 화면으로 간 프레임은 다시 열지 않고 알린다.
    - 사이트가 다른 `<iframe>`(맨 위 화면은 IP, 프레임은 서버 이름으로 연 곳 등)은 브라우저가 따로 띄우지만 그 프레임도 같게 맞추고 다시 열며, 지도·동영상 같은 바깥 사이트의 프레임이 끼어도 맨 위 화면은 그대로 맞춘다. 두 화면에 모두 JSP 중단점이 있으면 맨 위 화면을 다시 부른 뒤 새로 뜬 프레임도 한 번 더 열어, 두 화면의 맨 위 코드에서 차례로 멈춘다.
    - 워커에서 멈추면 호출 스택에 그 워커를 만든 화면 줄도 JSP 줄로 보인다.
    - 번들된 TS 등 소스 맵이 있는 스크립트는 원래 파일(`.ts`)에 걸고, 멈추면 그 파일의 줄로 보인다. 원래 파일이 웹 루트 밖(`src/main/ts` 등)에 있어도 된다: tsc·Vite 맵의 상대 경로는 번들 파일에서 풀고, webpack 맵(`webpack:///./…`)은 자동 구성이 webpack을 돌리는 폴더(`webpack.config.*`가 있는 폴더, 없으면 프로젝트 루트) 기준으로 푼다.
    - 맞출 JSP가 없으면 디버그 창의 "화면 스크립트"에서 브라우저가 받은 화면을 읽기 전용으로 열어 건다. 목록에는 사이트가 다른 `<iframe>`·워커의 스크립트도 나오고, JSP를 찾은 화면은 그 JSP 이름을 함께 보인다(`…/list.do → list.jsp`). Ferrite가 화면을 살피려고 돌린 식과 식 칸에 친 식은 목록에 나오지 않는다.
- 테스트(`실행 > 테스트`): 실행 구성의 `mvn test`·`gradle test`·`pytest`·`ctest`·`go test`를 돌리고 보고서(JUnit XML, `go test -json`)를 읽어 결과 트리(통과·실패·건너뜀, 걸린 시간)로 보인다. 실패의 스택을 누르면 그 테스트 줄로 간다(Java·Kotlin·Scala, Kotlin의 빈칸 든 백틱 이름과 공통 루트 패키지를 뺀 폴더 포함).
  - 통합 테스트: `mvn verify`면 Failsafe 통합 테스트까지, `gradle check`면 `integrationTest` 같은 다른 테스트 작업까지 보인다. `gradle integrationTest`·`gradlew :app:test`처럼 이름이 `test`로 끝나는 작업을 돌리는 구성도 테스트 구성으로 고를 수 있다(`gradle build`는 아니다).
  - [실패만 다시]로 실패한 것만 다시 돌린다. Maven은 단위 테스트를 `-Dtest`, 통합 테스트를 `-Dit.test`로 나눠 거르고, Gradle은 실패한 테스트 작업마다 `:app:integrationTest --tests …`처럼 붙인다(`gradle check`로 돌린 구성도 된다). 설정에서 폴더를 따로 적은 프로젝트(`project(":app").projectDir = …`)와 품은 빌드(`includeBuild`)의 테스트도 그 작업 경로로 다시 돌린다.
  - 여러 모듈: Gradle은 `--continue`, Maven은 `-fae`와 `-Dmaven.test.failure.ignore=true`를 붙여 한 모듈(하위 프로젝트)이 실패해도 다른 모듈의 테스트를 돌리고, 보고서에 없는 실패(컴파일이 깨진 모듈 등)는 종료 코드로 알아 결과 위에 알린다.
  - 한글 경로(Windows): 프로젝트 경로나 Gradle 홈의 한글 때문에 Gradle(JDK 18 이상)이 테스트를 돌리지 못하면 까닭과 해법을 알린다. 프로젝트는 영문 경로로 옮기고, Gradle 홈(Windows 사용자 이름이 한글이면 기본 홈 `~/.gradle`도 한글 경로다)은 환경 변수 `GRADLE_USER_HOME`을 영문 경로로 정하거나, 그 프로젝트만 `.ferrite/run.toml`의 실행 구성에 `env = { GRADLE_USER_HOME = "C:/gradle" }`을 적는다. 옮길 수 없으면 JDK 17로 돌리고 빌드 파일의 소스 문자셋을 UTF-8로 적는다.
  - egov 템플릿처럼 pom의 surefire 설정이 `skipTests=true`면(명령 줄로는 켤 수 없다) 건너뛴 까닭을 알리고 [pom.xml에서 테스트 켜기]로 그 줄을 지운다.
- 정적 분석(문제 창 > 정적 분석): SpotBugs·PMD·Semgrep·clang 등이 낸 SARIF 2.1.0을 불러와 파일별로 보이고, 열린 파일에는 물결선으로 보인다. 프로젝트의 `*.sarif`·`*.sarif.json`·`spotbugsSarif.json`을 찾아 고르며, 분석을 다시 돌려 파일이 바뀌면 다시 읽는다. 분석기는 실행 구성으로 돌린다(예: SpotBugs `mvn compile com.github.spotbugs:spotbugs-maven-plugin:spotbugs -Dspotbugs.sarifOutput=true`, PMD `mvn compile pmd:pmd -Dformat=net.sourceforge.pmd.renderers.SarifRenderer`, Semgrep `semgrep scan --sarif --output semgrep.sarif`, clang `clang --analyze --analyzer-output sarif -o clang.sarif 파일.c`. Semgrep 규칙에 한글이 있으면 Windows에서는 `PYTHONUTF8=1`로 돌린다). [egov 룰셋 넣기]는 PMD 7용 egov 표준 점검 룰셋(44개 규칙을 PMD 기본 규칙으로 가리킨다. 우선순위는 공식 룰셋, 메시지는 공식 문서의 한글 설명)을 프로젝트에 `egov-inspection-rules.xml`로 쓰고 pom의 maven-pmd-plugin 설정 `<rulesets>`에 적는다(명령 줄로는 룰셋을 고를 수 없다). 공식 4.3 룰셋은 PMD 7에서 5개 규칙(`EmptyCatchBlock`·`AvoidCatchingGenericException` 등)이 아무것도 잡지 못한다.
- 의존성 취약점(문제 창 > 취약점): Maven `pom.xml`·Gradle·npm `package-lock.json`·PyPI `requirements*.txt`(`==` 고정)·`Cargo.lock`의 의존성을 공개 취약점 DB(OSV)와 견줘 CVE·심각도·고친 버전을 보이고, 의존성 파일의 그 줄에 물결선을 긋는다. Maven·Gradle은 빌드 도구(`mvn dependency:tree`·`gradle dependencies`)로 전이 의존까지 풀어 그것을 끌어온 직접 의존 줄에 보인다. 온라인은 의존성 이름·버전만 api.osv.dev에 보내고, 오프라인은 생태계별 DB 묶음(`all.zip`)을 [받기]하거나 [파일에서…] 들여 읽는다. 들일 때 패키지별 색인(`all.index`)을 만들어, 검사는 프로젝트 의존성의 레코드만 읽는다(npm 220MB 묶음에서 0.03초).
- DB 연결(`도구 > DB 연결`): PostgreSQL·MariaDB·SQLite 연결을 추가·시험한다. 비밀번호는 파일이 아니라 OS 키체인에 둔다. Oracle·Tibero·CUBRID는 JDBC 사이드카(Java)로 연다. JDK(8 이상)가 필요하고, 드라이버 jar는 직접 고르거나 로컬 Maven 저장소·프로젝트의 `WEB-INF/lib`에서 찾은 것을 넣거나 [받기](Oracle 11g용 ojdbc6·orai18n, CUBRID 11.2용)로 받는다. Ferrite는 드라이버를 싣지 않는다.
- 스키마 브라우저(`도구 > 스키마 브라우저`): 고른 연결의 표·뷰와 열(형식)을 보고, 표·열 이름으로 찾고, [SQL 콘솔에서 조회]로 그 표를 조회한다. 콘솔 연결과 따로 연결해 읽는다. Oracle·Tibero·CUBRID는 사용자 동의어와 PUBLIC 동의어(시스템 계정 표를 가리키는 것은 뺀다)도 가리키는 표의 열과 함께 보인다. 스키마를 읽으면 매퍼 XML의 SQL에서 표·열 이름을 자동 완성하고(FROM 뒤에는 표, `별칭.` 뒤에는 그 표의 열), 스키마에 없는 표와 표에 없는 열을 경고한다.
- SQL 콘솔(`도구 > SQL 콘솔`): 고른 글이나 커서가 있는 문장을 실행(`Ctrl+Enter`)하고, 실행 계획·멈추기·자동 커밋·커밋·롤백을 쓴다. 결과 그리드는 스크롤하면 500행씩 더 받고, 10만 행을 넘으면 앞쪽을 버려 100만 행도 메모리가 늘지 않는다. CSV로 내보내기(UTF-8 BOM)는 조회를 다시 실행해 파일로 흘려 쓴다. 한 표에서 기본 키 열을 모두 넣어 조회하면 칸을 두 번 눌러 고친다(기본 키로 찾는 UPDATE, 값은 파라미터로 묶는다). Oracle·Tibero·CUBRID는 드라이버가 결과 열의 출처를 알려 주지 않아 SQL에서 표를 찾으므로, 조인 없는 한 표 조회에서 열 이름을 그대로 쓴 항목만 고친다. Tibero 6은 멈춤 요청 뒤에도 문장을 1분 가까이 돌려, 권한이 있으면 세션을 끊고 다시 연결해 멈춘다.
- 노트북(`.ipynb`): 셀(코드·마크다운·원본)을 고치고 더하고 옮기고 지우며, 저장된 출력(글, PNG 그림, 오류, HTML 요약)을 본다. Jupyter가 쓴 모양 그대로 저장해 고치지 않은 셀은 바이트까지 그대로 남는다. [JSON 보기]로 원래 글을 고친다. 코드 셀은 Jupyter 커널(ipykernel 등)로 실행한다(Shift+Enter, 모두 실행, 멈춤, 커널 다시 시작). Python은 노트북 위 폴더의 `.venv`·`venv`나 PATH에서 찾는다. [DataFrame 보기]로 커널의 pandas DataFrame을 그리드로 본다(보이는 행만 페이지로 받고, 머리글을 눌러 정렬).
- 도킹 창: 프로젝트·에디터·터미널·문제·검색·참조를 탭으로 묶는다. 탭을 끌어 옆·위아래로 나눈다. 배치는 다음 실행 때 되살아난다(`보기 > 창 배치 초기화`).
- Git(`Ctrl+Shift+G`): 거터에 변경 줄 표시(저장 전 편집 포함), 상태바에 브랜치, 소스 제어 창에서 diff·스테이지·커밋·푸시·브랜치 전환(git CLI)
- 통합 터미널(`` Ctrl+` ``): 탭 여러 개, Windows는 ConPTY(PowerShell), 한글 입출력, 256색·트루컬러, 복사(고른 글자)·붙여넣기. 터미널에 포커스가 있으면 셸이 쓰는 Ctrl+글자(Ctrl+D·Ctrl+L 등)는 셸로 간다.
- 단축키(`보기 > 단축키 목록`): 기본(VS Code식)과 Eclipse 배치를 settings.toml의 `keymap = "eclipse"`로 고르고, `[keys]`에 명령마다 키를 적어 바꾼다(예: `"edit.deleteLine" = "Ctrl+D"`). 목록 창에서 지금 키와 겹침을 본다. 줄 지우기·줄 복제·줄로 가기도 있다.
- 에이전트(`도구 > 에이전트`, ACP): 코딩 에이전트(Gemini CLI·Claude Code, settings.toml의 `[[agent]]`로 다른 ACP 에이전트)를 프로젝트 폴더에서 띄워 창에서 대화한다. 권한 요청은 단추로 답하고, 에이전트가 Ferrite에 파일 쓰기를 맡기면 diff를 보고 [받기]를 눌러야 버퍼에 적용한다(저장은 직접). 파일 읽기에는 저장 안 한 버퍼로 답한다. Claude Code는 편집 diff를 권한 요청에 붙여 보이고, 허락하면 디스크에 직접 쓴다(열린 문서는 다시 읽는다). 에이전트가 주는 고르기 설정(권한 모드·모델 등)은 창 위에서 바꾸고, settings.toml의 `agent_mode`(예: `"default"`, 고치기 전에 묻는 모드)로 세션을 열 때의 모드를 정한다. 비우면 에이전트 설정을 따르고, Claude Code 설정의 권한 모드가 `auto`·`acceptEdits`면 묻지 않고 고친다. Ferrite는 에이전트를 설치하거나 로그인하지 않는다.
- AI 에이전트 연결(`도구 > AI 에이전트 연결(MCP)…`): 켜면 이 PC(127.0.0.1)에서 MCP 서버로 듣고, 토큰을 아는 코딩 에이전트(Claude Code 등)가 열린 파일(저장하지 않은 내용 포함)·활성 에디터·진단·디버거 상태를 읽는다. 파일은 바꾸지 않는다. 에이전트에 넣을 명령과 JSON 설정을 창에서 복사하며, settings.toml의 `mcp_server = true`면 앱을 켤 때 함께 켠다.
- EditorConfig: 프로젝트의 `.editorconfig`(들여쓰기·탭 폭·줄바꿈·끝 공백·마지막 줄바꿈)를 따른다. 줄바꿈과 끝 공백은 저장할 때 맞춘다. `charset`은 따르지 않고 CP949 감지·보존을 지금대로 둔다.
- TODO 목록(`편집 > TODO 목록`): 프로젝트의 `TODO`·`FIXME`를 파일에서 찾기로 모아 보이고 누르면 그 줄로 간다.
- 저장 시 작업: `.ferrite/run.toml`의 `[[on_save]]`에 패턴과 명령(`${file}` 등)을 적으면 맞는 파일을 저장할 때 창 없이 돌리고(포매터·Sass 컴파일 등) 결과를 상태바에 알린다.
- 파일로 이동(`Ctrl+P`): 파일 이름 퍼지 검색. 파일에서 찾기(`Ctrl+Shift+F`): 프로젝트 전체 검색(대소문자·단어·정규식·파일 패턴, UTF-8·CP949 파일). 둘 다 `.gitignore`를 따른다.
- 기호: 개요 창(`Ctrl+Shift+O`, 지금 파일의 기호 나무), 기호로 이동(`Ctrl+T`, 떠 있는 언어 서버 모두에서 찾기), 엔드포인트 창(`도구 > 엔드포인트`, 컨트롤러별 URL·HTTP 메서드. Spring Boot LS가 없는 egov 3.x는 내장 egov 서버가 `@RequestMapping` 계열을 모은다. Java 파일을 고치거나 디스크에서 바뀌면 저절로 다시 모은다).
- HTTP 요청 파일(`.http`): `###`로 나눈 요청을 거터 ▶나 `Ctrl+Enter`로 보내고 HTTP 응답 창에서 상태·걸린 시간·헤더·본문을 본다.
  - `{{변수}}`는 파일 변수(`@이름 = 값`), 환경 파일(`http-client.env.json`, 비밀 값은 `http-client.private.env.json`), `{{$uuid}}`·`{{$timestamp}}`로 채운다. 개인 환경 파일이 `.gitignore`에 없으면 알리고 더한다.
  - 쿠키를 앱이 켜진 동안 둔다(egov 로그인 뒤 `.do` 요청). 리다이렉트를 따라가며 거치는 응답의 쿠키도 담는다.
  - JSON은 키 차례 그대로 들여 쓰고, 본문은 응답 문자 집합대로 푼다(없으면 UTF-8, 아니면 EUC-KR).
  - OS 인증서 저장소와 `HTTPS_PROXY`를 따른다. 자체 서명 인증서는 요청 위에 `# @insecure`를 적었을 때만 받는다. `# @no-redirect`·`# @no-cookie-jar`도 있다.
  - 엔드포인트 창의 [요청]은 `http/<컨트롤러>.http`에 그 URL의 요청을 만든다(환경 파일이 없으면 견본도).
  - 본문의 `< ./photo.png` 줄은 그 파일을 바이트 그대로 넣는다(멀티파트 업로드도). 요청 끝의 `>> out/a.json`은 응답 본문을 저장한다(있으면 번호를 붙이고, `>>!`는 덮어쓴다). 요청 뒤의 응답 처리 스크립트(`> {% … %}`, `> ./check.js`)는 응답을 받은 뒤 돌린다: `client.global.set("token", response.body.token)`으로 둔 값을 다음 요청의 `{{token}}`에 쓰고(앱이 켜진 동안), `client.test`·`client.assert`로 응답을 검사해 응답 창의 [스크립트] 탭에 통과·실패를 보인다. 요청 줄 앞의 요청 전 스크립트(`< {% … %}`)는 보내기 전에 돌린다: `request.variables.set("sign", …)`로 그 요청에만 쓰는 값(시각, `crypto.hmac.sha256()` 서명 등)을 정하고, `request.environment.get`으로 환경의 비밀 값을 읽는다.
  - 요청·헤더·`{{변수}}`·JSON 본문을 칠한다.
  - 응답 창의 [기록]에서 앞서 보낸 요청의 응답을 다시 보고 [다시 보내기]·[요청으로 가기]를 쓴다(최근 30개, 앱이 켜진 동안만 두고 디스크에 남기지 않는다).
- 트리 우클릭: 새 파일·새 폴더·이름 바꾸기·휴지통으로 삭제
- 가상화 코드 에디터: 100MB 파일도 스크롤·입력 1ms 미만, 줄 번호, 한글 IME 조합 표시
- 언어 서버(LSP): 에이전트·git·다른 도구가 디스크에서 바꾼 파일은 서버가 등록한 패턴(`workspace/didChangeWatchedFiles`)에 맞춰 알린다. 내장 egov 서버는 바뀐 Java·매퍼 XML을 다시 색인한다. rust-analyzer로 검증했다. gopls·clangd·basedpyright·phpactor·lemminx(XML)·jdt.ls(Java)도 연결한다. JS·TS는 TypeScript 7, HTML·CSS·JSON은 vscode-langservers-extracted를 npm으로 설치한다(npm 필요, HTML·CSS·JSON은 Node 필요). JSON은 스키마로도 검사한다: package.json·tsconfig 등은 내장 스키마(망 없이), 그 밖의 잘 알려진 파일은 SchemaStore 목록(망이 될 때, 인증서를 검증하고 `HTTPS_PROXY`와 OS 인증서 저장소를 따른다), 프로젝트의 JSON은 settings.toml의 `[[json_schema]]`로 잇는다.
  - egov 화면 스크립트: `<script>`로 따로 불러오는 jQuery와 다른 스크립트의 전역 함수는 jsconfig.json이 있어야 안다. 화면 폴더의 `.js`를 열었는데 없으면 에디터 위에서 만들자고 한다.
  - TypeScript 서버는 라이브러리 파일 이름(`jquery-*.js` 등)을 보고 타입 정보(`@types`)를 npm으로 사용자 캐시(Windows `%LOCALAPPDATA%\Microsoft\TypeScript\7.0`)에 받는다(VS Code와 같다). 인터넷이 막힌 PC는 프로젝트의 `node_modules/@types`에 넣는다.
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
  - 서버가 하는 일(jdt.ls의 Maven 가져오기, rust-analyzer의 색인, gopls의 패키지 읽기 등)의 진행을 상태바에 보인다(예: `rust-analyzer: Indexing 5/21 (core) 23%…`). 그동안은 자동완성·코드 액션 등이 빈 결과를 줄 수 있다. rust-analyzer는 진행 알림이 없어도 작업 공간을 읽는 동안 "색인 중…"으로 보인다.
  - 인터넷이 막힌 PC는 다른 PC에서 받은 파일로 설치한다("파일에서…").
  - 진단: 물결 밑줄, 말풍선, 문제 목록(상태바의 "오류 N · 경고 M" 클릭)
  - 자동완성 팝업, 호버 정보
  - 정의로 이동, 참조 목록
  - 문서 포맷, 기호 이름 바꾸기(여러 파일), 코드 액션(빠른 수정·리팩터링)
- egov CRUD 코드 생성(`도구 > egov CRUD 코드 생성`): DDL을 붙여 넣으면 VO·Mapper(인터페이스·XML)·Service·ServiceImpl·Controller·JSP·테스트를 만든다.
  - egov 3.x·4.x, Oracle·Tibero·PostgreSQL·MariaDB·CUBRID SQL. 매퍼 위치·이름, JSP 폴더, 인코딩(UTF-8·MS949)은 프로젝트에서 찾는다.
  - 공식 VS Code 확장과 같은 템플릿 문맥이라, 공식·직접 고친 Handlebars 템플릿을 `.ferrite/templates/code/`에 두면 그것으로 만든다. `.hbs` 템플릿은 내장 Handlebars 강조로 열린다.
- egov 설정 파일 생성(`도구 > egov 설정 파일 생성`): Datasource·Transaction·ID 생성·Logging(log4j2)·Scheduling(Quartz)·Cache(Ehcache)·Property 21종을 XML·Java Config·YAML·properties로 만든다.
  - 만든 설정은 실제 Spring(egov 3.x·4.x)에서 띄워 확인했다. DB별 기본값과 프로젝트의 `${Globals.…}` 키를 쓴다.
- 새 egov 프로젝트(`도구 > 새 egov 프로젝트`): 공식 VS Code 확장 v5.0.7의 템플릿 22종(웹·템플릿·Boot·MSA·모바일·배치·AI RAG, egov 5)으로 프로젝트를 만든다.
  - 템플릿은 처음 한 번 GitHub에서 받는다(크기·SHA-256 고정). 인터넷이 막힌 PC는 공식 vsix나 받아 온 파일로 만든다.
  - 만든 프로젝트를 빌드하려면 Java 17 이상과 Maven이 필요하다.
- 공통컴포넌트 추가(`도구 > 공통컴포넌트 추가`): 공식 개발환경(eGovFrameDev 3.x·4.x)의 공통컴포넌트 플러그인에서 목록을 가져와, 고른 컴포넌트를 의존 컴포넌트와 함께 프로젝트에 넣는다.
  - 미리 보기로 바뀔 파일을 먼저 보여 준다. 내용이 다른 기존 파일은 건너뛰거나 `.bak`으로 백업하고 바꾼다. `pom.xml`은 덮어쓰지 않고 빠진 의존성을 알린다.
  - 프로젝트 DB에 돌릴 테이블 생성·기초 자료 스크립트를 순서대로 알린다.
- 구문 강조(tree-sitter): Rust, Python, Go, C/C++, PHP, Java, SQL, TOML, JSON, YAML, XML, HTML, JSP(HTML + 스크립틀릿 Java + EL), JavaScript, TypeScript·TSX, CSS, Properties
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
| 기호로 이동 / 개요 | `Ctrl+T` / `Ctrl+Shift+O` |
| HTTP 요청 보내기(`.http`) | `Ctrl+Enter` |
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
