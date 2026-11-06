use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use ferrite_core::buffer::Selection;
use serde_json::Value;

use super::*;

fn grammars(list: &[(&str, &str)]) -> Arc<Grammar> {
    let map: HashMap<String, Value> = list
        .iter()
        .map(|(scope, json)| (scope.to_string(), serde_json::from_str(json).unwrap()))
        .collect();
    Arc::new(compile_grammar(list[0].0, &map).unwrap())
}

fn grammar(json: &str) -> Arc<Grammar> {
    grammars(&[("source.t", json)])
}

/// 줄마다 (강조한 글, 분류).
fn spans(grammar: Arc<Grammar>, text: &str) -> Vec<Vec<(String, Highlight)>> {
    let rope = Rope::from_str(text);
    let mut syntax = TextMateSyntax::new(grammar);
    named(&rope, syntax.highlight_lines(&rope, 0..rope.len_lines()), 0)
}

fn named(rope: &Rope, lines: LineHighlights, first: usize) -> Vec<Vec<(String, Highlight)>> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let line = rope.line(first + i).to_string();
            row.into_iter()
                .map(|(r, h)| (line[r].to_owned(), h))
                .collect()
        })
        .collect()
}

fn s(text: &str, kind: Highlight) -> (String, Highlight) {
    (text.to_owned(), kind)
}

use Highlight::{Comment, Constant, Function, Keyword, Property, Tag, Type};
const STR: Highlight = Highlight::String;

#[test]
fn 범위_이름을_분류로_바꾼다() {
    assert_eq!(classify("keyword.control.js"), Some(Keyword));
    assert_eq!(classify("comment.line.double-slash.t"), Some(Comment));
    assert_eq!(classify("keyword.operator.assignment"), None);
    assert_eq!(classify("keywordish"), None);
    assert_eq!(
        classify("string.quoted.double punctuation.definition.string"),
        Some(STR)
    );
    assert_eq!(
        classify("meta.block constant.character.escape.js"),
        Some(Constant)
    );
    assert_eq!(classify("entity.name.function.js"), Some(Function));
    assert_eq!(classify("storage.type.t"), Some(Keyword));
    assert_eq!(classify("support.class.builtin"), Some(Type));
    assert_eq!(classify("entity.name.tag.html"), Some(Tag));
    assert_eq!(classify("entity.other.attribute-name.html"), Some(Property));
}

const BASIC: &str = r##"{"scopeName": "source.t", "patterns": [
    {"match": "\\b(if|else)\\b", "name": "keyword.control.t"},
    {"match": "(fn)\\s+(\\w+)", "captures": {"1": {"name": "storage.type.t"}, "2": {"name": "entity.name.function.t"}}},
    {"begin": "\"", "end": "\"", "name": "string.quoted.t",
     "patterns": [{"match": "\\\\.", "name": "constant.character.escape.t"}]},
    {"begin": "/\\*", "end": "\\*/", "name": "comment.block.t"},
    {"include": "#number"}
], "repository": {"number": {"match": "\\d+", "name": "constant.numeric.t"}}}"##;

#[test]
fn match_캡처_begin_end와_저장소() {
    let got = spans(grammar(BASIC), "fn main if \"a\\\"b\" 42 /* x\ny */ else\n");
    assert_eq!(
        got[0],
        [
            s("fn", Keyword),
            s("main", Function),
            s("if", Keyword),
            s("\"a", STR),
            s("\\\"", Constant),
            s("b\"", STR),
            s("42", Constant),
            s("/* x", Comment),
        ]
    );
    assert_eq!(got[1], [s("y */", Comment), s("else", Keyword)]);
}

