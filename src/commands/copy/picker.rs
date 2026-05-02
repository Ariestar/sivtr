use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::commands::command_block_selector::CommandSelection;
use crate::tui::terminal::{init as init_tui, restore as restore_tui, Tui};

use super::{open_picker_vim, PickerTuiTarget, PICK_CANCELLED_MESSAGE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerSubmitMode {
    Selected,
    Highlighted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PickerOutcome {
    Submitted(CommandSelection),
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerFocus {
    List,
    Preview,
}

#[derive(Debug, Clone)]
pub(super) struct PickEntry {
    pub(super) recent: usize,
    pub(super) preview: String,
    pub(super) output_preview: String,
    pub(super) full_preview: String,
    pub(super) selected: bool,
}

pub(super) fn run_picker(
    entries: Vec<PickEntry>,
    total: usize,
    title: &str,
    tui_target: PickerTuiTarget,
) -> Result<CommandSelection> {
    let mut terminal = init_tui()?;
    let outcome = run_picker_on_terminal(
        &mut terminal,
        entries,
        total,
        title,
        tui_target,
        PickerSubmitMode::Selected,
        false,
    );
    restore_tui(&mut terminal)?;
    match outcome? {
        PickerOutcome::Submitted(selection) => Ok(selection),
        PickerOutcome::Back => anyhow::bail!(PICK_CANCELLED_MESSAGE),
    }
}

pub(super) fn run_single_picker(
    entries: Vec<PickEntry>,
    total: usize,
    title: &str,
    tui_target: PickerTuiTarget,
) -> Result<usize> {
    let mut terminal = init_tui()?;
    let outcome = run_single_picker_on_terminal(&mut terminal, entries, total, title, tui_target);
    restore_tui(&mut terminal)?;
    match outcome? {
        Some(selected) => Ok(selected),
        None => anyhow::bail!(PICK_CANCELLED_MESSAGE),
    }
}

pub(super) fn run_picker_with_back_on_terminal(
    terminal: &mut Tui,
    entries: Vec<PickEntry>,
    total: usize,
    title: &str,
    tui_target: PickerTuiTarget,
) -> Result<PickerOutcome> {
    run_picker_on_terminal(
        terminal,
        entries,
        total,
        title,
        tui_target,
        PickerSubmitMode::Selected,
        true,
    )
}

pub(super) fn run_single_picker_on_terminal(
    terminal: &mut Tui,
    entries: Vec<PickEntry>,
    total: usize,
    title: &str,
    tui_target: PickerTuiTarget,
) -> Result<Option<usize>> {
    let outcome = run_picker_on_terminal(
        terminal,
        entries,
        total,
        title,
        tui_target,
        PickerSubmitMode::Highlighted,
        false,
    )?;
    match outcome {
        PickerOutcome::Back => Ok(None),
        PickerOutcome::Submitted(selection) => match selection {
            CommandSelection::RecentExplicit(mut selected) if selected.len() == 1 => {
                Ok(Some(selected.remove(0)))
            }
            _ => anyhow::bail!("No item selected"),
        },
    }
}

fn run_picker_on_terminal(
    terminal: &mut Tui,
    mut entries: Vec<PickEntry>,
    total: usize,
    title: &str,
    tui_target: PickerTuiTarget,
    submit_mode: PickerSubmitMode,
    allow_back: bool,
) -> Result<PickerOutcome> {
    let mut state = ListState::default();
    state.select(Some(0));
    let mut range_anchor = None;
    let mut show_preview = false;
    let mut focus = PickerFocus::List;
    let mut preview_scroll = 0usize;
    let mut preview_search: Option<String> = None;
    let mut preview_search_input: Option<String> = None;
    let mut preview_search_match = 0usize;

    loop {
        let preview_line_count = current_preview_text(&entries, &state).lines().count();
        preview_scroll = preview_scroll.min(preview_line_count.saturating_sub(1));

        terminal.draw(|frame| {
            render_picker(
                frame,
                PickerRenderContext {
                    entries: &entries,
                    state: &state,
                    total,
                    range_anchor,
                    show_preview,
                    focus,
                    preview_scroll,
                    preview_search: preview_search.as_deref(),
                    preview_search_input: preview_search_input.as_deref(),
                    preview_search_match,
                    title,
                    submit_mode,
                    tui_available: tui_target.is_available(),
                    allow_back,
                },
            )
        })?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if let Some(input) = &mut preview_search_input {
                match key.code {
                    KeyCode::Esc => {
                        preview_search_input = None;
                    }
                    KeyCode::Enter => {
                        preview_search = (!input.is_empty()).then(|| input.clone());
                        preview_search_input = None;
                        preview_search_match = 0;
                        if let Some(pattern) = preview_search.as_deref() {
                            if let Some(offset) = next_preview_match(
                                current_preview_text(&entries, &state),
                                pattern,
                                0,
                                1,
                            ) {
                                preview_scroll = offset;
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char(c) => {
                        input.push(c);
                    }
                    _ => {}
                }
                continue;
            }

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    if allow_back {
                        return Ok(PickerOutcome::Back);
                    }
                    anyhow::bail!(PICK_CANCELLED_MESSAGE);
                }
                KeyCode::Tab => {
                    if show_preview {
                        focus = match focus {
                            PickerFocus::List => PickerFocus::Preview,
                            PickerFocus::Preview => PickerFocus::List,
                        };
                    }
                }
                KeyCode::Char('/') if show_preview && focus == PickerFocus::Preview => {
                    preview_search_input = Some(String::new());
                }
                KeyCode::Char('n') if show_preview && focus == PickerFocus::Preview => {
                    if let Some(pattern) = preview_search.as_deref() {
                        if let Some(offset) = next_preview_match(
                            current_preview_text(&entries, &state),
                            pattern,
                            preview_scroll.saturating_add(1),
                            1,
                        ) {
                            preview_scroll = offset;
                            preview_search_match = preview_search_match.saturating_add(1);
                        }
                    }
                }
                KeyCode::Char('N') if show_preview && focus == PickerFocus::Preview => {
                    if let Some(pattern) = preview_search.as_deref() {
                        if let Some(offset) = next_preview_match(
                            current_preview_text(&entries, &state),
                            pattern,
                            preview_scroll.saturating_sub(1),
                            -1,
                        ) {
                            preview_scroll = offset;
                            preview_search_match = preview_search_match.saturating_sub(1);
                        }
                    }
                }
                KeyCode::Down | KeyCode::Char('j')
                    if show_preview && focus == PickerFocus::Preview =>
                {
                    preview_scroll = preview_scroll.saturating_add(1);
                }
                KeyCode::Up | KeyCode::Char('k')
                    if show_preview && focus == PickerFocus::Preview =>
                {
                    preview_scroll = preview_scroll.saturating_sub(1);
                }
                KeyCode::PageDown if show_preview && focus == PickerFocus::Preview => {
                    preview_scroll = preview_scroll.saturating_add(10);
                }
                KeyCode::Char('d')
                    if show_preview
                        && focus == PickerFocus::Preview
                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    preview_scroll = preview_scroll.saturating_add(10);
                }
                KeyCode::PageUp if show_preview && focus == PickerFocus::Preview => {
                    preview_scroll = preview_scroll.saturating_sub(10);
                }
                KeyCode::Char('u')
                    if show_preview
                        && focus == PickerFocus::Preview
                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    preview_scroll = preview_scroll.saturating_sub(10);
                }
                KeyCode::Char('g') if show_preview && focus == PickerFocus::Preview => {
                    preview_scroll = 0;
                }
                KeyCode::Char('G') if show_preview && focus == PickerFocus::Preview => {
                    preview_scroll = current_preview_text(&entries, &state)
                        .lines()
                        .count()
                        .saturating_sub(1);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let current = state.selected().unwrap_or(0);
                    state.select(Some(current.saturating_sub(1)));
                    preview_scroll = 0;
                    preview_search_match = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let current = state.selected().unwrap_or(0);
                    let next = (current + 1).min(entries.len().saturating_sub(1));
                    state.select(Some(next));
                    preview_scroll = 0;
                    preview_search_match = 0;
                }
                KeyCode::Char('v')
                    if focus == PickerFocus::List && submit_mode == PickerSubmitMode::Selected =>
                {
                    let current = state.selected().unwrap_or(0);
                    range_anchor = match range_anchor {
                        Some(anchor) if anchor == current => None,
                        _ => Some(current),
                    };
                }
                KeyCode::Char(' ')
                    if focus == PickerFocus::List && submit_mode == PickerSubmitMode::Selected =>
                {
                    if let Some(idx) = state.selected() {
                        if let Some(anchor) = range_anchor.take() {
                            apply_range_toggle(&mut entries, anchor, idx);
                        } else if let Some(entry) = entries.get_mut(idx) {
                            entry.selected = !entry.selected;
                        }
                    }
                }
                KeyCode::Char('a')
                    if focus == PickerFocus::List && submit_mode == PickerSubmitMode::Selected =>
                {
                    let select_all = entries.iter().any(|entry| !entry.selected);
                    for entry in &mut entries {
                        entry.selected = select_all;
                    }
                    range_anchor = None;
                }
                KeyCode::Char('p') => {
                    show_preview = !show_preview;
                    preview_scroll = 0;
                    if !show_preview {
                        focus = PickerFocus::List;
                    }
                }
                KeyCode::Char('t') if tui_target.is_available() => {
                    restore_tui(terminal)?;
                    open_picker_vim(&tui_target)?;
                    *terminal = init_tui()?;
                }
                KeyCode::Enter => {
                    return Ok(PickerOutcome::Submitted(submit_selection(
                        &entries,
                        &state,
                        submit_mode,
                    )?));
                }
                _ => {}
            }
        }
    }
}

struct PickerRenderContext<'a> {
    entries: &'a [PickEntry],
    state: &'a ListState,
    total: usize,
    range_anchor: Option<usize>,
    show_preview: bool,
    focus: PickerFocus,
    preview_scroll: usize,
    preview_search: Option<&'a str>,
    preview_search_input: Option<&'a str>,
    preview_search_match: usize,
    title: &'a str,
    submit_mode: PickerSubmitMode,
    tui_available: bool,
    allow_back: bool,
}

