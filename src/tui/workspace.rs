use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use std::time::SystemTime;

use crate::commands::command_block_selector::CommandSelection;
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
    pub(crate) session_state: &'a ListState,
    pub(crate) dialogue_state: &'a ListState,
    pub(crate) selected_dialogues: &'a [bool],
    pub(crate) range_anchor: Option<usize>,
    pub(crate) focus: WorkspaceFocus,
    pub(crate) content_scroll: usize,
    pub(crate) show_help: bool,
    pub(crate) help_state: &'a ListState,
    pub(crate) fullscreen: Option<WorkspaceFocus>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkspaceLayout {
    pub(crate) source: Rect,
    pub(crate) sessions: Rect,
    pub(crate) dialogues: Rect,
    pub(crate) content: Rect,
}

#[derive(Clone, Copy)]
struct Panel {
    key: &'static str,
    name: &'static str,
    active: bool,
}

impl Panel {
    fn title(self) -> String {
        if self.active {
            format!("[{}] {} *", self.key, self.name)
        } else {
            format!("[{}] {}", self.key, self.name)
        }
    }
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

pub(crate) fn current_agent_dialogue_text(choice: &WorkspaceSession, dialogue_idx: usize) -> &str {
    choice
        .units
        .get(dialogue_idx)
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
            description: "toggle current source/dialogue",
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
            key: "q",
            description: "cancel picker",
            action: WorkspaceHelpAction::Cancel,
        },
    ]
}

pub(crate) fn agent_dialogue_selection(
    selected_dialogues: &[bool],
    highlighted_idx: usize,
) -> CommandSelection {
    let total = selected_dialogues.len();
    let mut selected: Vec<usize> = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(total - idx))
        .collect();

    if selected.is_empty() {
        selected.push(total - highlighted_idx);
    }

    CommandSelection::RecentExplicit(selected)
}

pub(crate) fn render_workspace(frame: &mut Frame, view: WorkspaceView<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let layout = workspace_layout(area, view.focus, view.fullscreen);

    let session = view
        .sessions
        .get(selected_index(view.session_state).min(view.sessions.len().saturating_sub(1)));
    let dialogue_idx = session
        .map(|session| {
            selected_index(view.dialogue_state).min(session.dialogue_titles.len().saturating_sub(1))
        })
        .unwrap_or(0);

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
        view.session_state,
        view.focus == WorkspaceFocus::Sessions,
    );
    render_dialogue_list(
        frame,
        layout.dialogues,
        session,
        view.dialogue_state,
        view.selected_dialogues,
        view.range_anchor,
        view.focus == WorkspaceFocus::Dialogues,
    );

    render_content_panel(
        frame,
        layout.content,
        Panel {
            key: WorkspaceFocus::Content.key(),
            name: "Content",
            active: view.focus == WorkspaceFocus::Content,
        },
        content_preview_text(session, view.selected_dialogues, dialogue_idx),
        view.content_scroll,
    );

    render_footer(
        frame,
        chunks[1],
        view.focus,
        view.show_help,
        view.fullscreen,
    );

    if view.show_help {
        render_help_panel(frame, chunks[0], view.help_state);
    }
}

