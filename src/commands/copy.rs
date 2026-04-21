use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use regex::Regex;
use std::fs;

use sivtr_core::capture::scrollback;
use sivtr_core::parse::ansi::strip_ansi;

use crate::tui::terminal::{init as init_tui, restore as restore_tui};

const PROMPT_SYMBOLS: &[char] = &[
    '>', '$', '#', '%', '\u{03BB}', // lambda
    '\u{276F}', // heavy right angle quote ornament
    '\u{279C}', // heavy round-tipped rightwards arrow
    '\u{203A}', // single right-pointing angle quote
    '\u{00BB}', // right-pointing double angle quote
];
const PICK_LIMIT: usize = 50;
const PICK_PREVIEW_LINES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyMode {
    Both,
    InputOnly,
    OutputOnly,
    CommandOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandBlock {
    input_with_prompt: String,
    input_without_prompt: String,
    output: String,
    command: String,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum CommandSelection {
    RecentCount(usize),
    RecentRange { newer: usize, older: usize },
    RecentExplicit(Vec<usize>),
}

/// Copy recent command blocks to clipboard.
pub fn execute(
    selector: Option<&str>,
    pick: bool,
    mode: CopyMode,
    include_prompt: bool,
    print_full: bool,
    regex: Option<&str>,
    lines: Option<&str>,
) -> Result<()> {
    let log_path = scrollback::session_log_path();
    let boundaries_path = log_path.with_extension("boundaries");

    if !log_path.exists() || !boundaries_path.exists() {
        eprintln!("sivtr: no session log found");
        eprintln!("  hint: run `sivtr init <shell>`, restart the shell, then run some commands");
        return Ok(());
    }

    let content = fs::read_to_string(&log_path).context("Failed to read session log")?;
    let boundaries: Vec<usize> = fs::read_to_string(&boundaries_path)?
        .lines()
        .filter_map(|line| line.parse().ok())
        .collect();

    if boundaries.is_empty() {
        eprintln!("sivtr: no commands recorded yet");
        eprintln!("  hint: run a few commands first, then try `sivtr copy` again");
        return Ok(());
    }

    let total = boundaries.len();
    let selection = if pick {
        pick_selection(&content, &boundaries)?
    } else {
        parse_selection(selector.unwrap_or("1"))?
    };

    let indices = resolve_selection(selection, total)?;
    if indices.is_empty() {
        eprintln!("sivtr: nothing selected");
        eprintln!("  hint: choose at least one command block");
        return Ok(());
    }

    let blocks: Vec<String> = indices
        .iter()
        .filter_map(|idx| extract_block(&content, &boundaries, *idx))
        .map(parse_command_block)
        .map(|block| format_block(&block, mode, include_prompt))
        .filter(|block| !block.trim().is_empty())
        .collect();

    if blocks.is_empty() {
        eprintln!("sivtr: selected commands are empty");
        eprintln!("  hint: try `sivtr copy --out` or choose a different block");
        return Ok(());
    }

    let mut text = blocks.join("\n\n");

    if let Some(pattern) = regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    let text = text.trim().to_string();
    if text.is_empty() {
        eprintln!("sivtr: filters removed everything");
        eprintln!("  hint: loosen `--regex` or `--lines`, or copy without filters");
        return Ok(());
    }

    arboard::Clipboard::new()
        .context("Failed to open clipboard")?
        .set_text(&text)
        .context("Failed to set clipboard")?;

    if print_full {
        for line in text.lines() {
            eprintln!("  {line}");
        }
    } else {
        let preview: Vec<&str> = text.lines().take(4).collect();
        let line_count = text.lines().count();
        for line in &preview {
            eprintln!("  {line}");
        }
        if line_count > 4 {
            eprintln!("  ... ({line_count} lines total)");
        }
    }

    eprintln!(
        "sivtr: copied {} command(s) to clipboard{}",
        indices.len(),
        if print_full { " (full text shown)" } else { "" }
    );

    Ok(())
}

fn extract_block<'a>(content: &'a str, boundaries: &[usize], idx: usize) -> Option<&'a str> {
    let start = *boundaries.get(idx)?;
    let end = boundaries.get(idx + 1).copied().unwrap_or(content.len());
    content.get(start..end)
}

