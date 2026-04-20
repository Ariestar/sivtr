use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use crate::app::App;
use sivtr_core::buffer::line::{AnsiColor, Line as CoreLine, StyledSpan as CoreStyledSpan};
use sivtr_core::selection::SelectionMode;

/// Convert an AnsiColor to a ratatui Color.
fn ansi_to_ratatui(c: &AnsiColor) -> Color {
    match c {
        AnsiColor::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
        AnsiColor::Indexed(idx) => match idx {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            9 => Color::LightRed,
            10 => Color::LightGreen,
            11 => Color::LightYellow,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::LightCyan,
            15 => Color::White,
            n => Color::Indexed(*n),
        },
    }
}

/// Convert a core StyledSpan to a ratatui Style.
fn span_to_style(span: &CoreStyledSpan) -> Style {
    let mut style = Style::default();
    if let Some(ref fg) = span.fg {
        style = style.fg(ansi_to_ratatui(fg));
    }
    if let Some(ref bg) = span.bg {
        style = style.bg(ansi_to_ratatui(bg));
    }
    if span.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if span.dim {
        style = style.add_modifier(Modifier::DIM);
    }
    if span.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if span.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

/// Render styled spans for a content line, respecting colors.
fn render_styled_content<'a>(
    content: &str,
    styles: &[CoreStyledSpan],
    spans: &mut Vec<Span<'a>>,
) {
    if styles.is_empty() {
        spans.push(Span::raw(content.to_string()));
        return;
    }
    for s in styles {
        let start = s.start.min(content.len());
        let end = s.end.min(content.len());
        if start >= end {
            continue;
        }
        let text = &content[start..end];
        let style = span_to_style(s);
        if style == Style::default() {
            spans.push(Span::raw(text.to_string()));
        } else {
            spans.push(Span::styled(text.to_string(), style));
        }
    }
}

fn split_by_display_cols(line: &CoreLine, start_col: usize, end_col_inclusive: usize) -> (String, String, String) {
    let max_width = line.display_width();
    let start = start_col.min(max_width);
    let end_exclusive = (end_col_inclusive + 1).min(max_width);
    let before = line.extract_by_display_cols(0, start);
    let selected = line.extract_by_display_cols(start, end_exclusive);
    let after = line.extract_by_display_cols(end_exclusive, max_width);
    (before, selected, after)
}

