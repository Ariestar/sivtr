use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Alignment, Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use regex::Regex;

use crate::tui::content_markdown::render_markdown_window;
use crate::tui::pane::{panel_block, Panel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContentViewMode {
    Raw,
    Reading,
}

impl ContentViewMode {
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Raw => Self::Reading,
            Self::Reading => Self::Raw,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Reading => "read",
        }
    }
}

pub(crate) struct ContentView<'a> {
    pub(crate) text: &'a str,
    pub(crate) scroll: usize,
    pub(crate) search_regex: Option<&'a Regex>,
    pub(crate) mode: ContentViewMode,
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
            view.mode,
        )),
        chunks[2],
    );
}

fn content_lines(
    text: &str,
    scroll: usize,
    height: usize,
    search_regex: Option<&Regex>,
    mode: ContentViewMode,
) -> Text<'static> {
    let lines = raw_lines(text);
    let visible = match mode {
        ContentViewMode::Raw => raw_content_window(&lines, scroll, height),
        ContentViewMode::Reading => render_markdown_window(&lines, scroll, height),
    }
    .into_iter()
    .map(|line| styled_content_line(line, search_regex))
    .collect::<Vec<_>>();
    Text::from(visible)
}

fn raw_content_window(lines: &[&str], scroll: usize, height: usize) -> Vec<Line<'static>> {
    lines
        .iter()
        .skip(scroll)
        .take(height)
        .map(|line| Line::from((*line).to_string()))
        .collect()
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
            .map(|_| Line::from(Span::styled("|", style)))
            .collect::<Vec<_>>(),
    )
}

fn styled_content_line(line: Line<'static>, search_regex: Option<&Regex>) -> Line<'static> {
    if search_regex.is_some() {
        return highlight_line(line, search_regex);
    }

    line
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

fn highlight_line(line: Line<'static>, regex: Option<&Regex>) -> Line<'static> {
    let Some(regex) = regex else {
        return line;
    };
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    if text.is_empty() {
        return line;
    }

    let matches = regex
        .find_iter(&text)
        .filter(|matched| matched.start() != matched.end())
        .map(|matched| matched.start()..matched.end())
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return line;
    }

    let mut offset = 0usize;
    let spans = line
        .spans
        .into_iter()
        .flat_map(|span| {
            let start = offset;
            let end = start + span.content.len();
            offset = end;
            split_span_by_matches(span, start, end, &matches)
        })
        .collect::<Vec<_>>();

    Line {
        spans,
        style: line.style,
        alignment: line.alignment,
    }
}

fn split_span_by_matches(
    span: Span<'static>,
    span_start: usize,
    span_end: usize,
    matches: &[std::ops::Range<usize>],
) -> Vec<Span<'static>> {
    if span_start == span_end {
        return vec![span];
    }

    let mut pieces = Vec::new();
    let text = span.content.to_string();
    let mut cursor = span_start;
    for matched in matches {
        if matched.end <= span_start || matched.start >= span_end {
            continue;
        }
        let start = matched.start.max(span_start);
        let end = matched.end.min(span_end);
        if start > cursor {
            pieces.push(Span::styled(
                text[cursor - span_start..start - span_start].to_string(),
                span.style,
            ));
        }
        pieces.push(Span::styled(
            text[start - span_start..end - span_start].to_string(),
            span.style.fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
        cursor = end;
    }
    if cursor < span_end {
        pieces.push(Span::styled(
            text[cursor - span_start..].to_string(),
            span.style,
        ));
    }

    if pieces.is_empty() {
        vec![span]
    } else {
        pieces
    }
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
    use super::{content_lines, line_count, line_number_width, ContentViewMode};
    use ratatui::prelude::{Color, Modifier};
    use regex::Regex;

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
        let rendered = content_lines("alpha\n\nomega", 0, 3, None, ContentViewMode::Reading);

        assert_eq!(rendered.lines.len(), 3);
        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "alpha");
        assert!(rendered.lines[1].spans.is_empty());
        assert_eq!(rendered.lines[2].spans[0].content.as_ref(), "omega");
    }

    #[test]
    fn content_lines_render_markdown_without_changing_line_count() {
        let rendered = content_lines(
            "## User\n**bold** and `code`",
            0,
            2,
            None,
            ContentViewMode::Reading,
        );

        assert_eq!(rendered.lines.len(), 2);
        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "## ");
        assert_eq!(rendered.lines[0].spans[1].content.as_ref(), "User");
        assert_eq!(rendered.lines[0].spans[1].style.fg, Some(Color::Cyan));
        assert!(rendered.lines[1].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(rendered.lines[1].spans[2].content.as_ref(), "code");
        assert_eq!(
            rendered.lines[1].spans[2].style.fg,
            Some(Color::LightYellow)
        );
        assert_eq!(line_count("## User\n**bold** and `code`"), 2);
    }

    #[test]
    fn content_search_highlight_overrides_markdown_spans() {
        let regex = Regex::new("bold").unwrap();
        let rendered = content_lines(
            "**bold** text",
            0,
            1,
            Some(&regex),
            ContentViewMode::Reading,
        );

        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "bold");
        assert_eq!(rendered.lines[0].spans[0].style.fg, Some(Color::Yellow));
        assert!(rendered.lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn raw_content_lines_preserve_markdown_syntax() {
        let rendered = content_lines("```text\n**bold**\n```", 0, 3, None, ContentViewMode::Raw);

        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "```text");
        assert_eq!(rendered.lines[1].spans[0].content.as_ref(), "**bold**");
        assert_eq!(rendered.lines[2].spans[0].content.as_ref(), "```");
    }
}
