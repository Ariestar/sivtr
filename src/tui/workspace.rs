use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, ListItem, ListState, Paragraph};
use regex::Regex;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::commands::command_block_selector::CommandSelection;
use crate::tui::content_view::{
    content_cursor_position, highlight_spans, render_content_view, ContentSelection, ContentView,
    ContentViewMode,
};
use crate::tui::pane::{
    active_item_style, panel_block, render_list_panel, render_panel_scrollbar, selected_item_style,
    Panel, PanelScroll,
};
use crate::tui::workspace_search::{workspace_search_regex_for_query, WorkspaceSearchScope};
use sivtr_core::ai::{AgentProvider, AgentSelection};
use sivtr_core::record::{WorkRecord, WorkRef, WorkRefTarget};

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
                "opencode" => "opencode",
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkspaceCopyParts {
    pub(crate) input: TextPair,
    pub(crate) output: TextPair,
    pub(crate) block: TextPair,
    pub(crate) command: TextPair,
}

impl WorkspaceCopyParts {
    pub(crate) fn from_block(block: TextPair) -> Self {
        Self {
            input: block.clone(),
            output: block.clone(),
            block,
            command: TextPair::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspacePickedContent {
    pub(crate) source: WorkspaceSource,
    pub(crate) units: Vec<TextPair>,
    pub(crate) selection: CommandSelection,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSessionLoad {
    pub(crate) provider: AgentProvider,
    pub(crate) path: PathBuf,
    pub(crate) id: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) modified: SystemTime,
    pub(crate) selection_mode: AgentSelection,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSession {
    pub(crate) source: WorkspaceSource,
    pub(crate) modified: SystemTime,
    pub(crate) title: String,
    pub(crate) search_title: String,
    pub(crate) notice: Option<String>,
    pub(crate) records: Vec<WorkRecord>,
    pub(crate) load: Option<WorkspaceSessionLoad>,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceDialogue {
    pub(crate) source: WorkspaceSource,
    pub(crate) work_ref: Option<WorkRef>,
    pub(crate) title: String,
    pub(crate) record: Option<WorkRecord>,
    pub(crate) unit: TextPair,
    pub(crate) copy: WorkspaceCopyParts,
}

impl WorkspaceDialogue {
    pub(crate) fn display_unit(&self, target: Option<WorkRefTarget>) -> TextPair {
        target
            .and_then(|target| self.targeted_unit(target))
            .unwrap_or_else(|| self.unit.clone())
    }

    pub(crate) fn content_text(
        &self,
        mode: ContentViewMode,
        target: Option<WorkRefTarget>,
    ) -> String {
        if let Some(target @ WorkRefTarget::Part { .. }) = target {
            return match mode {
                ContentViewMode::Raw => self
                    .targeted_plain_text(target)
                    .unwrap_or_else(|| self.unit.plain.clone()),
                ContentViewMode::Reading => self
                    .targeted_structured_text(target)
                    .or_else(|| self.targeted_plain_text(target))
                    .unwrap_or_else(|| self.unit.plain.clone()),
            };
        }
        if matches!(target, Some(WorkRefTarget::Line(_))) {
            return self.unit.plain.clone();
        }

        match mode {
            ContentViewMode::Raw => self.unit.plain.clone(),
            ContentViewMode::Reading => self
                .record
                .as_ref()
                .map(structured_record_text)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| self.unit.plain.clone()),
        }
    }

    pub(crate) fn content_ref(&self, target: Option<WorkRefTarget>) -> Option<WorkRef> {
        let work_ref = self.work_ref.as_ref()?;
        let target = match target {
            Some(target @ WorkRefTarget::Part { .. }) => target,
            _ => return Some(work_ref.clone()),
        };
        Some(work_ref.with_target(target))
    }

    fn targeted_unit(&self, target: WorkRefTarget) -> Option<TextPair> {
        let WorkRefTarget::Part { .. } = target else {
            return None;
        };
        let part = self.record.as_ref()?.part_for_target(target)?;
        Some(TextPair {
            plain: part.text.clone(),
            ansi: part.ansi.clone().unwrap_or_else(|| part.text.clone()),
        })
    }

    fn targeted_plain_text(&self, target: WorkRefTarget) -> Option<String> {
        let WorkRefTarget::Part { .. } = target else {
            return None;
        };
        self.record.as_ref()?.content_for_target(target)
    }

    fn targeted_structured_text(&self, target: WorkRefTarget) -> Option<String> {
        let WorkRefTarget::Part { .. } = target else {
            return None;
        };
        let part = self.record.as_ref()?.part_for_target(target)?;
        Some(structured_part_text(self.record.as_ref()?, part))
    }
}

fn structured_record_text(record: &WorkRecord) -> String {
    if record.parts.is_empty() {
        return record.combined_text();
    }

    record
        .parts
        .iter()
        .map(|part| structured_part_text(record, part))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn structured_part_text(record: &WorkRecord, part: &sivtr_core::record::WorkPart) -> String {
    let heading = structured_part_heading(part);
    let marker = structured_part_marker_line(record, part);
    match part.kind {
        sivtr_core::record::WorkPartKind::UserMessage
        | sivtr_core::record::WorkPartKind::AssistantMessage => {
            format!("{heading}\n{marker}\n{}", part.text)
        }
        sivtr_core::record::WorkPartKind::Prompt
        | sivtr_core::record::WorkPartKind::Command
        | sivtr_core::record::WorkPartKind::ToolCall
        | sivtr_core::record::WorkPartKind::ToolOutput
        | sivtr_core::record::WorkPartKind::Text
        | sivtr_core::record::WorkPartKind::Error => {
            let language = structured_part_code_language(part)
                .map(|language| format!("~~~{language}\n"))
                .unwrap_or_else(|| "~~~\n".to_string());
            format!("{heading}\n{marker}\n{language}{}\n~~~", part.text)
        }
    }
}

fn structured_part_marker_line(record: &WorkRecord, part: &sivtr_core::record::WorkPart) -> String {
    let mut markers = vec![format!(
        "`{}`",
        record.work_ref.with_part(part.io, part.index)
    )];
    if let Some(timestamp) = part.occurred_at.as_deref().or(record.time.primary_at()) {
        markers.push(format!("`{timestamp}`"));
    }
    markers.join("  ")
}

fn structured_part_heading(part: &sivtr_core::record::WorkPart) -> String {
    let title = match part.kind {
        sivtr_core::record::WorkPartKind::Prompt => "Prompt",
        sivtr_core::record::WorkPartKind::Command => "Command",
        sivtr_core::record::WorkPartKind::UserMessage => "User",
        sivtr_core::record::WorkPartKind::AssistantMessage => "Assistant",
        sivtr_core::record::WorkPartKind::ToolCall => "Tool Call",
        sivtr_core::record::WorkPartKind::ToolOutput => "Tool Output",
        sivtr_core::record::WorkPartKind::Text => "Output",
        sivtr_core::record::WorkPartKind::Error => "Error",
    };
    match part
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
    {
        Some(label) => format!("## {title} ({label})"),
        None => format!("## {title}"),
    }
}

fn structured_part_code_language(part: &sivtr_core::record::WorkPart) -> Option<&'static str> {
    match part.kind {
        sivtr_core::record::WorkPartKind::Command => Some("shell"),
        sivtr_core::record::WorkPartKind::ToolCall => Some("bash"),
        sivtr_core::record::WorkPartKind::Prompt
        | sivtr_core::record::WorkPartKind::ToolOutput
        | sivtr_core::record::WorkPartKind::Text
        | sivtr_core::record::WorkPartKind::Error
        | sivtr_core::record::WorkPartKind::UserMessage
        | sivtr_core::record::WorkPartKind::AssistantMessage => None,
    }
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
    ToggleContentMode,
    VisualTextSelect,
    Copy,
    CopyInput,
    CopyOutput,
    CopyBlock,
    CopyCommand,
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
    pub(crate) content_mode: ContentViewMode,
    pub(crate) content_target: Option<WorkRefTarget>,
    pub(crate) show_help: bool,
    pub(crate) help_state: &'a ListState,
    pub(crate) search: Option<WorkspaceSearchView<'a>>,
    pub(crate) line_filter_input_open: bool,
    pub(crate) line_filter: Option<&'a str>,
    pub(crate) line_filter_error: Option<&'a str>,
    pub(crate) fullscreen: Option<WorkspaceFocus>,
    pub(crate) content_selection: Option<ContentSelection>,
}

pub(crate) struct WorkspaceSearchView<'a> {
    pub(crate) query: &'a str,
    pub(crate) scope: WorkspaceSearchScope,
    pub(crate) result_count: usize,
    pub(crate) current_match: Option<usize>,
    pub(crate) match_count: usize,
    pub(crate) current_target: Option<String>,
    pub(crate) input_open: bool,
}

struct WorkspaceFooterView<'a> {
    focus: WorkspaceFocus,
    show_help: bool,
    search: Option<&'a WorkspaceSearchView<'a>>,
    line_filter_input_open: bool,
    line_filter: Option<&'a str>,
    line_filter_error: Option<&'a str>,
    fullscreen: Option<WorkspaceFocus>,
    content_mode: ContentViewMode,
    content_selection: Option<ContentSelection>,
    current_ref: Option<&'a WorkRef>,
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
            key: "r (Content)",
            description: "toggle raw/read content mode",
            action: WorkspaceHelpAction::ToggleContentMode,
        },
        WorkspaceHelpEntry {
            key: "v (Content)",
            description: "start visual text selection",
            action: WorkspaceHelpAction::VisualTextSelect,
        },
        WorkspaceHelpEntry {
            key: ":",
            description: "start line filter for next copy",
            action: WorkspaceHelpAction::CloseHelp,
        },
        WorkspaceHelpEntry {
            key: "i",
            description: "copy current input/question",
            action: WorkspaceHelpAction::CopyInput,
        },
        WorkspaceHelpEntry {
            key: "o",
            description: "copy current output/answer",
            action: WorkspaceHelpAction::CopyOutput,
        },
        WorkspaceHelpEntry {
            key: "y",
            description: "copy current input + output",
            action: WorkspaceHelpAction::CopyBlock,
        },
        WorkspaceHelpEntry {
            key: "c",
            description: "copy terminal command without prompt",
            action: WorkspaceHelpAction::CopyCommand,
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
    let current_ref = current_content_ref(
        view.dialogues,
        view.selected_dialogues,
        dialogue_idx,
        view.content_target,
    );
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

    let content_text = workspace_content_text(
        view.dialogues,
        view.selected_dialogues,
        dialogue_idx,
        view.content_mode,
        view.content_target,
    );
    render_content_panel(
        frame,
        layout.content,
        Panel::new(
            WorkspaceFocus::Content.key(),
            content_title(
                view.content_mode,
                view.selected_dialogues,
                current_ref.as_ref(),
            ),
            view.focus == WorkspaceFocus::Content,
        ),
        content_text.clone(),
        view.content_scroll,
        view.content_mode,
        view.content_selection,
        view.search.as_ref(),
        search_regex.as_ref(),
    );

    render_footer(
        frame,
        chunks[1],
        WorkspaceFooterView {
            focus: view.focus,
            show_help: view.show_help,
            search: view.search.as_ref(),
            line_filter_input_open: view.line_filter_input_open,
            line_filter: view.line_filter,
            line_filter_error: view.line_filter_error,
            fullscreen: view.fullscreen,
            content_mode: view.content_mode,
            content_selection: view.content_selection,
            current_ref: current_ref.as_ref(),
        },
    );

    if let Some(selection) = view.content_selection.and_then(|selection| {
        content_cursor_position(
            layout.content,
            &content_text,
            view.content_scroll,
            view.content_mode,
            selection.cursor,
        )
    }) {
        frame.set_cursor_position(selection);
    }

    if let Some(search) = view.search.filter(|search| search.input_open) {
        render_search_box(frame, centered_rect(chunks[0], 60, 12), search);
    } else if view.line_filter_input_open || view.line_filter_error.is_some() {
        render_line_filter_box(
            frame,
            centered_rect(chunks[0], 60, 14),
            view.line_filter,
            view.line_filter_error,
            view.line_filter_input_open,
        );
    } else if view.show_help {
        render_help_panel(frame, chunks[0], view.help_state);
    }
}

fn render_footer(frame: &mut Frame, area: Rect, footer: WorkspaceFooterView<'_>) {
    let WorkspaceFooterView {
        focus,
        show_help,
        search,
        line_filter_input_open,
        line_filter,
        line_filter_error,
        fullscreen,
        content_mode,
        content_selection,
        current_ref,
    } = footer;
    let controls = if search.is_some() {
        let suffix = search.and_then(search_position_label).unwrap_or_default();
        let target = search_target_footer_suffix(search);
        if search.map(|search| search.input_open).unwrap_or(false) {
            return frame.render_widget(
                Paragraph::new(format!(
                    "type search  > session  # dialogue  Enter accept  Esc clear  {suffix}{target}"
                )),
                area,
            );
        }
        return frame.render_widget(
            Paragraph::new(format!(
                "n next  N previous  Esc clear search  / edit  {suffix}{target}"
            )),
            area,
        );
    } else if content_selection.is_some() {
        "visual  h/j/k/l move  drag select  Ctrl-drag block  y/Enter copy  Esc/v return"
    } else if show_help {
        "j/k move  Enter execute  Esc/? close help  q cancel"
    } else {
        match focus {
            WorkspaceFocus::Source => "j/k move  Space toggle  a all  g agents  t terminal  Enter sessions  z fullscreen  / search  q/Esc cancel  ? help",
            WorkspaceFocus::Sessions => {
                "j/k move  Space toggle  0 source  l/Right/Enter dialogues  t vim  z fullscreen  / search  q/Esc cancel  ? help"
            }
            WorkspaceFocus::Dialogues => {
                "j/k move  Space toggle  v range  a all  : lines  i/o/y copy parts  c command  l/Right content  t vim  Enter copy  z fullscreen  / search  h/Esc back  ? help"
            }
            WorkspaceFocus::Content => {
                "j/k scroll  v select text  : lines  i/o/y copy parts  c command  Ctrl-d/PageDown down  Ctrl-u/PageUp up  r mode  t vim  Enter copy  z fullscreen  / search  h/Esc back  ? help"
            }
        }
    };
    let suffix = if fullscreen.is_some() {
        "  [fullscreen]"
    } else {
        ""
    };
    let line_filter_status = if let Some(error) = line_filter_error {
        format!("  [lines invalid: {error}]")
    } else if line_filter_input_open {
        format!("  [lines: {}]", line_filter.unwrap_or_default())
    } else if let Some(spec) = line_filter.filter(|spec| !spec.is_empty()) {
        format!("  [lines {spec}]")
    } else {
        String::new()
    };
    let mode = if focus == WorkspaceFocus::Content {
        format!("  [{}]", content_mode.label())
    } else {
        String::new()
    };
    let ref_status = matches!(focus, WorkspaceFocus::Dialogues | WorkspaceFocus::Content)
        .then_some(current_ref)
        .flatten()
        .map(|work_ref| format!("  [ref {work_ref}]"))
        .unwrap_or_default();
    let footer = Paragraph::new(format!(
        "{controls}{suffix}{line_filter_status}{mode}{ref_status}"
    ));
    frame.render_widget(footer, area);
}

fn search_position_label(search: &WorkspaceSearchView<'_>) -> Option<String> {
    let current = search.current_match?;
    Some(format!("[{}/{}]", current + 1, search.match_count))
}

fn search_target_footer_suffix(search: Option<&WorkspaceSearchView<'_>>) -> String {
    search
        .and_then(|search| search.current_target.as_deref())
        .map(|target| format!("  [match {target}]"))
        .unwrap_or_default()
}

fn current_content_dialogue<'a>(
    dialogues: &'a [WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
) -> Option<&'a WorkspaceDialogue> {
    let selected = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();
    match selected.as_slice() {
        [] => dialogues.get(highlighted_idx),
        [idx] => dialogues.get(*idx),
        _ => None,
    }
}

