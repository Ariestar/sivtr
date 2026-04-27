use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use regex::Regex;

use crate::command_blocks::ParsedCommandBlock as CommandBlock;
use crate::commands::command_block_selector::{parse_selector, resolve_selector, CommandSelection};
use sivtr_core::capture::scrollback;
use sivtr_core::codex::{
    find_current_session, format_blocks, parse_session_file, CodexBlock, CodexBlockKind,
    CodexSession,
};
use sivtr_core::session::{self, SessionEntry};

use crate::tui::terminal::{init as init_tui, restore as restore_tui};

const PICK_LIMIT: usize = 50;
const PICK_PREVIEW_LINES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyMode {
    Both,
    InputOnly,
    OutputOnly,
    CommandOnly,
}

#[derive(Clone, Copy, Debug)]
pub struct CopyRequest<'a> {
    pub selector: Option<&'a str>,
    pub pick: bool,
    pub mode: CopyMode,
    pub include_prompt: bool,
    pub prompt_override: Option<&'a str>,
    pub print_full: bool,
    pub ansi: bool,
    pub regex: Option<&'a str>,
    pub lines: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexSelectionMode {
    LastTurn,
    LastAssistant,
    LastUser,
    LastTool,
    All,
}

#[derive(Clone, Copy, Debug)]
pub struct CodexCopyRequest<'a> {
    pub selector: Option<&'a str>,
    pub pick: bool,
    pub selection_mode: CodexSelectionMode,
    pub print_full: bool,
    pub regex: Option<&'a str>,
    pub lines: Option<&'a str>,
}

#[derive(Clone, Debug)]
struct IndexedCommandBlock {
    plain: CommandBlock,
    ansi: Option<CommandBlock>,
}