#[test]
fn end의_뒤_참조는_begin에서_찾은_글로_채운다() {
    let g = grammar(
        r##"{"patterns": [
            {"begin": "<<(\\w+)$", "end": "^\\1$", "name": "string.unquoted.heredoc"},
            {"match": "\\bif\\b", "name": "keyword"}
        ]}"##,
    );
    let got = spans(g.clone(), "<<EOT\nhello EOT x\nEOT\nif\n");
    assert_eq!(got[1], [s("hello EOT x", STR)]);
    assert_eq!(got[2], [s("EOT", STR)]);
    assert_eq!(got[3], [s("if", Keyword)]);
    // 이스케이프: 정규식 글자도 글자 그대로 맞춘다.
    let got = spans(g, "<<A\na.b\nAx\nA\nif\n");
    assert_eq!(got[2], [s("Ax", STR)]);
    assert_eq!(got[4], [s("if", Keyword)]);
}

#[test]
fn while는_줄마다_확인하고_맞지_않으면_끝난다() {
    let g = grammar(r##"{"patterns": [{"begin": "^>", "while": "^>", "name": "comment.quote"}]}"##);
    let got = spans(g, "> a\n> b\nc\n");
    assert_eq!(got[0], [s("> a", Comment)]);
    assert_eq!(got[1], [s("> b", Comment)]);
    assert!(got[2].is_empty());
}

#[test]
fn applyendpatternlast면_안쪽_패턴이_먼저다() {
    let rule = |last: bool| {
        format!(
            r##"{{"patterns": [{{"begin": "\\(", "end": "\\)", "applyEndPatternLast": {last},
                "patterns": [{{"match": "\\)\\)", "name": "keyword"}}]}}]}}"##
        )
    };
    let got = spans(grammar(&rule(true)), "(x)) y\n");
    assert_eq!(got[0], [s("))", Keyword)]);
    let got = spans(grammar(&rule(false)), "(x)) y\n");
    assert!(got[0].is_empty());
}

#[test]
fn 다른_문법과_돌아_부르는_저장소() {
    let g = grammars(&[
        (
            "source.a",
            r##"{"patterns": [
                {"begin": "\\[", "end": "\\]", "patterns": [{"include": "source.b"}]},
                {"begin": "\\{", "end": "\\}", "patterns": [{"include": "source.b#num"}]},
                {"include": "#a"}
            ], "repository": {
                "a": {"patterns": [{"include": "#b"}, {"match": "x", "name": "keyword"}]},
                "b": {"patterns": [{"include": "#a"}]}
            }}"##,
        ),
        (
            "source.b",
            r##"{"patterns": [{"match": "b", "name": "keyword.b"}],
               "repository": {"num": {"match": "\\d", "name": "constant.numeric"}}}"##,
        ),
    ]);
    let got = spans(g, "b [b] {1b} x\n");
    assert_eq!(got[0], [s("b", Keyword), s("1", Constant), s("x", Keyword)]);
}

#[test]
fn 빈_매치와_잘못된_정규식에서_멈추지_않는다() {
    let g = grammar(
        r##"{"patterns": [
            {"match": "(?=a)", "name": "keyword"},
            {"begin": "(?=b)", "end": "(?=b)", "name": "string"},
            {"match": "(", "name": "comment"},
            {"match": "c", "name": "constant"}
        ]}"##,
    );
    let got = spans(g, "aabbc\n");
    assert_eq!(got[0].last(), Some(&s("c", Constant)));
}

#[test]
fn 편집한_줄부터_다시_센다() {
    let text = "/*\nx\n*/\ny\n";
    let mut buffer = Buffer::new(text);
    buffer.set_track_syntax_edits(true);
    let mut syntax = TextMateSyntax::new(grammar(BASIC));
    // 보이는 줄만 물어도 앞 줄의 쌓임을 먼저 센다.
    let got = named(
        buffer.rope(),
        syntax.highlight_lines(buffer.rope(), 1..2),
        1,
    );
    assert_eq!(got[0], [s("x", Comment)]);
    buffer.set_selection(Selection { anchor: 0, head: 2 });
    buffer.insert("", Instant::now());
    syntax.update(&mut buffer);
    let got = named(
        buffer.rope(),
        syntax.highlight_lines(buffer.rope(), 0..3),
        0,
    );
    assert!(got[1].is_empty(), "{got:?}");
}