fn current_content_ref(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    target: Option<WorkRefTarget>,
) -> Option<WorkRef> {
    current_content_dialogue(dialogues, selected_dialogues, highlighted_idx)
        .and_then(|dialogue| dialogue.content_ref(target))
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
    let title = search_box_title(&search);
    let paragraph =
        Paragraph::new(search_box_body(&search)).block(panel_block(&Panel::new("", title, true)));
    frame.render_widget(paragraph, area);
}

fn search_box_title(search: &WorkspaceSearchView<'_>) -> String {
    let result_label = if search.query.trim().is_empty() {
        "ready".to_string()
    } else if let Some(position) = search_position_label(search) {
        position
    } else if search.result_count == 1 {
        "1 result".to_string()
    } else {
        format!("{} results", search.result_count)
    };
    if search.scope == WorkspaceSearchScope::Content {
        format!("Search  ({result_label})")
    } else {
        format!("Search {}  ({})", search.scope.label(), result_label)
    }
}

fn search_box_body(search: &WorkspaceSearchView<'_>) -> String {
    match search.current_target.as_deref() {
        Some(target) => format!("{}\n\nTarget: {target}", search.query),
        None => search.query.to_string(),
    }
}

fn render_line_filter_box(
    frame: &mut Frame,
    area: Rect,
    line_filter: Option<&str>,
    line_filter_error: Option<&str>,
    input_open: bool,
) {
    frame.render_widget(Clear, area);
    let title = if line_filter_error.is_some() {
        "Line Filter  (invalid)".to_string()
    } else if input_open {
        "Line Filter  (editing)".to_string()
    } else {
        "Line Filter".to_string()
    };
    let prompt = line_filter_prompt_text(line_filter, line_filter_error, input_open);
    let paragraph = Paragraph::new(prompt).block(panel_block(&Panel::new(":", title, true)));
    frame.render_widget(paragraph, area);
}