fn parse_command_block(block: &str) -> CommandBlock {
    let clean = strip_ansi(block).replace("\r\n", "\n");
    let lines: Vec<&str> = clean.lines().collect();

    let first = lines.iter().position(|line| !line.trim().is_empty());
    let last = lines.iter().rposition(|line| !line.trim().is_empty());

    let Some(first) = first else {
        return CommandBlock {
            input_with_prompt: String::new(),
            input_without_prompt: String::new(),
            output: String::new(),
            command: String::new(),
        };
    };
    let last = last.unwrap_or(first);
    let lines = &lines[first..=last];

    let command_idx = lines.iter().position(|line| looks_like_command_line(line));

    match command_idx {
        Some(idx) => {
            let command_line = lines[idx].trim_start();
            let command = extract_command_text(command_line).unwrap_or_default();
            CommandBlock {
                input_with_prompt: join_lines(&lines[..=idx]),
                input_without_prompt: command.clone(),
                output: join_lines(&lines[idx + 1..]),
                command,
            }
        }
        None => CommandBlock {
            input_with_prompt: String::new(),
            input_without_prompt: String::new(),
            output: join_lines(lines),
            command: String::new(),
        },
    }
}

fn looks_like_command_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.starts_with("PS ") && trimmed.contains('>') {
        return true;
    }

    if trimmed.starts_with("In>") || trimmed.starts_with("Out>") {
        return true;
    }

    PROMPT_SYMBOLS
        .iter()
        .any(|symbol| trimmed.starts_with(*symbol) && trimmed.chars().nth(1).is_some())
}

fn extract_command_text(line: &str) -> Option<String> {
    let trimmed = line.trim_start();

    if trimmed.starts_with("PS ") {
        let (_, rest) = trimmed.split_once('>')?;
        return Some(rest.trim().to_string());
    }

    if let Some(rest) = trimmed.strip_prefix("In>") {
        return Some(rest.trim().to_string());
    }

    if let Some(rest) = trimmed.strip_prefix("Out>") {
        return Some(rest.trim().to_string());
    }

    for symbol in PROMPT_SYMBOLS {
        if let Some(rest) = trimmed.strip_prefix(*symbol) {
            return Some(rest.trim().to_string());
        }
    }

    None
}

fn join_lines(lines: &[&str]) -> String {
    lines.join("\n").trim().to_string()
}

fn format_block(block: &CommandBlock, mode: CopyMode, include_prompt: bool) -> String {
    match mode {
        CopyMode::Both => {
            let input = if include_prompt {
                &block.input_with_prompt
            } else {
                &block.input_without_prompt
            };
            match (input.is_empty(), block.output.is_empty()) {
                (false, false) => format!("{}\n{}", input, block.output),
                (false, true) => input.clone(),
                (true, false) => block.output.clone(),
                (true, true) => String::new(),
            }
        }
        CopyMode::InputOnly => {
            if include_prompt {
                block.input_with_prompt.clone()
            } else {
                block.input_without_prompt.clone()
            }
        }
        CopyMode::OutputOnly => block.output.clone(),
        CopyMode::CommandOnly => block.command.clone(),
    }
}

fn parse_selection(value: &str) -> Result<CommandSelection> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("Empty selector. Use `N`, `A..B`, or `--pick`.");
    }

    if let Some((a, b)) = value.split_once("..") {
        let a = parse_positive(a)?;
        let b = parse_positive(b)?;
        let (newer, older) = if a <= b { (a, b) } else { (b, a) };
        return Ok(CommandSelection::RecentRange { newer, older });
    }

    Ok(CommandSelection::RecentCount(parse_positive(value)?))
}

