//! 언어 확장: 널리 쓰는 편집기 확장 폴더(`package.json`의 `contributes.languages`·`grammars`,
//! `language-configuration.json`, `.tmLanguage.json`)를 읽어, tree-sitter 문법이 없는 형식을
//! TextMate 문법으로 강조한다.
//!
//! 토크나이저는 줄 단위다. 줄마다 시작할 때의 규칙 쌓임(state)을 기억해 두고, 편집이 있으면 그
//! 줄부터 다시 센다. 정규식은 문법이 기대하는 Oniguruma(onig)로 돈다. 범위는 TextMate 기본 규칙이다:
//! match·begin/end·begin/while·include(저장소, `$self`, 다른 문법)·captures·contentName.
//! 주입(injections)과 캡처 안의 patterns는 쓰지 않는다.

mod extension;
mod grammar;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferrite_core::buffer::Buffer;
use ferrite_core::syntax::{Highlight, LineHighlights};
use onig::{Regex, Region, SearchOptions};
use ropey::Rope;

pub use extension::{Language, Registry};
use grammar::{Captures, Kind, RuleId};
pub use grammar::{Grammar, compile_grammar};

/// 이보다 긴 줄은 강조하지 않는다(규칙 쌓임은 그대로 다음 줄로 넘긴다).
const MAX_LINE: usize = 10_000;
/// 한 줄에서 찾는 횟수의 상한. 넘으면 그 줄의 나머지는 강조하지 않는다.
const MAX_STEPS: usize = 5_000;
/// 보이는 줄 앞을 세는 데 한 번에 쓰는 시간. 넘으면 다음 프레임에 이어 센다(`is_pending`).
const BUDGET: Duration = Duration::from_millis(30);

/// TextMate 범위 이름(`keyword.control.js` 등, 공백으로 여럿)을 강조 분류로 바꾼다. 여럿이면 안쪽(뒤)이
/// 이긴다. 분류가 없으면 바깥 분류를 그대로 둔다.
pub fn classify(names: &str) -> Option<Highlight> {
    names.split_whitespace().rev().find_map(classify_one)
}

fn classify_one(scope: &str) -> Option<Highlight> {
    let is = |prefix: &str| {
        scope == prefix
            || (scope.starts_with(prefix) && scope.as_bytes().get(prefix.len()) == Some(&b'.'))
    };
    const PROPERTY: [&str; 5] = [
        "variable.other.property",
        "variable.other.member",
        "variable.other.object.property",
        "support.variable.property",
        "meta.object-literal.key",
    ];
    const TYPE: [&str; 6] = [
        "entity.name.type",
        "entity.name.class",
        "entity.name.namespace",
        "entity.other.inherited-class",
        "support.type",
        "support.class",
    ];
    Some(if is("comment") {
        Highlight::Comment
    } else if is("constant") || is("support.constant") {
        Highlight::Constant
    } else if is("string") {
        Highlight::String
    } else if is("keyword.operator") || is("invalid") {
        return None;
    } else if is("keyword")
        || is("storage")
        || is("variable.language")
        || is("entity.name.section")
        || is("markup.heading")
    {
        Highlight::Keyword
    } else if PROPERTY.iter().any(|p| is(p)) || is("entity.other.attribute-name") {
        Highlight::Property
    } else if is("entity.name.function") || is("support.function") {
        Highlight::Function
    } else if is("entity.name.tag") {
        Highlight::Tag
    } else if TYPE.iter().any(|p| is(p)) {
        Highlight::Type
    } else {
        return None;
    })
}

/// 규칙 쌓임의 한 단.
#[derive(Clone)]
struct Frame {
    rule: RuleId,
    /// 이 규칙의 begin·end 글의 분류(name, 없으면 바깥 것).
    outer: Option<Highlight>,
    /// 안쪽 글의 분류(contentName, name, 바깥 순).
    inside: Option<Highlight>,
    /// begin의 캡처로 뒤 참조를 채운 end(while). 뒤 참조가 없으면 규칙의 것을 쓴다.
    end: Option<Arc<str>>,
    /// begin이 줄 끝까지 맞았다. 그러면 다음 줄 처음이 닻(`\G`)이다.
    eol: bool,
    /// 이 단을 넣기 전의 닻. 내리면 되돌린다. 그 줄 안에서만 뜻이 있다.
    parent_anchor: Option<usize>,
}

/// 찾은 것: 캡처마다 (시작, 끝). 0번이 전체.
struct Found {
    groups: Vec<Option<(usize, usize)>>,
}

impl Found {
    fn range(&self) -> (usize, usize) {
        self.groups[0].unwrap_or_default()
    }
}

