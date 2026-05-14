use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, ListItem, ListState, Paragraph};
use regex::Regex;
use std::time::SystemTime;

use crate::commands::command_block_selector::CommandSelection;
use crate::tui::pane::{
    active_item_style, panel_block, render_list_panel, selected_item_style, Panel,
};
use crate::tui::workspace_search::{workspace_search_regex_for_query, WorkspaceSearchScope};
use sivtr_core::ai::AgentProvider;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceSource {
    Terminal,
    Agent(AgentProvider),
}

impl WorkspaceSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Agent(provider) => match provider.command_name() {
                "claude" => "claude",
                "codex" => "codex",
                _ => provider.command_name(),
            },
        }
    }

    pub(crate) fn is_agent(self) -> bool {
        matches!(self, Self::Agent(_))
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal)
    }
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
pub(crate) struct WorkspaceSession {
    pub(crate) source: WorkspaceSource,
    pub(crate) modified: SystemTime,
    pub(crate) title: String,
    pub(crate) units: Vec<TextPair>,
    pub(crate) dialogue_titles: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceDialogue {
    pub(crate) source: WorkspaceSource,
    pub(crate) title: String,
    pub(crate) unit: TextPair,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceFocus {
    Source,
    Sessions,
    Dialogues,
    Content,
}

impl WorkspaceFocus {
    pub(crate) const ORDER: [Self; 4] =
        [Self::Source, Self::Sessions, Self::Dialogues, Self::Content];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Source => "0",
            Self::Sessions => "1",
            Self::Dialogues => "2",
            Self::Content => "3",
        }
    }

    pub(crate) fn from_number_key(key: char, dialogue_count: usize) -> Option<Self> {
        let idx = key.to_digit(10)? as usize;
        Self::ORDER
            .get(idx)
            .copied()
            .filter(|focus| focus.is_available(dialogue_count))
    }

    pub(crate) fn previous(self, dialogue_count: usize) -> Option<Self> {
        let idx = self.order_index()?;
        Self::ORDER[..idx]
            .iter()
            .rev()
            .copied()
            .find(|focus| focus.is_available(dialogue_count))
    }

    pub(crate) fn next(self, dialogue_count: usize) -> Option<Self> {
        let idx = self.order_index()?;
        Self::ORDER[idx.saturating_add(1)..]
            .iter()
            .copied()
            .find(|focus| focus.is_available(dialogue_count))
    }

    fn is_available(self, dialogue_count: usize) -> bool {
        dialogue_count > 0 || !matches!(self, Self::Dialogues | Self::Content)
    }