fn resolve_selection(selection: CommandSelection, total: usize) -> Result<Vec<usize>> {
    match selection {
        CommandSelection::RecentCount(n) => {
            if n == 0 {
                anyhow::bail!("Selector values are 1-based. Use `1` for the last command.");
            }
            if n > total {
                anyhow::bail!(
                    "Only {total} command(s) recorded. Try a smaller selector or `--pick`."
                );
            }
            Ok(((total - n)..total).collect())
        }
        CommandSelection::RecentRange { newer, older } => {
            if newer == 0 || older == 0 {
                anyhow::bail!("Range selectors are 1-based. Example: `2..5`.");
            }
            if older > total {
                anyhow::bail!(
                    "Only {total} command(s) recorded. Try a smaller range or `--pick`."
                );
            }
            let start = total - older;
            let end = total - newer;
            Ok((start..=end).collect())
        }
        CommandSelection::RecentExplicit(selected) => {
            if selected.is_empty() {
                anyhow::bail!("No command blocks selected.");
            }

            let mut indices = Vec::with_capacity(selected.len());
            for recent in selected {
                if recent == 0 {
                    anyhow::bail!("Selector values are 1-based. Use `1` for the last command.");
                }
                if recent > total {
                    anyhow::bail!(
                        "Only {total} command(s) recorded. Try a smaller selector or `--pick`."
                    );
                }
                indices.push(total - recent);
            }

            indices.sort_unstable();
            indices.dedup();
            Ok(indices)
        }
    }
}

fn pick_selection(content: &str, boundaries: &[usize]) -> Result<CommandSelection> {
    let total = boundaries.len();
    let shown = total.min(PICK_LIMIT);
    let entries: Vec<PickEntry> = (1..=shown)
        .map(|recent| {
            let idx = total - recent;
            let block = extract_block(content, boundaries, idx)
                .map(parse_command_block)
                .unwrap_or_else(|| CommandBlock {
                    input_with_prompt: String::new(),
                    input_without_prompt: String::new(),
                    output: String::new(),
                    command: String::new(),
                });
            let output_preview = build_output_preview(&block);
            let preview = if !block.command.is_empty() {
                block.command.clone()
            } else if !block.output.is_empty() {
                block.output.lines().next().unwrap_or("").to_string()
            } else {
                "<empty>".to_string()
            };
            PickEntry {
                recent,
                preview,
                output_preview,
                selected: false,
            }
        })
        .collect();

    run_picker(entries, total)
}

fn filter_lines_by_regex(text: &str, pattern: &str) -> Result<String> {
    let regex = Regex::new(pattern)
        .with_context(|| format!("Invalid regex `{pattern}`. Check the pattern syntax."))?;
    Ok(text
        .lines()
        .filter(|line| regex.is_match(line))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn filter_lines_by_spec(text: &str, spec: &str) -> Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut selected = Vec::new();

    for part in spec
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let range = part.split_once(':');

        if let Some((start, end)) = range {
            let start = parse_line_number(start)?;
            let end = parse_line_number(end)?;
            if start == 0 || end == 0 {
                anyhow::bail!("Line ranges are 1-based. Example: `10:20`.");
            }
            let (start, end) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            for idx in start..=end {
                if let Some(line) = lines.get(idx - 1) {
                    selected.push((*line).to_string());
                }
            }
        } else {
            let idx = parse_line_number(part)?;
            if idx == 0 {
                anyhow::bail!("Line numbers are 1-based. Example: `1,3,8:12`.");
            }
            if let Some(line) = lines.get(idx - 1) {
                selected.push((*line).to_string());
            }
        }
    }

    Ok(selected.join("\n"))
}

fn parse_positive(value: &str) -> Result<usize> {
    let n = value
        .parse::<usize>()
        .with_context(|| format!("Invalid selector `{value}`. Use `N`, `A..B`, or `--pick`."))?;
    if n == 0 {
        anyhow::bail!("Selector values are 1-based. Use `1` for the last command.");
    }
    Ok(n)
}

