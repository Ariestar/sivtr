use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Alignment, Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use regex::Regex;

use crate::tui::pane::{panel_block, Panel};

pub(crate) struct ContentView<'a> {
    pub(crate) text: &'a str,
    pub(crate) scroll: usize,
    pub(crate) search_regex: Option<&'a Regex>,
}

pub(crate) fn render_content_view(
    frame: &mut Frame,
    area: Rect,
    panel: Panel,
    view: ContentView<'_>,
) {
    let block = panel_block(&panel);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let total_lines = line_count(view.text);
    let line_number_width = line_number_width(total_lines);
    let gutter_width = line_number_width.saturating_add(1);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(gutter_width),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    let visible_height = inner.height as usize;
    let scroll = view.scroll.min(total_lines.saturating_sub(1));
    frame.render_widget(
        Paragraph::new(line_number_lines(total_lines, scroll, visible_height))
            .alignment(Alignment::Right),
        chunks[0],
    );
    frame.render_widget(Paragraph::new(separator_lines(visible_height)), chunks[1]);
    frame.render_widget(
        Paragraph::new(content_lines(
            view.text,
            scroll,
            visible_height,
            view.search_regex,
        )),
        chunks[2],
    );
}

fn content_lines(
    text: &str,
    scroll: usize,
    height: usize,
    search_regex: Option<&Regex>,
) -> Text<'static> {
    let lines = raw_lines(text);
    let visible = lines
        .iter()
        .skip(scroll)
        .take(height)
        .map(|line| styled_content_line(line, search_regex))
        .collect::<Vec<_>>();
    Text::from(visible)
}

fn line_number_lines(total_lines: usize, scroll: usize, height: usize) -> Text<'static> {
    let style = Style::default().fg(Color::DarkGray);
    let lines = (scroll..total_lines.min(scroll.saturating_add(height)))
        .map(|idx| Line::from(Span::styled((idx + 1).to_string(), style)))
        .collect::<Vec<_>>();
    Text::from(lines)
}

fn separator_lines(height: usize) -> Text<'static> {
    let style = Style::default().fg(Color::DarkGray);
    Text::from(
        (0..height)
            .map(|_| Line::from(Span::styled("│", style)))
            .collect::<Vec<_>>(),
    )
}

fn styled_content_line(line: &str, search_regex: Option<&Regex>) -> Line<'static> {
    if search_regex.is_some() {
        return Line::from(highlight_spans(line, search_regex, Style::default()));
    }

    if let Some((prefix, rest)) = line.split_once("## User") {
        Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled("## User", Style::default().fg(Color::Cyan)),
            Span::raw(rest.to_string()),
        ])
    } else if let Some((prefix, rest)) = line.split_once("## Assistant") {
        Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled("## Assistant", Style::default().fg(Color::Green)),
            Span::raw(rest.to_string()),
        ])
    } else {
        Line::from(line.to_string())
    }
}

pub(crate) fn highlight_spans(
    text: &str,
    regex: Option<&Regex>,
    base_style: Style,
) -> Vec<Span<'static>> {
    let Some(regex) = regex else {
        return vec![Span::styled(text.to_string(), base_style)];
    };

    let mut spans = Vec::new();
    let mut cursor = 0;
    for matched in regex.find_iter(text) {
        if matched.start() == matched.end() {
            continue;
        }
        if matched.start() > cursor {
            spans.push(Span::styled(
                text[cursor..matched.start()].to_string(),
                base_style,
            ));
        }
        spans.push(Span::styled(
            text[matched.start()..matched.end()].to_string(),
            base_style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
        cursor = matched.end();
    }

    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), base_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }
    spans
}

pub(crate) fn line_count(text: &str) -> usize {
    raw_lines(text).len()
}

fn line_number_width(line_count: usize) -> u16 {
    line_count.max(1).to_string().len() as u16
}

fn raw_lines(text: &str) -> Vec<&str> {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        vec![""]
    } else {
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{content_lines, line_count, line_number_width};

    #[test]
    fn counts_empty_content_as_one_display_line() {
        assert_eq!(line_count(""), 1);
    }

    #[test]
    fn line_number_width_scales_with_line_count() {
        assert_eq!(line_number_width(9), 1);
        assert_eq!(line_number_width(10), 2);
        assert_eq!(line_number_width(100), 3);
    }

    #[test]
    fn content_lines_preserve_blank_lines_without_number_prefixes() {
        let rendered = content_lines("alpha\n\nomega", 0, 3, None);

        assert_eq!(rendered.lines.len(), 3);
        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "alpha");
        assert!(rendered.lines[1].spans.is_empty());
        assert_eq!(rendered.lines[2].spans[0].content.as_ref(), "omega");
    }
}