#[test]
fn 긴_줄은_강조하지_않지만_쌓임은_넘긴다() {
    let long = "x".repeat(MAX_LINE + 1);
    let got = spans(grammar(BASIC), &format!("/* a\n{long}\nb */ if\n"));
    assert_eq!(got[0], [s("/* a", Comment)]);
    assert!(got[1].is_empty());
    assert_eq!(got[2], [s("b */", Comment), s("if", Keyword)]);
}

#[test]
fn jsonc의_주석과_끝_쉼표를_뺀다() {
    let text = "\u{feff}{\n // 주석\n \"a\": \"http://x/*y*/\", /* 블록 */\n \"b\": [1, 2,],\n \"c\": \"\\\"//\",\n}";
    let v: Value = serde_json::from_str(&extension::strip_jsonc(text)).unwrap();
    assert_eq!(v["a"], "http://x/*y*/");
    assert_eq!(v["b"], serde_json::json!([1, 2]));
    assert_eq!(v["c"], "\"//");
}

#[test]
fn 확장_폴더를_읽어_언어를_고른다() {
    let root = std::env::temp_dir().join(format!("ferrite-textmate-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "ext-a/package.json",
        r##"{"name": "ext-a", "contributes": {
            "languages": [{"id": "foo", "aliases": ["Foo 언어", "foo"], "extensions": [".foo", ".Foo.TXT"],
                           "filenames": ["Foofile"], "configuration": "./language-configuration.json"}],
            "grammars": [
                {"language": "foo", "scopeName": "source.foo", "path": "./syntaxes/foo.tmLanguage.json"},
                {"scopeName": "source.bad", "path": "./syntaxes/bad.tmLanguage"},
                {"scopeName": "source.out", "path": "../outside.json"}
            ]}}"##,
    );
    write(
        "ext-a/language-configuration.json",
        "{\n  // 주석\n  \"comments\": { \"lineComment\": \"#\", \"blockComment\": [\"/*\", \"*/\"] },\n  \"brackets\": [[\"{\", \"}\"], [\"(\", \")\"], [\"begin\", \"end\"],],\n}\n",
    );
    write(
        "ext-a/syntaxes/foo.tmLanguage.json",
        r##"{"scopeName": "source.foo", "patterns": [{"match": "\\bfoo\\b", "name": "keyword.foo"}]}"##,
    );
    write("ext-b/package.json", "{ 깨짐");
    write(
        "ext-c/package.json",
        r##"{"contributes": {"languages": [{"id": "foo", "extensions": [".bar"]}]}}"##,
    );
    let (registry, errors) = Registry::load(&root);
    assert_eq!(registry.len(), 1);
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors[0].contains("bad.tmLanguage") && errors[0].contains("JSON 문법"));
    assert!(errors[1].contains("확장 폴더 밖"));
    assert!(errors[2].contains("ext-b") && errors[2].contains("package.json"));

    let foo = registry.language_for(Path::new("/p/a.foo")).unwrap();
    assert_eq!(foo.name, "Foo 언어");
    assert_eq!(foo.line_comment.as_deref(), Some("#"));
    assert_eq!(foo.brackets, [('{', '}'), ('(', ')')]);
    for name in ["X.FOO.TXT", "Foofile", "a.bar"] {
        assert!(registry.language_for(Path::new(name)).is_some(), "{name}");
    }
    assert!(registry.language_for(Path::new("a.txt")).is_none());
    let got = spans(foo.grammar().unwrap(), "foo bar\n");
    assert_eq!(got[0], [s("foo", Keyword)]);
    std::fs::remove_dir_all(&root).unwrap();
    assert!(Registry::load(&root).0.is_empty());
}