    fn order_index(self) -> Option<usize> {
        Self::ORDER.iter().position(|focus| *focus == self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceHelpAction {
    FocusSource,
    FocusSessions,
    FocusDialogues,
    FocusContent,
    MoveUp,
    MoveDown,
    PreviousPane,
    NextPane,
    ToggleSelection,
    SelectAllSources,
    SelectAgentSources,
    SelectTerminalSource,
    RangeSelect,
    ToggleAllDialogues,
    OpenVim,
    ScrollDown,
    ScrollUp,
    Copy,
    ToggleFullscreen,
    CloseHelp,
    OpenSearch,
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkspaceHelpEntry {
    pub(crate) key: &'static str,
    pub(crate) description: &'static str,
    pub(crate) action: WorkspaceHelpAction,
}

pub(crate) struct WorkspaceView<'a> {
    pub(crate) sources: &'a [WorkspaceSource],
    pub(crate) selected_sources: &'a [bool],
    pub(crate) source_state: &'a ListState,
    pub(crate) sessions: &'a [WorkspaceSession],
    pub(crate) selected_sessions: &'a [bool],
    pub(crate) session_state: &'a ListState,
    pub(crate) dialogues: &'a [WorkspaceDialogue],
    pub(crate) dialogue_state: &'a ListState,
    pub(crate) selected_dialogues: &'a [bool],
    pub(crate) range_anchor: Option<usize>,
    pub(crate) focus: WorkspaceFocus,
    pub(crate) content_scroll: usize,
    pub(crate) show_help: bool,
    pub(crate) help_state: &'a ListState,
    pub(crate) search: Option<WorkspaceSearchView<'a>>,
    pub(crate) fullscreen: Option<WorkspaceFocus>,
}

pub(crate) struct WorkspaceSearchView<'a> {
    pub(crate) query: &'a str,
    pub(crate) scope: WorkspaceSearchScope,
    pub(crate) result_count: usize,
    pub(crate) current_match: Option<usize>,
    pub(crate) match_count: usize,
    pub(crate) input_open: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkspaceLayout {
    pub(crate) source: Rect,
    pub(crate) sessions: Rect,
    pub(crate) dialogues: Rect,
    pub(crate) content: Rect,
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

pub(crate) fn workspace_layout(
    area: Rect,
    focus: WorkspaceFocus,
    fullscreen: Option<WorkspaceFocus>,
) -> WorkspaceLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    if let Some(fullscreen) = fullscreen {
        return match fullscreen {
            WorkspaceFocus::Source => WorkspaceLayout {
                source: chunks[0],
                sessions: Rect::default(),
                dialogues: Rect::default(),
                content: Rect::default(),
            },
            WorkspaceFocus::Sessions => WorkspaceLayout {
                source: Rect::default(),
                sessions: chunks[0],
                dialogues: Rect::default(),
                content: Rect::default(),
            },
            WorkspaceFocus::Dialogues => WorkspaceLayout {
                source: Rect::default(),
                sessions: Rect::default(),
                dialogues: chunks[0],
                content: Rect::default(),
            },
            WorkspaceFocus::Content => WorkspaceLayout {
                source: Rect::default(),
                sessions: Rect::default(),
                dialogues: Rect::default(),
                content: chunks[0],
            },
        };
    }

    let constraints = match focus {
        WorkspaceFocus::Source | WorkspaceFocus::Sessions => [
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ],
        WorkspaceFocus::Dialogues => [
            Constraint::Percentage(25),
            Constraint::Percentage(50),
            Constraint::Percentage(25),
        ],
        WorkspaceFocus::Content => [
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(50),
        ],
    };
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(chunks[0]);
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(main_chunks[0]);

    WorkspaceLayout {
        source: left_chunks[0],
        sessions: left_chunks[1],
        dialogues: main_chunks[1],
        content: main_chunks[2],
    }
}

pub(crate) fn workspace_hit_test(
    layout: WorkspaceLayout,
    column: u16,
    row: u16,
) -> Option<WorkspaceFocus> {
    if rect_contains(layout.source, column, row) {
        Some(WorkspaceFocus::Source)
    } else if rect_contains(layout.sessions, column, row) {
        Some(WorkspaceFocus::Sessions)
    } else if rect_contains(layout.dialogues, column, row) {
        Some(WorkspaceFocus::Dialogues)
    } else if rect_contains(layout.content, column, row) {
        Some(WorkspaceFocus::Content)
    } else {
        None
    }
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
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

pub(crate) fn workspace_help_entries() -> &'static [WorkspaceHelpEntry] {
    &[
        WorkspaceHelpEntry {
            key: "0",
            description: "focus Source pane",
            action: WorkspaceHelpAction::FocusSource,
        },
        WorkspaceHelpEntry {
            key: "1",
            description: "focus Sessions pane",
            action: WorkspaceHelpAction::FocusSessions,
        },
        WorkspaceHelpEntry {
            key: "2",
            description: "focus Dialogues pane",
            action: WorkspaceHelpAction::FocusDialogues,
        },
        WorkspaceHelpEntry {
            key: "3",
            description: "focus Content pane",
            action: WorkspaceHelpAction::FocusContent,
        },
        WorkspaceHelpEntry {
            key: "j / Down",
            description: "move down in current pane",
            action: WorkspaceHelpAction::MoveDown,
        },
        WorkspaceHelpEntry {
            key: "k / Up",
            description: "move up in current pane",
            action: WorkspaceHelpAction::MoveUp,
        },
        WorkspaceHelpEntry {
            key: "h / Left",
            description: "focus previous pane",
            action: WorkspaceHelpAction::PreviousPane,
        },
        WorkspaceHelpEntry {
            key: "l / Right",
            description: "focus next pane",
            action: WorkspaceHelpAction::NextPane,
        },
        WorkspaceHelpEntry {
            key: "Space",
            description: "toggle current source/session/dialogue",
            action: WorkspaceHelpAction::ToggleSelection,
        },
        WorkspaceHelpEntry {
            key: "a (Source)",
            description: "select all sources",
            action: WorkspaceHelpAction::SelectAllSources,
        },
        WorkspaceHelpEntry {
            key: "g (Source)",
            description: "select agent sources",
            action: WorkspaceHelpAction::SelectAgentSources,
        },
        WorkspaceHelpEntry {
            key: "t (Source)",
            description: "select terminal source",
            action: WorkspaceHelpAction::SelectTerminalSource,
        },
        WorkspaceHelpEntry {
            key: "v",
            description: "range select dialogues",
            action: WorkspaceHelpAction::RangeSelect,
        },
        WorkspaceHelpEntry {
            key: "a",
            description: "toggle all dialogues",
            action: WorkspaceHelpAction::ToggleAllDialogues,
        },
        WorkspaceHelpEntry {
            key: "t",
            description: "open current content in Vim",
            action: WorkspaceHelpAction::OpenVim,
        },
        WorkspaceHelpEntry {
            key: "Ctrl-d",
            description: "scroll Content down",
            action: WorkspaceHelpAction::ScrollDown,
        },
        WorkspaceHelpEntry {
            key: "Ctrl-u",
            description: "scroll Content up",
            action: WorkspaceHelpAction::ScrollUp,
        },
        WorkspaceHelpEntry {
            key: "Enter",
            description: "enter pane or copy selection",
            action: WorkspaceHelpAction::Copy,
        },
        WorkspaceHelpEntry {
            key: "z",
            description: "toggle current pane fullscreen",
            action: WorkspaceHelpAction::ToggleFullscreen,
        },
        WorkspaceHelpEntry {
            key: "?",
            description: "close Help",
            action: WorkspaceHelpAction::CloseHelp,
        },
        WorkspaceHelpEntry {
            key: "/",
            description: "search all sessions",
            action: WorkspaceHelpAction::OpenSearch,
        },
        WorkspaceHelpEntry {
            key: "q",
            description: "cancel picker",
            action: WorkspaceHelpAction::Cancel,
        },
    ]
}

pub(crate) fn render_workspace(frame: &mut Frame, view: WorkspaceView<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let layout = workspace_layout(area, view.focus, view.fullscreen);

    let dialogue_idx =
        selected_index(view.dialogue_state).min(view.dialogues.len().saturating_sub(1));
    let search_regex = view
        .search
        .as_ref()
        .and_then(|search| workspace_search_regex_for_query(search.query));

    render_source_list(
        frame,
        layout.source,
        view.sources,
        view.selected_sources,
        view.source_state,
        view.focus == WorkspaceFocus::Source,
    );
    render_session_list(
        frame,
        layout.sessions,
        view.sessions,
        view.selected_sources,
        view.selected_sessions,
        view.session_state,
        view.search.as_ref(),
        search_regex.as_ref(),
        view.focus == WorkspaceFocus::Sessions,
    );
    render_dialogue_list(
        frame,
        layout.dialogues,
        view.dialogues,
        view.dialogue_state,
        view.selected_sessions,
        view.selected_dialogues,
        view.range_anchor,
        view.search.as_ref(),
        search_regex.as_ref(),
        view.focus == WorkspaceFocus::Dialogues,
    );

    render_content_panel(
        frame,
        layout.content,
        Panel::new(
            WorkspaceFocus::Content.key(),
            selected_parent_title("Content", view.selected_dialogues, "dialogue", "dialogues"),
            view.focus == WorkspaceFocus::Content,
        ),
        content_preview_text(view.dialogues, view.selected_dialogues, dialogue_idx),
        view.content_scroll,
        view.search.as_ref(),
        search_regex.as_ref(),
    );

    render_footer(
        frame,
        chunks[1],
        view.focus,
        view.show_help,
        view.search.as_ref(),
        view.fullscreen,
    );

    if let Some(search) = view.search.filter(|search| search.input_open) {
        render_search_box(frame, centered_rect(chunks[0], 60, 12), search);
    } else if view.show_help {
        render_help_panel(frame, chunks[0], view.help_state);
    }
}

fn render_footer(
    frame: &mut Frame,
    area: Rect,
    focus: WorkspaceFocus,
    show_help: bool,
    search: Option<&WorkspaceSearchView<'_>>,
    fullscreen: Option<WorkspaceFocus>,
) {
    let controls = if search.is_some() {
        let suffix = search.and_then(search_position_label).unwrap_or_default();
        if search.map(|search| search.input_open).unwrap_or(false) {
            return frame.render_widget(
                Paragraph::new(format!(
                    "type search  > session  # dialogue  Enter accept  Esc clear  {suffix}"
                )),
                area,
            );
        }
        return frame.render_widget(
            Paragraph::new(format!(
                "n next  N previous  Esc clear search  / edit  {suffix}"
            )),
            area,
        );
    } else if show_help {
        "j/k move  Enter execute  Esc/? close help  q cancel"
    } else {
        match focus {
            WorkspaceFocus::Source => "j/k move  Space toggle  a all  g agents  t terminal  Enter sessions  z fullscreen  / search  q/Esc cancel  ? help",
            WorkspaceFocus::Sessions => {
                "j/k move  Space toggle  0 source  l/Right/Enter dialogues  t vim  z fullscreen  / search  q/Esc cancel  ? help"
            }
            WorkspaceFocus::Dialogues => {
                "j/k move  Space toggle  v range  a all  l/Right content  t vim  Enter copy  z fullscreen  / search  h/Esc back  ? help"
            }
            WorkspaceFocus::Content => {
                "j/k scroll  Ctrl-d/PageDown down  Ctrl-u/PageUp up  t vim  Enter copy  z fullscreen  / search  h/Esc back  ? help"
            }
        }
    };
    let suffix = if fullscreen.is_some() {
        "  [fullscreen]"
    } else {
        ""
    };
    let footer = Paragraph::new(format!("{controls}{suffix}"));
    frame.render_widget(footer, area);
}

fn search_position_label(search: &WorkspaceSearchView<'_>) -> Option<String> {
    let current = search.current_match?;
    Some(format!("[{}/{}]", current + 1, search.match_count))
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn render_search_box(frame: &mut Frame, area: Rect, search: WorkspaceSearchView<'_>) {
    frame.render_widget(Clear, area);
    let result_label = if search.query.trim().is_empty() {
        "ready".to_string()
    } else if let Some(position) = search_position_label(&search) {
        position
    } else if search.result_count == 1 {
        "1 result".to_string()
    } else {
        format!("{} results", search.result_count)
    };
    let title = if search.scope == WorkspaceSearchScope::Content {
        format!("Search  ({result_label})")
    } else {
        format!("Search {}  ({})", search.scope.label(), result_label)
    };
    let paragraph =
        Paragraph::new(search.query.to_string()).block(panel_block(&Panel::new("", title, true)));
    frame.render_widget(paragraph, area);
}

fn render_help_panel(frame: &mut Frame, area: Rect, state: &ListState) {
    frame.render_widget(Clear, area);
    let items = workspace_help_entries()
        .iter()
        .map(|entry| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<12}", entry.key),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(entry.description),
            ]))
        })
        .collect::<Vec<_>>();
    render_list_panel(frame, area, Panel::new("?", "Help", true), items, state);
}

