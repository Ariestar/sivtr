use pulldown_cmark::{Event, Options as MarkdownOptions, Parser, Tag, TagEnd};
use ratatui::prelude::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

const CODE_INDENT: &str = "  ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkdownLineKind {
    Normal,
    CodeFence,
    CodeBlock,
    Table,
}

#[derive(Clone, Debug)]
pub(crate) struct MarkdownLine {
    pub(crate) line: Line<'static>,
    pub(crate) kind: MarkdownLineKind,
    pub(crate) links: Vec<MarkdownLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkdownLink {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) target: String,
}

#[derive(Clone)]
struct TableRenderRow {
    cells: Vec<String>,
    widths: Vec<usize>,
    kind: TableRenderRowKind,
}

#[derive(Clone, Copy)]
enum TableRenderRowKind {
    Top,
    Header,
    Separator,
    Body,
    Bottom,
}

struct MarkdownLineParts<'a> {
    prefix: String,
    prefix_style: Style,
    content: &'a str,
    content_style: Style,
}

pub(crate) fn render_markdown_lines(lines: &[&str], width: usize) -> Vec<MarkdownLine> {
    let mut in_code_block = false;
    let table_rows = aligned_table_rows(lines);
    let mut rendered = Vec::with_capacity(lines.len());

    lines.iter().enumerate().for_each(|(idx, line)| {
        if let Some(table_rows) = &table_rows[idx] {
            rendered.extend(table_rows.iter().map(render_table_row));
        } else {
            rendered.push(render_markdown_line(line, &mut in_code_block, width));
        }
    });

    rendered
}

#[cfg(test)]
fn render_markdown_window(
    lines: &[&str],
    scroll: usize,
    height: usize,
    width: usize,
) -> Vec<MarkdownLine> {
    render_markdown_lines(lines, width)
        .into_iter()
        .skip(scroll)
        .take(height)
        .collect()
}

fn render_markdown_line(line: &str, in_code_block: &mut bool, width: usize) -> MarkdownLine {
    if let Some(language) = code_fence_language(line) {
        let opening = !*in_code_block;
        *in_code_block = !*in_code_block;
        return MarkdownLine {
            line: code_fence_line(opening.then_some(language).flatten()),
            kind: MarkdownLineKind::CodeFence,
            links: Vec::new(),
        };
    }

    if *in_code_block {
        return MarkdownLine {
            line: Line::from(vec![
                Span::styled(CODE_INDENT, code_block_margin_style()),
                Span::styled(line.to_string(), code_block_style()),
            ]),
            kind: MarkdownLineKind::CodeBlock,
            links: Vec::new(),
        };
    }

    if is_horizontal_rule(line) {
        return MarkdownLine {
            line: Line::from(Span::styled(
                "─".repeat(width.max(3)),
                structural_marker_style(),
            )),
            kind: MarkdownLineKind::Normal,
            links: Vec::new(),
        };
    }

    let parts = markdown_line_parts(line);
    let mut spans = Vec::new();
    let mut links = Vec::new();
    let mut prefix_width = 0;
    if !parts.prefix.is_empty() {
        prefix_width = UnicodeWidthStr::width(parts.prefix.as_str());
        spans.push(Span::styled(parts.prefix, parts.prefix_style));
    }
    let inline = markdown_inline(parts.content, parts.content_style);
    links.extend(inline.links.into_iter().map(|link| MarkdownLink {
        start: link.start + prefix_width,
        end: link.end + prefix_width,
        target: link.target,
    }));
    spans.extend(inline.spans);
    MarkdownLine {
        line: Line {
            spans,
            style: parts.content_style,
            alignment: None,
        },
        kind: MarkdownLineKind::Normal,
        links,
    }
}

fn code_fence_language(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("```")
        .or_else(|| trimmed.strip_prefix("~~~"))?;
    let language = rest.split_whitespace().next().filter(|lang| {
        !lang.is_empty() && !matches!(*lang, "text" | "txt" | "plain" | "plaintext")
    });
    Some(language.map(str::to_string))
}

