//! 에디터 문자 인덱스 ↔ LSP 위치(줄, 열) 변환. 열 단위는 서버와 합의한 인코딩(UTF-8 바이트
//! 또는 UTF-16 코드 단위)을 따른다. 한글은 UTF-8에서 3, UTF-16에서 1로 세므로 반드시 구분해야 한다.

use lsp_types::Position;
use ropey::Rope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    /// LSP 기본값. 서버가 다른 인코딩을 고르지 않으면 이것이다.
    Utf16,
}

fn line_end(rope: &Rope, line: usize) -> usize {
    let start = rope.line_to_char(line);
    let slice = rope.line(line);
    let len = slice.len_chars();
    start + len - usize::from(len > 0 && slice.char(len - 1) == '\n')
}

pub fn to_lsp(rope: &Rope, char_idx: usize, encoding: PositionEncoding) -> Position {
    let char_idx = char_idx.min(rope.len_chars());
    let line = rope.char_to_line(char_idx);
    let line_start = rope.line_to_char(line);
    let column = match encoding {
        PositionEncoding::Utf8 => rope.char_to_byte(char_idx) - rope.char_to_byte(line_start),
        PositionEncoding::Utf16 => {
            rope.char_to_utf16_cu(char_idx) - rope.char_to_utf16_cu(line_start)
        }
    };
    Position::new(line as u32, column as u32)
}

/// 범위를 벗어난 줄·열은 가장 가까운 위치로 맞춘다(서버가 오래된 내용 기준으로 보낸 진단 등).
pub fn from_lsp(rope: &Rope, pos: Position, encoding: PositionEncoding) -> usize {
    let line = (pos.line as usize).min(rope.len_lines() - 1);
    let start = rope.line_to_char(line);
    let end = line_end(rope, line);
    let column = pos.character as usize;
    let char_idx = match encoding {
        PositionEncoding::Utf8 => {
            let byte = (rope.char_to_byte(start) + column).min(rope.char_to_byte(end));
            rope.byte_to_char(byte)
        }
        PositionEncoding::Utf16 => {
            let unit = (rope.char_to_utf16_cu(start) + column).min(rope.char_to_utf16_cu(end));
            rope.utf16_cu_to_char(unit)
        }
    };
    char_idx.clamp(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 한글과_이모지의_열_단위가_인코딩마다_다르다() {
        let rope = Rope::from_str("ab\n한글😀x\n");
        // 'x'는 둘째 줄의 3번째 문자(0부터): 한(3B/1U) 글(3B/1U) 😀(4B/2U)
        let x = 6;
        assert_eq!(
            to_lsp(&rope, x, PositionEncoding::Utf8),
            Position::new(1, 10)
        );
        assert_eq!(
            to_lsp(&rope, x, PositionEncoding::Utf16),
            Position::new(1, 4)
        );
        for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
            assert_eq!(from_lsp(&rope, to_lsp(&rope, x, enc), enc), x);
        }
    }

    #[test]
    fn 범위를_벗어난_위치는_줄_끝으로_맞춘다() {
        let rope = Rope::from_str("ab\ncd");
        assert_eq!(
            from_lsp(&rope, Position::new(0, 99), PositionEncoding::Utf16),
            2
        );
        assert_eq!(
            from_lsp(&rope, Position::new(9, 0), PositionEncoding::Utf8),
            3
        );
    }
}