fn parse_line_number(value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .with_context(|| {
            format!("Invalid line number `{value}`. Use `N`, `A:B`, or comma-separated lists.")
        })
}

#[cfg(test)]
mod tests {
    use super::{
        apply_range_toggle, build_output_preview, extract_command_text, filter_lines_by_regex,
        filter_lines_by_spec, format_block, looks_like_command_line, parse_command_block,
        parse_selection, resolve_selection, selection_from_entries, CommandBlock, CommandSelection,
        CopyMode, PickEntry,
    };

    #[test]
    fn parses_powershell_input_and_output() {
        let block = "PS C:\\repo> git add --all -a\nwarning: file\nwarning: file2\n";
        let parsed = parse_command_block(block);
        assert_eq!(parsed.input_with_prompt, "PS C:\\repo> git add --all -a");
        assert_eq!(parsed.input_without_prompt, "git add --all -a");
        assert_eq!(parsed.output, "warning: file\nwarning: file2");
        assert_eq!(parsed.command, "git add --all -a");
    }

    #[test]
    fn parses_prompt_symbol_input_and_output() {
        let block = "sivtr on main +7\n\u{276F} git commit -m \"feat: basic vim\"\n[main 123] feat: basic vim\n";
        let parsed = parse_command_block(block);
        assert_eq!(
            parsed.input_with_prompt,
            "sivtr on main +7\n\u{276F} git commit -m \"feat: basic vim\""
        );
        assert_eq!(
            parsed.input_without_prompt,
            "git commit -m \"feat: basic vim\""
        );
        assert_eq!(parsed.output, "[main 123] feat: basic vim");
        assert_eq!(parsed.command, "git commit -m \"feat: basic vim\"");
    }

    #[test]
    fn keeps_output_only_when_no_prompt_detected() {
        let block = "warning: file\nwarning: file2\n";
        let parsed = parse_command_block(block);
        assert!(parsed.input_with_prompt.is_empty());
        assert!(parsed.input_without_prompt.is_empty());
        assert_eq!(parsed.output, "warning: file\nwarning: file2");
        assert!(parsed.command.is_empty());
    }

    #[test]
    fn detects_prompt_line_shapes() {
        assert!(looks_like_command_line("PS C:\\repo> sivtr copy 3"));
        assert!(looks_like_command_line("\u{276F} git add ."));
        assert!(!looks_like_command_line("warning: file"));
    }

    #[test]
    fn extracts_bare_command_text() {
        assert_eq!(
            extract_command_text("PS C:\\repo> git status --all -a").unwrap(),
            "git status --all -a"
        );
        assert_eq!(
            extract_command_text("\u{276F} cargo test").unwrap(),
            "cargo test"
        );
    }

