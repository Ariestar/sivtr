use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, ListItem, ListState, Paragraph};
use regex::Regex;
use std::time::SystemTime;

use crate::commands::select::CommandSelection;
use crate::tui::content_view::{
    content_cursor_position, highlight_spans, render_content_view, ContentSelection, ContentView,
    ContentViewMode,
};
use crate::tui::pane::{
    active_item_style, panel_block, render_list_panel, render_panel_scrollbar, selected_item_style,
    Panel, PanelScroll,
};
use crate::tui::theme;
use crate::tui::workspace_search::{workspace_search_regex_for_query, WorkspaceSearchScope};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};

/// Kind of memory source (local path body before any `scope:` prefix).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceSourceKind {
    Terminal,
    Agent(AgentProvider),
}

impl WorkspaceSourceKind {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Agent(provider) => provider.command_name(),
        }
    }

    pub(crate) fn badge(self) -> &'static str {
        match self {
            Self::Terminal => "term",
            Self::Agent(AgentProvider::Codex) => "cdx",
            Self::Agent(AgentProvider::Claude) => "cld",
            Self::Agent(AgentProvider::Cursor) => "cur",
            Self::Agent(AgentProvider::OpenCode) => "opc",
            Self::Agent(AgentProvider::OpenClaw) => "ocw",
            Self::Agent(AgentProvider::Hermes) => "hrm",
            Self::Agent(AgentProvider::Grok) => "grk",
            Self::Agent(AgentProvider::Pi) => "pi",
        }
    }

    pub(crate) fn color(self) -> Color {
        match self {
            Self::Terminal => theme::terminal_color(),
            Self::Agent(provider) => theme::provider_color(provider),
        }
    }

    pub(crate) fn is_agent(self) -> bool {
        matches!(self, Self::Agent(_))
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal)
    }
}

/// One selectable Source pane entry. Local and remote share the same shape —
/// remote is only a named scope that `workset::query` already understands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceSource {
    /// Named scope (`desk`, `docs`); `None` = current local workspace.
    pub(crate) scope: Option<String>,
    pub(crate) kind: WorkspaceSourceKind,
}

impl WorkspaceSource {
    pub(crate) fn local(kind: WorkspaceSourceKind) -> Self {
        Self { scope: None, kind }
    }

    pub(crate) fn terminal() -> Self {
        Self::local(WorkspaceSourceKind::Terminal)
    }

    pub(crate) fn agent(provider: AgentProvider) -> Self {
        Self::local(WorkspaceSourceKind::Agent(provider))
    }

    pub(crate) fn scoped(scope: impl Into<String>, kind: WorkspaceSourceKind) -> Self {
        Self {
            scope: Some(scope.into()),
            kind,
        }
    }

    /// Selector passed to `workset::query` (`codex`, `desk:terminal`, …).
    pub(crate) fn selector(&self) -> String {
        match &self.scope {
            Some(scope) => format!("{scope}:{}", self.kind.path()),
            None => self.kind.path().to_string(),
        }
    }

    /// Compact Source-pane label (`codex`, `desk/codex`).
    pub(crate) fn label(&self) -> String {
        match &self.scope {
            Some(scope) => format!("{scope}/{}", self.kind.path()),
            None => self.kind.path().to_string(),
        }
    }

    pub(crate) fn badge(&self) -> &'static str {
        self.kind.badge()
    }

    pub(crate) fn color(&self) -> Color {
        self.kind.color()
    }

    pub(crate) fn is_remote(&self) -> bool {
        self.scope.is_some()
    }

    pub(crate) fn is_agent(&self) -> bool {
        self.kind.is_agent()
    }

    pub(crate) fn is_terminal(&self) -> bool {
        self.kind.is_terminal()
    }
}

/// Compact load indicator for the Source pane selection/status column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceLoadMarker {
    Idle,
    Loading,
    Ready,
    Failed,
}

impl SourceLoadMarker {
    /// Leading status/selection glyph. `tick` animates Loading as a circle.
    pub(crate) fn status_glyph(self, selected: bool, tick: u8) -> &'static str {
        match self {
            Self::Loading => {
                const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];
                FRAMES[(tick as usize) % FRAMES.len()]
            }
            Self::Failed => "!",
            Self::Idle | Self::Ready => {
                if selected {
                    "●"
                } else {
                    "○"
                }
            }
        }
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
pub(crate) struct WorkspaceSession {
    pub(crate) source: WorkspaceSource,
    /// Stable session identity for hydrate / selection (not display title).
    pub(crate) session_id: String,
    pub(crate) modified: SystemTime,
    pub(crate) title: String,
    pub(crate) search_title: String,
    /// Dialogue bodies. Empty until the session is focused/selected and hydrated.
    pub(crate) records: Vec<WorkRecord>,
    /// True when `records` holds full dialogue bodies for this session.
    pub(crate) body_loaded: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceDialogue {
    pub(crate) source: WorkspaceSource,
    pub(crate) work_ref: Option<WorkRef>,
    /// Display title; list paint prefers `WorkspaceView::dialogue_titles`.
    #[allow(dead_code)]
    pub(crate) title: String,
    pub(crate) record: Option<WorkRecord>,
    pub(crate) copy: WorkspaceCopyParts,
}

impl WorkspaceDialogue {
    /// Text used for copy shortcuts / vim on the currently displayed content.
    /// Always derived from `record.parts` when present — never a stale cache.
    pub(crate) fn display_unit(&self, mode: ContentViewMode, target: Option<WorkAt>) -> TextPair {
        let plain = self.content_text(mode, target);
        TextPair {
            ansi: plain.clone(),
            plain,
        }
    }

