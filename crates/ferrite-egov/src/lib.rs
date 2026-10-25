//! Ferrite egov 언어 서버. 오픈소스 언어 서버가 다루지 못하는 egov 영역(MyBatis 매퍼, JSP 출력의
//! XSS 등)을 맡는다.
//! Ferrite에는 `ferrite --egov-ls`로 내장되고, 다른 에디터는 `ferrite-egov-ls`를 LSP 서버로 쓸 수 있다.

pub mod java;
pub mod jsp;
pub mod mapper;
pub mod server;
pub mod taglib;
pub mod web;

pub use server::run_stdio;