fn search(re: &Regex, line: &str, pos: usize) -> Option<Found> {
    let mut region = Region::new();
    re.search_with_options(
        line,
        pos,
        line.len(),
        SearchOptions::SEARCH_OPTION_NONE,
        Some(&mut region),
    )?;
    Some(Found {
        groups: (0..region.len()).map(|i| region.pos(i)).collect(),
    })
}

/// 정규식 글자를 이스케이프한다(뒤 참조 채우기). VS Code와 같은 글자만 한다. `` \` ``처럼 이스케이프하면
/// 뜻이 생기는 글자(Oniguruma에서 버퍼 처음)가 있어 모든 문장 부호를 이스케이프하면 안 된다.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "-\\{}*+?|^$.,[]()#".contains(c) || c.is_whitespace() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// end(while)의 `\N`을 begin에서 N번 캡처로 찾은 글(이스케이프)로 바꾼다.
fn fill_backrefs(source: &str, found: &Found, line: &str) -> String {
    let mut out = String::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some(d) if d.is_ascii_digit() => {
                    chars.next();
                    let n = d.to_digit(10).unwrap_or(0) as usize;
                    if let Some(Some((a, b))) = found.groups.get(n) {
                        out.push_str(&escape(&line[*a..*b]));
                    }
                    continue;
                }
                Some(next) => {
                    chars.next();
                    out.push(c);
                    out.push(next);
                    continue;
                }
                None => {}
            }
        }
        out.push(c);
    }
    out
}

/// 한 줄의 강조 구간을 모은다. 이웃한 같은 분류는 합친다.
#[derive(Default)]
struct Painter {
    out: Vec<(Range<usize>, Highlight)>,
}

impl Painter {
    fn paint(&mut self, range: Range<usize>, kind: Option<Highlight>) {
        let Some(kind) = kind else { return };
        if range.is_empty() {
            return;
        }
        match self.out.last_mut() {
            Some((last, k)) if *k == kind && last.end == range.start => last.end = range.end,
            _ => self.out.push((range, kind)),
        }
    }

    /// 찾은 글 전체를 `base`로, 캡처는 그 위에 번호 순으로(안쪽이 이긴다)칠한다.
    fn paint_match(&mut self, found: &Found, base: Option<Highlight>, captures: &Captures) {
        let (start, end) = found.range();
        if captures.is_empty() {
            self.paint(start..end, base);
            return;
        }
        let mut kinds = vec![base; end - start];
        for &(i, kind) in captures {
            if let Some(Some((a, b))) = found.groups.get(i) {
                let (a, b) = ((*a).max(start) - start, (*b).min(end).saturating_sub(start));
                kinds
                    .get_mut(a..b.max(a))
                    .into_iter()
                    .flatten()
                    .for_each(|k| *k = Some(kind));
            }
        }
        let mut run = start;
        for i in 1..=kinds.len() {
            if i == kinds.len() || kinds[i] != kinds[i - 1] {
                self.paint(run..start + i, kinds[i - 1]);
                run = start + i;
            }
        }
    }
}

/// 다음 글자 경계.
fn next_char(line: &str, pos: usize) -> usize {
    line[pos..]
        .chars()
        .next()
        .map_or(line.len(), |c| pos + c.len_utf8())
}

/// 문서 하나의 TextMate 강조. 줄마다 시작할 때의 규칙 쌓임을 기억한다.
pub struct TextMateSyntax {
    grammar: Arc<Grammar>,
    /// `states[i]`: i번 줄을 시작할 때의 쌓임.
    states: Vec<Vec<Frame>>,
    /// 뒤 참조를 채워 컴파일한 end·while.
    dynamic: Dynamic,
    budget: Duration,
    /// 보이는 줄 앞을 다 세지 못했다. 다시 그려 이어 센다.
    pending: bool,
}

impl TextMateSyntax {
    pub fn new(grammar: Arc<Grammar>) -> Self {
        let root = Frame {
            rule: grammar.root,
            outer: None,
            inside: None,
            end: None,
            eol: false,
            parent_anchor: None,
        };
        Self {
            grammar,
            states: vec![vec![root]],
            dynamic: HashMap::new(),
            budget: BUDGET,
            pending: false,
        }
    }

    /// 보이는 줄 앞을 다 세지 못해 강조가 비었다. 다시 그리면 이어 센다.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// 버퍼에 쌓인 변경을 받아, 바뀐 첫 줄부터 다시 세게 한다. 버퍼는
    /// `set_track_syntax_edits(true)` 상태여야 한다.
    pub fn update(&mut self, buffer: &mut Buffer) {
        if let Some(row) = buffer
            .take_syntax_edits()
            .iter()
            .map(|e| e.start_point.0)
            .min()
        {
            self.states.truncate(row + 1);
        }
    }

