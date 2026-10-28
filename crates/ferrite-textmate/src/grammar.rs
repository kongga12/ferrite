//! TextMate 문법(`.tmLanguage.json`)을 규칙 배열로 옮긴다. `include`는 규칙 번호로 풀고, 다른
//! 문법(`source.x`, `source.x#이름`)은 등록된 것이면 같은 배열에 함께 옮긴다. 정규식은 처음 쓸 때
//! 컴파일한다(큰 문법은 규칙이 수천 개다).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use ferrite_core::syntax::Highlight;
use onig::{Regex, RegexOptions, Syntax};
use serde_json::{Map, Value};

use crate::classify;

pub(crate) type RuleId = usize;

/// 처음 쓸 때 컴파일하는 정규식. 컴파일하지 못하면 맞지 않는 것으로 친다.
pub(crate) struct Re {
    source: String,
    compiled: OnceLock<Option<Regex>>,
    /// `\G`가 맞지 않게 바꾼 것(닻 자리가 아닌 곳에서 찾을 때).
    no_anchor: OnceLock<Option<Regex>>,
}

impl Re {
    fn new(source: &str) -> Self {
        Self {
            source: source.to_owned(),
            compiled: OnceLock::new(),
            no_anchor: OnceLock::new(),
        }
    }

    /// `anchor`: 찾기 시작하는 자리가 닻(바로 앞 begin이 끝난 자리)인지. 아니면 `\G`가 맞지 않는다.
    pub(crate) fn get(&self, anchor: bool) -> Option<&Regex> {
        if anchor || !self.anchored() {
            self.compiled.get_or_init(|| compile(&self.source))
        } else {
            self.no_anchor
                .get_or_init(|| compile(&without_anchor(&self.source)))
        }
        .as_ref()
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    /// begin의 캡처를 가리키는 뒤 참조(`\1`)가 있다(end·while).
    pub(crate) fn has_backrefs(&self) -> bool {
        backrefs(&self.source)
    }

    /// 검색 시작 위치에 매이는 `\G`가 있다. 이런 정규식은 앞서 찾은 결과를 다시 쓰지 못한다.
    pub(crate) fn anchored(&self) -> bool {
        self.source.contains("\\G")
    }
}

pub(crate) fn compile(source: &str) -> Option<Regex> {
    Regex::with_options(
        source,
        RegexOptions::REGEX_OPTION_CAPTURE_GROUP,
        Syntax::oniguruma(),
    )
    .ok()
}

/// `\G`를 글에 나오지 않는 글자(U+FFFF)로 바꾼다. 검색은 늘 지금 자리에서 시작하므로 `\G`는 그 자리에
/// 맞는데, TextMate에서는 닻 자리에서만 맞아야 한다.
pub(crate) fn without_anchor(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('G') => out.push_str("\\x{FFFF}"),
            Some(n) => {
                out.push(c);
                out.push(n);
            }
            None => out.push(c),
        }
    }
    out
}

/// `\1`~`\9`가 있는지(`\\1`처럼 백슬래시 자체를 이스케이프한 것은 빼고).
fn backrefs(source: &str) -> bool {
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.next().is_some_and(|n| n.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// (캡처 번호, 분류). 분류가 없는 캡처는 넣지 않는다(바깥 분류를 그대로 둔다).
pub(crate) type Captures = Vec<(usize, Highlight)>;

pub(crate) enum Kind {
    Match {
        re: Re,
        captures: Captures,
    },
    Begin {
        begin: Re,
        /// end, 또는 `while`이면 while.
        end: Re,
        is_while: bool,
        begin_captures: Captures,
        end_captures: Captures,
        content: Option<Highlight>,
        /// end보다 안쪽 패턴을 먼저 본다(`applyEndPatternLast`).
        end_last: bool,
    },
    /// 패턴만 담은 규칙(저장소 항목, 문법 뿌리). 찾을 때는 펼쳐서 쓴다.
    Patterns,
}

pub(crate) struct Rule {
    pub(crate) name: Option<Highlight>,
    pub(crate) kind: Kind,
    patterns: Vec<RuleId>,
    /// `patterns`를 펼친 것(패턴만 담은 규칙은 그 안의 규칙으로, 순서대로, 중복 없이).
    pub(crate) flat: Vec<RuleId>,
}

pub struct Grammar {
    pub(crate) rules: Vec<Rule>,
    pub(crate) root: RuleId,
}

/// `grammars`(scopeName → 문법 JSON)에서 `scope` 문법을 옮긴다.
pub fn compile_grammar(scope: &str, grammars: &HashMap<String, Value>) -> Result<Grammar, String> {
    let mut c = Compiler {
        grammars,
        rules: Vec::new(),
        memo: HashMap::new(),
        roots: HashMap::new(),
    };
    let root = c
        .grammar(scope, None)
        .ok_or_else(|| format!("문법이 없습니다: {scope}"))?;
    let flats: Vec<Vec<RuleId>> = (0..c.rules.len())
        .map(|id| {
            let mut flat = Vec::new();
            flatten(
                &c.rules,
                &c.rules[id].patterns,
                &mut flat,
                &mut HashSet::new(),
            );
            flat
        })
        .collect();
    for (rule, flat) in c.rules.iter_mut().zip(flats) {
        rule.flat = flat;
    }
    Ok(Grammar {
        rules: c.rules,
        root,
    })
}

fn flatten(rules: &[Rule], list: &[RuleId], out: &mut Vec<RuleId>, seen: &mut HashSet<RuleId>) {
    for &id in list {
        if !seen.insert(id) {
            continue;
        }
        match rules[id].kind {
            Kind::Patterns => flatten(rules, &rules[id].patterns, out, seen),
            _ => out.push(id),
        }
    }
}

struct Compiler<'a> {
    grammars: &'a HashMap<String, Value>,
    rules: Vec<Rule>,
    /// 이미 옮긴 규칙(JSON 위치 → 번호). 저장소 항목은 서로를 부를 수 있다.
    memo: HashMap<*const Value, RuleId>,
    /// 옮긴 문법의 뿌리.
    roots: HashMap<String, RuleId>,
}

/// 규칙을 옮기는 문맥: 저장소 사슬(안쪽이 뒤)과 `$self`·`$base`.
#[derive(Clone)]
struct Ctx<'a> {
    repos: Vec<&'a Map<String, Value>>,
    self_root: RuleId,
    base_root: RuleId,
}