impl IndexedCommandBlock {
    fn from_session_entry(entry: &SessionEntry) -> Self {
        let plain = CommandBlock::from_session_entry(entry);
        let ansi = entry.has_ansi().then(|| CommandBlock {
            input_with_prompt: entry.render_input_ansi(),
            input_without_prompt: plain.input_without_prompt.clone(),
            output: entry
                .output_ansi
                .clone()
                .unwrap_or_else(|| plain.output.clone()),
            command: plain.command.clone(),
        });

        Self { plain, ansi }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TextPair {
    plain: String,
    ansi: String,
}

/// Copy recent command blocks to clipboard.
pub fn execute(request: CopyRequest<'_>) -> Result<()> {
    let CopyRequest {
        selector,
        pick,
        mode,
        include_prompt,
        prompt_override,
        print_full,
        ansi,
        regex,
        lines,
    } = request;

    let log_path = scrollback::session_log_path();
    if !log_path.exists() {
        eprintln!("sivtr: no session log found");
        eprintln!("  hint: run `sivtr init <shell>`, restart the shell, then run some commands");
        return Ok(());
    }

    let entries = session::load_entries(&log_path).context("Failed to read session log")?;
    if entries.is_empty() {
        eprintln!("sivtr: no commands recorded yet");
        eprintln!("  hint: run a few commands first, then try `sivtr copy` again");
        return Ok(());
    }

    let blocks: Vec<IndexedCommandBlock> = entries
        .iter()
        .map(IndexedCommandBlock::from_session_entry)
        .collect();

    let total = blocks.len();
    if total == 0 {
        eprintln!("sivtr: no commands recorded yet");
        eprintln!("  hint: run a command first, then try `sivtr copy` again");
        return Ok(());
    }

    let selection = if pick {
        pick_selection(&blocks)?
    } else {
        parse_selector(selector.unwrap_or("1"))?
    };

    let indices = resolve_selector(selection, total)?;
    if indices.is_empty() {
        eprintln!("sivtr: nothing selected");
        eprintln!("  hint: choose at least one command block");
        return Ok(());
    }

    let copied_blocks: Vec<TextPair> = indices
        .iter()
        .filter_map(|idx| blocks.get(*idx))
        .map(|block| format_block_pair(block, mode, include_prompt, prompt_override))
        .filter(|block| !block.plain.trim().is_empty())
        .collect();

    if copied_blocks.is_empty() {
        eprintln!("sivtr: selected commands are empty");
        eprintln!("  hint: try `sivtr copy --out` or choose a different block");
        return Ok(());
    }

    let mut text = join_text_pairs(&copied_blocks, "\n\n");

    if let Some(pattern) = regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    let text = if ansi {
        text.ansi.trim().to_string()
    } else {
        text.plain.trim().to_string()
    };
    finish_copy(
        text,
        print_full,
        format!("sivtr: copied {} command(s) to clipboard", indices.len()),
    )
}

pub fn execute_codex(request: CodexCopyRequest<'_>) -> Result<()> {
    let path = resolve_codex_session_path()?;
    let session = parse_session_file(&path)?;

    if session.blocks.is_empty() {
        eprintln!("sivtr: Codex session has no parsed conversation blocks");
        return Ok(());
    }

    let units = build_codex_units(&session, request.selection_mode);
    if units.is_empty() {
        eprintln!("sivtr: selected Codex content is empty");
        return Ok(());
    }

    let selection = if request.pick {
        pick_text_selection(&units, "sivtr copy codex --pick")?
    } else {
        parse_selector(request.selector.unwrap_or("1"))?
    };
    let indices = resolve_selector(selection, units.len())?;
    let selected_units: Vec<TextPair> = indices
        .iter()
        .filter_map(|idx| units.get(*idx).cloned())
        .filter(|unit| !unit.plain.trim().is_empty())
        .collect();
    if selected_units.is_empty() {
        eprintln!("sivtr: selected Codex content is empty");
        return Ok(());
    }

    let mut text = join_text_pairs(&selected_units, "\n\n");

    if let Some(pattern) = request.regex {
        text = filter_lines_by_regex(&text, pattern)?;
    }

    if let Some(spec) = request.lines {
        text = filter_lines_by_spec(&text, spec)?;
    }

    finish_copy(
        text.plain.trim().to_string(),
        request.print_full,
        "sivtr: copied Codex content to clipboard".to_string(),
    )
}

fn format_block_pair(
    block: &IndexedCommandBlock,
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
) -> TextPair {
    let plain = format_block(&block.plain, mode, include_prompt, prompt_override);
    let ansi = format_block(
        block.ansi.as_ref().unwrap_or(&block.plain),
        mode,
        include_prompt,
        prompt_override,
    );

    TextPair { plain, ansi }
}

fn join_text_pairs(pairs: &[TextPair], separator: &str) -> TextPair {
    TextPair {
        plain: pairs
            .iter()
            .map(|pair| pair.plain.as_str())
            .collect::<Vec<_>>()
            .join(separator),
        ansi: pairs
            .iter()
            .map(|pair| pair.ansi.as_str())
            .collect::<Vec<_>>()
            .join(separator),
    }
}

fn format_block(
    block: &CommandBlock,
    mode: CopyMode,
    include_prompt: bool,
    prompt_override: Option<&str>,
) -> String {
    match mode {
        CopyMode::Both => {
            let input = if include_prompt {
                format_input(block, prompt_override)
            } else {
                block.input_without_prompt.clone()
            };
            match (input.is_empty(), block.output.is_empty()) {
                (false, false) => format!("{}\n{}", input, block.output),
                (false, true) => input,
                (true, false) => block.output.clone(),
                (true, true) => String::new(),
            }
        }
        CopyMode::InputOnly => {
            if include_prompt {
                format_input(block, prompt_override)
            } else {
                block.input_without_prompt.clone()
            }
        }
        CopyMode::OutputOnly => block.output.clone(),
        CopyMode::CommandOnly => block.command.clone(),
    }
}

fn format_input(block: &CommandBlock, prompt_override: Option<&str>) -> String {
    match prompt_override {
        Some(prompt) if !block.command.is_empty() => render_prompt_override(prompt, &block.command),
        Some(_) => block.input_with_prompt.clone(),
        None => block.input_with_prompt.clone(),
    }
}

fn render_prompt_override(prompt: &str, command: &str) -> String {
    let prompt = prompt.trim_end_matches(['\r', '\n']);
    if prompt.is_empty() {
        return command.to_string();
    }

    if prompt.ends_with(' ') || prompt.ends_with('\t') {
        format!("{prompt}{command}")
    } else {
        format!("{prompt} {command}")
    }
}

fn pick_selection(blocks: &[IndexedCommandBlock]) -> Result<CommandSelection> {
    let total = blocks.len();
    let shown = total.min(PICK_LIMIT);
    let entries: Vec<PickEntry> = blocks
        .iter()
        .rev()
        .take(shown)
        .enumerate()
        .map(|(offset, block)| {
            let recent = offset + 1;
            let output_preview = build_output_preview(&block.plain);
            let preview = if !block.plain.command.is_empty() {
                block.plain.command.clone()
            } else if !block.plain.output.is_empty() {
                block.plain.output.lines().next().unwrap_or("").to_string()
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

    run_picker(entries, total, "sivtr copy --pick")
}

fn filter_lines_by_regex(text: &TextPair, pattern: &str) -> Result<TextPair> {
    let regex = Regex::new(pattern)
        .with_context(|| format!("Invalid regex `{pattern}`. Check the pattern syntax."))?;
    let indices = text
        .plain
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| regex.is_match(line).then_some(idx))
        .collect::<Vec<_>>();
    Ok(select_lines(text, &indices))
}

fn filter_lines_by_spec(text: &TextPair, spec: &str) -> Result<TextPair> {
    let lines: Vec<&str> = text.plain.lines().collect();
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
                if lines.get(idx - 1).is_some() {
                    selected.push(idx - 1);
                }
            }
        } else {
            let idx = parse_line_number(part)?;
            if idx == 0 {
                anyhow::bail!("Line numbers are 1-based. Example: `1,3,8:12`.");
            }
            if lines.get(idx - 1).is_some() {
                selected.push(idx - 1);
            }
        }
    }

    Ok(select_lines(text, &selected))
}

fn select_lines(text: &TextPair, indices: &[usize]) -> TextPair {
    let plain_lines: Vec<&str> = text.plain.lines().collect();
    let ansi_lines: Vec<&str> = text.ansi.lines().collect();
    let mut plain_selected = Vec::new();
    let mut ansi_selected = Vec::new();

    for &idx in indices {
        if let Some(line) = plain_lines.get(idx) {
            plain_selected.push((*line).to_string());
            ansi_selected.push(ansi_lines.get(idx).copied().unwrap_or(line).to_string());
        }
    }

    TextPair {
        plain: plain_selected.join("\n"),
        ansi: ansi_selected.join("\n"),
    }
}

fn parse_line_number(value: &str) -> Result<usize> {
    value.parse::<usize>().with_context(|| {
        format!("Invalid line number `{value}`. Use `N`, `A:B`, or comma-separated lists.")
    })
}

fn finish_copy(text: String, print_full: bool, success_message: String) -> Result<()> {
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
    }