    /// `lines` 범위 각 줄의 강조 구간(줄 안 바이트 범위). 앞 줄들의 쌓임이 없으면 먼저 세는데, 시간을
    /// 넘기면 빈 강조를 돌려주고 `is_pending`이 된다.
    // ponytail: 보이는 줄은 그릴 때마다 다시 센다(쌓임만 기억). 느리면 줄 결과도 판으로 기억한다.
    pub fn highlight_lines(&mut self, rope: &Rope, lines: Range<usize>) -> LineHighlights {
        let lines = lines.start..lines.end.min(rope.len_lines());
        let started = Instant::now();
        while self.states.len() <= lines.start {
            let line = self.states.len() - 1;
            self.line(rope, line);
            if started.elapsed() >= self.budget && self.states.len() <= lines.start {
                self.pending = true;
                return vec![Vec::new(); lines.len()];
            }
        }
        self.pending = false;
        lines.map(|line| self.line(rope, line)).collect()
    }

    /// `line` 번 줄을 센다. 다음 줄의 쌓임을 기억하고 이 줄의 강조 구간을 돌려준다.
    fn line(&mut self, rope: &Rope, line: usize) -> Vec<(Range<usize>, Highlight)> {
        let mut stack = self.states[line].clone();
        // 앞 줄의 닻 자리는 이 줄에서 뜻이 없다.
        stack.iter_mut().for_each(|f| f.parent_anchor = None);
        let slice = rope.line(line);
        let mut text = slice.to_string();
        let visible = text.trim_end_matches(['\n', '\r']).len();
        let mut out = Vec::new();
        if text.len() <= MAX_LINE {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            out = tokenize(&self.grammar, &mut self.dynamic, &text, &mut stack);
            for (range, _) in &mut out {
                range.end = range.end.min(visible);
            }
            out.retain(|(range, _)| range.start < range.end);
        }
        // 뒤 줄의 쌓임이 이미 있으면 같은 입력으로 센 것이다(편집은 `update`가 지운다).
        if self.states.len() == line + 1 {
            self.states.push(stack);
        }
        out
    }
}

/// 규칙의 end(while) 정규식. 뒤 참조를 채운 것은 `dynamic`에 컴파일해 둔다.
fn end_regex<'a>(
    grammar: &'a Grammar,
    dynamic: &'a mut Dynamic,
    frame: &Frame,
    anchor: bool,
) -> Option<&'a Regex> {
    let Kind::Begin { end, .. } = &grammar.rules[frame.rule].kind else {
        return None;
    };
    match &frame.end {
        None => end.get(anchor),
        Some(source) => dynamic
            .entry((Arc::clone(source), anchor))
            .or_insert_with(|| {
                if anchor {
                    grammar::compile(source)
                } else {
                    grammar::compile(&grammar::without_anchor(source))
                }
            })
            .as_ref(),
    }
}

type Dynamic = HashMap<(Arc<str>, bool), Option<Regex>>;

