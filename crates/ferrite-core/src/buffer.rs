//! 편집 버퍼: 로프(ropey) 텍스트, 선택 영역, 실행 취소/다시 실행, 구문 분석기용 변경 기록.
//!
//! 모든 위치는 문자(char) 인덱스다. 줄바꿈은 `\n`만 쓴다(`text_file`이 읽을 때 정규화한다).

use std::ops::Range;
use std::time::{Duration, Instant};

use ropey::Rope;

/// 연속 입력을 하나의 실행 취소 단위로 묶는 최대 간격.
const MERGE_WINDOW: Duration = Duration::from_millis(1000);

/// 들여쓰기 감지 시 살펴볼 최대 줄 수.
const INDENT_SAMPLE_LINES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indent {
    Tabs,
    Spaces(u8),
}

impl Indent {
    pub fn unit(self) -> String {
        match self {
            Indent::Tabs => "\t".into(),
            Indent::Spaces(n) => " ".repeat(n as usize),
        }
    }
}

/// tree-sitter `InputEdit`로 그대로 옮길 수 있는 변경 정보. 점(point)은 (행, 행 안의 바이트 열)이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start_point: (usize, usize),
    pub old_end_point: (usize, usize),
    pub new_end_point: (usize, usize),
}

#[derive(Debug, Clone)]
struct Change {
    start: usize,
    removed: String,
    inserted: String,
}

#[derive(Debug, Clone)]
struct Transaction {
    changes: Vec<Change>,
    before: Selection,
    after: Selection,
    revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    /// 한 글자 입력. 연속되면 하나로 묶는다.
    Insert,
    /// 한 글자 삭제. 연속되면 하나로 묶는다.
    Delete,
    Other,
}

pub struct Buffer {
    rope: Rope,
    selection: Selection,
    /// 위·아래 이동 중 유지할 열. 짧은 줄을 지나도 원래 열로 돌아오게 한다.
    preferred_col: Option<usize>,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    next_revision: u64,
    last_edit: Option<(EditKind, Instant)>,
    syntax_edits: Vec<SyntaxEdit>,
    version: u64,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            selection: Selection::default(),
            preferred_col: None,
            undo: Vec::new(),
            redo: Vec::new(),
            next_revision: 1,
            last_edit: None,
            syntax_edits: Vec::new(),
            version: 0,
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// 내용이 바뀔 때마다 증가한다. 화면·구문 캐시 무효화용.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// 현재 내용 상태의 식별자. 저장 시점의 값과 비교해 미저장 여부를 판단한다.
    /// 실행 취소로 저장 시점 상태에 돌아오면 같은 값이 된다.
    pub fn revision(&self) -> u64 {
        self.undo.last().map_or(0, |t| t.revision)
    }

    /// 마지막 호출 이후의 변경 목록을 꺼낸다(구문 트리의 증분 갱신용).
    pub fn take_syntax_edits(&mut self) -> Vec<SyntaxEdit> {
        std::mem::take(&mut self.syntax_edits)
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// 줄바꿈을 뺀 줄 길이(문자 수).
    pub fn line_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len - 1
        } else {
            len
        }
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    /// 줄바꿈을 뺀 줄 내용.
    pub fn line_text(&self, line: usize) -> String {
        let start = self.line_start(line);
        self.rope
            .slice(start..start + self.line_len(line))
            .to_string()
    }

    pub fn pos_to_line_col(&self, pos: usize) -> (usize, usize) {
        let line = self.rope.char_to_line(pos);
        (line, pos - self.line_start(line))
    }