    #[test]
    fn formats_modes() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> git status --all -a".to_string(),
            input_without_prompt: "git status --all -a".to_string(),
            output: "clean".to_string(),
            command: "git status --all -a".to_string(),
        };
        assert_eq!(
            format_block(&block, CopyMode::Both, false),
            "git status --all -a\nclean"
        );
        assert_eq!(
            format_block(&block, CopyMode::Both, true),
            "PS C:\\repo> git status --all -a\nclean"
        );
        assert_eq!(
            format_block(&block, CopyMode::InputOnly, false),
            "git status --all -a"
        );
        assert_eq!(
            format_block(&block, CopyMode::InputOnly, true),
            "PS C:\\repo> git status --all -a"
        );
        assert_eq!(format_block(&block, CopyMode::OutputOnly, false), "clean");
        assert_eq!(
            format_block(&block, CopyMode::CommandOnly, false),
            "git status --all -a"
        );
    }

    #[test]
    fn parses_selection_count() {
        assert_eq!(
            parse_selection("3").unwrap(),
            CommandSelection::RecentCount(3)
        );
    }

    #[test]
    fn parses_selection_range() {
        assert_eq!(
            parse_selection("5..2").unwrap(),
            CommandSelection::RecentRange { newer: 2, older: 5 }
        );
    }

    #[test]
    fn resolves_selection_range() {
        assert_eq!(
            resolve_selection(CommandSelection::RecentRange { newer: 2, older: 5 }, 10).unwrap(),
            vec![5, 6, 7, 8]
        );
    }

    #[test]
    fn resolves_explicit_selection_as_disjoint_commands() {
        assert_eq!(
            resolve_selection(CommandSelection::RecentExplicit(vec![1, 4, 7]), 10).unwrap(),
            vec![3, 6, 9]
        );
    }

    #[test]
    fn picker_selection_keeps_disjoint_blocks() {
        let selection = selection_from_entries(&[
            PickEntry {
                recent: 1,
                preview: "latest".to_string(),
                output_preview: "out1".to_string(),
                selected: true,
            },
            PickEntry {
                recent: 2,
                preview: "second".to_string(),
                output_preview: "out2".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 4,
                preview: "fourth".to_string(),
                output_preview: "out4".to_string(),
                selected: true,
            },
        ])
        .unwrap();

        assert_eq!(selection, CommandSelection::RecentExplicit(vec![1, 4]));
    }

    #[test]
    fn filters_by_regex() {
        let filtered = filter_lines_by_regex("a\nwarn: b\nc", "warn").unwrap();
        assert_eq!(filtered, "warn: b");
    }

    #[test]
    fn filters_by_line_spec_with_colon_ranges() {
        let filtered = filter_lines_by_spec("a\nb\nc\nd", "2,4:3").unwrap();
        assert_eq!(filtered, "b\nc\nd");
    }

    #[test]
    fn rejects_dash_ranges_for_lines() {
        assert!(filter_lines_by_spec("a\nb\nc", "1-2").is_err());
    }

    #[test]
    fn toggles_selected_range_in_picker() {
        let mut entries = vec![
            PickEntry {
                recent: 1,
                preview: "one".to_string(),
                output_preview: "out1".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 2,
                preview: "two".to_string(),
                output_preview: "out2".to_string(),
                selected: false,
            },
            PickEntry {
                recent: 3,
                preview: "three".to_string(),
                output_preview: "out3".to_string(),
                selected: true,
            },
        ];

        apply_range_toggle(&mut entries, 0, 2);
        assert!(entries.iter().all(|entry| entry.selected));

        apply_range_toggle(&mut entries, 0, 2);
        assert!(entries.iter().all(|entry| !entry.selected));
    }

    #[test]
    fn builds_output_preview_from_first_lines() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> cargo test".to_string(),
            input_without_prompt: "cargo test".to_string(),
            output: "line1\nline2\nline3".to_string(),
            command: "cargo test".to_string(),
        };

        assert_eq!(build_output_preview(&block), "line1\nline2\nline3");
    }
}

#[derive(Debug, Clone)]
struct PickEntry {
    recent: usize,
    preview: String,
    output_preview: String,
    selected: bool,
}

fn run_picker(mut entries: Vec<PickEntry>, total: usize) -> Result<CommandSelection> {
    let mut terminal = init_tui()?;
    let mut state = ListState::default();
    state.select(Some(0));
    let mut range_anchor = None;
    let mut show_preview = false;

    loop {
        terminal.draw(|frame| {
            render_picker(frame, &entries, &state, total, range_anchor, show_preview)
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    restore_tui(&mut terminal)?;
                    anyhow::bail!("Pick cancelled");
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let current = state.selected().unwrap_or(0);
                    state.select(Some(current.saturating_sub(1)));
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let current = state.selected().unwrap_or(0);
                    let next = (current + 1).min(entries.len().saturating_sub(1));
                    state.select(Some(next));
                }
                KeyCode::Char('v') => {
                    let current = state.selected().unwrap_or(0);
                    range_anchor = match range_anchor {
                        Some(anchor) if anchor == current => None,
                        _ => Some(current),
                    };
                }
                KeyCode::Char(' ') => {
                    if let Some(idx) = state.selected() {
                        if let Some(anchor) = range_anchor.take() {
                            apply_range_toggle(&mut entries, anchor, idx);
                        } else if let Some(entry) = entries.get_mut(idx) {
                            entry.selected = !entry.selected;
                        }
                    }
                }
                KeyCode::Char('a') => {
                    let select_all = entries.iter().any(|entry| !entry.selected);
                    for entry in &mut entries {
                        entry.selected = select_all;
                    }
                    range_anchor = None;
                }
                KeyCode::Char('p') => {
                    show_preview = !show_preview;
                }
                KeyCode::Enter => {
                    restore_tui(&mut terminal)?;
                    return selection_from_entries(&entries);
                }
                _ => {}
            }
        }
    }
}

