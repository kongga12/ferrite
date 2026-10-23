//! egov 언어 서버 단독 실행 파일(다른 에디터용). Ferrite는 `ferrite --egov-ls`로 같은 서버를 띄운다.

fn main() -> std::io::Result<()> {
    ferrite_egov::run_stdio()
}