fn code_fence_line(language: Option<String>) -> Line<'static> {
    match language {
        Some(language) => Line::from(Span::styled(
            format!("{CODE_INDENT}{language}"),
            code_fence_style(),
        )),
        None => Line::default(),
    }
}

fn markdown_line_parts(line: &str) -> MarkdownLineParts<'_> {
    let leading_width = line.len() - line.trim_start().len();
    let leading = &line[..leading_width];
    let trimmed = &line[leading_width..];

    if let Some((level, rest)) = markdown_heading(trimmed) {
        let style = agent_heading_style(rest).unwrap_or_else(|| heading_style(level));
        return MarkdownLineParts {
            prefix: format!("{leading}{} ", "#".repeat(level)),
            prefix_style: structural_marker_style(),
            content: rest,
            content_style: style,
        };
    }

    if let Some(rest) = trimmed.strip_prefix("> ") {
        return MarkdownLineParts {
            prefix: format!("{leading}> "),
            prefix_style: structural_marker_style(),
            content: rest,
            content_style: blockquote_style(),
        };
    }

    if let Some((marker, rest, marker_style)) = markdown_task_item(trimmed) {
        return MarkdownLineParts {
            prefix: format!("{leading}{marker}"),
            prefix_style: marker_style,
            content: rest,
            content_style: Style::default(),
        };
    }

    if let Some((marker, rest)) = markdown_list_item(trimmed) {
        return MarkdownLineParts {
            prefix: format!("{leading}{marker}"),
            prefix_style: structural_marker_style(),
            content: rest,
            content_style: Style::default(),
        };
    }

    MarkdownLineParts {
        prefix: String::new(),
        prefix_style: Style::default(),
        content: line,
        content_style: Style::default(),
    }
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
        Some((level, &line[level + 1..]))
    } else {
        None
    }
}

fn markdown_task_item(line: &str) -> Option<(String, &str, Style)> {
    let (_marker, rest) = markdown_list_item(line)?;
    let (symbol, rest, style) = if let Some(rest) = rest.strip_prefix("[ ] ") {
        ("□ ", rest, structural_marker_style())
    } else if let Some(rest) = rest
        .strip_prefix("[x] ")
        .or_else(|| rest.strip_prefix("[X] "))
    {
        ("■ ", rest, task_done_marker_style())
    } else {
        return None;
    };

    Some((symbol.to_string(), rest, style))
}

