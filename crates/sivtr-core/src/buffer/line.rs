/// An ANSI color value.
#[derive(Debug, Clone, PartialEq)]
pub enum AnsiColor {
    /// Standard/bright color (0-15).
    Indexed(u8),
    /// RGB true color.
    Rgb(u8, u8, u8),
}

/// Style information for a span of text within a line.
#[derive(Debug, Clone, PartialEq)]
pub struct StyledSpan {
    /// Start byte offset in the cleaned content.
    pub start: usize,
    /// End byte offset (exclusive) in the cleaned content.
    pub end: usize,
    /// Foreground color.
    pub fg: Option<AnsiColor>,
    /// Background color.
    pub bg: Option<AnsiColor>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
}

/// A single line of terminal output.
#[derive(Debug, Clone)]
pub struct Line {
    /// Plain text content (ANSI stripped).
    pub content: String,
    /// Display width of each character (0, 1, or 2 for wide chars).
    pub display_widths: Vec<u8>,
    /// Style spans for colored rendering.
    pub styles: Vec<StyledSpan>,
}

impl Line {
    /// Total display width of this line.
    pub fn display_width(&self) -> usize {
        self.display_widths.iter().map(|&w| w as usize).sum()
    }

    /// Number of characters in this line.
    pub fn char_count(&self) -> usize {
        self.content.chars().count()
    }

    /// Convert a display column to the corresponding character index.
    pub fn char_index_for_display_col(&self, target_col: usize) -> usize {
        let mut display_col = 0usize;
        for (idx, width) in self.display_widths.iter().enumerate() {
            let width = *width as usize;
            if display_col + width > target_col {
                return idx;
            }
            display_col += width;
        }
        self.display_widths.len()
    }

    /// Convert a character index to its starting display column.
    pub fn display_col_for_char_index(&self, char_idx: usize) -> usize {
        self.display_widths
            .iter()
            .take(char_idx.min(self.display_widths.len()))
            .map(|&w| w as usize)
            .sum()
    }

    /// Extract a substring by display column range [col_start, col_end).
    /// Returns the extracted string. Short lines return what's available.
    pub fn extract_by_display_cols(&self, col_start: usize, col_end: usize) -> String {
        let (char_start, char_end) =
            crate::parse::unicode::display_col_to_char_range(&self.content, col_start, col_end);
        self.content.chars().skip(char_start).take(char_end - char_start).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_line(s: &str) -> Line {
        let content = s.to_string();
        let display_widths = crate::parse::unicode::compute_display_widths(&content);
        Line {
            content,
            display_widths,
            styles: Vec::new(),
        }
    }

    #[test]
    fn test_display_width() {
        let line = make_line("hello");
        assert_eq!(line.display_width(), 5);
    }

    #[test]
    fn test_display_width_cjk() {
        let line = make_line("浣犲ソ");
        assert_eq!(line.display_width(), 4);
    }

    #[test]
    fn test_extract_ascii() {
        let line = make_line("hello world");
        assert_eq!(line.extract_by_display_cols(0, 5), "hello");
    }

    #[test]
    fn test_extract_cjk() {
        let line = make_line("浣犲ソ涓栫晫");
        assert_eq!(line.extract_by_display_cols(0, 4), "浣犲ソ");
    }

    #[test]
    fn test_extract_beyond_line() {
        let line = make_line("hi");
        // Requesting cols 0..10 on a 2-char line should return "hi"
        assert_eq!(line.extract_by_display_cols(0, 10), "hi");
    }

    #[test]
    fn test_char_index_for_display_col() {
        let line = make_line("a濂絙");
        assert_eq!(line.char_index_for_display_col(0), 0);
        assert_eq!(line.char_index_for_display_col(1), 1);
        assert_eq!(line.char_index_for_display_col(2), 1);
        assert_eq!(line.char_index_for_display_col(3), 2);
        assert_eq!(line.char_index_for_display_col(4), 3);
    }

    #[test]
    fn test_display_col_for_char_index() {
        let line = make_line("a濂絙");
        assert_eq!(line.display_col_for_char_index(0), 0);
        assert_eq!(line.display_col_for_char_index(1), 1);
        assert_eq!(line.display_col_for_char_index(2), 3);
        assert_eq!(line.display_col_for_char_index(3), 4);
    }
}
