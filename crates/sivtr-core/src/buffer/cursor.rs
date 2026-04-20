/// Cursor position in the buffer, using display columns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    /// Row index (0-based line number).
    pub row: usize,
    /// Column index (0-based display column).
    pub col: usize,
}

impl Cursor {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }
}