fn markdown_list_item(line: &str) -> Option<(String, &str)> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some((marker.to_string(), rest));
        }
    }

    let dot = line.find(". ")?;
    if dot == 0 || !line[..dot].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((line[..dot + 2].to_string(), &line[dot + 2..]))
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    let mut chars = trimmed.chars();
    let Some(marker @ ('-' | '*' | '_')) = chars.next() else {
        return false;
    };
    let mut count = 1;
    for ch in chars {
        if ch != marker {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn aligned_table_rows(lines: &[&str]) -> Vec<Option<Vec<TableRenderRow>>> {
    let mut rows = vec![None; lines.len()];
    let mut in_code_block = false;
    let mut idx = 0;

    while idx < lines.len() {
        if code_fence_language(lines[idx]).is_some() {
            in_code_block = !in_code_block;
            idx += 1;
            continue;
        }
        if in_code_block {
            idx += 1;
            continue;
        }

        let Some(header) = parse_table_row(lines[idx]) else {
            idx += 1;
            continue;
        };
        let Some(separator_line) = lines.get(idx + 1) else {
            idx += 1;
            continue;
        };
        let Some(separator) = parse_table_row(separator_line).filter(|row| row.separator) else {
            idx += 1;
            continue;
        };
        if separator.cells.len() != header.cells.len() {
            idx += 1;
            continue;
        }

        let mut parsed_rows = vec![header, separator];
        let mut end = idx + 2;
        while let Some(row) = lines.get(end).and_then(|line| parse_table_row(line)) {
            if row.separator || row.cells.len() != parsed_rows[0].cells.len() {
                break;
            }
            parsed_rows.push(row);
            end += 1;
        }

        let widths = table_column_widths(&parsed_rows);
        let header = parsed_rows[0].cells.clone();
        rows[idx] = Some(vec![
            TableRenderRow {
                cells: Vec::new(),
                widths: widths.clone(),
                kind: TableRenderRowKind::Top,
            },
            TableRenderRow {
                cells: header,
                widths: widths.clone(),
                kind: TableRenderRowKind::Header,
            },
        ]);
        rows[idx + 1] = Some(vec![TableRenderRow {
            cells: Vec::new(),
            widths: widths.clone(),
            kind: TableRenderRowKind::Separator,
        }]);
        let body_count = parsed_rows.len().saturating_sub(2);
        for (body_idx, row) in parsed_rows.into_iter().skip(2).enumerate() {
            let is_last_body = body_idx + 1 == body_count;
            let mut render_rows = vec![TableRenderRow {
                cells: row.cells,
                widths: widths.clone(),
                kind: TableRenderRowKind::Body,
            }];
            if is_last_body {
                render_rows.push(TableRenderRow {
                    cells: Vec::new(),
                    widths: widths.clone(),
                    kind: TableRenderRowKind::Bottom,
                });
            }
            rows[idx + 2 + body_idx] = Some(render_rows);
        }
        idx = end;
    }

    rows
}

#[derive(Clone)]
struct ParsedTableRow {
    cells: Vec<String>,
    separator: bool,
}

fn parse_table_row(line: &str) -> Option<ParsedTableRow> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }

    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let cells = inner
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect::<Vec<_>>();
    if cells.len() < 2 {
        return None;
    }

    let separator = cells.iter().all(|cell| is_table_separator_cell(cell));
    Some(ParsedTableRow { cells, separator })
}

fn is_table_separator_cell(cell: &str) -> bool {
    let trimmed = cell.trim();
    let body = trimmed
        .strip_prefix(':')
        .unwrap_or(trimmed)
        .strip_suffix(':')
        .unwrap_or(trimmed);
    body.len() >= 3 && body.chars().all(|ch| ch == '-')
}

fn table_column_widths(rows: &[ParsedTableRow]) -> Vec<usize> {
    let column_count = rows[0].cells.len();
    let mut widths = vec![3; column_count];
    for row in rows.iter().filter(|row| !row.separator) {
        for (idx, cell) in row.cells.iter().enumerate() {
            widths[idx] = widths[idx].max(inline_render_width(cell));
        }
    }
    widths
}

fn render_table_row(row: &TableRenderRow) -> MarkdownLine {
    if let Some(line) = render_table_border(row) {
        return line;
    }

    let mut spans = Vec::new();
    spans.push(Span::styled("│ ", structural_marker_style()));

    for (idx, width) in row.widths.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" │ ", structural_marker_style()));
        }

        let cell = row.cells.get(idx).map(String::as_str).unwrap_or_default();
        let cell_width = inline_render_width(cell);
        spans.extend(markdown_inline_spans(cell, Style::default()));
        if *width > cell_width {
            spans.push(Span::raw(" ".repeat(*width - cell_width)));
        }
    }

    spans.push(Span::styled(" │", structural_marker_style()));
    MarkdownLine {
        line: Line::from(spans),
        kind: MarkdownLineKind::Table,
        links: Vec::new(),
    }
}

fn render_table_border(row: &TableRenderRow) -> Option<MarkdownLine> {
    let (left, join, right) = match row.kind {
        TableRenderRowKind::Top => ("┌", "┬", "┐"),
        TableRenderRowKind::Separator => ("├", "┼", "┤"),
        TableRenderRowKind::Bottom => ("└", "┴", "┘"),
        TableRenderRowKind::Header | TableRenderRowKind::Body => return None,
    };

    let mut line = String::new();
    line.push_str(left);
    for (idx, width) in row.widths.iter().enumerate() {
        if idx > 0 {
            line.push_str(join);
        }
        line.push_str(&"─".repeat(width.saturating_add(2)));
    }
    line.push_str(right);

    Some(MarkdownLine {
        line: Line::from(Span::styled(line, structural_marker_style())),
        kind: MarkdownLineKind::Table,
        links: Vec::new(),
    })
}

