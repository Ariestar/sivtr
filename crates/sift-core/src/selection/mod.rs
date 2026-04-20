pub mod mode;
pub mod extract;

use crate::buffer::cursor::Cursor;

/// Selection mode matching Vim visual modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Character-wise selection (v).
    Visual,
    /// Line-wise selection (V).
    VisualLine,
    /// Block/column selection (Ctrl-V).
    VisualBlock,
}

/// A selection region defined by an anchor and the current cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub mode: SelectionMode,
    /// The position where selection started.
    pub anchor: Cursor,
}

impl Selection {
    pub fn new(mode: SelectionMode, anchor: Cursor) -> Self {
        Self { mode, anchor }
    }

    /// Get the ordered row range (top, bottom) given the current cursor.
    pub fn row_range(&self, cursor: &Cursor) -> (usize, usize) {
        let r1 = self.anchor.row;
        let r2 = cursor.row;
        (r1.min(r2), r1.max(r2))
    }

    /// Get the ordered column range (left, right) given the current cursor.
    /// Only meaningful for Visual and VisualBlock modes.
    pub fn col_range(&self, cursor: &Cursor) -> (usize, usize) {
        let c1 = self.anchor.col;
        let c2 = cursor.col;
        (c1.min(c2), c1.max(c2))
    }
}
