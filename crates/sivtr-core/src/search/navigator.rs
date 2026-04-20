use super::matcher::SearchMatch;

/// Holds the current search state including all matches and current index.
#[derive(Debug, Clone)]
pub struct SearchState {
    pub pattern: String,
    pub matches: Vec<SearchMatch>,
    pub current: Option<usize>,
}

impl SearchState {
    pub fn new(pattern: String, matches: Vec<SearchMatch>) -> Self {
        let current = if matches.is_empty() { None } else { Some(0) };
        Self {
            pattern,
            matches,
            current,
        }
    }

    /// Jump to the next match. Wraps around.
    pub fn next(&mut self) {
        if let Some(ref mut idx) = self.current {
            *idx = (*idx + 1) % self.matches.len();
        }
    }

    /// Jump to the previous match. Wraps around.
    pub fn prev(&mut self) {
        if let Some(ref mut idx) = self.current {
            if *idx == 0 {
                *idx = self.matches.len() - 1;
            } else {
                *idx -= 1;
            }
        }
    }

    /// Get the current match, if any.
    pub fn current_match(&self) -> Option<&SearchMatch> {
        self.current.and_then(|idx| self.matches.get(idx))
    }

    /// Total number of matches.
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// Find the nearest match at or after the given row, and set it as current.
    pub fn jump_to_row(&mut self, row: usize) {
        if self.matches.is_empty() {
            return;
        }
        // Find first match at or after this row
        let idx = self
            .matches
            .iter()
            .position(|m| m.row >= row)
            .unwrap_or(0);
        self.current = Some(idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_matches() -> Vec<SearchMatch> {
        vec![
            SearchMatch { row: 0, byte_start: 0, byte_end: 3 },
            SearchMatch { row: 2, byte_start: 5, byte_end: 8 },
            SearchMatch { row: 5, byte_start: 0, byte_end: 3 },
        ]
    }

    #[test]
    fn test_next_wraps() {
        let mut state = SearchState::new("test".into(), make_matches());
        assert_eq!(state.current, Some(0));
        state.next();
        assert_eq!(state.current, Some(1));
        state.next();
        assert_eq!(state.current, Some(2));
        state.next();
        assert_eq!(state.current, Some(0)); // wrapped
    }

    #[test]
    fn test_prev_wraps() {
        let mut state = SearchState::new("test".into(), make_matches());
        state.prev();
        assert_eq!(state.current, Some(2)); // wrapped to end
    }

    #[test]
    fn test_jump_to_row() {
        let mut state = SearchState::new("test".into(), make_matches());
        state.jump_to_row(3);
        assert_eq!(state.current, Some(2)); // match at row 5 is first >= 3
    }
}
