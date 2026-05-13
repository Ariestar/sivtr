use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use std::time::SystemTime;

use crate::commands::command_block_selector::CommandSelection;
use sivtr_core::ai::AgentProvider;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceSource {
    Terminal,
    Agent(AgentProvider),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextPair {
    pub(crate) plain: String,
    pub(crate) ansi: String,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspacePickedContent {
    pub(crate) source: WorkspaceSource,
    pub(crate) units: Vec<TextPair>,
    pub(crate) selection: CommandSelection,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceGroup {
    pub(crate) source: WorkspaceSource,
    pub(crate) title: String,
    pub(crate) choices: Vec<WorkspaceSession>,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSession {
    pub(crate) source: WorkspaceSource,
    pub(crate) modified: SystemTime,
    pub(crate) title: String,
    pub(crate) units: Vec<TextPair>,
    pub(crate) dialogue_titles: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceFocus {
    Agents,
    Sessions,
    Dialogues,
    Content,
}

pub(crate) struct WorkspaceView<'a> {
    pub(crate) groups: &'a [WorkspaceGroup],
    pub(crate) agent_state: &'a ListState,
    pub(crate) session_state: &'a ListState,
    pub(crate) dialogue_state: &'a ListState,
    pub(crate) selected_dialogues: &'a [bool],
    pub(crate) focus: WorkspaceFocus,
    pub(crate) content_scroll: usize,
}

pub(crate) fn selected_index(state: &ListState) -> usize {
    state.selected().unwrap_or(0)
}

pub(crate) fn can_open_dialogue_vim(focus: WorkspaceFocus, dialogue_count: usize) -> bool {
    dialogue_count > 0
        && matches!(
            focus,
            WorkspaceFocus::Sessions | WorkspaceFocus::Dialogues | WorkspaceFocus::Content
        )
}

pub(crate) fn current_agent_dialogue_text(
    choice: &WorkspaceSession,
    dialogue_idx: usize,
) -> &str {
    let total = choice.units.len();
    if total == 0 {
        return "<empty>";
    }
    let unit_idx = total.saturating_sub(dialogue_idx + 1);
    choice
        .units
        .get(unit_idx)
        .map(|unit| unit.plain.as_str())
        .unwrap_or("<empty>")
}

pub(crate) fn format_content_with_line_numbers(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let line_count = lines.len().max(1);
    let width = line_count.to_string().len();

    if lines.is_empty() {
        return format!("{:>width$} | ", 1, width = width);
    }

    lines
        .iter()
        .enumerate()
        .map(|(idx, line)| format!("{:>width$} | {line}", idx + 1, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn agent_dialogue_selection(
    selected_dialogues: &[bool],
    highlighted_idx: usize,
) -> CommandSelection {
    let mut selected: Vec<usize> = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx + 1))
        .collect();

    if selected.is_empty() {
        selected.push(highlighted_idx + 1);
    }

    CommandSelection::RecentExplicit(selected)
}

pub(crate) fn render_workspace(title: &str, frame: &mut Frame, view: WorkspaceView<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(area);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(chunks[0]);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Percentage(46),
            Constraint::Percentage(54),
        ])
        .split(main_chunks[0]);

    let agent_idx = selected_index(view.agent_state).min(view.groups.len().saturating_sub(1));
    let choices = &view.groups[agent_idx].choices;
    let session_idx = selected_index(view.session_state).min(choices.len().saturating_sub(1));
    let dialogue_idx =
        selected_index(view.dialogue_state).min(choices[session_idx].dialogue_titles.len().saturating_sub(1));

    render_source_list(
        frame,
        left_chunks[0],
        view.groups,
        view.agent_state,
        view.focus == WorkspaceFocus::Agents,
    );
    render_session_list(
        frame,
        left_chunks[1],
        choices,
        view.session_state,
        view.focus == WorkspaceFocus::Sessions,
    );
    render_dialogue_list(
        frame,
        left_chunks[2],
        &choices[session_idx],
        view.dialogue_state,
        view.selected_dialogues,
        view.focus == WorkspaceFocus::Dialogues,
    );

    let content_title = if view.focus == WorkspaceFocus::Content {
        "Content *"
    } else {
        "Content"
    };
    let content = Paragraph::new(format_content_with_line_numbers(
        current_agent_dialogue_text(&choices[session_idx], dialogue_idx),
    ))
    .scroll((view.content_scroll as u16, 0))
    .wrap(ratatui::widgets::Wrap { trim: false })
    .block(Block::default().borders(Borders::ALL).title(content_title));
    frame.render_widget(content, main_chunks[1]);

    render_footer(title, frame, chunks[1], view);
}

fn render_footer(title: &str, frame: &mut Frame, area: Rect, view: WorkspaceView<'_>) {
    let controls = match view.focus {
        WorkspaceFocus::Agents => "j/k move  l/Right/Enter sessions  q/Esc cancel",
        WorkspaceFocus::Sessions => "j/k move  l/Right/Enter dialogues  t vim  q/Esc cancel",
        WorkspaceFocus::Dialogues => {
            "j/k move  Space toggle  a all  l/Right content  t vim  Enter copy  h/Esc back"
        }
        WorkspaceFocus::Content => {
            "j/k scroll  Ctrl-d/PageDown down  Ctrl-u/PageUp up  t vim  Enter copy  h/Esc back"
        }
    };
    let agent_idx = selected_index(view.agent_state).min(view.groups.len().saturating_sub(1));
    let choices = &view.groups[agent_idx].choices;
    let session_idx = selected_index(view.session_state).min(choices.len().saturating_sub(1));
    let selected_count = view
        .selected_dialogues
        .iter()
        .filter(|selected| **selected)
        .count();
    let status = format!(
        "{} | {} source(s), {} session(s), {} dialogue(s){}",
        title,
        view.groups.len(),
        choices.len(),
        choices[session_idx].dialogue_titles.len(),
        if selected_count == 0 {
            String::new()
        } else {
            format!(", {selected_count} selected")
        }
    );
    let footer = Paragraph::new(format!("{controls}\n{status}"))
        .block(Block::default().borders(Borders::TOP).title("Keys"));
    frame.render_widget(footer, area);
}

fn render_source_list(
    frame: &mut Frame,
    area: Rect,
    groups: &[WorkspaceGroup],
    state: &ListState,
    active: bool,
) {
    let items: Vec<ListItem> = groups
        .iter()
        .enumerate()
        .map(|(idx, group)| ListItem::new(format!("{:>2}. {}", idx + 1, group.title)))
        .collect();
    let title = if active { "Sources *" } else { "Sources" };
    render_list(frame, area, title, items, state, active);
}

fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    choices: &[WorkspaceSession],
    state: &ListState,
    active: bool,
) {
    let items: Vec<ListItem> = choices
        .iter()
        .enumerate()
        .map(|(idx, choice)| ListItem::new(format!("{:>2}. {}", idx + 1, choice.title)))
        .collect();
    let title = if active { "Sessions *" } else { "Sessions" };
    render_list(frame, area, title, items, state, active);
}

fn render_dialogue_list(
    frame: &mut Frame,
    area: Rect,
    choice: &WorkspaceSession,
    state: &ListState,
    selected_dialogues: &[bool],
    active: bool,
) {
    let mut items: Vec<ListItem> = choice
        .dialogue_titles
        .iter()
        .enumerate()
        .map(|(idx, title)| {
            let marker = if active {
                if selected_dialogues.get(idx).copied().unwrap_or(false) {
                    "[x] "
                } else {
                    "[ ] "
                }
            } else {
                ""
            };
            ListItem::new(format!("{marker}{:>2}. {title}", idx + 1))
        })
        .collect();

    if items.is_empty() {
        items.push(ListItem::new("<empty>"));
    }

    let title = if active { "Dialogues *" } else { "Dialogues" };
    render_list(frame, area, title, items, state, active);
}

fn render_list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: Vec<ListItem>,
    state: &ListState,
    active: bool,
) {
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(if active {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        })
        .highlight_symbol(if active { ">> " } else { "   " });
    let mut local_state = state.clone();
    frame.render_stateful_widget(list, area, &mut local_state);
}
