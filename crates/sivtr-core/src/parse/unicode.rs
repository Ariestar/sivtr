use unicode_width::UnicodeWidthChar;

/// Compute the display width of each character in the string.
///
/// Returns a Vec where each element is the display column width (0, 1, or 2)
/// of the corresponding character. This is essential for correct cursor
/// positioning and block selection with CJK/wide characters.
pub fn compute_display_widths(s: &str) -> Vec<u8> {
    s.chars()
        .map(|ch| {
            if ch == '\t' {
                // Tab is treated as 8 spaces for display purposes.
                // This can be made configurable later.
                8u8
            } else {
                ch.width().unwrap_or(0) as u8
            }
        })
        .collect()
}

/// Compute the total display width of a string.
pub fn display_width(s: &str) -> usize {
    compute_display_widths(s).iter().map(|&w| w as usize).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_widths() {
        let widths = compute_display_widths("hello");
        assert_eq!(widths, vec![1, 1, 1, 1, 1]);
    }

    #[test]
    fn test_cjk_widths() {
        let widths = compute_display_widths("你好");
        assert_eq!(widths, vec![2, 2]);
    }

    #[test]
    fn test_mixed_widths() {
        let widths = compute_display_widths("hi你好");
        assert_eq!(widths, vec![1, 1, 2, 2]);
    }

    #[test]
    fn test_display_width() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width("你好"), 4);
        assert_eq!(display_width("hi你好"), 6);
    }

    #[test]
    fn test_tab_width() {
        let widths = compute_display_widths("\t");
        assert_eq!(widths, vec![8]);
    }
}