#[test]
fn g_닻은_begin이_끝난_자리에서만_맞는다() {
    let g = grammar(
        r##"{"patterns": [
            {"begin": "a", "end": "(?=;)", "patterns": [{"match": "\\Gb", "name": "keyword"}]},
            {"begin": "x\\n", "end": "^;", "patterns": [{"match": "\\Gy", "name": "constant"}]}
        ]}"##,
    );
    let got = spans(g.clone(), "ab b;\n");
    assert_eq!(got[0], [s("b", Keyword)]);
    // begin이 줄 끝까지 맞으면 그 규칙 안의 줄 처음이 닻이다(VS Code와 같다). 줄 처음이 아니면 아니다.
    let got = spans(g, "x\ny\nzy\n;\n");
    assert_eq!(got[1], [s("y", Constant)]);
    assert!(got[2].is_empty());
}

#[test]
fn 앞_줄을_다_세지_못하면_다음에_이어_센다() {
    let text = format!("/*\n{}*/ if\n", "x\n".repeat(200));
    let rope = Rope::from_str(&text);
    let mut syntax = TextMateSyntax::new(grammar(BASIC));
    syntax.budget = Duration::ZERO;
    let last = rope.len_lines() - 2;
    let mut calls = 0;
    let got = loop {
        calls += 1;
        let got = syntax.highlight_lines(&rope, last..last + 1);
        if !syntax.is_pending() {
            break got;
        }
        assert!(got[0].is_empty());
    };
    assert!(calls > 1);
    assert_eq!(
        named(&rope, got, last)[0],
        [s("*/", Comment), s("if", Keyword)]
    );
}

/// 마크다운 문법의 펜스 코드 블록과 같은 모양: 뒤 참조로 닫는 펜스, 그 안의 begin/while, 다른 문법.
#[test]
fn 펜스_코드_블록은_안쪽_문법으로_칠하고_같은_펜스로_닫는다() {
    let g = grammars(&[
        (
            "text.md",
            r##"{"patterns": [
                {"begin": "(^|\\G)(\\s*)(`{3,}|~{3,})\\s*(?i:(rust)((\\s+|:|,|\\{|\\?)[^`]*)?$)",
                 "end": "(^|\\G)(\\2|\\s{0,3})(\\3)\\s*$",
                 "beginCaptures": {"4": {"name": "entity.name.function"}},
                 "patterns": [{"begin": "(^|\\G)(\\s*)(.*)", "while": "(^|\\G)(?!\\s*([`~]{3,})\\s*$)",
                               "contentName": "meta.embedded.block.rust",
                               "patterns": [{"include": "source.rust"}]}]},
                {"match": "^#+ .*", "name": "markup.heading"}
            ]}"##,
        ),
        (
            "source.rust",
            r##"{"patterns": [{"match": "\\bfn\\b", "name": "keyword"}]}"##,
        ),
    ]);
    let got = spans(g, "```rust\nfn a\nfn b\n```\n# after\n");
    assert_eq!(got[0], [s("rust", Function)]);
    assert_eq!(got[1], [s("fn", Keyword)]);
    assert_eq!(got[2], [s("fn", Keyword)]);
    assert!(got[3].is_empty());
    assert_eq!(got[4], [s("# after", Keyword)]);
}

