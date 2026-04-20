pub mod ansi;
pub mod unicode;

use crate::buffer::line::Line;

/// Parse raw text (potentially containing ANSI escape sequences) into Line objects.
pub fn parse_lines(raw: &str) -> Vec<Line> {
    raw.lines()
        .map(|line_str| {
            let clean = ansi::strip_ansi(line_str);
            let display_widths = unicode::compute_display_widths(&clean);
            let styles = ansi::parse_styles(line_str);
            Line {
                content: clean,
                display_widths,
                styles,
            }
        })
        .collect()
}
