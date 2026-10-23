# Ferrite

Rust로 작성한 크로스플랫폼(Windows / macOS / Linux) 경량 IDE.
언어 지능은 LSP와 DAP 같은 오픈 표준과 오픈소스 언어 서버에 위임한다.

- 개발 워크플로우와 로드맵: [docs/WORKFLOW.md](docs/WORKFLOW.md)

## 현재 상태: Phase 1 (에디터 코어)

- 폴더 열기, 지연 로딩 프로젝트 트리(파일 감시로 자동 갱신), 탭 에디터
- 가상화 코드 에디터: 100MB 파일도 스크롤·입력 1ms 미만, 줄 번호, 한글 IME 조합 표시
- 구문 강조(tree-sitter): Rust, Python, Go, C/C++, PHP, Java, SQL, TOML, JSON, YAML, XML, HTML, JavaScript, CSS
- UTF-8(BOM 포함)·CP949 인코딩과 CRLF 감지·보존, 들여쓰기 방식 감지
- 원자적 저장, 미저장 변경 확인(탭 닫기, 창 종료), 외부 변경 시 다시 불러오기·충돌 안내
- 명령줄: `ferrite <폴더>` 또는 `ferrite <파일>`

### 단축키 (macOS는 Ctrl 대신 Cmd)

| 동작 | 키 |
|---|---|
| 폴더 열기 / 저장 / 탭 닫기 | `Ctrl+O` / `Ctrl+S` / `Ctrl+W` |
| 찾기 / 바꾸기 / 다음·이전 | `Ctrl+F` / `Ctrl+H` / `F3`·`Shift+F3` |
| 실행 취소 / 다시 실행 | `Ctrl+Z` / `Ctrl+Y`, `Ctrl+Shift+Z` |
| 줄 주석 토글 | `Ctrl+/` |
| 줄 위·아래로 이동 | `Alt+↑` / `Alt+↓` |
| 들여쓰기 / 내어쓰기 | `Tab` / `Shift+Tab` |
| 단어 단위 이동·삭제 | `Ctrl+←/→`, `Ctrl+Backspace` (macOS는 `Option`) |

### 설정

`파일 > 설정 열기`를 누르면 `settings.toml`이 만들어지고 에디터로 열립니다. 저장하면 바로 적용됩니다.
- 위치: Windows `%APPDATA%\ferrite`, macOS `~/Library/Application Support/ferrite`, Linux `~/.config/ferrite`
- 항목: 글꼴 크기, 테마(system/dark/light), 탭 크기, 기본 들여쓰기

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