fn render_source_list(
    frame: &mut Frame,
    area: Rect,
    sources: &[WorkspaceSource],
    selected_sources: &[bool],
    state: &ListState,
    active: bool,
) {
    let panel = Panel::new(WorkspaceFocus::Source.key(), "Source", active);
    let current = selected_index(state).min(sources.len().saturating_sub(1));
    let mut spans = Vec::new();
    for (idx, source) in sources.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw("  "));
        }
        let text = {
            let marker = if selected_sources.get(idx).copied().unwrap_or(false) {
                "[x]"
            } else {
                "[ ]"
            };
            format!("{marker} {}", source.label())
        };
        let style = if idx == current && active {
            active_item_style()
        } else {
            Style::default()
        };
        spans.push(Span::styled(text, style));
    }
    if spans.is_empty() {
        spans.push(Span::raw("<empty>"));
    }
    let paragraph = Paragraph::new(Line::from(spans)).block(panel_block(&panel));
    frame.render_widget(paragraph, area);
}

fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    choices: &[WorkspaceSession],
    selected_sources: &[bool],
    selected_sessions: &[bool],
    state: &ListState,
    search: Option<&WorkspaceSearchView<'_>>,
    search_regex: Option<&Regex>,
    active: bool,
) {
    let cursor_idx = selected_index(state);
    let has_selection = selected_sessions.iter().any(|selected| *selected);
    let mut items: Vec<ListItem> = choices
        .iter()
        .enumerate()
        .map(|(idx, choice)| {
            let selected = selected_sessions.get(idx).copied().unwrap_or(false);
            let marker = if active {
                if selected {
                    "[x] "
                } else {
                    "[ ] "
                }
            } else {
                ""
            };
            let line = format!("{marker}[{:<8}] {}", choice.source.label(), choice.title);
            let highlight = search
                .filter(|search| search.scope == WorkspaceSearchScope::Session)
                .and(search_regex);
            let base_style = if selected {
                selected_item_style()
            } else if !has_selection && idx == cursor_idx {
                active_item_style()
            } else {
                Style::default()
            };
            if selected || (!has_selection && idx == cursor_idx) {
                ListItem::new(Line::from(highlight_spans(&line, highlight, base_style)))
            } else {
                ListItem::new(Line::from(highlight_spans(
                    &line,
                    highlight,
                    Style::default(),
                )))
            }
        })
        .collect();
    if items.is_empty() {
        items.push(ListItem::new("<empty>"));
    }
    render_list_panel(
        frame,
        area,
        Panel::new(
            WorkspaceFocus::Sessions.key(),
            selected_parent_title("Sessions", selected_sources, "source", "sources"),
            active,
        ),
        items,
        state,
    );
}