/// 한 줄(끝에 줄바꿈 포함)을 센다. `stack`은 줄을 시작할 때의 쌓임이고 줄 끝의 쌓임이 된다.
fn tokenize(
    grammar: &Grammar,
    dynamic: &mut Dynamic,
    line: &str,
    stack: &mut Vec<Frame>,
) -> Vec<(Range<usize>, Highlight)> {
    let rules = &grammar.rules;
    let mut painter = Painter::default();
    let (mut pos, mut anchor) = while_conditions(grammar, dynamic, line, stack, &mut painter);
    // 앞서 찾은 결과(규칙 → 그때 찾은 것). 시작이 지금 위치 뒤면 다시 찾지 않아도 같다.
    let mut cache: HashMap<RuleId, Option<Found>> = HashMap::new();
    // 빈 begin이 같은 자리에서 되풀이되는 것을 막는다.
    let mut pushed: Vec<(usize, RuleId)> = Vec::new();
    for _ in 0..MAX_STEPS {
        // 줄 끝에서도 한 번 더 찾는다(VS Code와 같다). 줄바꿈까지 맞은 begin 뒤에 빈 begin이 들어가야
        // 다음 줄이 그 규칙 안에서 시작한다(마크다운 펜스 코드 블록). 아무것도 맞지 않으면 끝난다.
        let top = stack.last().cloned().expect("뿌리 단은 늘 있다");
        let rule = &rules[top.rule];
        let (end_found, end_last) = match &rule.kind {
            Kind::Begin {
                is_while: false,
                end_last,
                ..
            } => (
                end_regex(grammar, dynamic, &top, anchor == Some(pos))
                    .and_then(|re| search(re, line, pos)),
                *end_last,
            ),
            _ => (None, false),
        };
        let mut best: Option<(RuleId, usize)> = None;
        for &id in &rule.flat {
            let re = match &rules[id].kind {
                Kind::Match { re, .. } => re,
                Kind::Begin { begin, .. } => begin,
                Kind::Patterns => continue,
            };
            let reuse = !re.anchored()
                && cache
                    .get(&id)
                    .is_some_and(|f| f.as_ref().is_none_or(|f| f.range().0 >= pos));
            if !reuse {
                let found = re
                    .get(anchor == Some(pos))
                    .and_then(|re| search(re, line, pos));
                cache.insert(id, found);
            }
            if let Some(Some(f)) = cache.get(&id) {
                let start = f.range().0;
                if best.is_none_or(|(_, s)| start < s) {
                    best = Some((id, start));
                }
            }
        }
        let end_wins = match (&end_found, best) {
            (Some(e), Some((_, s))) => {
                let e = e.range().0;
                if end_last { e < s } else { e <= s }
            }
            (Some(_), None) => true,
            _ => false,
        };
        if end_wins {
            let found = end_found.expect("end가 이겼다");
            let (start, end) = found.range();
            let Kind::Begin { end_captures, .. } = &rule.kind else {
                unreachable!("end는 begin 규칙에만 있다");
            };
            painter.paint(pos..start, top.inside);
            painter.paint_match(&found, top.outer, end_captures);
            if stack.len() > 1
                && let Some(popped) = stack.pop()
            {
                anchor = popped.parent_anchor;
            }
            pos = end;
            continue;
        }
        let Some((id, _)) = best else {
            painter.paint(pos..line.len(), top.inside);
            break;
        };
        let found = cache.remove(&id).flatten().expect("가장 앞의 것");
        let (start, end) = found.range();
        painter.paint(pos..start, top.inside);
        let outer = rules[id].name.or(top.inside);
        match &rules[id].kind {
            Kind::Match { captures, .. } => {
                painter.paint_match(&found, outer, captures);
                if end > start {
                    pos = end;
                } else if start >= line.len() {
                    break;
                } else {
                    pos = next_char(line, start);
                }
            }
            Kind::Begin {
                end: end_re,
                begin_captures,
                content,
                ..
            } => {
                painter.paint_match(&found, outer, begin_captures);
                if end == start && pushed.contains(&(start, id)) {
                    if start >= line.len() {
                        break;
                    }
                    pos = next_char(line, start);
                    continue;
                }
                pushed.push((start, id));
                let filled = end_re
                    .has_backrefs()
                    .then(|| Arc::from(fill_backrefs(end_re.source(), &found, line)));
                stack.push(Frame {
                    rule: id,
                    outer,
                    inside: content.or(outer),
                    end: filled,
                    eol: end >= line.len(),
                    parent_anchor: anchor,
                });
                anchor = Some(end);
                pos = end;
            }
            Kind::Patterns => unreachable!("펼친 목록에는 패턴만 담은 규칙이 없다"),
        }
    }
    painter.out
}

/// 줄 처음에 begin/while 규칙의 while을 바깥부터 확인한다. 맞지 않는 단부터 위는 내린다.
/// 확인한 뒤의 위치와 닻을 돌려준다.
fn while_conditions(
    grammar: &Grammar,
    dynamic: &mut Dynamic,
    line: &str,
    stack: &mut Vec<Frame>,
    painter: &mut Painter,
) -> (usize, Option<usize>) {
    let mut pos = 0;
    // 맨 위 단의 begin이 앞 줄 끝까지 맞았으면 이 줄 처음이 닻이다.
    let mut anchor = stack.last().filter(|f| f.eol).map(|_| 0);
    let mut i = 1;
    while i < stack.len() {
        if let Kind::Begin {
            is_while: true,
            end_captures,
            ..
        } = &grammar.rules[stack[i].rule].kind
        {
            let frame = stack[i].clone();
            match end_regex(grammar, dynamic, &frame, anchor == Some(pos))
                .and_then(|re| search(re, line, pos))
            {
                Some(found) => {
                    painter.paint_match(&found, frame.outer, end_captures);
                    pos = found.range().1;
                    anchor = Some(pos);
                }
                None => {
                    stack.truncate(i);
                    break;
                }
            }
        }
        i += 1;
    }
    (pos, anchor)
}

#[cfg(test)]
mod tests;