fn render_picker(
    frame: &mut Frame,
    entries: &[PickEntry],
    state: &ListState,
    total: usize,
    range_anchor: Option<usize>,
    show_preview: bool,
) {
    let area = centered_rect(80, 70, frame.area());
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);

    let anchor_hint = range_anchor
        .map(|anchor| format!("  v range@{}", anchor + 1))
        .unwrap_or_default();
    let title = Paragraph::new(format!(
        "Pick command blocks  Space toggle  v mark-range  p preview  a toggle-all  Enter confirm  Esc cancel{}\nshowing last {} of {} commands",
        anchor_hint,
        entries.len(),
        total
    ))
    .block(Block::default().borders(Borders::ALL).title("sivtr copy --pick"));
    frame.render_widget(title, chunks[0]);

    let body_chunks = if show_preview {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
            .split(chunks[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100), Constraint::Percentage(0)])
            .split(chunks[1])
    };

    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let marker = if entry.selected { "[x]" } else { "[ ]" };
            let is_in_pending_range = range_anchor
                .map(|anchor| range_bounds(anchor, state.selected().unwrap_or(0)))
                .map(|(start, end)| (start..=end).contains(&idx))
                .unwrap_or(false);
            let line = format!("{marker} {:>2}. {}", entry.recent, entry.preview);
            if is_in_pending_range {
                ListItem::new(Line::styled(
                    line,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Commands"))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");
    let mut local_state = state.clone();
    frame.render_stateful_widget(list, body_chunks[0], &mut local_state);

    if show_preview {
        let preview_text = state
            .selected()
            .and_then(|idx| entries.get(idx))
            .map(|entry| entry.output_preview.as_str())
            .unwrap_or("<no output>");
        let preview = Paragraph::new(preview_text)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Output Preview"),
            );
        frame.render_widget(preview, body_chunks[1]);
    }

    let footer = Paragraph::new(
        "Space toggles one row. v marks a range anchor; move and press Space to toggle the whole range. Multiple selections are copied oldest to newest.",
    )
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn selection_from_entries(entries: &[PickEntry]) -> Result<CommandSelection> {
    let mut selected: Vec<usize> = entries
        .iter()
        .filter(|entry| entry.selected)
        .map(|entry| entry.recent)
        .collect();

    if selected.is_empty() {
        anyhow::bail!("No command blocks selected. Toggle one or more entries, then press Enter.");
    }

    selected.sort_unstable();
    selected.dedup();

    Ok(CommandSelection::RecentExplicit(selected))
}

fn apply_range_toggle(entries: &mut [PickEntry], a: usize, b: usize) {
    let (start, end) = range_bounds(a, b);
    let select_range = entries[start..=end].iter().any(|entry| !entry.selected);
    for entry in &mut entries[start..=end] {
        entry.selected = select_range;
    }
}

fn range_bounds(a: usize, b: usize) -> (usize, usize) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn build_output_preview(block: &CommandBlock) -> String {
    if block.output.trim().is_empty() {
        return "<no output>".to_string();
    }

    let mut lines: Vec<&str> = block.output.lines().take(PICK_PREVIEW_LINES).collect();
    let total_lines = block.output.lines().count();
    if total_lines > PICK_PREVIEW_LINES {
        lines.push("...");
    }
    lines.join("\n")
}