fn inline_render_width(text: &str) -> usize {
    markdown_inline(text, Style::default())
        .spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn markdown_inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    markdown_inline(text, base_style).spans
}

struct MarkdownInline {
    spans: Vec<Span<'static>>,
    links: Vec<MarkdownLink>,
}

fn markdown_inline(text: &str, base_style: Style) -> MarkdownInline {
    if text.is_empty() {
        return MarkdownInline {
            spans: Vec::new(),
            links: Vec::new(),
        };
    }

    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(text, options);
    let mut spans = Vec::new();
    let mut links = Vec::new();
    let mut styles = vec![base_style];
    let mut pending_link = None;
    let mut pending_link_start = 0usize;
    let mut cursor = 0usize;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Emphasis => {
                    push_style(&mut styles, Style::default().add_modifier(Modifier::ITALIC))
                }
                Tag::Strong => {
                    push_style(&mut styles, Style::default().add_modifier(Modifier::BOLD));
                }
                Tag::Strikethrough => {
                    push_style(
                        &mut styles,
                        Style::default().add_modifier(Modifier::CROSSED_OUT),
                    );
                }
                Tag::Link { dest_url, .. } => {
                    pending_link = Some(dest_url.to_string());
                    pending_link_start = cursor;
                    push_style(&mut styles, link_style());
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    pop_style(&mut styles);
                }
                TagEnd::Link => {
                    pop_style(&mut styles);
                    if let Some(link) = pending_link.take() {
                        if cursor > pending_link_start {
                            links.push(MarkdownLink {
                                start: pending_link_start,
                                end: cursor,
                                target: link,
                            });
                        }
                    }
                }
                _ => {}
            },
            Event::Text(value) => {
                if pending_link.is_some() {
                    push_inline_span(
                        &mut spans,
                        value.to_string(),
                        current_style(&styles),
                        &mut cursor,
                    );
                } else {
                    push_text_with_autolinks(
                        &mut spans,
                        &mut links,
                        value.as_ref(),
                        current_style(&styles),
                        &mut cursor,
                    );
                }
            }
            Event::Code(value) => {
                push_inline_span(&mut spans, value.to_string(), code_style(), &mut cursor);
            }
            Event::SoftBreak | Event::HardBreak => {
                push_inline_span(
                    &mut spans,
                    " ".to_string(),
                    current_style(&styles),
                    &mut cursor,
                );
            }
            Event::Html(value) | Event::InlineHtml(value) => {
                push_inline_span(
                    &mut spans,
                    value.to_string(),
                    current_style(&styles),
                    &mut cursor,
                );
            }
            _ => {}
        }
    }

    if spans.is_empty() {
        push_inline_span(&mut spans, text.to_string(), base_style, &mut cursor);
    }
    MarkdownInline { spans, links }
}

fn push_text_with_autolinks(
    spans: &mut Vec<Span<'static>>,
    links: &mut Vec<MarkdownLink>,
    text: &str,
    style: Style,
    cursor: &mut usize,
) {
    let mut byte_cursor = 0;
    for (token_start, token) in
        text.split_inclusive(char::is_whitespace)
            .scan(0usize, |offset, token| {
                let start = *offset;
                *offset += token.len();
                Some((start, token))
            })
    {
        let trimmed = token.trim_end_matches(char::is_whitespace);
        let whitespace = &token[trimmed.len()..];
        if let Some((link_start, link_end)) = autolink_bounds(trimmed) {
            if token_start + link_start > byte_cursor {
                push_inline_span(
                    spans,
                    text[byte_cursor..token_start + link_start].to_string(),
                    style,
                    cursor,
                );
            }
            let target = &text[token_start + link_start..token_start + link_end];
            let start = *cursor;
            push_inline_span(spans, target.to_string(), style.patch(link_style()), cursor);
            links.push(MarkdownLink {
                start,
                end: *cursor,
                target: target.to_string(),
            });
            byte_cursor = token_start + link_end;
            if token_start + trimmed.len() > byte_cursor {
                push_inline_span(
                    spans,
                    text[byte_cursor..token_start + trimmed.len()].to_string(),
                    style,
                    cursor,
                );
                byte_cursor = token_start + trimmed.len();
            }
            if !whitespace.is_empty() {
                push_inline_span(spans, whitespace.to_string(), style, cursor);
                byte_cursor = token_start + token.len();
            }
        } else if token_start + token.len() > byte_cursor {
            push_inline_span(
                spans,
                text[byte_cursor..token_start + token.len()].to_string(),
                style,
                cursor,
            );
            byte_cursor = token_start + token.len();
        }
    }

    if byte_cursor < text.len() {
        push_inline_span(spans, text[byte_cursor..].to_string(), style, cursor);
    }
}