fn render_dialogue_list(
    frame: &mut Frame,
    area: Rect,
    dialogues: &[WorkspaceDialogue],
    state: &ListState,
    selected_sessions: &[bool],
    selected_dialogues: &[bool],
    range_anchor: Option<usize>,
    search: Option<&WorkspaceSearchView<'_>>,
    search_regex: Option<&Regex>,
    active: bool,
) {
    let highlighted_idx = selected_index(state);
    let has_selection = selected_dialogues.iter().any(|selected| *selected);
    let mut items: Vec<ListItem> = dialogues
        .iter()
        .enumerate()
        .map(|(idx, dialogue)| {
            let in_range = range_anchor
                .map(|anchor| {
                    idx >= anchor.min(highlighted_idx) && idx <= anchor.max(highlighted_idx)
                })
                .unwrap_or(false);
            let selected = selected_dialogues.get(idx).copied().unwrap_or(false);
            let marker = if active {
                if selected {
                    "[x] "
                } else {
                    "[ ] "
                }
            } else {
                ""
            };
            let line = format!("{marker}{}", dialogue.title);
            let highlight = search
                .filter(|search| search.scope == WorkspaceSearchScope::Dialogue)
                .and(search_regex);
            if in_range {
                ListItem::new(Line::from(Span::styled(
                    line,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )))
            } else if selected {
                ListItem::new(Line::from(highlight_spans(
                    &line,
                    highlight,
                    selected_item_style(),
                )))
            } else if !has_selection && idx == highlighted_idx {
                ListItem::new(Line::from(highlight_spans(
                    &line,
                    highlight,
                    active_item_style(),
                )))
            } else {
                ListItem::new(Line::from(highlight_spans(
                    &line,
                    highlight,
                    Style::default(),
                )))
            }
        })
        .collect();

    if items.is_empty() {
        items.push(ListItem::new("<empty>"));
    }

    render_list_panel(
        frame,
        area,
        Panel::new(
            WorkspaceFocus::Dialogues.key(),
            selected_parent_title("Dialogues", selected_sessions, "session", "sessions"),
            active,
        ),
        items,
        state,
    );
}

