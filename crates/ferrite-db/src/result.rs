//! 결과 그리드가 들고 있는 행. 받은 페이지를 뒤에 붙이고, 상한을 넘으면 앞쪽을 버린다. 커서는 앞으로만
//! 읽으므로 버린 행은 다시 받을 수 없다(다시 보려면 다시 실행한다). 그래서 100만 행을 끝까지 내려도
//! 메모리는 상한만큼만 쓴다.

use std::collections::VecDeque;

use crate::{Cell, Column, PAGE_ROWS, Row};

/// 기본 상한(행).
pub const ROW_LIMIT: usize = 100_000;

pub struct ResultBuffer {
    pub columns: Vec<Column>,
    rows: VecDeque<Row>,
    /// 앞에서 버린 행 수. 화면의 행 번호는 이것을 더해 센다.
    dropped: usize,
    limit: usize,
    more: bool,
    /// 다음 페이지를 요청하고 기다리는 중.
    waiting: bool,
}

impl ResultBuffer {
    pub fn new(columns: Vec<Column>, limit: usize) -> Self {
        Self {
            columns,
            rows: VecDeque::new(),
            dropped: 0,
            limit: limit.max(PAGE_ROWS),
            more: true,
            waiting: false,
        }
    }

    /// 받은 페이지를 붙인다.
    pub fn push(&mut self, rows: Vec<Row>, more: bool) {
        self.rows.extend(rows);
        self.more = more;
        self.waiting = false;
        let excess = self.rows.len().saturating_sub(self.limit);
        if excess > 0 {
            self.rows.drain(..excess);
            self.dropped += excess;
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// 지금까지 받은 행 수(버린 것 포함).
    pub fn received(&self) -> usize {
        self.dropped + self.rows.len()
    }

    /// 받은 행 번호(버린 행 포함, 0부터)의 칸을 바꾼다. 이미 버린 행이면 false.
    pub fn set(&mut self, number: usize, column: usize, cell: Cell) -> bool {
        let slot = number
            .checked_sub(self.dropped)
            .and_then(|index| self.rows.get_mut(index))
            .and_then(|row| row.get_mut(column));
        match slot {
            Some(slot) => {
                *slot = cell;
                true
            }
            None => false,
        }
    }

    pub fn has_more(&self) -> bool {
        self.more
    }

    /// 화면에 보이는 마지막 행(버퍼 안 번호)이 끝에 가까우면 다음 페이지를 요청할 때다. 한 번 요청하면
    /// 페이지가 올 때까지 다시 요청하지 않는다.
    pub fn wants_more(&mut self, visible_end: usize) -> bool {
        if self.more && !self.waiting && visible_end + PAGE_ROWS >= self.rows.len() {
            self.waiting = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(from: usize, n: usize) -> Vec<Row> {
        (from..from + n)
            .map(|i| vec![Cell::Int(i as i64)])
            .collect()
    }

    #[test]
    fn 상한을_넘으면_앞쪽을_버리고_번호를_이어_센다() {
        let mut buffer = ResultBuffer::new(Vec::new(), 1_000);
        assert!(buffer.wants_more(0));
        assert!(
            !buffer.wants_more(0),
            "기다리는 동안은 다시 요청하지 않는다"
        );
        buffer.push(page(0, 500), true);
        buffer.push(page(500, 500), true);
        buffer.push(page(1_000, 500), true);
        assert_eq!(
            (buffer.len(), buffer.dropped(), buffer.received()),
            (1_000, 500, 1_500)
        );
        assert_eq!(buffer.row(0), Some(&vec![Cell::Int(500)]));
        assert!(!buffer.wants_more(0), "위쪽을 보고 있으면 더 받지 않는다");
        assert!(buffer.wants_more(990));
        buffer.push(page(1_500, 10), false);
        assert!(!buffer.has_more());
        assert!(!buffer.wants_more(1_000), "끝까지 받았다");

        // 칸은 받은 행 번호로 고친다. 버린 행은 고치지 않는다.
        assert!(buffer.set(1_509, 0, Cell::Null));
        assert_eq!(buffer.row(buffer.len() - 1), Some(&vec![Cell::Null]));
        assert!(!buffer.set(3, 0, Cell::Null), "버린 행");
        assert!(!buffer.set(1_509, 1, Cell::Null), "없는 열");
    }
}