fn line_filter_prompt_text(
    line_filter: Option<&str>,
    line_filter_error: Option<&str>,
    input_open: bool,
) -> String {
    if let Some(error) = line_filter_error {
        return format!(
            "{error}\n\nCurrent: {}\nUse 1-based specs like 2:8 or 1,3,8:12.\nEsc clears the error.",
            line_filter.unwrap_or_default()
        );
    }

    if input_open {
        return format!(
            "{}\n\nEnter keeps displayed lines.\ni/o/y/c copy filtered input, output, block, or command.\nBackspace edits. Esc clears.",
            line_filter.unwrap_or_default()
        );
    }

    format!(
        "{}\n\nUse 1-based specs like 2:8 or 1,3,8:12.",
        line_filter.unwrap_or_default()
    )
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
    render_list_scrollbar(
        frame,
        area,
        selected_index(state),
        workspace_help_entries().len(),
        true,
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

#[allow(clippy::too_many_arguments)]
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
    render_list_scrollbar(frame, area, cursor_idx, choices.len(), active);
}

#[allow(clippy::too_many_arguments)]
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
    render_list_scrollbar(frame, area, highlighted_idx, dialogues.len(), active);
}

fn render_list_scrollbar(
    frame: &mut Frame,
    area: Rect,
    selected_idx: usize,
    total: usize,
    active: bool,
) {
    render_panel_scrollbar(
        frame,
        area,
        PanelScroll::new(selected_idx, total, panel_viewport_height(area)),
        active,
    );
}

