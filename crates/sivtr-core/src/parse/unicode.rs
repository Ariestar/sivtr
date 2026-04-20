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

/// Given a string and a display column range [col_start, col_end),
/// return the byte range and char range that covers those display columns.
///
/// Returns (char_start_idx, char_end_idx) 鈥?indices into the char iterator.
pub fn display_col_to_char_range(s: &str, col_start: usize, col_end: usize) -> (usize, usize) {
    let mut current_col = 0usize;
    let mut char_start = None;
    let mut char_end = 0;

    for (i, ch) in s.chars().enumerate() {
        let w = if ch == '\t' {
            8
        } else {
            ch.width().unwrap_or(0)
        };

        if char_start.is_none() && current_col + w > col_start {
            char_start = Some(i);
        }

        current_col += w;

        if char_start.is_some() {
            char_end = i + 1;
        }

        if current_col >= col_end {
            break;
        }
    }

    (char_start.unwrap_or(0), char_end)
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
        let widths = compute_display_widths("浣犲ソ");
        assert_eq!(widths, vec![2, 2]);
    }

    #[test]
    fn test_mixed_widths() {
        let widths = compute_display_widths("hi浣犲ソ");
        assert_eq!(widths, vec![1, 1, 2, 2]);
    }

    #[test]
    fn test_display_width() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width("浣犲ソ"), 4);
        assert_eq!(display_width("hi浣犲ソ"), 6);
    }

    #[test]
    fn test_tab_width() {
        let widths = compute_display_widths("\t");
        assert_eq!(widths, vec![8]);
    }

    #[test]
    fn test_display_col_to_char_range_ascii() {
        // "hello" 鈥?select columns 1..4 鈫?chars 1..4 = "ell"
        let (start, end) = display_col_to_char_range("hello", 1, 4);
        assert_eq!(start, 1);
        assert_eq!(end, 4);
    }

    #[test]
    fn test_display_col_to_char_range_cjk() {
        // "浣犲ソ涓栫晫" 鈥?each char is 2 cols wide
        // cols 0..4 鈫?chars 0..2 = "浣犲ソ"
        let (start, end) = display_col_to_char_range("浣犲ソ涓栫晫", 0, 4);
        assert_eq!(start, 0);
        assert_eq!(end, 2);
    }

    #[test]
    fn test_display_col_to_char_range_mixed() {
        // "hi浣犲ソ" 鈥?h(1) i(1) 浣?2) 濂?2) = cols 0,1,2-3,4-5
        // cols 1..5 鈫?chars 1..4 = "i浣犲ソ"... wait, col 5 is end of 濂?
        let (start, end) = display_col_to_char_range("hi浣犲ソ", 1, 5);
        assert_eq!(start, 1);
        assert_eq!(end, 4); // 'i', '浣?, '濂?
    }
}