fn render_picker(frame: &mut Frame, context: PickerRenderContext<'_>) {
    let PickerRenderContext {
        entries,
        state,
        total,
        range_anchor,
        show_preview,
        focus,
        preview_scroll,
        preview_search,
        preview_search_input,
        preview_search_match,
        title,
        submit_mode,
        tui_available,
        allow_back,
    } = context;

    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    let anchor_hint = range_anchor
        .map(|anchor| format!("  v range@{}", anchor + 1))
        .unwrap_or_default();
    let focus_hint = if show_preview {
        match focus {
            PickerFocus::List => "  focus:list",
            PickerFocus::Preview => "  focus:preview",
        }
    } else {
        ""
    };
    let exit_hint = if allow_back {
        "q/Esc back"
    } else {
        "q/Esc cancel"
    };
    let controls = match submit_mode {
        PickerSubmitMode::Selected => {
            if tui_available {
                format!("Space toggle  v mark-range  p preview  Tab focus  / search  t tui  a toggle-all  Enter confirm  {exit_hint}")
            } else {
                format!("Space toggle  v mark-range  p preview  Tab focus  / search  a toggle-all  Enter confirm  {exit_hint}")
            }
        }
        PickerSubmitMode::Highlighted => {
            format!("p preview  Tab focus  / search  Enter choose  {exit_hint}")
        }
    };
    let title_widget = Paragraph::new(format!(
        "{controls}{}{}\nshowing last {} of {}",
        anchor_hint,
        focus_hint,
        entries.len(),
        total
    ))
    .block(
        Block::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .title(title),
    );
    frame.render_widget(title_widget, chunks[0]);

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
            let marker = if submit_mode == PickerSubmitMode::Selected {
                if entry.selected {
                    "[x] "
                } else {
                    "[ ] "
                }
            } else {
                ""
            };
            let is_in_pending_range = range_anchor
                .map(|anchor| range_bounds(anchor, state.selected().unwrap_or(0)))
                .map(|(start, end)| (start..=end).contains(&idx))
                .unwrap_or(false);
            let line = format!("{marker}{:>2}. {}", entry.recent, entry.preview);
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

    let list_title = if focus == PickerFocus::List {
        "Commands *"
    } else {
        "Commands"
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(list_title))
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");
    let mut local_state = state.clone();
    frame.render_stateful_widget(list, body_chunks[0], &mut local_state);

    if show_preview {
        let preview_text = current_preview_text(entries, state);
        let search_title = preview_search_input
            .map(|input| format!(" /{input}"))
            .or_else(|| {
                preview_search.map(|pattern| format!(" /{pattern} #{}", preview_search_match + 1))
            })
            .unwrap_or_default();
        let preview_title = if focus == PickerFocus::Preview {
            format!("Preview *{search_title}")
        } else {
            format!("Preview{search_title}")
        };
        let preview = Paragraph::new(preview_text)
            .scroll((preview_scroll as u16, 0))
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(preview_title));
        frame.render_widget(preview, body_chunks[1]);
    }
}