    pub(crate) fn content_text(&self, mode: ContentViewMode, target: Option<WorkAt>) -> String {
        if let Some(target @ WorkAt::Part { .. }) = target {
            return match mode {
                ContentViewMode::Raw => self
                    .targeted_plain_text(target)
                    .unwrap_or_else(|| "<empty>".to_string()),
                ContentViewMode::Reading => self
                    .targeted_structured_text(target)
                    .unwrap_or_else(|| "<empty>".to_string()),
            };
        }
        if matches!(target, Some(WorkAt::Line(_))) {
            return self
                .record
                .as_ref()
                .map(|record| match mode {
                    ContentViewMode::Raw => raw_record_text(record),
                    ContentViewMode::Reading => structured_record_text(record),
                })
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "<empty>".to_string());
        }

        // Single source of truth: record.parts.
        let Some(record) = self.record.as_ref() else {
            return "<empty>".to_string();
        };
        if record.parts.is_empty() {
            return "<empty>".to_string();
        }
        match mode {
            ContentViewMode::Raw => raw_record_text(record),
            ContentViewMode::Reading => structured_record_text(record),
        }
    }

    pub(crate) fn content_ref(&self, target: Option<WorkAt>) -> Option<WorkRef> {
        let work_ref = self.work_ref.as_ref()?;
        let target = match target {
            Some(target @ WorkAt::Part { .. }) => target,
            _ => return Some(work_ref.clone()),
        };
        Some(work_ref.with_at(target))
    }

    fn targeted_plain_text(&self, target: WorkAt) -> Option<String> {
        let WorkAt::Part { .. } = target else {
            return None;
        };
        let record = self.record.as_ref()?;
        let part = record.part_for_at(target)?;
        if part.kind.is_structure() {
            return Some(raw_part_text(record, part));
        }
        Some(part.text.clone())
    }

    fn targeted_structured_text(&self, target: WorkAt) -> Option<String> {
        let WorkAt::Part { .. } = target else {
            return None;
        };
        let part = self.record.as_ref()?.part_for_at(target)?;
        Some(structured_part_text(self.record.as_ref()?, part))
    }
}


/// Reading mode: Input / Output sections; structure channels folded. Within each
/// IO section, identical original markers are counted (`xN`) regardless of order
/// or adjacency; distinct markers stay as themselves. Raw expands payloads.
fn structured_record_text(record: &WorkRecord) -> String {
    debug_assert!(
        !record.parts.is_empty(),
        "structured_record_text requires parts"
    );
    format_record_by_io(record, true)
}

/// Raw mode: Input / Output sections; every part fully expanded with markers.
fn raw_record_text(record: &WorkRecord) -> String {
    debug_assert!(!record.parts.is_empty(), "raw_record_text requires parts");
    format_record_by_io(record, false)
}