    /// 범위를 벗어난 줄·열은 가장 가까운 위치로 맞춘다.
    pub fn line_col_to_pos(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.line_count() - 1);
        self.line_start(line) + col.min(self.line_len(line))
    }

    pub fn selected_text(&self) -> String {
        self.rope.slice(self.selection.range()).to_string()
    }

    // ---------------------------------------------------------------- 이동

    pub fn set_selection(&mut self, selection: Selection) {
        let len = self.len_chars();
        self.selection = Selection {
            anchor: selection.anchor.min(len),
            head: selection.head.min(len),
        };
        self.preferred_col = None;
        self.last_edit = None;
    }

    pub fn move_to(&mut self, pos: usize, extend: bool) {
        let anchor = if extend { self.selection.anchor } else { pos };
        self.set_selection(Selection { anchor, head: pos });
    }

    /// `pos`의 문자가 속한 단어(같은 종류의 문자가 이어진 구간)를 선택한다(더블클릭).
    pub fn select_word_at(&mut self, pos: usize) {
        let same =
            |c: Option<char>, class: u8| c.is_some_and(|c| c != '\n' && char_class(c) == class);
        let Some(class) = self
            .rope
            .get_char(pos)
            .filter(|c| *c != '\n')
            .map(char_class)
        else {
            self.move_to(pos, false);
            return;
        };
        let mut start = pos;
        while start > 0 && same(self.rope.get_char(start - 1), class) {
            start -= 1;
        }
        let mut end = pos + 1;
        while same(self.rope.get_char(end), class) {
            end += 1;
        }
        self.set_selection(Selection {
            anchor: start,
            head: end,
        });
    }

    pub fn select_all(&mut self) {
        self.set_selection(Selection {
            anchor: 0,
            head: self.len_chars(),
        });
    }

    pub fn move_left(&mut self, extend: bool) {
        let sel = self.selection;
        if !extend && !sel.is_empty() {
            self.move_to(sel.range().start, false);
        } else {
            self.move_to(sel.head.saturating_sub(1), extend);
        }
    }

    pub fn move_right(&mut self, extend: bool) {
        let sel = self.selection;
        if !extend && !sel.is_empty() {
            self.move_to(sel.range().end, false);
        } else {
            self.move_to((sel.head + 1).min(self.len_chars()), extend);
        }
    }

    /// `lines`만큼 위(음수)·아래(양수)로 이동한다. 첫 줄 위는 문서 처음, 마지막 줄 아래는 문서 끝이다.
    pub fn move_vertical(&mut self, lines: isize, extend: bool) {
        let (line, col) = self.pos_to_line_col(self.selection.head);
        let col = self.preferred_col.unwrap_or(col);
        let target = line as isize + lines;
        let pos = if target < 0 {
            0
        } else if target as usize >= self.line_count() {
            self.len_chars()
        } else {
            self.line_col_to_pos(target as usize, col)
        };
        self.move_to(pos, extend);
        self.preferred_col = Some(col);
    }

    /// 스마트 Home: 첫 비공백 문자로, 이미 거기 있으면 줄 처음으로.
    pub fn move_line_start(&mut self, extend: bool) {
        let (line, col) = self.pos_to_line_col(self.selection.head);
        let first = self
            .rope
            .line(line)
            .chars()
            .take(self.line_len(line))
            .take_while(|c| *c == ' ' || *c == '\t')
            .count();
        let col = if col == first { 0 } else { first };
        self.move_to(self.line_start(line) + col, extend);
    }

    pub fn move_line_end(&mut self, extend: bool) {
        let line = self.rope.char_to_line(self.selection.head);
        self.move_to(self.line_start(line) + self.line_len(line), extend);
    }

    pub fn move_word(&mut self, forward: bool, extend: bool) {
        let head = self.selection.head;
        let pos = if forward {
            self.word_end_after(head)
        } else {
            self.word_start_before(head)
        };
        self.move_to(pos, extend);
    }

    fn word_end_after(&self, pos: usize) -> usize {
        let mut chars = self.rope.chars_at(pos).peekable();
        let mut pos = pos;
        if chars.peek() == Some(&'\n') {
            return pos + 1;
        }
        while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {
            pos += 1;
        }
        if let Some(&first) = chars.peek() {
            let class = char_class(first);
            while chars
                .next_if(|c| *c != '\n' && char_class(*c) == class)
                .is_some()
            {
                pos += 1;
            }
        }
        pos
    }

    fn word_start_before(&self, pos: usize) -> usize {
        let mut chars = self.rope.chars_at(pos);
        let mut pos = pos;
        let mut prev = chars.prev();
        if prev == Some('\n') {
            return pos - 1;
        }
        while matches!(prev, Some(' ' | '\t')) {
            pos -= 1;
            prev = chars.prev();
        }
        if let Some(first) = prev.filter(|c| *c != '\n') {
            let class = char_class(first);
            while prev.is_some_and(|c| c != '\n' && char_class(c) == class) {
                pos -= 1;
                prev = chars.prev();
            }
        }
        pos
    }

    // ---------------------------------------------------------------- 편집

    /// 선택 영역을 `text`로 바꾼다(타이핑, 붙여넣기).
    pub fn insert(&mut self, text: &str, now: Instant) {
        let range = self.selection.range();
        let typed_char = range.is_empty() && text.chars().count() == 1 && text != "\n";
        let kind = if typed_char {
            EditKind::Insert
        } else {
            EditKind::Other
        };
        let caret = Selection::caret(range.start + text.chars().count());
        self.transact(vec![(range, text.to_owned())], caret, kind, now);
    }

    /// 줄바꿈과 함께 현재 줄의 들여쓰기(커서 앞부분까지)를 이어 쓴다.
    pub fn newline(&mut self, now: Instant) {
        let (line, col) = self.pos_to_line_col(self.selection.range().start);
        let indent: String = self
            .rope
            .line(line)
            .chars()
            .take(col)
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        self.insert(&format!("\n{indent}"), now);
    }

    pub fn backspace(&mut self, now: Instant) {
        let sel = self.selection;
        if !sel.is_empty() {
            self.delete_selection(now);
        } else if sel.head > 0 {
            let caret = Selection::caret(sel.head - 1);
            self.transact(
                vec![(sel.head - 1..sel.head, String::new())],
                caret,
                EditKind::Delete,
                now,
            );
        }
    }

    pub fn delete_forward(&mut self, now: Instant) {
        let sel = self.selection;
        if !sel.is_empty() {
            self.delete_selection(now);
        } else if sel.head < self.len_chars() {
            self.transact(
                vec![(sel.head..sel.head + 1, String::new())],
                sel,
                EditKind::Delete,
                now,
            );
        }
    }

    pub fn delete_word_back(&mut self, now: Instant) {
        let sel = self.selection;
        if !sel.is_empty() {
            self.delete_selection(now);
            return;
        }
        let start = self.word_start_before(sel.head);
        if start < sel.head {
            let caret = Selection::caret(start);
            self.transact(
                vec![(start..sel.head, String::new())],
                caret,
                EditKind::Other,
                now,
            );
        }
    }

    /// 선택 영역을 잘라낸 텍스트를 돌려준다. 선택이 없으면 빈 문자열.
    pub fn cut(&mut self, now: Instant) -> String {
        let text = self.selected_text();
        if !text.is_empty() {
            self.delete_selection(now);
        }
        text
    }

    fn delete_selection(&mut self, now: Instant) {
        let range = self.selection.range();
        let caret = Selection::caret(range.start);
        self.transact(vec![(range, String::new())], caret, EditKind::Other, now);
    }

    /// 여러 범위를 한 번에 바꾸는 하나의 실행 취소 단위(모두 바꾸기 등). 범위는 겹치지 않아야 한다.
    pub fn replace_ranges(&mut self, edits: Vec<(Range<usize>, String)>, now: Instant) {
        if edits.is_empty() {
            return;
        }
        let after = self.map_selection(&edits);
        self.transact(edits, after, EditKind::Other, now);
    }

    /// 여러 줄을 선택했으면 각 줄을 들여쓰고, 아니면 커서 위치에 들여쓰기 단위를 넣는다.
    pub fn indent(&mut self, unit: &str, now: Instant) {
        let lines = self.selected_lines();
        if lines.start + 1 == lines.end {
            self.insert(unit, now);
            return;
        }
        let edits = lines
            .filter(|&l| self.line_len(l) > 0)
            .map(|l| {
                let start = self.line_start(l);
                (start..start, unit.to_owned())
            })
            .collect();
        self.replace_ranges(edits, now);
    }

    /// 선택한 줄(없으면 커서 줄)에서 들여쓰기 한 단위(탭 하나 또는 공백 최대 `width`개)를 뺀다.
    pub fn dedent(&mut self, width: usize, now: Instant) {
        let edits = self
            .selected_lines()
            .filter_map(|l| {
                let start = self.line_start(l);
                let mut chars = self.rope.line(l).chars();
                let n = if chars.next() == Some('\t') {
                    1
                } else {
                    self.rope
                        .line(l)
                        .chars()
                        .take(width)
                        .take_while(|c| *c == ' ')
                        .count()
                };
                (n > 0).then(|| (start..start + n, String::new()))
            })
            .collect();
        self.replace_ranges(edits, now);
    }

    /// 선택한 줄의 줄 주석을 켜고 끈다. 비어 있지 않은 줄이 모두 주석이면 해제한다.
    pub fn toggle_line_comment(&mut self, token: &str, now: Instant) {
        let lines: Vec<(usize, usize)> = self
            .selected_lines()
            .filter_map(|l| {
                let text = self.line_text(l);
                let indent = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
                (indent < text.chars().count()).then_some((l, indent))
            })
            .collect();
        if lines.is_empty() {
            return;
        }
        let all_commented = lines.iter().all(|&(l, indent)| {
            self.line_text(l)
                .chars()
                .skip(indent)
                .collect::<String>()
                .starts_with(token)
        });
        let token_len = token.chars().count();
        let edits = if all_commented {
            lines
                .iter()
                .map(|&(l, indent)| {
                    let start = self.line_start(l) + indent;
                    let after_token = self.rope.get_char(start + token_len);
                    let len = token_len + usize::from(after_token == Some(' '));
                    (start..start + len, String::new())
                })
                .collect()
        } else {
            let min_indent = lines.iter().map(|&(_, i)| i).min().unwrap_or(0);
            lines
                .iter()
                .map(|&(l, _)| {
                    let at = self.line_start(l) + min_indent;
                    (at..at, format!("{token} "))
                })
                .collect()
        };
        self.replace_ranges(edits, now);
    }

    /// 선택한 줄(없으면 커서 줄)을 한 줄 위 또는 아래로 옮긴다.
    pub fn move_lines(&mut self, up: bool, now: Instant) {
        let lines = self.selected_lines();
        let (first, last) = if up {
            if lines.start == 0 {
                return;
            }
            (lines.start - 1, lines.end - 1)
        } else {
            if lines.end >= self.line_count() {
                return;
            }
            (lines.start, lines.end)
        };
        let mut texts: Vec<String> = (first..=last).map(|l| self.line_text(l)).collect();
        let neighbor_len = if up {
            let neighbor = texts.remove(0);
            let len = neighbor.chars().count();
            texts.push(neighbor);
            len
        } else {
            let neighbor = texts.pop().unwrap_or_default();
            let len = neighbor.chars().count();
            texts.insert(0, neighbor);
            len
        };
        let start = self.line_start(first);
        let end = self.line_start(last) + self.line_len(last);
        let shift = neighbor_len + 1;
        let sel = self.selection;
        let moved = |p: usize| if up { p - shift } else { p + shift };
        let after = Selection {
            anchor: moved(sel.anchor),
            head: moved(sel.head),
        };
        self.transact(
            vec![(start..end, texts.join("\n"))],
            after,
            EditKind::Other,
            now,
        );
    }

    pub fn undo(&mut self) -> bool {
        let Some(tx) = self.undo.pop() else {
            return false;
        };
        for c in tx.changes.iter().rev() {
            self.apply_change(c.start, c.start + c.inserted.chars().count(), &c.removed);
        }
        self.selection = tx.before;
        self.preferred_col = None;
        self.last_edit = None;
        self.redo.push(tx);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(tx) = self.redo.pop() else {
            return false;
        };
        for c in &tx.changes {
            self.apply_change(c.start, c.start + c.removed.chars().count(), &c.inserted);
        }
        self.selection = tx.after;
        self.preferred_col = None;
        self.last_edit = None;
        self.undo.push(tx);
        true
    }

    // ---------------------------------------------------------------- 내부

    /// 선택 영역이 걸친 줄 범위. 여러 줄 선택이 다음 줄 처음에서 끝나면 그 줄은 빼고 센다.
    fn selected_lines(&self) -> Range<usize> {
        let range = self.selection.range();
        let first = self.rope.char_to_line(range.start);
        let mut last = self.rope.char_to_line(range.end);
        if last > first && range.end == self.line_start(last) {
            last -= 1;
        }
        first..last + 1
    }

    fn map_selection(&self, edits: &[(Range<usize>, String)]) -> Selection {
        let mut sorted: Vec<&(Range<usize>, String)> = edits.iter().collect();
        sorted.sort_by_key(|(r, _)| r.start);
        let sel = self.selection;
        Selection {
            anchor: map_pos(sel.anchor, &sorted),
            head: map_pos(sel.head, &sorted),
        }
    }

    /// 겹치지 않는 변경들을 하나의 실행 취소 단위로 적용한다. 위치는 적용 전 문서 기준이다.
    fn transact(
        &mut self,
        mut edits: Vec<(Range<usize>, String)>,
        after: Selection,
        kind: EditKind,
        now: Instant,
    ) {
        // 뒤쪽부터 적용하면 앞쪽 변경의 위치가 바뀌지 않는다.
        edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        let before = self.selection;
        let changes: Vec<Change> = edits
            .iter()
            .map(|(r, text)| self.apply_change(r.start, r.end, text))
            .collect();
        self.selection = after;
        self.preferred_col = None;
        self.redo.clear();

        let revision = self.next_revision;
        self.next_revision += 1;
        let mergeable = kind != EditKind::Other
            && self
                .last_edit
                .is_some_and(|(k, t)| k == kind && now.duration_since(t) <= MERGE_WINDOW)
            && self.undo.last().is_some_and(|t| t.after == before);
        match self.undo.last_mut() {
            Some(last) if mergeable => {
                last.changes.extend(changes);
                last.after = after;
                // 묶인 뒤에도 내용이 바뀌었으므로 저장 시점과 구별되는 새 식별자를 준다.
                last.revision = revision;
            }
            _ => self.undo.push(Transaction {
                changes,
                before,
                after,
                revision,
            }),
        }
        self.last_edit = (kind != EditKind::Other).then_some((kind, now));
    }

    fn apply_change(&mut self, start: usize, end: usize, text: &str) -> Change {
        let start_byte = self.rope.char_to_byte(start);
        let old_end_byte = self.rope.char_to_byte(end);
        let start_point = self.point(start_byte);
        let old_end_point = self.point(old_end_byte);
        let removed = self.rope.slice(start..end).to_string();
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        let new_end_byte = start_byte + text.len();
        self.syntax_edits.push(SyntaxEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_point,
            old_end_point,
            new_end_point: self.point(new_end_byte),
        });
        self.version += 1;
        Change {
            start,
            removed,
            inserted: text.to_owned(),
        }
    }

    fn point(&self, byte: usize) -> (usize, usize) {
        let row = self.rope.byte_to_line(byte);
        (row, byte - self.rope.line_to_byte(row))
    }
}