    eprintln!("{success_message}");
    Ok(())
}

fn resolve_codex_session_path() -> Result<std::path::PathBuf> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    find_current_session(&cwd)?.context("No Codex sessions found")
}

fn build_codex_units(session: &CodexSession, selection_mode: CodexSelectionMode) -> Vec<TextPair> {
    match selection_mode {
        CodexSelectionMode::LastTurn => build_codex_turn_units(session),
        CodexSelectionMode::LastAssistant => {
            build_codex_kind_units(session, CodexBlockKind::Assistant)
        }
        CodexSelectionMode::LastUser => build_codex_kind_units(session, CodexBlockKind::User),
        CodexSelectionMode::LastTool => build_codex_kind_units(session, CodexBlockKind::ToolOutput),
        CodexSelectionMode::All => vec![TextPair {
            plain: format_blocks(&session.blocks),
            ansi: String::new(),
        }],
    }
}

fn build_codex_turn_units(session: &CodexSession) -> Vec<TextPair> {
    let mut turns = Vec::new();

    for (idx, block) in session.blocks.iter().enumerate() {
        if block.kind != CodexBlockKind::Assistant {
            continue;
        }

        let start = session.blocks[..idx]
            .iter()
            .rposition(|block| block.kind == CodexBlockKind::User)
            .unwrap_or(idx);

        let turn_blocks: Vec<CodexBlock> = session.blocks[start..=idx]
            .iter()
            .filter(|block| matches!(block.kind, CodexBlockKind::User | CodexBlockKind::Assistant))
            .cloned()
            .collect();

        let text = format_blocks(&turn_blocks);
        if !text.trim().is_empty() {
            turns.push(TextPair {
                plain: text,
                ansi: String::new(),
            });
        }
    }

    turns
}

fn build_codex_kind_units(session: &CodexSession, kind: CodexBlockKind) -> Vec<TextPair> {
    session
        .blocks
        .iter()
        .filter(|block| block.kind == kind)
        .map(|block| TextPair {
            plain: block.text.clone(),
            ansi: String::new(),
        })
        .collect()
}