fn format_record_by_io(record: &WorkRecord, reading: bool) -> String {
    use sivtr_core::record::WorkPartIo;

    let mut sections = Vec::new();
    for io in [WorkPartIo::Input, WorkPartIo::Output] {
        let parts: Vec<&sivtr_core::record::WorkPart> =
            record.parts.iter().filter(|part| part.io == io).collect();
        if parts.is_empty() {
            continue;
        }
        let body = if reading {
            structured_parts_text(record, &parts)
        } else {
            raw_parts_text(record, &parts)
        };
        if body.trim().is_empty() {
            continue;
        }
        sections.push(format!("## {}\n\n{body}", io_section_label(io)));
    }
    if sections.is_empty() {
        "<empty>".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn io_section_label(io: sivtr_core::record::WorkPartIo) -> &'static str {
    match io {
        sivtr_core::record::WorkPartIo::Input => "Input",
        sivtr_core::record::WorkPartIo::Output => "Output",
    }
}

/// Reading: dialogue text in order; structure folded.
///
/// Within one IO section ("一块"), every structure part contributes its original
/// open marker. Identical markers are counted (`xN`) — no adjacency / order
/// requirement. The fold line is emitted once, at the first structure position.
fn structured_parts_text(
    record: &WorkRecord,
    parts: &[&sivtr_core::record::WorkPart],
) -> String {
    let structure: Vec<&sivtr_core::record::WorkPart> = parts
        .iter()
        .copied()
        .filter(|part| part.kind.is_structure())
        .collect();
    let fold = (!structure.is_empty()).then(|| collapse_structure_markers(&structure));

    let mut chunks = Vec::new();
    let mut fold_emitted = false;
    for part in parts {
        if part.kind.is_structure() {
            if !fold_emitted {
                if let Some(fold) = fold.as_ref() {
                    chunks.push(fold.clone());
                }
                fold_emitted = true;
            }
            continue;
        }
        chunks.push(structured_part_text(record, part));
    }
    chunks.join("\n\n")
}

fn raw_parts_text(record: &WorkRecord, parts: &[&sivtr_core::record::WorkPart]) -> String {
    parts
        .iter()
        .map(|part| raw_part_text(record, part))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn structured_part_text(_record: &WorkRecord, part: &sivtr_core::record::WorkPart) -> String {
    if part.kind.is_structure() {
        // Folded: only the content-block marker — no ref/time/size noise.
        return structure_fold_label(part);
    }

    // Dialogue / terminal body only.
    part.text.clone()
}

fn raw_part_text(_record: &WorkRecord, part: &sivtr_core::record::WorkPart) -> String {
    if part.kind.is_structure() {
        return part
            .kind
            .as_agent_block_kind()
            .map(|kind| {
                sivtr_core::ai::format_structured_block(
                    kind,
                    part.label.as_deref(),
                    part.text.trim(),
                )
            })
            .unwrap_or_else(|| part.text.clone());
    }

    part.text.clone()
}

fn structure_fold_label(part: &sivtr_core::record::WorkPart) -> String {
    part.kind
        .as_agent_block_kind()
        .and_then(|kind| kind.open_marker(part.label.as_deref()))
        .unwrap_or_else(|| "<:structure:>".to_string())
}

/// Collapse structure parts onto one line of original markers.
///
/// Keeps each part's open marker (`<:tool:Bash call:>`, `<:skill:review:>`,
/// `<:thinking:>`, …). Identical markers anywhere in the set are counted:
/// `<:tool:Bash call:> x3` — order of first appearance only affects display order
/// of *distinct* markers, not whether they merge.
fn collapse_structure_markers(parts: &[&sivtr_core::record::WorkPart]) -> String {
    if parts.is_empty() {
        return String::new();
    }

    let mut counts: Vec<(String, usize)> = Vec::new();
    for part in parts {
        let label = structure_fold_label(part);
        if let Some((_, count)) = counts.iter_mut().find(|(existing, _)| *existing == label) {
            *count += 1;
        } else {
            counts.push((label, 1));
        }
    }
    counts
        .into_iter()
        .map(|(label, count)| {
            if count == 1 {
                label
            } else {
                format!("{label} x{count}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    /// Refresh next level under active rows (source→sessions, session→dialogues).
    Refresh,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkspaceHelpEntry {
    pub(crate) key: &'static str,
    pub(crate) description: &'static str,
    pub(crate) action: WorkspaceHelpAction,
    /// Short label for the footer (`refresh`). `None` = help-only, not in footer.
    pub(crate) footer_label: Option<&'static str>,
    /// Footer panes; empty = every focus pane.
    pub(crate) footer_panes: &'static [WorkspaceFocus],
}

pub(crate) struct WorkspaceView<'a> {
    pub(crate) sources: &'a [WorkspaceSource],
    pub(crate) selected_sources: &'a [bool],
    /// Per-source load marker (idle remote / ready / failed).
    pub(crate) source_markers: &'a [SourceLoadMarker],
    pub(crate) loading_tick: u8,
    pub(crate) source_state: &'a ListState,
    pub(crate) sessions: &'a [WorkspaceSession],
    pub(crate) selected_sessions: &'a [bool],
    pub(crate) session_state: &'a ListState,
    /// Dialogue list titles only (no body materialize on paint).
    pub(crate) dialogue_titles: &'a [&'a str],
    /// Materialized dialogues for content/copy (focus ∪ multi-select bodies).
    pub(crate) dialogues: &'a [WorkspaceDialogue],
    pub(crate) dialogue_state: &'a ListState,
    pub(crate) selected_dialogues: &'a [bool],
    pub(crate) range_anchor: Option<usize>,
    pub(crate) focus: WorkspaceFocus,
    pub(crate) content_scroll: usize,
    pub(crate) content_mode: ContentViewMode,
    pub(crate) content_at: Option<WorkAt>,
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
    use WorkspaceFocus::*;
    const ALL: &[WorkspaceFocus] = &[];
    const SRC: &[WorkspaceFocus] = &[Source];
    const DIA: &[WorkspaceFocus] = &[Dialogues];
    const CNT: &[WorkspaceFocus] = &[Content];
    const NAV: &[WorkspaceFocus] = &[Source, Sessions, Dialogues, Content];
    const SD: &[WorkspaceFocus] = &[Sessions, Dialogues];
    const DC: &[WorkspaceFocus] = &[Dialogues, Content];

    &[
        WorkspaceHelpEntry {
            key: "0",
            description: "focus Source pane",
            action: WorkspaceHelpAction::FocusSource,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "1",
            description: "focus Sessions pane",
            action: WorkspaceHelpAction::FocusSessions,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "2",
            description: "focus Dialogues pane",
            action: WorkspaceHelpAction::FocusDialogues,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "3",
            description: "focus Content pane",
            action: WorkspaceHelpAction::FocusContent,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "j",
            description: "move down",
            action: WorkspaceHelpAction::MoveDown,
            footer_label: Some("move"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "k",
            description: "move up",
            action: WorkspaceHelpAction::MoveUp,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "h",
            description: "previous pane",
            action: WorkspaceHelpAction::PreviousPane,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "l",
            description: "next pane",
            action: WorkspaceHelpAction::NextPane,
            footer_label: Some("pane"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "Space",
            description: "toggle selection",
            action: WorkspaceHelpAction::ToggleSelection,
            footer_label: Some("toggle"),
            footer_panes: &[Source, Sessions, Dialogues],
        },
        WorkspaceHelpEntry {
            key: "a",
            description: "select all sources",
            action: WorkspaceHelpAction::SelectAllSources,
            footer_label: Some("all"),
            footer_panes: SRC,
        },
        WorkspaceHelpEntry {
            key: "g",
            description: "select agent sources",
            action: WorkspaceHelpAction::SelectAgentSources,
            footer_label: Some("agents"),
            footer_panes: SRC,
        },
        WorkspaceHelpEntry {
            key: "t",
            description: "select terminal source",
            action: WorkspaceHelpAction::SelectTerminalSource,
            footer_label: Some("terminal"),
            footer_panes: SRC,
        },
        WorkspaceHelpEntry {
            key: "R",
            description: "refresh next level under active rows",
            action: WorkspaceHelpAction::Refresh,
            footer_label: Some("refresh"),
            footer_panes: &[Source, Sessions, Dialogues],
        },
        WorkspaceHelpEntry {
            key: "v",
            description: "range select dialogues",
            action: WorkspaceHelpAction::RangeSelect,
            footer_label: Some("range"),
            footer_panes: DIA,
        },
        WorkspaceHelpEntry {
            key: "a",
            description: "toggle all dialogues",
            action: WorkspaceHelpAction::ToggleAllDialogues,
            footer_label: Some("all"),
            footer_panes: DIA,
        },
        WorkspaceHelpEntry {
            key: "t",
            description: "open in Vim",
            action: WorkspaceHelpAction::OpenVim,
            footer_label: Some("vim"),
            footer_panes: SD,
        },
        WorkspaceHelpEntry {
            key: "Ctrl-d",
            description: "scroll content down",
            action: WorkspaceHelpAction::ScrollDown,
            footer_label: None,
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "Ctrl-u",
            description: "scroll content up",
            action: WorkspaceHelpAction::ScrollUp,
            footer_label: None,
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "r",
            description: "toggle fold/full content",
            action: WorkspaceHelpAction::ToggleContentMode,
            footer_label: Some("fold/full"),
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "v",
            description: "visual text select",
            action: WorkspaceHelpAction::VisualTextSelect,
            footer_label: Some("select"),
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "Esc",
            description: "close help / back / cancel",
            action: WorkspaceHelpAction::CloseHelp,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "i",
            description: "copy input",
            action: WorkspaceHelpAction::CopyInput,
            footer_label: Some("in"),
            footer_panes: DC,
        },
        WorkspaceHelpEntry {
            key: "o",
            description: "copy output",
            action: WorkspaceHelpAction::CopyOutput,
            footer_label: Some("out"),
            footer_panes: DC,
        },
        WorkspaceHelpEntry {
            key: "y",
            description: "copy block",
            action: WorkspaceHelpAction::CopyBlock,
            footer_label: Some("copy"),
            footer_panes: DC,
        },
        WorkspaceHelpEntry {
            key: "c",
            description: "copy command",
            action: WorkspaceHelpAction::CopyCommand,
            footer_label: Some("cmd"),
            footer_panes: DIA,
        },
        WorkspaceHelpEntry {
            key: "Enter",
            description: "confirm / open next / copy",
            action: WorkspaceHelpAction::Copy,
            footer_label: Some("enter"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "z",
            description: "toggle fullscreen",
            action: WorkspaceHelpAction::ToggleFullscreen,
            footer_label: Some("full"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "?",
            description: "toggle help",
            action: WorkspaceHelpAction::CloseHelp,
            footer_label: Some("help"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "/",
            description: "search",
            action: WorkspaceHelpAction::OpenSearch,
            footer_label: Some("search"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "q",
            description: "cancel",
            action: WorkspaceHelpAction::Cancel,
            footer_label: Some("quit"),
            footer_panes: NAV,
        },
    ]
}

/// Footer hotkeys for the focused pane, built from the help registry.
pub(crate) fn workspace_footer_hotkeys(focus: WorkspaceFocus) -> String {
    let mut parts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in workspace_help_entries() {
        let Some(label) = entry.footer_label else {
            continue;
        };
        if !entry.footer_panes.is_empty() && !entry.footer_panes.contains(&focus) {
            continue;
        }
        if !seen.insert(label) {
            continue;
        }
        parts.push(format!("{} {label}", entry.key));
    }
    parts.join("  ")
}

pub(crate) fn render_workspace(frame: &mut Frame, view: WorkspaceView<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let layout = workspace_layout(area, view.focus, view.fullscreen);

    let dialogue_idx = selected_index(view.dialogue_state)
        .min(view.dialogue_titles.len().saturating_sub(1));
    let current_ref = current_content_ref(
        view.dialogues,
        view.selected_dialogues,
        dialogue_idx,
        view.content_at,
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
        view.source_markers,
        view.loading_tick,
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
        view.dialogue_titles,
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
        view.content_at,
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

    let mut spans = if search.is_some() {
        let mut spans = if search.map(|search| search.input_open).unwrap_or(false) {
            footer_control_spans("type search  > session  # dialogue  Enter accept  Esc clear")
        } else {
            footer_control_spans("n next  N previous  Esc clear search  / edit")
        };
        if let Some(label) = search.and_then(search_position_label) {
            spans.extend(footer_status_spans(&label));
        }
        if let Some(target) = search
            .and_then(|search| search.current_target.as_deref())
            .map(|target| format!("match {target}"))
        {
            spans.extend(footer_status_spans(&target));
        }
        spans
    } else {
        let controls = if content_selection.is_some() {
            "select  drag / Ctrl-drag block  y/Enter/Ctrl-c copy  Esc/v clear"
                .to_string()
        } else if show_help {
            "j/k move  Enter execute  Esc/? close help  q cancel".to_string()
        } else {
            workspace_footer_hotkeys(focus)
        };
        footer_control_spans(&controls)
    };

    if fullscreen.is_some() {
        spans.extend(footer_status_spans("fullscreen"));
    }
    if let Some(error) = line_filter_error {
        spans.extend(footer_status_spans(&format!("lines invalid: {error}")));
    } else if line_filter_input_open {
        spans.extend(footer_status_spans(&format!(
            "lines: {}",
            line_filter.unwrap_or_default()
        )));
    } else if let Some(spec) = line_filter.filter(|spec| !spec.is_empty()) {
        spans.extend(footer_status_spans(&format!("lines {spec}")));
    }
    if focus == WorkspaceFocus::Content {
        spans.extend(footer_status_spans(content_mode.label()));
    }
    if matches!(focus, WorkspaceFocus::Dialogues | WorkspaceFocus::Content) {
        if let Some(work_ref) = current_ref {
            spans.extend(footer_status_spans(&format!("ref {work_ref}")));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn footer_control_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (idx, part) in text
        .split("  ")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if idx > 0 {
            spans.push(Span::styled("  ", theme::footer_style()));
        }
        match part.split_once(' ') {
            Some((key, rest)) => {
                spans.push(Span::styled(
                    key.to_string(),
                    theme::key_hint_style().add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(format!(" {rest}"), theme::footer_style()));
            }
            None => spans.push(Span::styled(
                part.to_string(),
                theme::key_hint_style().add_modifier(Modifier::BOLD),
            )),
        }
    }
    spans
}

fn footer_status_spans(label: &str) -> Vec<Span<'static>> {
    let text = label.trim().trim_start_matches('[').trim_end_matches(']');
    vec![
        Span::styled("  ", theme::footer_style()),
        Span::styled("[", Style::default().fg(theme::muted())),
        Span::styled(
            text.to_string(),
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("]", Style::default().fg(theme::muted())),
    ]
}

/// Compact session row: `· cdx  title…` (origin glyph + short badge + title).
///
/// `·` = local, `↗` = remote (named scope on the source or work_ref).
fn session_row_line(
    choice: &WorkspaceSession,
    selected: bool,
    active_panel: bool,
    base_style: Style,
    highlight: Option<&Regex>,
) -> Line<'static> {
    let remote = choice.source.is_remote()
        || choice
            .records
            .first()
            .is_some_and(|record| !record.work_ref.is_local());
    let check = if active_panel {
        if selected {
            "● "
        } else {
            "○ "
        }
    } else {
        ""
    };
    let origin = theme::origin_glyph(remote);
    let badge = choice.source.badge();
    let title = compact_session_title(choice);
    // Keep search highlighting over the full visible text, but paint origin/badge
    // with their own colors when the row is not using a solid selection background.
    let plain = format!("{check}{origin} {badge}  {title}");
    if base_style.bg.is_some() {
        return Line::from(highlight_spans(&plain, highlight, base_style));
    }

    let mut spans = Vec::new();
    if !check.is_empty() {
        spans.push(Span::styled(
            check.to_string(),
            Style::default().fg(theme::muted()),
        ));
    }
    spans.push(Span::styled(
        format!("{origin} "),
        theme::origin_style(remote),
    ));
    spans.push(Span::styled(
        format!("{badge}  "),
        Style::default()
            .fg(choice.source.color())
            .add_modifier(Modifier::BOLD),
    ));
    spans.extend(highlight_spans(
        &title,
        highlight,
        Style::default().fg(Color::Rgb(226, 232, 240)),
    ));
    Line::from(spans)
}

fn compact_session_title(choice: &WorkspaceSession) -> String {
    let raw = choice.search_title.trim();
    let raw = if raw.is_empty() {
        choice.title.trim()
    } else {
        raw
    };
    // Strip trailing `  [id]` / ` [N blocks]` noise for the list; full title stays in search_title.
    let without_bracket = raw
        .rsplit_once("  [")
        .map(|(head, _)| head)
        .unwrap_or(raw)
        .trim();
    let without_bracket = without_bracket
        .rsplit_once(" [")
        .filter(|(_, tail)| tail.ends_with(']'))
        .map(|(head, _)| head)
        .unwrap_or(without_bracket)
        .trim();
    truncate_chars(without_bracket, 64)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

fn search_position_label(search: &WorkspaceSearchView<'_>) -> Option<String> {
    let current = search.current_match?;
    Some(format!("{}/{}", current + 1, search.match_count))
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
    target: Option<WorkAt>,
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
        format!("[{position}]")
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
                Span::styled(format!("{:<12}", entry.key), theme::key_hint_style()),
                Span::styled(
                    entry.description.to_string(),
                    Style::default().fg(Color::Rgb(203, 213, 225)),
                ),
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

#[allow(clippy::too_many_arguments)]
fn render_source_list(
    frame: &mut Frame,
    area: Rect,
    sources: &[WorkspaceSource],
    selected_sources: &[bool],
    source_markers: &[SourceLoadMarker],
    loading_tick: u8,
    state: &ListState,
    active: bool,
) {
    let panel = Panel::new(WorkspaceFocus::Source.key(), "Source", active);
    let current = selected_index(state).min(sources.len().saturating_sub(1));
    let mut spans = Vec::new();
    for (idx, source) in sources.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled("  ", Style::default().fg(theme::dim())));
        }
        let selected = selected_sources.get(idx).copied().unwrap_or(false);
        let focused = idx == current && active;
        let load = source_markers
            .get(idx)
            .copied()
            .unwrap_or(SourceLoadMarker::Idle);
        let marker = load.status_glyph(selected, loading_tick);
        let marker_style = if focused {
            active_item_style()
        } else {
            match load {
                SourceLoadMarker::Failed => Style::default().fg(Color::Rgb(248, 113, 113)),
                SourceLoadMarker::Loading => Style::default().fg(theme::accent()),
                SourceLoadMarker::Ready if selected => Style::default().fg(source.color()),
                SourceLoadMarker::Idle if selected => Style::default().fg(theme::muted()),
                _ => Style::default().fg(theme::muted()),
            }
        };
        let label_style = if focused {
            active_item_style()
        } else {
            Style::default()
                .fg(source.color())
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                })
        };
        spans.push(Span::styled(format!("{marker} "), marker_style));
        spans.push(Span::styled(source.label(), label_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled("<empty>", Style::default().fg(theme::dim())));
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
            let focused = active && !has_selection && idx == cursor_idx;
            let base_style = if selected {
                selected_item_style()
            } else if focused {
                active_item_style()
            } else {
                Style::default()
            };
            let highlight = search
                .filter(|search| search.scope == WorkspaceSearchScope::Session)
                .and(search_regex);
            ListItem::new(session_row_line(
                choice, selected, active, base_style, highlight,
            ))
        })
        .collect();
    if items.is_empty() {
        items.push(ListItem::new(Span::styled(
            "<empty>",
            Style::default().fg(theme::dim()),
        )));
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
    titles: &[&str],
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
    let mut items: Vec<ListItem> = titles
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
                    "● "
                } else {
                    "○ "
                }
            } else {
                ""
            };
            let line = format!("{marker}{title}");
            let highlight = search
                .filter(|search| search.scope == WorkspaceSearchScope::Dialogue)
                .and(search_regex);
            if in_range {
                ListItem::new(Line::from(Span::styled(line, theme::range_row())))
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
                    Style::default().fg(Color::Rgb(203, 213, 225)),
                )))
            }
        })
        .collect();

    if items.is_empty() {
        items.push(ListItem::new(Span::styled(
            "<empty>",
            Style::default().fg(theme::dim()),
        )));
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
    render_list_scrollbar(frame, area, highlighted_idx, titles.len(), active);
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

/// Inner row count for a panel rect (borders already accounted).
pub(crate) fn panel_inner_rows(area: Rect) -> usize {
    panel_viewport_height(area).max(1)
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
    target: Option<WorkAt>,
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
        WorkspaceCopyParts, WorkspaceDialogue, WorkspaceSearchView, WorkspaceSource,
    };
    use crate::tui::workspace_search::WorkspaceSearchScope;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};

    #[test]
    fn can_open_dialogue_vim_accepts_sessions_when_dialogues_exist() {
        assert!(can_open_dialogue_vim(WorkspaceFocus::Sessions, 1));
        assert!(can_open_dialogue_vim(WorkspaceFocus::Dialogues, 1));
        assert!(can_open_dialogue_vim(WorkspaceFocus::Content, 1));
        assert!(!can_open_dialogue_vim(WorkspaceFocus::Sessions, 0));
    }

    #[test]
    fn content_preview_text_preserves_raw_text_without_line_number_prefixes() {
        let record = WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
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
            parts: vec![
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::UserMessage,
                    index: 1,
                    occurred_at: None,
                    label: None,
                    text: "alpha".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Output,
                    kind: sivtr_core::record::WorkPartKind::AssistantMessage,
                    index: 1,
                    occurred_at: None,
                    label: None,
                    text: "omega".to_string(),
                    ansi: None,
                },
            ],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            title: "cmd".to_string(),
            record: Some(record),
            copy: WorkspaceCopyParts::default(),
        };

        let text = workspace_content_text(&[dialogue], &[false], 0, ContentViewMode::Raw, None);
        assert!(text.contains("## Input"));
        assert!(text.contains("## Output"));
        assert!(text.contains("alpha"));
        assert!(text.contains("omega"));
        assert!(!text.contains("[r expand]"));
    }

    #[test]
    fn content_preview_text_uses_targeted_part_text_in_raw_mode() {
        let record = WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
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
            }],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            title: "cmd".to_string(),
            record: Some(record),
            copy: WorkspaceCopyParts::default(),
        };

        let text = workspace_content_text(
            &[dialogue],
            &[false],
            0,
            ContentViewMode::Raw,
            Some(WorkAt::Part {
                io: sivtr_core::record::WorkPartIo::Input,
                index: 1,
            }),
        );
        assert!(text.contains("<:tool:tool call:>"));
        assert!(text.contains("hidden tool call"));
        assert!(text.contains("<:/tool:tool call:>"));
    }

    #[test]
    fn content_preview_text_uses_structured_targeted_part_text_in_reading_mode() {
        let record = WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
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
            }],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            title: "cmd".to_string(),
            record: Some(record),
            copy: WorkspaceCopyParts::default(),
        };

        let text = workspace_content_text(
            &[dialogue],
            &[false],
            0,
            ContentViewMode::Reading,
            Some(WorkAt::Part {
                io: sivtr_core::record::WorkPartIo::Input,
                index: 1,
            }),
        );

        // Reading folds structure to one open marker only.
        assert_eq!(text.trim(), "<:tool:tool call:>");
        assert!(!text.contains("hidden tool call"));
        assert!(!text.contains("codex/session"));
        assert!(!text.contains("[r expand]"));
    }

    #[test]
    fn reading_mode_folds_structure_and_raw_expands() {
        let record = WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
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
                    text: "question".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::ToolCall,
                    index: 2,
                    occurred_at: None,
                    label: Some("Bash".to_string()),
                    text: "cargo test".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Output,
                    kind: sivtr_core::record::WorkPartKind::ToolOutput,
                    index: 1,
                    occurred_at: None,
                    label: Some("Bash".to_string()),
                    text: "ok".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Output,
                    kind: sivtr_core::record::WorkPartKind::AssistantMessage,
                    index: 2,
                    occurred_at: None,
                    label: None,
                    text: "answer".to_string(),
                    ansi: None,
                },
            ],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            title: "cmd".to_string(),
            record: Some(record),
            copy: WorkspaceCopyParts::default(),
        };

        let reading = workspace_content_text(
            std::slice::from_ref(&dialogue),
            &[false],
            0,
            ContentViewMode::Reading,
            None,
        );
        assert!(reading.contains("## Input"));
        assert!(reading.contains("## Output"));
        assert!(reading.contains("question"));
        // Single structure part keeps the detailed open marker.
        assert!(reading.contains("<:tool:Bash call:>"));
        assert!(reading.contains("<:tool:Bash result:>"));
        assert!(reading.contains("answer"));
        assert!(!reading.contains("cargo test"));
        assert!(!reading.contains("codex/session"));
        assert!(!reading.contains("## User"));
        assert!(!reading.contains("[r expand]"));

        let raw = workspace_content_text(&[dialogue], &[false], 0, ContentViewMode::Raw, None);
        assert!(raw.contains("## Input"));
        assert!(raw.contains("## Output"));
        assert!(raw.contains("question"));
        assert!(raw.contains("cargo test"));
        assert!(raw.contains("<:tool:Bash call:>"));
        assert!(raw.contains("<:/tool:Bash call:>"));
        assert!(raw.contains("<:tool:Bash result:>"));
        assert!(raw.contains("ok"));
        assert!(raw.contains("answer"));
        assert!(!raw.contains("codex/session"));
        assert!(!raw.contains("## User"));
    }

    #[test]
    fn reading_mode_collapses_adjacent_structure_runs() {
        let record = WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
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
            parts: vec![
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::UserMessage,
                    index: 1,
                    occurred_at: None,
                    label: None,
                    text: "do it".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::ToolCall,
                    index: 2,
                    occurred_at: None,
                    label: Some("Bash".to_string()),
                    text: "ls".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::ToolCall,
                    index: 3,
                    occurred_at: None,
                    label: Some("Read".to_string()),
                    text: "file".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::Skill,
                    index: 4,
                    occurred_at: None,
                    label: Some("review".to_string()),
                    text: "skill body".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::Skill,
                    index: 5,
                    occurred_at: None,
                    label: Some("deploy".to_string()),
                    text: "skill body 2".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Output,
                    kind: sivtr_core::record::WorkPartKind::AssistantMessage,
                    index: 1,
                    occurred_at: None,
                    label: None,
                    text: "done".to_string(),
                    ansi: None,
                },
            ],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            title: "cmd".to_string(),
            record: Some(record),
            copy: WorkspaceCopyParts::default(),
        };

        let reading = workspace_content_text(
            std::slice::from_ref(&dialogue),
            &[false],
            0,
            ContentViewMode::Reading,
            None,
        );
        assert!(reading.contains("## Input"));
        assert!(reading.contains("do it"));
        // Original open markers kept (not generic <:tool:> / <:skill:>).
        assert!(reading.contains("<:tool:Bash call:>"));
        assert!(reading.contains("<:tool:Read call:>"));
        assert!(reading.contains("<:skill:review:>"));
        assert!(reading.contains("<:skill:deploy:>"));
        // Same IO section: all structure markers share one fold line.
        let fold_line = reading
            .lines()
            .find(|line| line.contains("<:tool:Bash call:>"))
            .expect("collapsed structure line");
        assert!(fold_line.contains("<:tool:Read call:>"));
        assert!(fold_line.contains("<:skill:review:>"));
        assert!(fold_line.contains("<:skill:deploy:>"));
        assert!(reading.contains("## Output"));
        assert!(reading.contains("done"));
        assert!(!reading.contains("ls"));
        assert!(!reading.contains("skill body"));
    }

    #[test]
    fn reading_mode_counts_identical_structure_markers_regardless_of_order() {
        let record = WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
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
            parts: vec![
                // Interleaved with dialogue — still one IO-section fold, same markers count.
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::ToolCall,
                    index: 1,
                    occurred_at: None,
                    label: Some("Bash".to_string()),
                    text: "ls".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::UserMessage,
                    index: 2,
                    occurred_at: None,
                    label: None,
                    text: "middle note".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::ToolCall,
                    index: 3,
                    occurred_at: None,
                    label: Some("Read".to_string()),
                    text: "file".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::ToolCall,
                    index: 4,
                    occurred_at: None,
                    label: Some("Bash".to_string()),
                    text: "pwd".to_string(),
                    ansi: None,
                },
                sivtr_core::record::WorkPart {
                    io: sivtr_core::record::WorkPartIo::Input,
                    kind: sivtr_core::record::WorkPartKind::ToolCall,
                    index: 5,
                    occurred_at: None,
                    label: Some("Bash".to_string()),
                    text: "date".to_string(),
                    ansi: None,
                },
            ],
        };
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            title: "cmd".to_string(),
            record: Some(record),
            copy: WorkspaceCopyParts::default(),
        };

        let reading = workspace_content_text(
            std::slice::from_ref(&dialogue),
            &[false],
            0,
            ContentViewMode::Reading,
            None,
        );
        // Identical markers in the same IO section merge even when not adjacent.
        assert!(reading.contains("<:tool:Bash call:> x3"));
        assert!(reading.contains("<:tool:Read call:>"));
        assert!(reading.contains("middle note"));
        assert!(!reading.contains("ls"));
        assert!(!reading.contains("pwd"));
        // Single fold line for the section (not split by the dialogue part).
        let fold_hits = reading
            .lines()
            .filter(|line| line.contains("<:tool:Bash call:>"))
            .count();
        assert_eq!(fold_hits, 1);
    }

    #[test]
    fn content_title_includes_view_mode() {
        assert_eq!(
            content_title(ContentViewMode::Reading, &[false, false], None),
            "Content (read/fold)"
        );
        assert_eq!(
            content_title(ContentViewMode::Raw, &[true, false], None),
            "Content (raw/full): 1 dialogue selected"
        );
    }

    #[test]
    fn content_title_includes_current_dialogue_ref() {
        let work_ref = WorkRef::agent(AgentProvider::Codex, "session", 2);

        assert_eq!(
            content_title(ContentViewMode::Reading, &[false], Some(&work_ref)),
            "Content (read/fold) [codex/session/2]"
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
                source: WorkspaceSource::agent(AgentProvider::Codex),
                work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
                title: "first".to_string(),
                record: None,
                copy: WorkspaceCopyParts::default(),
            },
            WorkspaceDialogue {
                source: WorkspaceSource::agent(AgentProvider::Codex),
                work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 2)),
                title: "second".to_string(),
                record: None,
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
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 2)),
            title: "second".to_string(),
            record: None,
            copy: WorkspaceCopyParts::default(),
        }];

        let current = super::current_content_ref(
            &dialogues,
            &[false],
            0,
            Some(WorkAt::Part {
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