fn current_preview_text<'a>(entries: &'a [PickEntry], state: &ListState) -> &'a str {
    state
        .selected()
        .and_then(|idx| entries.get(idx))
        .map(|entry| {
            if entry.full_preview.trim().is_empty() {
                entry.output_preview.as_str()
            } else {
                entry.full_preview.as_str()
            }
        })
        .unwrap_or("<no output>")
}

fn next_preview_match(text: &str, pattern: &str, start: usize, direction: i32) -> Option<usize> {
    let pattern = pattern.to_lowercase();
    if pattern.is_empty() {
        return None;
    }

    let lines: Vec<String> = text.lines().map(str::to_lowercase).collect();
    if lines.is_empty() {
        return None;
    }

    if direction < 0 {
        let start = start.min(lines.len().saturating_sub(1));
        (0..=start)
            .rev()
            .find(|idx| lines[*idx].contains(&pattern))
            .or_else(|| {
                (start + 1..lines.len())
                    .rev()
                    .find(|idx| lines[*idx].contains(&pattern))
            })
    } else {
        let start = start.min(lines.len().saturating_sub(1));
        (start..lines.len())
            .find(|idx| lines[*idx].contains(&pattern))
            .or_else(|| (0..start).find(|idx| lines[*idx].contains(&pattern)))
    }
}