/// Render the main output browse view.
pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let buffer = &app.buffer;
    let offset = buffer.viewport.offset;
    let visible_height = area.height as usize;
    let preserve_colors = app.config.general.preserve_colors;

    let mut lines: Vec<Line> = Vec::with_capacity(visible_height);

    for i in 0..visible_height {
        let line_idx = offset + i;
        if let Some(content_line) = buffer.get_line(line_idx) {
            // Build the line number prefix
            let line_num = format!("{:>5} ", line_idx + 1);
            let mut spans = vec![Span::styled(
                line_num,
                Style::default().fg(Color::DarkGray),
            )];

            // Check if this line is within a selection
            if let Some(ref sel) = buffer.selection {
                let cursor = &buffer.cursor;
                let (top, bot) = sel.row_range(cursor);

                if line_idx >= top && line_idx <= bot {
                    match sel.mode {
                        SelectionMode::VisualLine => {
                            spans.push(Span::styled(
                                content_line.content.clone(),
                                Style::default().bg(Color::Blue).fg(Color::White),
                            ));
                        }
                        SelectionMode::VisualBlock => {
                            let (left, right) = sel.col_range(cursor);
                            let (before, selected, after) =
                                split_by_display_cols(content_line, left, right);
                            spans.push(Span::raw(before));
                            spans.push(Span::styled(
                                selected,
                                Style::default().bg(Color::Blue).fg(Color::White),
                            ));
                            spans.push(Span::raw(after));
                        }
                        SelectionMode::Visual => {
                            let anchor = &sel.anchor;
                            let (start, end) = if anchor.row < cursor.row
                                || (anchor.row == cursor.row && anchor.col <= cursor.col)
                            {
                                (*anchor, *cursor)
                            } else {
                                (*cursor, *anchor)
                            };

                            if line_idx == top && line_idx == bot {
                                let (before, selected, after) =
                                    split_by_display_cols(content_line, start.col, end.col);
                                spans.push(Span::raw(before));
                                spans.push(Span::styled(
                                    selected,
                                    Style::default().bg(Color::Blue).fg(Color::White),
                                ));
                                spans.push(Span::raw(after));
                            } else if line_idx == top {
                                let max_width = content_line.display_width().saturating_sub(1);
                                let (before, selected, _) =
                                    split_by_display_cols(content_line, start.col, max_width);
                                spans.push(Span::raw(before));
                                spans.push(Span::styled(
                                    selected,
                                    Style::default().bg(Color::Blue).fg(Color::White),
                                ));
                            } else if line_idx == bot {
                                let (_, selected, after) =
                                    split_by_display_cols(content_line, 0, end.col);
                                spans.push(Span::styled(
                                    selected,
                                    Style::default().bg(Color::Blue).fg(Color::White),
                                ));
                                spans.push(Span::raw(after));
                            } else {
                                spans.push(Span::styled(
                                    content_line.content.clone(),
                                    Style::default().bg(Color::Blue).fg(Color::White),
                                ));
                            }
                        }
                    }
                } else {
                    render_content_with_search(app, line_idx, content_line, preserve_colors, &mut spans);
                }
            } else {
                render_content_with_search(app, line_idx, content_line, preserve_colors, &mut spans);
            }

            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(vec![
                Span::styled("    ~ ", Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines);
    paragraph.render(area, buf);
}

/// Render line content with search match highlighting and optional ANSI colors.
fn render_content_with_search<'a>(
    app: &App,
    line_idx: usize,
    content_line: &sivtr_core::buffer::line::Line,
    preserve_colors: bool,
    spans: &mut Vec<Span<'a>>,
) {
    let content = &content_line.content;

    if let Some(ref search_state) = app.search_state {
        let line_matches: Vec<_> = search_state
            .matches
            .iter()
            .enumerate()
            .filter(|(_, m)| m.row == line_idx)
            .collect();

        if line_matches.is_empty() {
            if preserve_colors {
                render_styled_content(content, &content_line.styles, spans);
            } else {
                spans.push(Span::raw(content.to_string()));
            }
        } else {
            // Search matches override ANSI colors in matched regions
            let mut last_end: usize = 0;
            for (match_idx, m) in &line_matches {
                if m.byte_start > last_end {
                    if preserve_colors {
                        render_styled_slice(content, &content_line.styles, last_end, m.byte_start, spans);
                    } else {
                        spans.push(Span::raw(content[last_end..m.byte_start].to_string()));
                    }
                }

                let is_current = search_state.current == Some(*match_idx);
                let style = if is_current {
                    Style::default().bg(Color::Yellow).fg(Color::Black)
                } else {
                    Style::default().bg(Color::DarkGray).fg(Color::White)
                };
                spans.push(Span::styled(
                    content[m.byte_start..m.byte_end].to_string(),
                    style,
                ));
                last_end = m.byte_end;
            }
            if last_end < content.len() {
                if preserve_colors {
                    render_styled_slice(content, &content_line.styles, last_end, content.len(), spans);
                } else {
                    spans.push(Span::raw(content[last_end..].to_string()));
                }
            }
        }
    } else {
        if preserve_colors {
            render_styled_content(content, &content_line.styles, spans);
        } else {
            spans.push(Span::raw(content.to_string()));
        }
    }
}

/// Render a byte-range slice of content using the overlapping styled spans.
fn render_styled_slice<'a>(
    content: &str,
    styles: &[CoreStyledSpan],
    slice_start: usize,
    slice_end: usize,
    spans: &mut Vec<Span<'a>>,
) {
    // Find spans that overlap [slice_start, slice_end)
    let mut pos = slice_start;
    for s in styles {
        if s.end <= slice_start || s.start >= slice_end {
            continue;
        }
        let start = s.start.max(slice_start);
        let end = s.end.min(slice_end);

        // Gap before this span (unstyled)
        if start > pos {
            spans.push(Span::raw(content[pos..start].to_string()));
        }

        let style = span_to_style(s);
        if style == Style::default() {
            spans.push(Span::raw(content[start..end].to_string()));
        } else {
            spans.push(Span::styled(content[start..end].to_string(), style));
        }
        pos = end;
    }

    // Remaining unstyled tail
    if pos < slice_end {
        spans.push(Span::raw(content[pos..slice_end].to_string()));
    }
}
