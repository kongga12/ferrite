//! 구문 강조 색과 에디터 연결.

use std::ops::Range;

use eframe::egui::Color32;
use ferrite_core::buffer::Buffer;
use ferrite_core::syntax::{Highlight, Syntax};

use crate::editor_view::{LineStyles, Styler};

pub fn highlight_color(kind: Highlight, dark: bool) -> Color32 {
    let hex = match (kind, dark) {
        (Highlight::Keyword, true) => 0xCC7832,
        (Highlight::Keyword, false) => 0x0033B3,
        (Highlight::String, true) => 0x6A8759,
        (Highlight::String, false) => 0x067D17,
        (Highlight::Comment, true) => 0x808080,
        (Highlight::Comment, false) => 0x8C8C8C,
        (Highlight::Function, true) => 0xFFC66D,
        (Highlight::Function, false) => 0x00627A,
        (Highlight::Type, true) => 0x4EC9B0,
        (Highlight::Type, false) => 0x20999D,
        (Highlight::Constant, true) => 0x6897BB,
        (Highlight::Constant, false) => 0x1750EB,
        (Highlight::Property, true) => 0x9876AA,
        (Highlight::Property, false) => 0x871094,
        (Highlight::Tag, true) => 0xE8BF6A,
        (Highlight::Tag, false) => 0x0033B3,
    };
    let [_, r, g, b] = u32::to_be_bytes(hex);
    Color32::from_rgb(r, g, b)
}

impl Styler for ferrite_textmate::TextMateSyntax {
    fn styles(&mut self, buffer: &mut Buffer, lines: Range<usize>, dark: bool) -> LineStyles {
        self.update(buffer);
        colored(self.highlight_lines(buffer.rope(), lines), dark)
    }
}

fn colored(lines: ferrite_core::syntax::LineHighlights, dark: bool) -> LineStyles {
    lines
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(range, kind)| (range, highlight_color(kind, dark)))
                .collect()
        })
        .collect()
}

impl Styler for Syntax {
    fn styles(&mut self, buffer: &mut Buffer, lines: Range<usize>, dark: bool) -> LineStyles {
        self.update(buffer);
        colored(self.highlight_lines(buffer.rope(), lines), dark)
    }
}