fn render_footer(
    frame: &mut Frame,
    area: Rect,
    focus: WorkspaceFocus,
    show_help: bool,
    fullscreen: Option<WorkspaceFocus>,
) {
    let controls = if show_help {
        "j/k move  Enter execute  Esc/? close help  q cancel"
    } else {
        match focus {
            WorkspaceFocus::Source => "j/k move  Space toggle  a all  g agents  t terminal  Enter sessions  z fullscreen  q/Esc cancel  ? help",
            WorkspaceFocus::Sessions => {
                "j/k move  0 source  l/Right/Enter dialogues  t vim  z fullscreen  q/Esc cancel  ? help"
            }
            WorkspaceFocus::Dialogues => {
                "j/k move  Space toggle  v range  a all  l/Right content  t vim  Enter copy  z fullscreen  h/Esc back  ? help"
            }
            WorkspaceFocus::Content => {
                "j/k scroll  Ctrl-d/PageDown down  Ctrl-u/PageUp up  t vim  Enter copy  z fullscreen  h/Esc back  ? help"
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
    render_list_panel(
        frame,
        area,
        Panel {
            key: "?",
            name: "Help",
            active: true,
        },
        items,
        state,
    );
}

fn render_source_list(
    frame: &mut Frame,
    area: Rect,
    sources: &[WorkspaceSource],
    selected_sources: &[bool],
    state: &ListState,
    active: bool,
) {
    let panel = Panel {
        key: WorkspaceFocus::Source.key(),
        name: "Source",
        active,
    };
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
            Style::default().bg(Color::Blue).fg(Color::White)
        } else if idx == current {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };
        spans.push(Span::styled(text, style));
    }
    if spans.is_empty() {
        spans.push(Span::raw("<empty>"));
    }
    let paragraph = Paragraph::new(Line::from(spans)).block(panel_block(panel));
    frame.render_widget(paragraph, area);
}

fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    choices: &[WorkspaceSession],
    state: &ListState,
    active: bool,
) {
    let mut items: Vec<ListItem> = choices
        .iter()
        .enumerate()
        .map(|(_, choice)| {
            ListItem::new(format!("[{:<8}] {}", choice.source.label(), choice.title))
        })
        .collect();
    if items.is_empty() {
        items.push(ListItem::new("<empty>"));
    }
    render_list_panel(
        frame,
        area,
        Panel {
            key: WorkspaceFocus::Sessions.key(),
            name: "Sessions",
            active,
        },
        items,
        state,
    );
}

fn render_dialogue_list(
    frame: &mut Frame,
    area: Rect,
    choice: Option<&WorkspaceSession>,
    state: &ListState,
    selected_dialogues: &[bool],
    range_anchor: Option<usize>,
    active: bool,
) {
    let highlighted_idx = selected_index(state);
    let mut items: Vec<ListItem> = choice
        .map(|choice| {
            choice
                .dialogue_titles
                .iter()
                .enumerate()
                .map(|(idx, title)| {
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
                    let line = format!("{marker}{title}");
                    if in_range {
                        ListItem::new(Line::from(Span::styled(
                            line,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        )))
                    } else if selected {
                        ListItem::new(Line::from(Span::styled(
                            line,
                            Style::default().bg(Color::DarkGray).fg(Color::White),
                        )))
                    } else {
                        ListItem::new(line)
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if items.is_empty() {
        items.push(ListItem::new("<empty>"));
    }

    render_list_panel(
        frame,
        area,
        Panel {
            key: WorkspaceFocus::Dialogues.key(),
            name: "Dialogues",
            active,
        },
        items,
        state,
    );
}

fn render_list_panel(
    frame: &mut Frame,
    area: Rect,
    panel: Panel,
    items: Vec<ListItem>,
    state: &ListState,
) {
    let list = List::new(items)
        .block(panel_block(panel))
        .highlight_style(if panel.active {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else {
            Style::default().bg(Color::DarkGray)
        })
        .highlight_symbol(if panel.active { ">> " } else { "   " });
    let mut local_state = state.clone();
    frame.render_stateful_widget(list, area, &mut local_state);
}

fn render_content_panel(frame: &mut Frame, area: Rect, panel: Panel, text: String, scroll: usize) {
    let paragraph = Paragraph::new(highlighted_content_text(&text))
        .scroll((scroll as u16, 0))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(panel_block(panel));
    frame.render_widget(paragraph, area);
}

fn content_preview_text(
    session: Option<&WorkspaceSession>,
    selected_dialogues: &[bool],
    highlighted_idx: usize,
) -> String {
    let Some(session) = session else {
        return "<empty>".to_string();
    };

    let selected = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return format_content_with_line_numbers(current_agent_dialogue_text(
            session,
            highlighted_idx,
        ));
    }

    let text = selected
        .into_iter()
        .filter_map(|dialogue_idx| {
            session
                .units
                .get(dialogue_idx)
                .map(|unit| unit.plain.as_str())
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format_content_with_line_numbers(&text)
}

fn highlighted_content_text(text: &str) -> Text<'static> {
    let lines = text
        .lines()
        .map(|line| {
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
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

fn panel_block(panel: Panel) -> Block<'static> {
    let block = Block::default().borders(Borders::ALL).title(panel.title());
    if panel.active {
        block.border_style(Style::default().fg(Color::Cyan))
    } else {
        block
    }
}