fn text<'v>(obj: &'v Map<String, Value>, key: &str) -> Option<&'v str> {
    obj.get(key).and_then(Value::as_str)
}

fn captures(value: Option<&Value>) -> Captures {
    let mut out: Captures = value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.parse().ok()?, classify(text(v.as_object()?, "name")?)?)))
        .collect();
    out.sort_by_key(|(i, _)| *i);
    out
}

impl<'a> Compiler<'a> {
    fn reserve(&mut self) -> RuleId {
        self.rules.push(Rule {
            name: None,
            kind: Kind::Patterns,
            patterns: Vec::new(),
            flat: Vec::new(),
        });
        self.rules.len() - 1
    }

    fn grammar(&mut self, scope: &str, base: Option<RuleId>) -> Option<RuleId> {
        if let Some(&id) = self.roots.get(scope) {
            return Some(id);
        }
        let grammar = self.grammars.get(scope)?.as_object()?;
        let id = self.reserve();
        self.roots.insert(scope.to_owned(), id);
        let ctx = Ctx {
            repos: grammar
                .get("repository")
                .and_then(Value::as_object)
                .into_iter()
                .collect(),
            self_root: id,
            base_root: base.unwrap_or(id),
        };
        self.rules[id].patterns = self.patterns(grammar.get("patterns"), &ctx);
        Some(id)
    }

    fn patterns(&mut self, list: Option<&'a Value>, ctx: &Ctx<'a>) -> Vec<RuleId> {
        list.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| self.rule(v, ctx))
            .collect()
    }

    fn rule(&mut self, value: &'a Value, ctx: &Ctx<'a>) -> Option<RuleId> {
        let obj = value.as_object()?;
        if let Some(include) = text(obj, "include") {
            return self.include(include, ctx);
        }
        if let Some(&id) = self.memo.get(&(value as *const Value)) {
            return Some(id);
        }
        let id = self.reserve();
        self.memo.insert(value as *const Value, id);
        let mut inner = ctx.clone();
        inner
            .repos
            .extend(obj.get("repository").and_then(Value::as_object));
        let patterns = self.patterns(obj.get("patterns"), &inner);
        let shared = obj.get("captures");
        let kind = match (text(obj, "match"), text(obj, "begin")) {
            (Some(re), _) => Kind::Match {
                re: Re::new(re),
                captures: captures(shared),
            },
            (None, Some(begin)) => match (text(obj, "end"), text(obj, "while")) {
                (Some(end), _) | (None, Some(end)) => {
                    let is_while = !obj.contains_key("end");
                    let end_key = if is_while {
                        "whileCaptures"
                    } else {
                        "endCaptures"
                    };
                    Kind::Begin {
                        begin: Re::new(begin),
                        end: Re::new(end),
                        is_while,
                        begin_captures: captures(obj.get("beginCaptures").or(shared)),
                        end_captures: captures(obj.get(end_key).or(shared)),
                        content: text(obj, "contentName").and_then(classify),
                        end_last: obj
                            .get("applyEndPatternLast")
                            .is_some_and(|v| v.as_bool() == Some(true) || v.as_i64() == Some(1)),
                    }
                }
                // end가 없는 begin은 잘못된 규칙이다. 안쪽 패턴만 쓴다.
                (None, None) => Kind::Patterns,
            },
            (None, None) => Kind::Patterns,
        };
        self.rules[id] = Rule {
            name: text(obj, "name").and_then(classify),
            kind,
            patterns,
            flat: Vec::new(),
        };
        Some(id)
    }

    fn include(&mut self, include: &str, ctx: &Ctx<'a>) -> Option<RuleId> {
        match include {
            "$self" => Some(ctx.self_root),
            "$base" => Some(ctx.base_root),
            _ if include.starts_with('#') => {
                let key = &include[1..];
                // 항목을 정의한 저장소 단계의 문맥으로 옮긴다(그 안쪽 저장소는 보이지 않는다).
                let (level, value) = ctx
                    .repos
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(i, repo)| Some((i, repo.get(key)?)))?;
                let mut defined = ctx.clone();
                defined.repos.truncate(level + 1);
                self.rule(value, &defined)
            }
            _ => {
                let (scope, key) = include
                    .split_once('#')
                    .map_or((include, None), |(s, k)| (s, Some(k)));
                let root = self.grammar(scope, Some(ctx.base_root))?;
                let Some(key) = key else {
                    return Some(root);
                };
                let repo = self.grammars.get(scope)?.get("repository")?.as_object()?;
                let other = Ctx {
                    repos: vec![repo],
                    self_root: root,
                    base_root: ctx.base_root,
                };
                self.rule(repo.get(key)?, &other)
            }
        }
    }
}