fn panel_viewport_height(area: Rect) -> usize {
    area.height.saturating_sub(2) as usize
}

#[allow(clippy::too_many_arguments)]
fn render_content_panel(
    frame: &mut Frame,
    area: Rect,
    panel: Panel,
    text: String,
    scroll: usize,
    mode: ContentViewMode,
    selection: Option<ContentSelection>,
    search: Option<&WorkspaceSearchView<'_>>,
    search_regex: Option<&Regex>,
) {
    let content_search = search
        .filter(|search| search.scope == WorkspaceSearchScope::Content)
        .and(search_regex);
    render_content_view(
        frame,
        area,
        panel,
        ContentView {
            text: &text,
            scroll,
            search_regex: content_search,
            mode,
            selection,
        },
    );
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

fn content_title(
    mode: ContentViewMode,
    selected_dialogues: &[bool],
    current_ref: Option<&WorkRef>,
) -> String {
    let title = selected_parent_title(
        &format!("Content ({})", mode.label()),
        selected_dialogues,
        "dialogue",
        "dialogues",
    );
    current_ref
        .map(|work_ref| format!("{title} [{work_ref}]"))
        .unwrap_or(title)
}

pub(crate) fn workspace_content_text(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkRefTarget>,
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
        return dialogues
            .get(highlighted_idx)
            .map(|dialogue| dialogue.content_text(mode, target))
            .unwrap_or_else(|| "<empty>".to_string());
    }

    selected
        .into_iter()
        .filter_map(|dialogue_idx| dialogues.get(dialogue_idx))
        .map(|dialogue| dialogue.content_text(mode, None))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::WorkspaceFocus;
    use super::{
        can_open_dialogue_vim, content_title, current_content_dialogue, line_filter_prompt_text,
        search_box_body, search_box_title, workspace_content_text,
    };
    use crate::tui::content_view::ContentViewMode;
    use crate::tui::workspace::{
        TextPair, WorkspaceCopyParts, WorkspaceDialogue, WorkspaceSearchView, WorkspaceSource,
    };
    use crate::tui::workspace_search::WorkspaceSearchScope;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{WorkRecord, WorkRef, WorkRefTarget};

    #[test]
    fn can_open_dialogue_vim_accepts_sessions_when_dialogues_exist() {
        assert!(can_open_dialogue_vim(WorkspaceFocus::Sessions, 1));
        assert!(can_open_dialogue_vim(WorkspaceFocus::Dialogues, 1));
        assert!(can_open_dialogue_vim(WorkspaceFocus::Content, 1));
        assert!(!can_open_dialogue_vim(WorkspaceFocus::Sessions, 0));
    }

    #[test]
    fn content_preview_text_preserves_raw_text_without_line_number_prefixes() {
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::Terminal,
            work_ref: None,
            title: "cmd".to_string(),
            record: None,
            unit: TextPair {
                plain: "alpha\n\nomega".to_string(),
                ansi: String::new(),
            },
            copy: WorkspaceCopyParts::default(),
        };

        assert_eq!(
            workspace_content_text(&[dialogue], &[false], 0, ContentViewMode::Raw, None),
            "alpha\n\nomega"
        );
    }

    #[test]
    fn content_preview_text_uses_targeted_part_text_in_raw_mode() {
        let record = WorkRecord {
            schema_version: 1,
            work_ref: WorkRef::agent_record(AgentProvider::Codex, "session", 1),
            kind: sivtr_core::record::WorkRecordKind::ChatTurn,
            source: sivtr_core::record::WorkSource {
                channel: sivtr_core::record::WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: sivtr_core::record::WorkSessionRef {
                id: "session".to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: sivtr_core::record::WorkTime::default(),
            status: None,
            title: "cmd".to_string(),
            parts: vec![sivtr_core::record::WorkPart {
                io: sivtr_core::record::WorkPartIo::Input,
                kind: sivtr_core::record::WorkPartKind::ToolCall,
                index: 1,
                occurred_at: None,
                label: Some("tool".to_string()),
                text: "hidden tool call".to_string(),
                ansi: None,
                tags: Vec::new(),
            }],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent_record(AgentProvider::Codex, "session", 1)),
            title: "cmd".to_string(),
            record: Some(record),
            unit: TextPair {
                plain: "visible\nanswer".to_string(),
                ansi: String::new(),
            },
            copy: WorkspaceCopyParts::default(),
        };

        assert_eq!(
            workspace_content_text(
                &[dialogue],
                &[false],
                0,
                ContentViewMode::Raw,
                Some(WorkRefTarget::Part {
                    io: sivtr_core::record::WorkPartIo::Input,
                    index: 1,
                }),
            ),
            "hidden tool call"
        );
    }

    #[test]
    fn content_preview_text_uses_structured_targeted_part_text_in_reading_mode() {
        let record = WorkRecord {
            schema_version: 1,
            work_ref: WorkRef::agent_record(AgentProvider::Codex, "session", 1),
            kind: sivtr_core::record::WorkRecordKind::ChatTurn,
            source: sivtr_core::record::WorkSource {
                channel: sivtr_core::record::WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: sivtr_core::record::WorkSessionRef {
                id: "session".to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: sivtr_core::record::WorkTime::default(),
            status: None,
            title: "cmd".to_string(),
            parts: vec![sivtr_core::record::WorkPart {
                io: sivtr_core::record::WorkPartIo::Input,
                kind: sivtr_core::record::WorkPartKind::ToolCall,
                index: 1,
                occurred_at: None,
                label: Some("tool".to_string()),
                text: "hidden tool call".to_string(),
                tags: Vec::new(),
                ansi: None,
            }],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent_record(AgentProvider::Codex, "session", 1)),
            title: "cmd".to_string(),
            record: Some(record),
            unit: TextPair {
                plain: "visible\nanswer".to_string(),
                ansi: String::new(),
            },
            copy: WorkspaceCopyParts::default(),
        };

        let text = workspace_content_text(
            &[dialogue],
            &[false],
            0,
            ContentViewMode::Reading,
            Some(WorkRefTarget::Part {
                io: sivtr_core::record::WorkPartIo::Input,
                index: 1,
            }),
        );

        assert!(text.contains("## Tool Call (tool)"));
        assert!(text.contains("`codex/session/1/i/1`"));
        assert!(text.contains("~~~bash\nhidden tool call\n~~~"));
    }

    #[test]
    fn reading_mode_structures_parts_with_headings_and_code_fences() {
        let record = WorkRecord {
            schema_version: 1,
            work_ref: WorkRef::agent_record(AgentProvider::Codex, "session", 1),
            kind: sivtr_core::record::WorkRecordKind::ChatTurn,
            source: sivtr_core::record::WorkSource {
                channel: sivtr_core::record::WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: sivtr_core::record::WorkSessionRef {
                id: "session".to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: sivtr_core::record::WorkTime {
                started_at: Some("2026-05-24T12:00:00Z".to_string()),
                ended_at: None,
                duration_ms: None,
            },
            status: None,
            title: "cmd".to_string(),
            parts: vec![
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::UserMessage,
                    index: 1,
                    occurred_at: None,
                    label: None,
                tags: Vec::new(),
                    text: "question".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::ToolCall,
                    index: 2,
                    occurred_at: None,
                tags: Vec::new(),
                    label: Some("Bash".to_string()),
                    text: "cargo test".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Output,
                    kind: sivtr_core::record::WorkPartKind::ToolOutput,
                    index: 1,
                tags: Vec::new(),
                    occurred_at: None,
                    label: Some("Bash".to_string()),
                    text: "ok".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Output,
                    kind: sivtr_core::record::WorkPartKind::AssistantMessage,
                tags: Vec::new(),
                    index: 2,
                    occurred_at: None,
                    label: None,
                    text: "answer".to_string(),
                    ansi: None,
                },
            ],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent_record(AgentProvider::Codex, "session", 1)),
            title: "cmd".to_string(),
            record: Some(record),
            unit: TextPair {
                plain: "question\nanswer".to_string(),
                ansi: String::new(),
            },
            copy: WorkspaceCopyParts::default(),
        };

        let text = workspace_content_text(&[dialogue], &[false], 0, ContentViewMode::Reading, None);

        assert!(text.contains("## User\n`codex/session/1/i/1`  `2026-05-24T12:00:00Z`\nquestion"));
        assert!(text.contains(
            "## Tool Call (Bash)\n`codex/session/1/i/2`  `2026-05-24T12:00:00Z`\n~~~bash\ncargo test\n~~~"
        ));
        assert!(text.contains(
            "## Tool Output (Bash)\n`codex/session/1/o/1`  `2026-05-24T12:00:00Z`\n~~~\nok\n~~~"
        ));
        assert!(
            text.contains("## Assistant\n`codex/session/1/o/2`  `2026-05-24T12:00:00Z`\nanswer")
        );
    }

    #[test]
    fn content_title_includes_view_mode() {
        assert_eq!(
            content_title(ContentViewMode::Reading, &[false, false], None),
            "Content (read)"
        );
        assert_eq!(
            content_title(ContentViewMode::Raw, &[true, false], None),
            "Content (raw): 1 dialogue selected"
        );
    }

    #[test]
    fn content_title_includes_current_dialogue_ref() {
        let work_ref = WorkRef::agent_record(AgentProvider::Codex, "session", 2);

        assert_eq!(
            content_title(ContentViewMode::Reading, &[false], Some(&work_ref)),
            "Content (read) [codex/session/2]"
        );
    }

    #[test]
    fn line_filter_prompt_text_shows_current_input() {
        let prompt = line_filter_prompt_text(Some("2:8"), None, true);
        assert!(prompt.contains("2:8"));
        assert!(prompt.contains("Enter keeps displayed lines."));
    }

    #[test]
    fn line_filter_prompt_text_shows_error_and_current_value() {
        let prompt = line_filter_prompt_text(Some("23"), Some("Invalid line number"), false);
        assert!(prompt.contains("Invalid line number"));
        assert!(prompt.contains("Current: 23"));
    }

    #[test]
    fn current_content_dialogue_uses_single_selected_dialogue() {
        let dialogues = vec![
            WorkspaceDialogue {
                source: WorkspaceSource::Agent(AgentProvider::Codex),
                work_ref: Some(WorkRef::agent_record(AgentProvider::Codex, "session", 1)),
                title: "first".to_string(),
                record: None,
                unit: TextPair::default(),
                copy: WorkspaceCopyParts::default(),
            },
            WorkspaceDialogue {
                source: WorkspaceSource::Agent(AgentProvider::Codex),
                work_ref: Some(WorkRef::agent_record(AgentProvider::Codex, "session", 2)),
                title: "second".to_string(),
                record: None,
                unit: TextPair::default(),
                copy: WorkspaceCopyParts::default(),
            },
        ];

        let current = current_content_dialogue(&dialogues, &[false, true], 0).unwrap();

        assert_eq!(
            current.work_ref.as_ref().unwrap().to_string(),
            "codex/session/2"
        );
    }

    #[test]
    fn current_content_ref_round_trips_active_part_target() {
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::Agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent_record(AgentProvider::Codex, "session", 2)),
            title: "second".to_string(),
            record: None,
            unit: TextPair::default(),
            copy: WorkspaceCopyParts::default(),
        }];

        let current = super::current_content_ref(
            &dialogues,
            &[false],
            0,
            Some(WorkRefTarget::Part {
                io: sivtr_core::record::WorkPartIo::Output,
                index: 1,
            }),
        )
        .unwrap();

        assert_eq!(current.to_string(), "codex/session/2/o/1");
    }

    #[test]
    fn search_box_body_includes_current_target_ref() {
        let search = WorkspaceSearchView {
            query: "needle",
            scope: WorkspaceSearchScope::Content,
            result_count: 1,
            current_match: Some(0),
            match_count: 1,
            current_target: Some("codex/session/1/4".to_string()),
            input_open: true,
        };

        assert_eq!(search_box_title(&search), "Search  ([1/1])");
        assert_eq!(
            search_box_body(&search),
            "needle\n\nTarget: codex/session/1/4"
        );
    }
}
