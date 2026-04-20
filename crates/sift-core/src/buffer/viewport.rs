/// Viewport represents the visible window into the buffer.
#[derive(Debug, Clone)]
pub struct Viewport {
    /// First visible line index (0-based).
    pub offset: usize,
    /// Number of visible lines.
    pub height: usize,
    /// Number of visible columns.
    pub width: usize,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            offset: 0,
            height: 24,
            width: 80,
        }
    }
}
