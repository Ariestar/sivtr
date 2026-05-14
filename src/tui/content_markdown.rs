use pulldown_cmark::{Event, Options as MarkdownOptions, Parser, Tag, TagEnd};
use ratatui::prelude::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const CODE_INDENT: &str = "  ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkdownLineKind {
    Normal,
    CodeFence,
    CodeBlock,
}

#[derive(Clone, Debug)]
pub(crate) struct MarkdownLine {
    pub(crate) line: Line<'static>,
    pub(crate) kind: MarkdownLineKind,
}

pub(crate) fn render_markdown_window(
    lines: &[&str],
    scroll: usize,
    height: usize,
) -> Vec<MarkdownLine> {
    let mut in_code_block = false;
    let end = scroll.saturating_add(height).min(lines.len());
    let mut rendered = Vec::with_capacity(height);

    lines.iter().take(end).enumerate().for_each(|(idx, line)| {
        let line = render_markdown_line(line, &mut in_code_block);
        if idx >= scroll {
            rendered.push(line);
        }
    });

    rendered
}

fn render_markdown_line(line: &str, in_code_block: &mut bool) -> MarkdownLine {
    if let Some(language) = code_fence_language(line) {
        let opening = !*in_code_block;
        *in_code_block = !*in_code_block;
        return MarkdownLine {
            line: code_fence_line(opening.then_some(language).flatten()),
            kind: MarkdownLineKind::CodeFence,
        };
    }

    if *in_code_block {
        return MarkdownLine {
            line: Line::from(vec![
                Span::styled(CODE_INDENT, code_block_margin_style()),
                Span::styled(line.to_string(), code_block_style()),
            ]),
            kind: MarkdownLineKind::CodeBlock,
        };
    }

    let (prefix, content, line_style) = markdown_line_parts(line);
    let mut spans = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix, line_style));
    }
    spans.extend(markdown_inline_spans(content, line_style));
    MarkdownLine {
        line: Line {
            spans,
            style: line_style,
            alignment: None,
        },
        kind: MarkdownLineKind::Normal,
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

fn markdown_line_parts(line: &str) -> (String, &str, Style) {
    let leading_width = line.len() - line.trim_start().len();
    let leading = &line[..leading_width];
    let trimmed = &line[leading_width..];

    if let Some((level, rest)) = markdown_heading(trimmed) {
        let style = agent_heading_style(rest).unwrap_or_else(|| heading_style(level));
        return (format!("{leading}{} ", "#".repeat(level)), rest, style);
    }

    if let Some(rest) = trimmed.strip_prefix("> ") {
        return (format!("{leading}> "), rest, blockquote_style());
    }

    if let Some((marker, rest)) = markdown_list_item(trimmed) {
        return (format!("{leading}{marker}"), rest, Style::default());
    }

    (String::new(), line, Style::default())
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
        Some((level, &line[level + 1..]))
    } else {
        None
    }
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

fn markdown_inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(text, options);
    let mut spans = Vec::new();
    let mut styles = vec![base_style];
    let mut pending_link = None;

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
                    push_style(
                        &mut styles,
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::UNDERLINED),
                    );
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
                        spans.push(Span::styled(format!(" ({link})"), link_style()));
                    }
                }
                _ => {}
            },
            Event::Text(value) => {
                spans.push(Span::styled(value.to_string(), current_style(&styles)));
            }
            Event::Code(value) => spans.push(Span::styled(value.to_string(), code_style())),
            Event::SoftBreak | Event::HardBreak => {
                spans.push(Span::styled(" ".to_string(), current_style(&styles)));
            }
            Event::Html(value) | Event::InlineHtml(value) => {
                spans.push(Span::styled(value.to_string(), current_style(&styles)));
            }
            _ => {}
        }
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }
    spans
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
    } else {
        None
    }
}

fn heading_style(level: usize) -> Style {
    match level {
        1 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        2 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        3 => Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::LightCyan),
    }
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
        let lines = render_markdown_window(&["## User", "## Assistant"], 0, 2);
        let user = &lines[0].line;
        let assistant = &lines[1].line;

        assert_eq!(user.spans[0].content.as_ref(), "## ");
        assert_eq!(user.spans[1].content.as_ref(), "User");
        assert_eq!(user.spans[1].style.fg, Some(Color::Cyan));
        assert_eq!(assistant.spans[1].style.fg, Some(Color::Green));
    }

    #[test]
    fn renders_inline_markdown_without_removing_structural_prefixes() {
        let lines = render_markdown_window(&["- **bold** and `code`"], 0, 1);
        let line = &lines[0].line;

        assert_eq!(line.spans[0].content.as_ref(), "- ");
        assert_eq!(line.spans[1].content.as_ref(), "bold");
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[3].content.as_ref(), "code");
        assert_eq!(line.spans[3].style.fg, Some(Color::Gray));
        assert_eq!(line.spans[3].style.bg, None);
    }

    #[test]
    fn renders_fenced_code_blocks_as_indented_code() {
        let lines = render_markdown_window(&["```text", "-> value", "```"], 0, 3);

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
        let lines = render_markdown_window(&["```text", "alpha", "beta", "```"], 2, 1);

        assert_eq!(lines[0].kind, MarkdownLineKind::CodeBlock);
        assert_eq!(lines[0].line.spans[1].content.as_ref(), "beta");
        assert_eq!(lines[0].line.spans[1].style.fg, Some(Color::Gray));
    }

    #[test]
    fn renders_useful_code_language_as_a_subtle_label() {
        let lines = render_markdown_window(&["```sql", "select 1", "```"], 0, 1);

        assert_eq!(lines[0].kind, MarkdownLineKind::CodeFence);
        assert_eq!(lines[0].line.spans[0].content.as_ref(), "  sql");
    }
}