/// 공백, 단어(한글 포함 영숫자와 `_`), 그 밖의 기호로 문자를 나눈다.
fn char_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

/// 변경 전 위치를 변경 후 위치로 옮긴다. `edits`는 시작 위치 순으로 정렬되어 있어야 한다.
/// 위치와 같은 지점에 삽입된 내용은 위치 뒤로 간다(줄 처음의 선택 시작이 들여쓰기를 감싸도록).
fn map_pos(pos: usize, edits: &[&(Range<usize>, String)]) -> usize {
    let mut delta = 0isize;
    for (range, text) in edits {
        if range.start >= pos {
            break;
        }
        let inserted = text.chars().count();
        if pos < range.end {
            return (range.start as isize + delta) as usize + inserted;
        }
        delta += inserted as isize - (range.end - range.start) as isize;
    }
    (pos as isize + delta) as usize
}

/// 파일의 들여쓰기 방식을 추정한다. 근거가 없으면 `default`.
pub fn detect_indent(rope: &Rope, default: Indent) -> Indent {
    let (mut tab_lines, mut space_lines) = (0usize, 0usize);
    let mut diffs = [0usize; 9];
    let mut prev_spaces = 0usize;
    for line in rope.lines().take(INDENT_SAMPLE_LINES) {
        let mut chars = line.chars();
        match chars.next() {
            Some('\t') => tab_lines += 1,
            Some(' ') => {
                let spaces = 1 + chars.clone().take_while(|c| *c == ' ').count();
                if chars.nth(spaces - 1).is_some_and(|c| c != '\n') {
                    space_lines += 1;
                    let diff = spaces.abs_diff(prev_spaces);
                    if (2..=8).contains(&diff) {
                        diffs[diff] += 1;
                    }
                    prev_spaces = spaces;
                }
            }
            Some('\n') | None => {}
            Some(_) => prev_spaces = 0,
        }
    }
    if tab_lines > space_lines {
        return Indent::Tabs;
    }
    (2..=8)
        .max_by_key(|&n| (diffs[n], std::cmp::Reverse(n)))
        .filter(|&n| diffs[n] > 0)
        .map_or(default, |n| Indent::Spaces(n as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str, anchor: usize, head: usize) -> Buffer {
        let mut b = Buffer::new(text);
        b.set_selection(Selection { anchor, head });
        b
    }

    #[test]
    fn 연속_입력은_하나의_실행취소_단위() {
        let t0 = Instant::now();
        let mut b = buf("", 0, 0);
        for (i, ch) in ["a", "b", "c"].into_iter().enumerate() {
            b.insert(ch, t0 + Duration::from_millis(100 * i as u64));
        }
        // 1초 넘게 쉬면 새 단위
        b.insert("d", t0 + Duration::from_secs(5));
        assert_eq!(b.text(), "abcd");
        b.undo();
        assert_eq!(b.text(), "abc");
        b.undo();
        assert_eq!(b.text(), "");
        b.redo();
        assert_eq!(b.text(), "abc");
        assert_eq!(b.selection(), Selection::caret(3));
    }

    #[test]
    fn 실행취소로_저장시점에_돌아오면_같은_리비전() {
        let t0 = Instant::now();
        let mut b = buf("x", 1, 1);
        b.insert("a", t0);
        let saved = b.revision();
        b.insert("b", t0); // 같은 단위로 묶여도 리비전은 바뀐다
        assert_ne!(b.revision(), saved);
        b.backspace(t0 + Duration::from_secs(5));
        b.undo();
        b.undo();
        assert_eq!(b.revision(), 0);
        b.redo();
        assert_ne!(b.revision(), saved, "묶인 단위 전체가 다시 적용된 상태");
        assert_eq!(b.text(), "xab");
    }

    #[test]
    fn 새_편집은_다시실행_기록을_지운다() {
        let t0 = Instant::now();
        let mut b = buf("", 0, 0);
        b.insert("a", t0);
        b.undo();
        b.insert("b", t0);
        assert!(!b.redo());
        assert_eq!(b.text(), "b");
    }

    #[test]
    fn 줄바꿈은_커서_앞_들여쓰기를_잇는다() {
        let mut b = buf("    foo", 7, 7);
        b.newline(Instant::now());
        assert_eq!(b.text(), "    foo\n    ");
        assert_eq!(b.selection(), Selection::caret(12));
    }

    #[test]
    fn 백스페이스와_삭제() {
        let t0 = Instant::now();
        let mut b = buf("abc", 2, 2);
        b.backspace(t0);
        assert_eq!(b.text(), "ac");
        b.delete_forward(t0);
        assert_eq!(b.text(), "a");
        let mut b = buf("hello world", 11, 11);
        b.delete_word_back(t0);
        assert_eq!(b.text(), "hello ");
    }

    #[test]
    fn 여러_줄_들여쓰기와_내어쓰기() {
        let t0 = Instant::now();
        let mut b = buf("a\n\nb\nc", 0, 5); // a, 빈 줄, b 까지 선택(c 줄 처음에서 끝남 → c 제외)
        b.indent("  ", t0);
        assert_eq!(b.text(), "  a\n\n  b\nc");
        assert_eq!(b.selection(), Selection { anchor: 0, head: 9 });
        b.dedent(4, t0);
        assert_eq!(b.text(), "a\n\nb\nc");
        // 한 줄 안에서는 커서 위치에 삽입
        let mut b = buf("ab", 1, 1);
        b.indent("\t", t0);
        assert_eq!(b.text(), "a\tb");
    }

    #[test]
    fn 줄_주석_토글() {
        let t0 = Instant::now();
        let mut b = buf("    a\n  b\n", 0, 8);
        b.toggle_line_comment("//", t0);
        assert_eq!(b.text(), "  //   a\n  // b\n");
        b.toggle_line_comment("//", t0);
        assert_eq!(b.text(), "    a\n  b\n");
    }

    #[test]
    fn 줄_이동() {
        let t0 = Instant::now();
        let mut b = buf("one\ntwo\nthree", 5, 5); // two의 w
        b.move_lines(true, t0);
        assert_eq!(b.text(), "two\none\nthree");
        assert_eq!(b.selection(), Selection::caret(1));
        b.move_lines(false, t0);
        b.move_lines(false, t0);
        assert_eq!(b.text(), "one\nthree\ntwo");
        b.move_lines(false, t0); // 마지막 줄은 더 내려가지 않음
        assert_eq!(b.text(), "one\nthree\ntwo");
        b.undo();
        assert_eq!(b.text(), "one\ntwo\nthree");
    }

    #[test]
    fn 위아래_이동은_원래_열을_기억한다() {
        let mut b = buf("long line\nab\nlong line", 7, 7);
        b.move_vertical(1, false);
        assert_eq!(b.pos_to_line_col(b.selection().head), (1, 2));
        b.move_vertical(1, false);
        assert_eq!(b.pos_to_line_col(b.selection().head), (2, 7));
        b.move_vertical(5, false);
        assert_eq!(b.selection().head, b.len_chars());
    }

    #[test]
    fn 단어_단위_이동_한글_포함() {
        let mut b = buf("let 한글_이름 = 1;", 0, 0);
        b.move_word(true, false);
        assert_eq!(b.selection().head, 3);
        b.move_word(true, false);
        assert_eq!(b.selection().head, 9);
        b.move_word(false, false);
        assert_eq!(b.selection().head, 4);
    }

    #[test]
    fn 스마트_홈() {
        let mut b = buf("    code", 8, 8);
        b.move_line_start(false);
        assert_eq!(b.selection().head, 4);
        b.move_line_start(false);
        assert_eq!(b.selection().head, 0);
    }

    #[test]
    fn 구문_변경_기록은_바이트와_행열() {
        let mut b = buf("가\nb", 3, 3);
        b.insert("c", Instant::now());
        let edits = b.take_syntax_edits();
        assert_eq!(
            edits,
            vec![SyntaxEdit {
                start_byte: 5,
                old_end_byte: 5,
                new_end_byte: 6,
                start_point: (1, 1),
                old_end_point: (1, 1),
                new_end_point: (1, 2),
            }]
        );
        assert!(b.take_syntax_edits().is_empty());
    }

    #[test]
    fn 들여쓰기_감지() {
        let rope = |s: &str| Rope::from_str(s);
        assert_eq!(
            detect_indent(&rope("a\n\tb\n\tc\n"), Indent::Spaces(4)),
            Indent::Tabs
        );
        assert_eq!(
            detect_indent(&rope("a\n  b\n    c\n  d\n"), Indent::Tabs),
            Indent::Spaces(2)
        );
        assert_eq!(
            detect_indent(&rope("/**\n * doc\n */\nx\n"), Indent::Spaces(4)),
            Indent::Spaces(4)
        );
    }
}