fn autolink_bounds(token: &str) -> Option<(usize, usize)> {
    let start = token
        .find("://")
        .map(|idx| {
            token[..idx]
                .rfind(|ch: char| ch.is_whitespace() || ch == '(' || ch == '[' || ch == '<')
                .map_or(0, |pos| pos + 1)
        })
        .or_else(|| token.find("www."))?;
    let mut end = token.len();
    while end > start
        && token[..end].chars().next_back().is_some_and(|ch| {
            matches!(
                ch,
                '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>'
            )
        })
    {
        end -= token[..end]
            .chars()
            .next_back()
            .map(char::len_utf8)
            .unwrap_or(0);
    }
    (end > start).then_some((start, end))
}

fn push_inline_span(
    spans: &mut Vec<Span<'static>>,
    content: String,
    style: Style,
    cursor: &mut usize,
) {
    if content.is_empty() {
        return;
    }
    *cursor += UnicodeWidthStr::width(content.as_str());
    if let Some(last) = spans.last_mut().filter(|span| span.style == style) {
        last.content.to_mut().push_str(&content);
    } else {
        spans.push(Span::styled(content, style));
    }
}

fn push_style(styles: &mut Vec<Style>, style: Style) {
    let next = current_style(styles).patch(style);
    styles.push(next);
}

fn pop_style(styles: &mut Vec<Style>) {
    if styles.len() > 1 {
        styles.pop();
    }
}

fn current_style(styles: &[Style]) -> Style {
    styles.last().copied().unwrap_or_default()
}