fn render_content_panel(
    frame: &mut Frame,
    area: Rect,
    panel: Panel,
    text: String,
    scroll: usize,
    search: Option<&WorkspaceSearchView<'_>>,
    search_regex: Option<&Regex>,
) {
    let content_search = search
        .filter(|search| search.scope == WorkspaceSearchScope::Content)
        .and(search_regex);
    let paragraph = Paragraph::new(highlighted_content_text(&text, content_search))
        .scroll((scroll as u16, 0))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(panel_block(&panel));
    frame.render_widget(paragraph, area);
}

fn selected_parent_title(
    title: &str,
    selected_parent_items: &[bool],
    singular: &str,
    plural: &str,
) -> String {
    let count = selected_parent_items
        .iter()
        .filter(|selected| **selected)
        .count();
    if count == 0 {
        title.to_string()
    } else if count == 1 {
        format!("{title}: 1 {singular} selected")
    } else {
        format!("{title}: {count} {plural} selected")
    }
}

fn content_preview_text(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
) -> String {
    if dialogues.is_empty() {
        return "<empty>".to_string();
    }

    let selected = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();

    if selected.is_empty() {
        let text = dialogues
            .get(highlighted_idx)
            .map(|dialogue| dialogue.unit.plain.as_str())
            .unwrap_or("<empty>");
        return format_content_with_line_numbers(text);
    }

    let text = selected
        .into_iter()
        .filter_map(|dialogue_idx| dialogues.get(dialogue_idx))
        .map(|dialogue| dialogue.unit.plain.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    format_content_with_line_numbers(&text)
}

fn highlighted_content_text(text: &str, search_regex: Option<&Regex>) -> Text<'static> {
    let lines = text
        .lines()
        .map(|line| {
            if search_regex.is_some() {
                if let Some((prefix, rest)) = line.split_once(" | ") {
                    let mut spans = vec![Span::raw(format!("{prefix} | "))];
                    spans.extend(highlight_spans(rest, search_regex, Style::default()));
                    Line::from(spans)
                } else {
                    Line::from(highlight_spans(line, search_regex, Style::default()))
                }
            } else if let Some((prefix, rest)) = line.split_once("## User") {
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
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

fn highlight_spans(text: &str, regex: Option<&Regex>, base_style: Style) -> Vec<Span<'static>> {
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

#[cfg(test)]
mod tests {
    use super::{can_open_dialogue_vim, format_content_with_line_numbers, WorkspaceFocus};

    #[test]
    fn can_open_dialogue_vim_accepts_sessions_when_dialogues_exist() {
        assert!(can_open_dialogue_vim(WorkspaceFocus::Sessions, 1));
        assert!(can_open_dialogue_vim(WorkspaceFocus::Dialogues, 1));
        assert!(can_open_dialogue_vim(WorkspaceFocus::Content, 1));
        assert!(!can_open_dialogue_vim(WorkspaceFocus::Sessions, 0));
    }

    #[test]
    fn format_content_with_line_numbers_adds_aligned_prefixes() {
        let text = (1..=12)
            .map(|idx| format!("line {idx}"))
            .collect::<Vec<_>>()
            .join("\n");

        let formatted = format_content_with_line_numbers(&text);
        let lines: Vec<&str> = formatted.lines().collect();

        assert_eq!(lines.len(), 12);
        assert_eq!(lines[0], " 1 | line 1");
        assert_eq!(lines[8], " 9 | line 9");
        assert_eq!(lines[9], "10 | line 10");
        assert_eq!(lines[11], "12 | line 12");
    }

    #[test]
    fn format_content_with_line_numbers_preserves_blank_lines() {
        let formatted = format_content_with_line_numbers("alpha\n\nomega");
        assert_eq!(formatted, "1 | alpha\n2 | \n3 | omega");
    }
}