pub(super) fn selection_from_entries(entries: &[PickEntry]) -> Result<CommandSelection> {
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

fn submit_selection(
    entries: &[PickEntry],
    state: &ListState,
    submit_mode: PickerSubmitMode,
) -> Result<CommandSelection> {
    match submit_mode {
        PickerSubmitMode::Selected => selection_from_entries(entries),
        PickerSubmitMode::Highlighted => state
            .selected()
            .and_then(|idx| entries.get(idx))
            .map(|entry| CommandSelection::RecentExplicit(vec![entry.recent]))
            .ok_or_else(|| anyhow::anyhow!("No item selected")),
    }
}

pub(super) fn apply_range_toggle(entries: &mut [PickEntry], a: usize, b: usize) {
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

#[cfg(test)]
mod tests {
    use super::{next_preview_match, PickEntry};

    #[test]
    fn preview_search_wraps_forward() {
        let text = "alpha\nbeta\ngamma";
        assert_eq!(next_preview_match(text, "ALPHA", 2, 1), Some(0));
    }

    #[test]
    fn preview_search_wraps_backward() {
        let text = "alpha\nbeta\ngamma";
        assert_eq!(next_preview_match(text, "gamma", 0, -1), Some(2));
    }

    #[test]
    fn pick_entry_is_constructible_for_parent_tests() {
        let entry = PickEntry {
            recent: 1,
            preview: "cmd".to_string(),
            output_preview: "out".to_string(),
            full_preview: "full".to_string(),
            selected: false,
        };
        assert_eq!(entry.full_preview, "full");
    }
}