fn agent_heading_style(text: &str) -> Option<Style> {
    if text.starts_with("Assistant") {
        Some(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else if text.starts_with("User") {
        Some(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else if text.starts_with("Tool Call") || text.starts_with("Command") {
        Some(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else if text.starts_with("Tool Output") || text.starts_with("Output") {
        Some(
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )
    } else if text.starts_with("Error") {
        Some(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    } else if text.starts_with("Prompt") {
        Some(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        None
    }
}

fn heading_style(level: usize) -> Style {
    match level {
        1 => Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        _ => Style::default(),
    }
}

fn structural_marker_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn task_done_marker_style() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD)
}

fn code_style() -> Style {
    Style::default().fg(Color::Gray)
}

fn code_block_style() -> Style {
    Style::default().fg(Color::Gray)
}

fn code_block_margin_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn code_fence_style() -> Style {
    Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::ITALIC)
}

fn link_style() -> Style {
    Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::UNDERLINED)
}

fn blockquote_style() -> Style {
    Style::default().fg(Color::Green)
}

#[cfg(test)]
mod tests {
    use super::{render_markdown_window, MarkdownLineKind, CODE_INDENT};
    use ratatui::prelude::{Color, Modifier};

    #[test]
    fn renders_agent_headings_with_provider_roles() {
        let lines = render_markdown_window(&["## User", "## Assistant"], 0, 2, 80);
        let user = &lines[0].line;
        let assistant = &lines[1].line;

        assert_eq!(user.spans[0].content.as_ref(), "## ");
        assert_eq!(user.spans[1].content.as_ref(), "User");
        assert_eq!(user.spans[1].style.fg, Some(Color::Cyan));
        assert_eq!(assistant.spans[1].style.fg, Some(Color::Green));
    }

    #[test]
    fn renders_work_part_headings_with_distinct_styles() {
        let lines = render_markdown_window(
            &[
                "## Command",
                "## Tool Call (Bash)",
                "## Tool Output (Bash)",
                "## Error",
            ],
            0,
            4,
            80,
        );

        assert_eq!(lines[0].line.spans[1].style.fg, Some(Color::Yellow));
        assert_eq!(lines[1].line.spans[1].style.fg, Some(Color::Yellow));
        assert_eq!(lines[2].line.spans[1].style.fg, Some(Color::Blue));
        assert_eq!(lines[3].line.spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn renders_only_h1_as_strong_document_heading() {
        let lines = render_markdown_window(&["# Title", "## Section"], 0, 2, 80);
        let h1 = &lines[0].line;
        let h2 = &lines[1].line;

        assert_eq!(h1.spans[0].content.as_ref(), "# ");
        assert_eq!(h1.spans[1].content.as_ref(), "Title");
        assert!(h1.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert!(h1.spans[1]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(h1.spans[1].style.fg, None);

        assert_eq!(h2.spans[0].content.as_ref(), "## ");
        assert_eq!(h2.spans[1].content.as_ref(), "Section");
        assert_eq!(h2.spans[1].style, ratatui::prelude::Style::default());
    }

    #[test]
    fn renders_inline_markdown_without_removing_structural_prefixes() {
        let lines = render_markdown_window(&["- **bold** and `code`"], 0, 1, 80);
        let line = &lines[0].line;

        assert_eq!(line.spans[0].content.as_ref(), "- ");
        assert_eq!(line.spans[0].style.fg, Some(Color::DarkGray));
        assert_eq!(line.spans[1].content.as_ref(), "bold");
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[3].content.as_ref(), "code");
        assert_eq!(line.spans[3].style.fg, Some(Color::Gray));
        assert_eq!(line.spans[3].style.bg, None);
    }

    #[test]
    fn renders_emphasis_and_links_for_terminal_reading() {
        let lines = render_markdown_window(&["*em* [site](https://example.com)"], 0, 1, 80);
        let line = &lines[0].line;

        assert_eq!(line.spans[0].content.as_ref(), "em");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(line.spans[2].content.as_ref(), "site");
        assert_eq!(line.spans[2].style.fg, Some(Color::Blue));
        assert!(line.spans[2]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(lines[0].links.len(), 1);
        assert_eq!(lines[0].links[0].start, 3);
        assert_eq!(lines[0].links[0].end, 7);
        assert_eq!(lines[0].links[0].target, "https://example.com");
    }

    #[test]
    fn detects_plain_urls_as_clickable_links() {
        let lines = render_markdown_window(&["See https://example.com/docs."], 0, 1, 80);
        let line = &lines[0].line;

        assert_eq!(line.spans[0].content.as_ref(), "See ");
        assert_eq!(line.spans[1].content.as_ref(), "https://example.com/docs");
        assert_eq!(line.spans[1].style.fg, Some(Color::Blue));
        assert_eq!(line.spans[2].content.as_ref(), ".");
        assert_eq!(lines[0].links[0].start, 4);
        assert_eq!(lines[0].links[0].end, 28);
        assert_eq!(lines[0].links[0].target, "https://example.com/docs");
    }

    #[test]
    fn renders_structural_markers_as_dim_prefixes() {
        let lines = render_markdown_window(&["> quote", "1. item"], 0, 2, 80);

        assert_eq!(lines[0].line.spans[0].content.as_ref(), "> ");
        assert_eq!(lines[0].line.spans[0].style.fg, Some(Color::DarkGray));
        assert_eq!(lines[0].line.spans[1].content.as_ref(), "quote");
        assert_eq!(lines[0].line.spans[1].style.fg, Some(Color::Green));

        assert_eq!(lines[1].line.spans[0].content.as_ref(), "1. ");
        assert_eq!(lines[1].line.spans[0].style.fg, Some(Color::DarkGray));
        assert_eq!(lines[1].line.spans[1].content.as_ref(), "item");
    }

    #[test]
    fn renders_task_list_markers_as_square_symbols() {
        let lines = render_markdown_window(&["- [ ] open", "- [x] done"], 0, 2, 80);

        assert_eq!(lines[0].line.spans[0].content.as_ref(), "□ ");
        assert_eq!(lines[0].line.spans[0].style.fg, Some(Color::DarkGray));
        assert_eq!(lines[0].line.spans[1].content.as_ref(), "open");

        assert_eq!(lines[1].line.spans[0].content.as_ref(), "■ ");
        assert_eq!(lines[1].line.spans[0].style.fg, Some(Color::Green));
        assert_eq!(lines[1].line.spans[1].content.as_ref(), "done");
    }

    #[test]
    fn renders_horizontal_rules_to_content_width() {
        let lines = render_markdown_window(&["---"], 0, 1, 12);

        assert_eq!(lines[0].line.spans[0].content.as_ref(), "────────────");
        assert_eq!(lines[0].line.spans[0].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn aligns_markdown_tables_by_visible_cell_width() {
        let lines = render_markdown_window(
            &[
                "| Name | Status |",
                "| --- | --- |",
                "| sivtr | ok |",
                "| cli | needs review |",
            ],
            0,
            6,
            80,
        );

        assert_eq!(lines.len(), 6);
        assert_eq!(
            lines[0].line.spans[0].content.as_ref(),
            "┌───────┬──────────────┐"
        );
        assert_eq!(lines[1].line.spans[0].content.as_ref(), "│ ");
        assert_eq!(lines[1].line.spans[1].content.as_ref(), "Name");
        assert_eq!(lines[1].line.spans[2].content.as_ref(), " ");
        assert_eq!(lines[1].line.spans[3].content.as_ref(), " │ ");
        assert_eq!(
            lines[2].line.spans[0].content.as_ref(),
            "├───────┼──────────────┤"
        );
        assert_eq!(lines[3].line.spans[1].content.as_ref(), "sivtr");
        assert_eq!(lines[3].line.spans[2].content.as_ref(), " │ ");
        assert_eq!(lines[4].line.spans[1].content.as_ref(), "cli");
        assert_eq!(lines[4].line.spans[2].content.as_ref(), "  ");
        assert_eq!(
            lines[5].line.spans[0].content.as_ref(),
            "└───────┴──────────────┘"
        );
    }

    #[test]
    fn renders_fenced_code_blocks_as_indented_code() {
        let lines = render_markdown_window(&["```text", "-> value", "```"], 0, 3, 80);

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].kind, MarkdownLineKind::CodeFence);
        assert!(lines[0].line.spans.is_empty());
        assert_eq!(lines[1].kind, MarkdownLineKind::CodeBlock);
        assert_eq!(lines[1].line.spans[0].content.as_ref(), CODE_INDENT);
        assert_eq!(lines[1].line.spans[1].content.as_ref(), "-> value");
        assert_eq!(lines[1].line.spans[1].style.fg, Some(Color::Gray));
        assert_eq!(lines[2].kind, MarkdownLineKind::CodeFence);
        assert!(lines[2].line.spans.is_empty());
    }

    #[test]
    fn keeps_code_block_state_when_window_starts_inside_block() {
        let lines = render_markdown_window(&["```text", "alpha", "beta", "```"], 2, 1, 80);

        assert_eq!(lines[0].kind, MarkdownLineKind::CodeBlock);
        assert_eq!(lines[0].line.spans[1].content.as_ref(), "beta");
        assert_eq!(lines[0].line.spans[1].style.fg, Some(Color::Gray));
    }

    #[test]
    fn renders_useful_code_language_as_a_subtle_label() {
        let lines = render_markdown_window(&["```sql", "select 1", "```"], 0, 1, 80);

        assert_eq!(lines[0].kind, MarkdownLineKind::CodeFence);
        assert_eq!(lines[0].line.spans[0].content.as_ref(), "  sql");
    }
}
