pub mod line;
pub mod viewport;
pub mod cursor;

use line::Line;
use viewport::Viewport;
use cursor::Cursor;
use crate::selection::Selection;

/// The main buffer holding all parsed lines and view state.
pub struct Buffer {
    pub lines: Vec<Line>,
    pub viewport: Viewport,
    pub cursor: Cursor,
    pub selection: Option<Selection>,
}

impl Buffer {
    /// Create a new buffer from parsed lines.
    pub fn new(lines: Vec<Line>) -> Self {
        Self {
            lines,
            viewport: Viewport::default(),
            cursor: Cursor::default(),
            selection: None,
        }
    }

    /// Total number of lines in the buffer.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Get a reference to a line by index, if it exists.
    pub fn get_line(&self, index: usize) -> Option<&Line> {
        self.lines.get(index)
    }

    /// Get the lines currently visible in the viewport.
    pub fn visible_lines(&self) -> &[Line] {
        let start = self.viewport.offset;
        let end = (start + self.viewport.height).min(self.lines.len());
        &self.lines[start..end]
    }

    /// Scroll down by n lines, clamping to bounds.
    pub fn scroll_down(&mut self, n: usize) {
        let max_offset = self.lines.len().saturating_sub(self.viewport.height);
        self.viewport.offset = (self.viewport.offset + n).min(max_offset);
    }

    /// Scroll up by n lines, clamping to bounds.
    pub fn scroll_up(&mut self, n: usize) {
        self.viewport.offset = self.viewport.offset.saturating_sub(n);
    }

    /// Move cursor down, scrolling viewport if needed.
    pub fn cursor_down(&mut self, n: usize) {
        let max_row = self.lines.len().saturating_sub(1);
        self.cursor.row = (self.cursor.row + n).min(max_row);
        self.ensure_cursor_visible();
    }

    /// Move cursor up, scrolling viewport if needed.
    pub fn cursor_up(&mut self, n: usize) {
        self.cursor.row = self.cursor.row.saturating_sub(n);
        self.ensure_cursor_visible();
    }

    /// Move cursor right within the current line.
    pub fn cursor_right(&mut self, n: usize) {
        if let Some(line) = self.lines.get(self.cursor.row) {
            let max_col = line.display_width().saturating_sub(1);
            self.cursor.col = (self.cursor.col + n).min(max_col);
        }
    }

    /// Move cursor left within the current line.
    pub fn cursor_left(&mut self, n: usize) {
        self.cursor.col = self.cursor.col.saturating_sub(n);
    }

    /// Jump cursor to the first line.
    pub fn cursor_top(&mut self) {
        self.cursor.row = 0;
        self.cursor.col = 0;
        self.ensure_cursor_visible();
    }

    /// Jump cursor to the last line.
    pub fn cursor_bottom(&mut self) {
        self.cursor.row = self.lines.len().saturating_sub(1);
        self.ensure_cursor_visible();
    }

    /// Half-page down.
    pub fn half_page_down(&mut self) {
        let half = self.viewport.height / 2;
        self.cursor_down(half);
    }

    /// Half-page up.
    pub fn half_page_up(&mut self) {
        let half = self.viewport.height / 2;
        self.cursor_up(half);
    }

    /// Ensure the cursor row is within the visible viewport, adjusting offset if needed.
    fn ensure_cursor_visible(&mut self) {
        if self.cursor.row < self.viewport.offset {
            self.viewport.offset = self.cursor.row;
        } else if self.cursor.row >= self.viewport.offset + self.viewport.height {
            self.viewport.offset = self.cursor.row - self.viewport.height + 1;
        }
    }

    /// Update viewport dimensions (called on terminal resize).
    pub fn resize(&mut self, width: usize, height: usize) {
        self.viewport.width = width;
        self.viewport.height = height;
    }

    /// Public version of ensure_cursor_visible for external callers.
    pub fn ensure_cursor_visible_pub(&mut self) {
        self.ensure_cursor_visible();
    }
}