#[test]
fn 내장_http_문법으로_요청_파일을_칠한다() {
    let registry = Registry::default();
    let http = registry.language_for(Path::new("api/board.REST")).unwrap();
    assert_eq!(
        (http.name.as_str(), http.line_comment.as_deref()),
        ("HTTP 요청", Some("#"))
    );
    let got = spans(
        http.grammar().unwrap(),
        "@host = {{baseUrl}}/app\n\
         ### 글 저장\n\
         # @name save\n\
         POST {{host}}/save.do HTTP/1.1\n\
         \x20   &size=10\n\
         Content-Type: application/json\n\
         \n\
         {\"title\": \"{{$uuid}}\", \"n\": 1.5, \"ok\": true}\n\
         < ./a.png\n\
         > {%\n\
         client.global.set(\"t\", 1); // 토큰\n\
         %}\n\
         >>! out/r.json\n\
         ###\n\
         http://localhost/ping\n\
         \n\
         Accept: x\n",
    );
    assert_eq!(got[0][0], s("@host", Property));
    assert!(got[0].contains(&s("{{baseUrl}}", Tag)));
    assert_eq!(got[1], [s("### 글 저장", Keyword)]);
    assert_eq!(
        got[2],
        [s("# ", Comment), s("@name", Keyword), s(" save", Comment)]
    );
    assert_eq!(
        got[3],
        [
            s("POST", Keyword),
            s("{{host}}", Tag),
            s("/save.do ", STR),
            s("HTTP/1.1", Constant)
        ]
    );
    assert_eq!(got[4], [s("&size=10", STR)], "질의 이어 쓰기");
    assert_eq!(got[5], [s("Content-Type", Property)]);
    assert_eq!(
        got[7],
        [
            s("\"title\"", Property),
            s("\"", STR),
            s("{{$uuid}}", Tag),
            s("\"", STR),
            s("\"n\"", Property),
            s("1.5", Constant),
            s("\"ok\"", Property),
            s("true", Constant)
        ]
    );
    assert_eq!(got[8], [s("<", Keyword), s("./a.png", STR)]);
    assert_eq!(got[9], [s(">", Keyword), s("{%", Keyword)], "응답 스크립트");
    assert_eq!(
        got[10],
        [
            s("client", Keyword),
            s("set", Function),
            s("\"t\"", STR),
            s("1", Constant),
            s("// 토큰", Comment)
        ]
    );
    assert_eq!(got[11], [s("%}", Keyword)]);
    assert_eq!(got[12], [s(">>!", Keyword), s("out/r.json", STR)]);
    assert_eq!(got[13], [s("###", Keyword)]);
    assert_eq!(
        got[14],
        [s("http://localhost/ping", STR)],
        "메서드 없는 요청 줄"
    );
    assert!(got[16].is_empty(), "빈 줄 뒤는 본문이라 헤더가 아니다");

    let got = spans(
        http.grammar().unwrap(),
        "< {% request.variables.set(\"a\", 1) %}\n< ./pre.js\nGET /a\n",
    );
    assert_eq!(
        got[0],
        [
            s("<", Keyword),
            s("{%", Keyword),
            s("set", Function),
            s("\"a\"", STR),
            s("1", Constant),
            s("%}", Keyword)
        ],
        "요청 전 스크립트는 요청 줄이 아니다"
    );
    assert_eq!(got[1], [s("<", Keyword), s("./pre.js", STR)]);
    assert_eq!(got[2], [s("GET", Keyword), s("/a", STR)]);
}

#[test]
fn 내장_handlebars_문법으로_템플릿을_칠한다() {
    // 확장이 없어도 `.hbs`는 내장 언어로 보고, 확장 수에는 세지 않는다.
    let registry = Registry::default();
    assert!(registry.is_empty());
    let hbs = registry
        .language_for(Path::new("templates/controller.HBS"))
        .unwrap();
    assert_eq!(hbs.name, "Handlebars");
    assert!(registry.language_for(Path::new("a.java")).is_none());
    let got = spans(
        hbs.grammar().unwrap(),
        "{{!-- 주석 --}}<div class=\"x\">{{#if user.name}}{{camelCase name \"x\" 3}}{{/if}}</div>\n\
         public class {{className}}Controller {}\n",
    );
    assert_eq!(got[0][0], s("{{!-- 주석 --}}", Comment));
    assert!(got[0].contains(&s("div", Tag)));
    assert!(got[0].contains(&s("#if", Keyword)));
    assert!(got[0].contains(&s("/if", Keyword)));
    assert!(got[0].contains(&s("user.name", Property)));
    assert!(got[0].contains(&s("camelCase", Function)));
    assert!(
        got[0].contains(&s("name", Property)),
        "도우미의 인자는 함수가 아니다: {got:?}"
    );
    assert!(got[0].contains(&s("\"x\"", Highlight::String)));
    assert!(got[0].contains(&s("3", Constant)));
    assert!(got[1].contains(&s("className", Property)));
    assert!(got[1].contains(&s("{{", Tag)));
}
