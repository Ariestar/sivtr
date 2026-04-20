use regex::Regex;
use crate::buffer::line::Line;

/// A single search match within a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// Line index in the buffer.
    pub row: usize,
    /// Byte start offset in the line content.
    pub byte_start: usize,
    /// Byte end offset (exclusive) in the line content.
    pub byte_end: usize,
}

/// Find all matches of a pattern across all lines.
///
/// If `use_regex` is true, the pattern is interpreted as a regex.
/// Otherwise, it's treated as a literal string (case-insensitive).
pub fn find_all(lines: &[Line], pattern: &str, use_regex: bool) -> Vec<SearchMatch> {
    if pattern.is_empty() {
        return Vec::new();
    }

    let re = if use_regex {
        match Regex::new(pattern) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        }
    } else {
        // Escape the pattern and make it case-insensitive
        let escaped = regex::escape(pattern);
        match Regex::new(&format!("(?i){}", escaped)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        }
    };

    let mut matches = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        for m in re.find_iter(&line.content) {
            matches.push(SearchMatch {
                row,
                byte_start: m.start(),
                byte_end: m.end(),
            });
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn test_find_literal() {
        let lines = parse::parse_lines("hello world\nfoo hello bar\nbaz");
        let matches = find_all(&lines, "hello", false);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].row, 0);
        assert_eq!(matches[1].row, 1);
    }

    #[test]
    fn test_find_case_insensitive() {
        let lines = parse::parse_lines("Hello World\nhello world");
        let matches = find_all(&lines, "hello", false);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_find_regex() {
        let lines = parse::parse_lines("error: foo\nwarning: bar\nerror: baz");
        let matches = find_all(&lines, r"^error:", true);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_find_empty_pattern() {
        let lines = parse::parse_lines("hello");
        let matches = find_all(&lines, "", false);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_find_no_match() {
        let lines = parse::parse_lines("hello world");
        let matches = find_all(&lines, "xyz", false);
        assert!(matches.is_empty());
    }
}
