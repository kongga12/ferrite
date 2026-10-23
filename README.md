# Ferrite

Rust로 작성한 크로스플랫폼(Windows / macOS / Linux) 경량 IDE.
언어 지능은 LSP와 DAP 같은 오픈 표준과 오픈소스 언어 서버에 위임한다.

- 개발 워크플로우와 로드맵: [docs/WORKFLOW.md](docs/WORKFLOW.md)

## 현재 상태: Phase 0 (스켈레톤)

- 폴더 열기, 지연 로딩 프로젝트 트리, 탭 에디터
- UTF-8(BOM 포함), CP949 인코딩과 CRLF 감지·보존. 상태바에서 저장 인코딩과 줄바꿈 변경 가능
- 원자적 저장, 미저장 변경 확인(탭 닫기, 창 종료)
- 단축키: 폴더 열기 `Ctrl/Cmd+O`, 저장 `Ctrl/Cmd+S`, 탭 닫기 `Ctrl/Cmd+W`
- 명령줄: `ferrite <폴더>`로 바로 프로젝트 열기

## 빌드

공통으로 [rustup](https://rustup.rs)으로 stable 툴체인을 설치한다.

| OS | 추가 준비 |
|---|---|
| Windows | Visual Studio Build Tools(C++ 워크로드) |
| macOS | `xcode-select --install` |
| Linux (Debian/Ubuntu) | `sudo apt-get install libgtk-3-dev libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev` |

```bash
cargo run --release -- .
```

```bash
cargo test
```

## 라이선스

MIT OR Apache-2.0