fn pick_text_selection(units: &[TextPair], title: &str) -> Result<CommandSelection> {
    let total = units.len();
    let shown = total.min(PICK_LIMIT);
    let entries: Vec<PickEntry> = units
        .iter()
        .rev()
        .take(shown)
        .enumerate()
        .map(|(offset, unit)| PickEntry {
            recent: offset + 1,
            preview: build_text_preview(&unit.plain),
            output_preview: build_text_preview_lines(&unit.plain),
            selected: false,
        })
        .collect();

    run_picker(entries, total, title)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_range_toggle, build_output_preview, filter_lines_by_regex, filter_lines_by_spec,
        format_block, selection_from_entries, CommandBlock, CommandSelection, CopyMode, PickEntry,
        TextPair,
    };

    #[test]
    fn formats_modes() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> git status --all -a".to_string(),
            input_without_prompt: "git status --all -a".to_string(),
            output: "clean".to_string(),
            command: "git status --all -a".to_string(),
        };
        assert_eq!(
            format_block(&block, CopyMode::Both, false, None),
            "git status --all -a\nclean"
        );
        assert_eq!(
            format_block(&block, CopyMode::Both, true, None),
            "PS C:\\repo> git status --all -a\nclean"
        );
        assert_eq!(
            format_block(&block, CopyMode::InputOnly, false, None),
            "git status --all -a"
        );
        assert_eq!(
            format_block(&block, CopyMode::InputOnly, true, None),
            "PS C:\\repo> git status --all -a"
        );
        assert_eq!(
            format_block(&block, CopyMode::OutputOnly, false, None),
            "clean"
        );
        assert_eq!(
            format_block(&block, CopyMode::CommandOnly, false, None),
            "git status --all -a"
        );
    }

    #[test]
    fn rewrites_prompt_in_copied_input() {
        let block = CommandBlock {
            input_with_prompt: "PS C:\\repo> cargo test".to_string(),
            input_without_prompt: "cargo test".to_string(),
            output: "ok".to_string(),
            command: "cargo test".to_string(),
        };

        assert_eq!(
            format_block(&block, CopyMode::InputOnly, true, Some(":")),
            ": cargo test"
        );
        assert_eq!(
            format_block(&block, CopyMode::Both, true, Some(">>>")),
            ">>> cargo test\nok"
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
        let filtered = filter_lines_by_regex(
            &TextPair {
                plain: "a\nwarn: b\nc".to_string(),
                ansi: "a\nwarn: b\nc".to_string(),
            },
            "warn",
        )
        .unwrap();
        assert_eq!(filtered.plain, "warn: b");
    }

    #[test]
    fn filters_ansi_by_plain_regex_matches() {
        let filtered = filter_lines_by_regex(
            &TextPair {
                plain: "a\nwarn: b\nc".to_string(),
                ansi: "a\n\x1b[31mwarn: b\x1b[0m\nc".to_string(),
            },
            "warn",
        )
        .unwrap();
        assert_eq!(filtered.ansi, "\x1b[31mwarn: b\x1b[0m");
    }

    #[test]
    fn filters_by_line_spec_with_colon_ranges() {
        let filtered = filter_lines_by_spec(
            &TextPair {
                plain: "a\nb\nc\nd".to_string(),
                ansi: "a\nb\nc\nd".to_string(),
            },
            "2,4:3",
        )
        .unwrap();
        assert_eq!(filtered.plain, "b\nc\nd");
    }

    #[test]
    fn rejects_dash_ranges_for_lines() {
        assert!(filter_lines_by_spec(
            &TextPair {
                plain: "a\nb\nc".to_string(),
                ansi: "a\nb\nc".to_string(),
            },
            "1-2"
        )
        .is_err());
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

fn run_picker(mut entries: Vec<PickEntry>, total: usize, title: &str) -> Result<CommandSelection> {
    let mut terminal = init_tui()?;
    let mut state = ListState::default();
    state.select(Some(0));
    let mut range_anchor = None;
    let mut show_preview = false;

    loop {
        terminal.draw(|frame| {
            render_picker(
                frame,
                &entries,
                &state,
                total,
                range_anchor,
                show_preview,
                title,
            )
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
    title: &str,
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
    .block(Block::default().borders(Borders::ALL).title(title));
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

fn build_text_preview(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("## "))
        .unwrap_or("<empty>")
        .chars()
        .take(80)
        .collect()
}

fn build_text_preview_lines(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(PICK_PREVIEW_LINES)
        .collect();
    if text.lines().count() > PICK_PREVIEW_LINES {
        lines.push("...");
    }
    lines.join("\n")
}
