use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Alignment, Color, Frame, Modifier, Position, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use regex::Regex;
use std::rc::Rc;
use unicode_width::UnicodeWidthChar;

use crate::tui::content_markdown::{render_markdown_lines, MarkdownLineKind};
use crate::tui::pane::{panel_block, render_panel_scrollbar, Panel, PanelScroll};

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
    pub(crate) selection: Option<ContentSelection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentPosition {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentSelection {
    pub(crate) anchor: ContentPosition,
    pub(crate) cursor: ContentPosition,
    pub(crate) kind: ContentSelectionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContentSelectionKind {
    Linear,
    Block,
}

#[derive(Clone)]
struct ContentLine {
    line: Line<'static>,
    kind: MarkdownLineKind,
    links: Vec<ContentLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContentLink {
    start: usize,
    end: usize,
    target: String,
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

    let (total_lines, chunks) = content_layout(area, view.text, view.mode);

    let visible_height = inner.height as usize;
    let scroll = view.scroll.min(total_lines.saturating_sub(1));
    let visible = visible_content_lines(
        view.text,
        scroll,
        visible_height,
        chunks[2].width as usize,
        view.mode,
    );
    frame.render_widget(
        Paragraph::new(line_number_lines(total_lines, scroll, &visible))
            .alignment(Alignment::Right),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(separator_lines(visible_height, &visible)),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new(content_lines(
            visible,
            scroll,
            view.search_regex,
            view.selection,
        )),
        chunks[2],
    );
    render_panel_scrollbar(
        frame,
        area,
        PanelScroll::new(scroll, total_lines, visible_height),
        panel.active(),
    );
}

pub(crate) fn content_text_area(area: Rect, text: &str, mode: ContentViewMode) -> Rect {
    content_layout(area, text, mode).1[2]
}

pub(crate) fn content_link_at(
    area: Rect,
    text: &str,
    scroll: usize,
    mode: ContentViewMode,
    column: u16,
    row: u16,
) -> Option<String> {
    let inner = panel_block(&Panel::new("", "", false)).inner(area);
    if inner.width == 0
        || inner.height == 0
        || column < inner.x
        || row < inner.y
        || column >= inner.x.saturating_add(inner.width)
        || row >= inner.y.saturating_add(inner.height)
    {
        return None;
    }

    let (total_lines, chunks) = content_layout(area, text, mode);
    let content_area = chunks[2];
    if column < content_area.x
        || column >= content_area.x.saturating_add(content_area.width)
        || row < content_area.y
        || row >= content_area.y.saturating_add(content_area.height)
    {
        return None;
    }

    let scroll = scroll.min(total_lines.saturating_sub(1));
    let visible = visible_content_lines(
        text,
        scroll,
        content_area.height as usize,
        content_area.width as usize,
        mode,
    );
    let line = visible.get((row - content_area.y) as usize)?;
    let hit_col = (column - content_area.x) as usize;
    line.links
        .iter()
        .find(|link| link.start <= hit_col && hit_col < link.end)
        .map(|link| link.target.clone())
}

pub(crate) fn content_position_at(
    area: Rect,
    text: &str,
    scroll: usize,
    mode: ContentViewMode,
    column: u16,
    row: u16,
) -> Option<ContentPosition> {
    let content_area = content_text_area(area, text, mode);
    if content_area.width == 0
        || content_area.height == 0
        || column < content_area.x
        || row < content_area.y
        || column >= content_area.x.saturating_add(content_area.width)
        || row >= content_area.y.saturating_add(content_area.height)
    {
        return None;
    }

    let line = scroll.saturating_add((row - content_area.y) as usize);
    let column = (column - content_area.x) as usize;
    Some(clamp_content_position(
        area,
        text,
        mode,
        ContentPosition { line, column },
    ))
}

pub(crate) fn content_position_in_text_row(
    area: Rect,
    text: &str,
    scroll: usize,
    mode: ContentViewMode,
    column: u16,
    row: u16,
) -> Option<ContentPosition> {
    let inner = panel_block(&Panel::new("", "", false)).inner(area);
    let content_area = content_text_area(area, text, mode);
    if inner.width == 0
        || inner.height == 0
        || content_area.width == 0
        || content_area.height == 0
        || row < content_area.y
        || row >= content_area.y.saturating_add(content_area.height)
        || column < inner.x
        || column >= inner.x.saturating_add(inner.width)
    {
        return None;
    }

    if column >= content_area.x && column < content_area.x.saturating_add(content_area.width) {
        return content_position_at(area, text, scroll, mode, column, row);
    }

    let raw_line = scroll.saturating_add((row - content_area.y) as usize);
    let line_count = all_content_lines(text, content_area.width as usize, mode)
        .len()
        .max(1);
    let line = raw_line.min(line_count.saturating_sub(1));
    let column = column
        .saturating_sub(content_area.x)
        .min(content_area.width.saturating_sub(1)) as usize;
    Some(ContentPosition { line, column })
}

pub(crate) fn content_cursor_position(
    area: Rect,
    text: &str,
    scroll: usize,
    mode: ContentViewMode,
    position: ContentPosition,
) -> Option<Position> {
    let content_area = content_text_area(area, text, mode);
    if content_area.width == 0
        || content_area.height == 0
        || position.line < scroll
        || position.line >= scroll.saturating_add(content_area.height as usize)
    {
        return None;
    }

    let column = position
        .column
        .min(content_area.width.saturating_sub(1) as usize);
    Some(Position::new(
        content_area.x.saturating_add(column as u16),
        content_area
            .y
            .saturating_add(position.line.saturating_sub(scroll) as u16),
    ))
}

pub(crate) fn clamp_content_position(
    area: Rect,
    text: &str,
    mode: ContentViewMode,
    position: ContentPosition,
) -> ContentPosition {
    let content_area = content_text_area(area, text, mode);
    let width = content_area.width as usize;
    let lines = all_content_lines(text, width, mode);
    let line = position.line.min(lines.len().saturating_sub(1));
    let max_column = line_text_width(lines.get(line).map(|line| &line.line)).saturating_sub(1);
    ContentPosition {
        line,
        column: position.column.min(max_column),
    }
}

pub(crate) fn selected_content_text(
    area: Rect,
    text: &str,
    mode: ContentViewMode,
    selection: ContentSelection,
) -> String {
    let content_area = content_text_area(area, text, mode);
    let width = content_area.width as usize;
    let lines = all_content_lines(text, width, mode);
    if lines.is_empty() {
        return String::new();
    }

    let selection = clamp_content_selection(area, text, mode, selection);
    let (start, end) = normalized_selection(selection);
    (start.line..=end.line)
        .filter_map(|line_idx| {
            let line = lines.get(line_idx)?;
            let width = line_text_width(Some(&line.line));
            let range = selection_range_for_line(selection, start, end, line_idx, width)?;
            Some(line_text_columns(&line.line, range.start, range.end))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn clamp_content_selection(
    area: Rect,
    text: &str,
    mode: ContentViewMode,
    selection: ContentSelection,
) -> ContentSelection {
    if selection.kind == ContentSelectionKind::Linear {
        return ContentSelection {
            anchor: clamp_content_position(area, text, mode, selection.anchor),
            cursor: clamp_content_position(area, text, mode, selection.cursor),
            kind: selection.kind,
        };
    }

    let content_area = content_text_area(area, text, mode);
    let lines = all_content_lines(text, content_area.width as usize, mode);
    let max_line = lines.len().saturating_sub(1);
    let max_column = content_area.width.saturating_sub(1) as usize;
    ContentSelection {
        anchor: ContentPosition {
            line: selection.anchor.line.min(max_line),
            column: selection.anchor.column.min(max_column),
        },
        cursor: ContentPosition {
            line: selection.cursor.line.min(max_line),
            column: selection.cursor.column.min(max_column),
        },
        kind: selection.kind,
    }
}

pub(crate) fn content_view_line_count(area: Rect, text: &str, mode: ContentViewMode) -> usize {
    let inner = panel_block(&Panel::new("", "", false)).inner(area);
    if inner.width == 0 || inner.height == 0 {
        return 1;
    }
    content_layout_metrics(inner.width, text, mode).0
}

fn content_layout(area: Rect, text: &str, mode: ContentViewMode) -> (usize, Rc<[Rect]>) {
    let inner = panel_block(&Panel::new("", "", false)).inner(area);
    let (total_lines, gutter_width) = content_layout_metrics(inner.width, text, mode);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(gutter_width),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);
    (total_lines, chunks)
}

fn visible_content_lines(
    text: &str,
    scroll: usize,
    height: usize,
    width: usize,
    mode: ContentViewMode,
) -> Vec<ContentLine> {
    all_content_lines(text, width, mode)
        .into_iter()
        .skip(scroll)
        .take(height)
        .collect()
}

fn all_content_lines(text: &str, width: usize, mode: ContentViewMode) -> Vec<ContentLine> {
    let lines = raw_lines(text);
    let logical_lines = match mode {
        ContentViewMode::Raw => lines
            .iter()
            .map(|line| Line::from((*line).to_string()))
            .map(|line| ContentLine {
                line,
                kind: MarkdownLineKind::Normal,
                links: Vec::new(),
            })
            .collect::<Vec<_>>(),
        ContentViewMode::Reading => render_markdown_lines(&lines, width)
            .into_iter()
            .map(|line| ContentLine {
                line: line.line,
                kind: line.kind,
                links: line
                    .links
                    .into_iter()
                    .map(|link| ContentLink {
                        start: link.start,
                        end: link.end,
                        target: link.target,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>(),
    };

    logical_lines
        .into_iter()
        .flat_map(|line| fit_content_line(line, width))
        .collect()
}

fn display_line_count(text: &str, mode: ContentViewMode, width: usize) -> usize {
    all_content_lines(text, width, mode).len().max(1)
}

fn content_layout_metrics(inner_width: u16, text: &str, mode: ContentViewMode) -> (usize, u16) {
    let mut total_lines = line_count(text);
    let mut gutter_width = line_number_width(total_lines).saturating_add(1);

    for _ in 0..4 {
        let content_width = inner_width.saturating_sub(gutter_width.saturating_add(1)) as usize;
        let next_total_lines = display_line_count(text, mode, content_width);
        let next_gutter_width = line_number_width(next_total_lines).saturating_add(1);
        if next_total_lines == total_lines && next_gutter_width == gutter_width {
            break;
        }
        total_lines = next_total_lines;
        gutter_width = next_gutter_width;
    }

    (total_lines, gutter_width)
}

fn content_lines(
    visible: Vec<ContentLine>,
    scroll: usize,
    search_regex: Option<&Regex>,
    selection: Option<ContentSelection>,
) -> Text<'static> {
    let lines = visible
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            let line_idx = scroll.saturating_add(idx);
            let line = styled_content_line(line.line, search_regex);
            styled_selection_line(line, selection, line_idx)
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

fn styled_selection_line(
    line: Line<'static>,
    selection: Option<ContentSelection>,
    line_idx: usize,
) -> Line<'static> {
    let Some(selection) = selection else {
        return line;
    };
    let width = line_text_width(Some(&line));
    let (start, end) = normalized_selection(selection);
    let Some(range) = selection_range_for_line(selection, start, end, line_idx, width) else {
        return line;
    };
    style_line_columns(line, range.start, range.end, visual_selection_style())
}

fn normalized_selection(selection: ContentSelection) -> (ContentPosition, ContentPosition) {
    if (selection.anchor.line, selection.anchor.column)
        <= (selection.cursor.line, selection.cursor.column)
    {
        (selection.anchor, selection.cursor)
    } else {
        (selection.cursor, selection.anchor)
    }
}

fn selection_range_for_line(
    selection: ContentSelection,
    start: ContentPosition,
    end: ContentPosition,
    line_idx: usize,
    line_width: usize,
) -> Option<std::ops::Range<usize>> {
    if line_idx < start.line || line_idx > end.line {
        return None;
    }
    if selection.kind == ContentSelectionKind::Block {
        let start_column = selection.anchor.column.min(selection.cursor.column);
        let end_column = selection.anchor.column.max(selection.cursor.column);
        let start = start_column.min(line_width);
        let end = end_column.min(line_width).saturating_add(1);
        return (start < end).then_some(start..end);
    }

    let fallback_width = line_width.max(1);
    let range = if start.line == end.line {
        start.column.min(line_width)..end.column.min(line_width).saturating_add(1)
    } else if line_idx == start.line {
        start.column.min(line_width)..fallback_width
    } else if line_idx == end.line {
        0..end.column.min(line_width).saturating_add(1)
    } else {
        0..fallback_width
    };
    (range.start < range.end).then_some(range)
}

fn visual_selection_style() -> Style {
    Style::default().fg(Color::White).bg(Color::DarkGray)
}

fn style_line_columns(
    line: Line<'static>,
    start: usize,
    end: usize,
    overlay: Style,
) -> Line<'static> {
    let mut output = Vec::new();
    let mut column = 0usize;
    for span in line.spans {
        for ch in span.content.chars() {
            let width = char_width(ch);
            let ch_start = column;
            let ch_end = column.saturating_add(width.max(1));
            let style = if ch_start < end && ch_end > start {
                span.style.patch(overlay)
            } else {
                span.style
            };
            push_char_span(&mut output, ch, style);
            column = column.saturating_add(width);
        }
    }

    if output.is_empty() && start < end {
        output.push(Span::styled(" ", overlay));
    }

    Line {
        spans: output,
        style: line.style,
        alignment: line.alignment,
    }
}

fn line_text_columns(line: &Line<'static>, start: usize, end: usize) -> String {
    let mut output = String::new();
    let mut column = 0usize;
    for span in &line.spans {
        for ch in span.content.chars() {
            let width = char_width(ch);
            let ch_start = column;
            let ch_end = column.saturating_add(width.max(1));
            if ch_start < end && ch_end > start {
                output.push(ch);
            }
            column = column.saturating_add(width);
        }
    }
    output
}

fn line_text_width(line: Option<&Line<'static>>) -> usize {
    line.map(|line| {
        line.spans
            .iter()
            .map(|span| text_width(span.content.as_ref()))
            .sum()
    })
    .unwrap_or(0)
}

fn fit_content_line(line: ContentLine, width: usize) -> Vec<ContentLine> {
    if width == 0 {
        return vec![ContentLine {
            line: Line::default(),
            kind: line.kind,
            links: Vec::new(),
        }];
    }

    match line.kind {
        MarkdownLineKind::Normal => wrap_content_line(line, width),
        MarkdownLineKind::CodeFence | MarkdownLineKind::CodeBlock | MarkdownLineKind::Table => {
            vec![ContentLine {
                links: clip_links_to_width(&line.links, width),
                line: clip_line_to_width(line.line, width),
                kind: line.kind,
            }]
        }
    }
}

fn wrap_content_line(line: ContentLine, width: usize) -> Vec<ContentLine> {
    if line.line.spans.is_empty() {
        return vec![line];
    }

    let style = line.line.style;
    let alignment = line.line.alignment;
    let mut wrapped = Vec::new();
    let mut current = Vec::new();
    let mut current_links = Vec::new();
    let mut current_width = 0usize;
    let mut source_width = 0usize;

    for span in line.line.spans {
        let span_start = source_width;
        source_width += text_width(span.content.as_ref());
        for token in span_wrap_tokens(&span, span_start) {
            let token_width = text_width(&token.content);
            if token_width == 0 {
                append_token(&mut current, token);
                continue;
            }

            if token.whitespace && current_width == 0 {
                continue;
            }

            if token_width > width && !token.whitespace && !token.unbreakable {
                if current_width > 0 {
                    trim_trailing_spaces(&mut current, &mut current_width);
                    push_wrapped_line(
                        &mut wrapped,
                        &mut current,
                        &mut current_links,
                        &mut current_width,
                        style,
                        alignment,
                        line.kind,
                    );
                }
                for part in break_token_to_width(token, width) {
                    let part_width = text_width(&part.content);
                    if part_width == width {
                        append_token_links(&mut current_links, &line.links, current_width, &part);
                        append_token(&mut current, part);
                        push_wrapped_line(
                            &mut wrapped,
                            &mut current,
                            &mut current_links,
                            &mut current_width,
                            style,
                            alignment,
                            line.kind,
                        );
                    } else {
                        append_token_links(&mut current_links, &line.links, current_width, &part);
                        append_token(&mut current, part);
                        current_width += part_width;
                    }
                }
                continue;
            }

            if current_width > 0 && current_width + token_width > width {
                trim_trailing_spaces(&mut current, &mut current_width);
                push_wrapped_line(
                    &mut wrapped,
                    &mut current,
                    &mut current_links,
                    &mut current_width,
                    style,
                    alignment,
                    line.kind,
                );
                if token.whitespace {
                    continue;
                }
            }

            append_token_links(&mut current_links, &line.links, current_width, &token);
            append_token(&mut current, token);
            current_width += token_width;
        }
    }

    trim_trailing_spaces(&mut current, &mut current_width);
    if current_width > 0 || wrapped.is_empty() {
        wrapped.push(ContentLine {
            line: Line {
                spans: current,
                style,
                alignment,
            },
            kind: line.kind,
            links: current_links,
        });
    }

    wrapped
}

struct WrapToken {
    content: String,
    style: Style,
    whitespace: bool,
    unbreakable: bool,
    source_start: usize,
    source_end: usize,
}

fn span_wrap_tokens(span: &Span<'static>, source_start: usize) -> Vec<WrapToken> {
    let content = span.content.as_ref();
    if is_unbreakable_span(span) {
        let width = text_width(content);
        return vec![WrapToken {
            content: content.to_string(),
            style: span.style,
            whitespace: false,
            unbreakable: true,
            source_start,
            source_end: source_start + width,
        }];
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_whitespace = None;
    let mut current_source_start = source_start;
    let mut current_source_end = source_start;
    for ch in content.chars() {
        let whitespace = ch.is_whitespace();
        if current_whitespace == Some(whitespace) || current.is_empty() {
            current.push(ch);
            current_whitespace = Some(whitespace);
            current_source_end += char_width(ch);
            continue;
        }

        tokens.push(WrapToken {
            content: std::mem::take(&mut current),
            style: span.style,
            whitespace: current_whitespace.unwrap_or(false),
            unbreakable: false,
            source_start: current_source_start,
            source_end: current_source_end,
        });
        current_source_start = current_source_end;
        current.push(ch);
        current_whitespace = Some(whitespace);
        current_source_end += char_width(ch);
    }

    if !current.is_empty() {
        tokens.push(WrapToken {
            content: current,
            style: span.style,
            whitespace: current_whitespace.unwrap_or(false),
            unbreakable: false,
            source_start: current_source_start,
            source_end: current_source_end,
        });
    }

    tokens
}

fn is_unbreakable_span(span: &Span<'static>) -> bool {
    let text = span.content.as_ref();
    span.style.fg == Some(Color::DarkGray)
        && text.starts_with(" (")
        && text.ends_with(')')
        && (text.contains("://")
            || text.contains(":/")
            || text.contains(":\\")
            || text.contains('/')
            || text.contains('\\'))
}

fn break_token_to_width(token: WrapToken, width: usize) -> Vec<WrapToken> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut current_source_start = token.source_start;
    let mut current_source_end = token.source_start;

    for ch in token.content.chars() {
        let ch_width = char_width(ch);
        if ch_width > width {
            continue;
        }
        if current_width > 0 && current_width + ch_width > width {
            parts.push(WrapToken {
                content: std::mem::take(&mut current),
                style: token.style,
                whitespace: false,
                unbreakable: false,
                source_start: current_source_start,
                source_end: current_source_end,
            });
            current_source_start = current_source_end;
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
        current_source_end += ch_width;
    }

    if !current.is_empty() || parts.is_empty() {
        parts.push(WrapToken {
            content: current,
            style: token.style,
            whitespace: false,
            unbreakable: false,
            source_start: current_source_start,
            source_end: current_source_end,
        });
    }

    parts
}

fn append_token_links(
    current_links: &mut Vec<ContentLink>,
    source_links: &[ContentLink],
    current_width: usize,
    token: &WrapToken,
) {
    for link in source_links {
        let start = link.start.max(token.source_start);
        let end = link.end.min(token.source_end);
        if start >= end {
            continue;
        }
        current_links.push(ContentLink {
            start: current_width + start.saturating_sub(token.source_start),
            end: current_width + end.saturating_sub(token.source_start),
            target: link.target.clone(),
        });
    }
}

fn append_token(spans: &mut Vec<Span<'static>>, token: WrapToken) {
    if token.content.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut().filter(|span| span.style == token.style) {
        last.content.to_mut().push_str(&token.content);
    } else {
        spans.push(Span::styled(token.content, token.style));
    }
}

fn trim_trailing_spaces(spans: &mut Vec<Span<'static>>, width: &mut usize) {
    while let Some(last) = spans.last_mut() {
        let trimmed = last.content.as_ref().trim_end().to_string();
        if trimmed.len() == last.content.len() {
            break;
        }
        *width = width.saturating_sub(text_width(&last.content) - text_width(&trimmed));
        if trimmed.is_empty() {
            spans.pop();
        } else {
            last.content = trimmed.into();
            break;
        }
    }
}

fn push_wrapped_line(
    wrapped: &mut Vec<ContentLine>,
    current: &mut Vec<Span<'static>>,
    current_links: &mut Vec<ContentLink>,
    current_width: &mut usize,
    style: Style,
    alignment: Option<Alignment>,
    kind: MarkdownLineKind,
) {
    wrapped.push(ContentLine {
        line: Line {
            spans: std::mem::take(current),
            style,
            alignment,
        },
        kind,
        links: std::mem::take(current_links),
    });
    *current_width = 0;
}

fn clip_line_to_width(line: Line<'static>, width: usize) -> Line<'static> {
    if width == 0 {
        return Line {
            spans: Vec::new(),
            style: line.style,
            alignment: line.alignment,
        };
    }

    let mut clipped = Vec::new();
    let mut used_width = 0usize;

    'spans: for span in line.spans {
        for ch in span.content.chars() {
            let ch_width = char_width(ch);
            if ch_width > width || used_width + ch_width > width {
                break 'spans;
            }
            push_char_span(&mut clipped, ch, span.style);
            used_width += ch_width;
        }
    }

    Line {
        spans: clipped,
        style: line.style,
        alignment: line.alignment,
    }
}

fn clip_links_to_width(links: &[ContentLink], width: usize) -> Vec<ContentLink> {
    links
        .iter()
        .filter_map(|link| {
            let end = link.end.min(width);
            (link.start < end).then(|| ContentLink {
                start: link.start,
                end,
                target: link.target.clone(),
            })
        })
        .collect()
}

fn push_char_span(spans: &mut Vec<Span<'static>>, ch: char, style: Style) {
    if let Some(last) = spans.last_mut().filter(|span| span.style == style) {
        last.content.to_mut().push(ch);
    } else {
        spans.push(Span::styled(ch.to_string(), style));
    }
}

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn text_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

fn line_number_lines(total_lines: usize, scroll: usize, visible: &[ContentLine]) -> Text<'static> {
    let lines = (scroll..total_lines.min(scroll.saturating_add(visible.len())))
        .enumerate()
        .map(|(visible_idx, idx)| {
            Line::from(Span::styled(
                (idx + 1).to_string(),
                gutter_style(visible.get(visible_idx).map(|line| line.kind)),
            ))
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

fn separator_lines(height: usize, visible: &[ContentLine]) -> Text<'static> {
    Text::from(
        (0..height)
            .map(|idx| {
                Line::from(Span::styled(
                    "|",
                    gutter_style(visible.get(idx).map(|line| line.kind)),
                ))
            })
            .collect::<Vec<_>>(),
    )
}

fn gutter_style(kind: Option<MarkdownLineKind>) -> Style {
    match kind {
        Some(MarkdownLineKind::CodeBlock | MarkdownLineKind::CodeFence) => {
            Style::default().fg(Color::Blue)
        }
        Some(MarkdownLineKind::Table) => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::DarkGray),
    }
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
    use super::{
        content_lines, content_link_at, content_position_at, content_position_in_text_row,
        line_count, line_number_width, render_content_view, selected_content_text,
        visible_content_lines, ContentPosition, ContentSelection, ContentSelectionKind,
        ContentView, ContentViewMode,
    };
    use crate::tui::pane::Panel;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::prelude::{Color, Modifier};
    use ratatui::text::Text;
    use ratatui::Terminal;
    use regex::Regex;
    use unicode_width::UnicodeWidthStr;

    fn render_content_lines(
        text: &str,
        scroll: usize,
        height: usize,
        search_regex: Option<&Regex>,
        mode: ContentViewMode,
    ) -> Text<'static> {
        content_lines(
            visible_content_lines(text, scroll, height, 80, mode),
            scroll,
            search_regex,
            None,
        )
    }

    fn rendered_line_text(text: &Text<'static>, idx: usize) -> String {
        text.lines[idx]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn backend_row(backend: &TestBackend, y: u16) -> String {
        (0..backend.buffer().area.width)
            .filter_map(|x| backend.buffer().cell((x, y)).map(|cell| cell.symbol()))
            .collect::<String>()
    }

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
        let rendered = render_content_lines("alpha\n\nomega", 0, 3, None, ContentViewMode::Reading);

        assert_eq!(rendered.lines.len(), 3);
        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "alpha");
        assert!(rendered.lines[1].spans.is_empty());
        assert_eq!(rendered.lines[2].spans[0].content.as_ref(), "omega");
    }

    #[test]
    fn content_lines_render_markdown_without_changing_line_count() {
        let rendered = render_content_lines(
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
        assert_eq!(rendered.lines[1].spans[2].style.fg, Some(Color::Gray));
        assert_eq!(rendered.lines[1].spans[2].style.bg, None);
        assert_eq!(line_count("## User\n**bold** and `code`"), 2);
    }

    #[test]
    fn content_search_highlight_overrides_markdown_spans() {
        let regex = Regex::new("bold").unwrap();
        let rendered = render_content_lines(
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
    fn content_lines_wrap_wide_characters_before_terminal_clipping() {
        let rendered = content_lines(
            visible_content_lines("甲乙丙", 0, 3, 4, ContentViewMode::Reading),
            0,
            None,
            None,
        );

        assert_eq!(rendered.lines.len(), 2);
        assert_eq!(rendered_line_text(&rendered, 0), "甲乙");
        assert_eq!(rendered_line_text(&rendered, 1), "丙");
        assert!(UnicodeWidthStr::width(rendered_line_text(&rendered, 0).as_str()) <= 4);
        assert!(UnicodeWidthStr::width(rendered_line_text(&rendered, 1).as_str()) <= 4);
    }

    #[test]
    fn markdown_link_targets_stay_atomic_when_wrapping() {
        let visible = visible_content_lines(
            "[docker-compose.funasr.yml](V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15)",
            0,
            4,
            16,
            ContentViewMode::Reading,
        );
        let rendered = content_lines(visible.clone(), 0, None, None);

        assert_eq!(rendered.lines.len(), 2);
        assert_eq!(rendered_line_text(&rendered, 0), "docker-compose.f");
        assert_eq!(rendered_line_text(&rendered, 1), "unasr.yml");
        assert_eq!(
            visible[0].links[0].target,
            "V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15"
        );
        assert_eq!(
            visible[1].links[0].target,
            "V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15"
        );
    }

    #[test]
    fn content_link_hit_test_returns_full_target_after_wrapping() {
        let target = content_link_at(
            Rect::new(0, 0, 24, 5),
            "[docker-compose.funasr.yml](V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15)",
            0,
            ContentViewMode::Reading,
            4,
            1,
        );

        assert_eq!(
            target.as_deref(),
            Some("V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15")
        );
    }

    #[test]
    fn content_position_at_rejects_line_number_gutter() {
        let position = content_position_at(
            Rect::new(0, 0, 24, 5),
            "alpha\nbeta",
            0,
            ContentViewMode::Reading,
            1,
            1,
        );

        assert_eq!(position, None);
    }

    #[test]
    fn content_position_in_text_row_maps_gutter_to_text_start() {
        let position = content_position_in_text_row(
            Rect::new(0, 0, 24, 5),
            "alpha\nbeta",
            0,
            ContentViewMode::Reading,
            1,
            1,
        );

        assert_eq!(position, Some(ContentPosition { line: 0, column: 0 }));
    }

    #[test]
    fn content_view_line_count_uses_wrapped_reading_lines() {
        assert_eq!(
            super::content_view_line_count(
                Rect::new(0, 0, 24, 5),
                "[docker-compose.funasr.yml](V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15)",
                ContentViewMode::Reading,
            ),
            2
        );
    }

    #[test]
    fn fixed_shape_lines_clip_without_cutting_wide_characters() {
        let rendered = content_lines(
            visible_content_lines(
                "| 编号 | 项目 |\n| --- | --- |\n| 001 | 用户调研 |",
                0,
                5,
                10,
                ContentViewMode::Reading,
            ),
            0,
            None,
            None,
        );

        assert!(!rendered.lines.is_empty());
        for idx in 0..rendered.lines.len() {
            let text = rendered_line_text(&rendered, idx);
            assert!(
                UnicodeWidthStr::width(text.as_str()) <= 10,
                "line exceeds render width: {text:?}"
            );
        }
    }

    #[test]
    fn raw_content_lines_preserve_markdown_syntax() {
        let rendered =
            render_content_lines("```text\n**bold**\n```", 0, 3, None, ContentViewMode::Raw);

        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "```text");
        assert_eq!(rendered.lines[1].spans[0].content.as_ref(), "**bold**");
        assert_eq!(rendered.lines[2].spans[0].content.as_ref(), "```");
    }

    #[test]
    fn selected_content_text_uses_rendered_content_coordinates() {
        let selected = selected_content_text(
            Rect::new(0, 0, 40, 6),
            "alpha\nbeta\nomega",
            ContentViewMode::Reading,
            ContentSelection {
                anchor: ContentPosition { line: 0, column: 1 },
                cursor: ContentPosition { line: 1, column: 2 },
                kind: ContentSelectionKind::Linear,
            },
        );

        assert_eq!(selected, "lpha\nbet");
    }

    #[test]
    fn selected_content_text_never_includes_line_number_gutter() {
        let selected = selected_content_text(
            Rect::new(0, 0, 24, 5),
            "alpha\nbeta",
            ContentViewMode::Reading,
            ContentSelection {
                anchor: ContentPosition { line: 0, column: 0 },
                cursor: ContentPosition { line: 1, column: 1 },
                kind: ContentSelectionKind::Linear,
            },
        );

        assert_eq!(selected, "alpha\nbe");
    }

    #[test]
    fn selected_content_text_supports_block_selection() {
        let selected = selected_content_text(
            Rect::new(0, 0, 32, 6),
            "abcdef\n12\nuvwxyz",
            ContentViewMode::Reading,
            ContentSelection {
                anchor: ContentPosition { line: 0, column: 1 },
                cursor: ContentPosition { line: 2, column: 3 },
                kind: ContentSelectionKind::Block,
            },
        );

        assert_eq!(selected, "bcd\n2\nvwx");
    }

    #[test]
    fn content_lines_highlight_block_selection_per_row() {
        let rendered = content_lines(
            visible_content_lines("abcdef\nuvwxyz", 0, 2, 80, ContentViewMode::Reading),
            0,
            None,
            Some(ContentSelection {
                anchor: ContentPosition { line: 0, column: 1 },
                cursor: ContentPosition { line: 1, column: 3 },
                kind: ContentSelectionKind::Block,
            }),
        );

        assert_eq!(rendered.lines[0].spans[1].content.as_ref(), "bcd");
        assert_eq!(rendered.lines[1].spans[1].content.as_ref(), "vwx");
        assert_eq!(rendered.lines[0].spans[1].style.bg, Some(Color::DarkGray));
        assert_eq!(rendered.lines[1].spans[1].style.bg, Some(Color::DarkGray));
    }

    #[test]
    fn content_lines_highlight_visual_selection() {
        let rendered = content_lines(
            visible_content_lines("alpha", 0, 1, 80, ContentViewMode::Reading),
            0,
            None,
            Some(ContentSelection {
                anchor: ContentPosition { line: 0, column: 1 },
                cursor: ContentPosition { line: 0, column: 3 },
                kind: ContentSelectionKind::Linear,
            }),
        );

        assert_eq!(rendered_line_text(&rendered, 0), "alpha");
        assert_eq!(rendered.lines[0].spans[1].content.as_ref(), "lph");
        assert_eq!(rendered.lines[0].spans[1].style.bg, Some(Color::DarkGray));
    }

    #[test]
    fn render_content_view_shows_line_numbers_in_the_visible_gutter() {
        let backend = TestBackend::new(24, 6);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_content_view(
                    frame,
                    Rect::new(0, 0, 24, 6),
                    Panel::new("3", "Content (read)", true),
                    ContentView {
                        text: "alpha\nbeta",
                        scroll: 0,
                        search_regex: None,
                        mode: ContentViewMode::Reading,
                        selection: None,
                    },
                );
            })
            .unwrap();

        let backend = terminal.backend();
        assert!(backend_row(backend, 1).contains("1|alpha"));
        assert!(backend_row(backend, 2).contains("2|beta"));
    }
}
