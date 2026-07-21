//! Workspace browser domain types (sources, sessions, dialogues, view state).

use ratatui::prelude::Color;
use ratatui::widgets::ListState;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};
use std::time::SystemTime;

use crate::commands::select::CommandSelection;
use crate::tui::content::io::{ContentIoFocus, ContentIoTexts, ContentScrolls};
use crate::tui::content::text::{content_io_from_record, raw_part_text, structured_part_text};
use crate::tui::content::view::{ContentSelection, ContentViewMode};
use crate::tui::search::WorkspaceSearchScope;
use crate::tui::theme;

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
        self.content_io_texts(mode, target).join_displayed()
    }

    /// Input / Output bodies for the dual content panes (no section headers).
    pub(crate) fn content_io_texts(
        &self,
        mode: ContentViewMode,
        target: Option<WorkAt>,
    ) -> ContentIoTexts {
        if let Some(target @ WorkAt::Part { .. }) = target {
            let text = match mode {
                ContentViewMode::Raw => self.targeted_plain_text(target),
                ContentViewMode::Reading => self.targeted_structured_text(target),
            }
            .unwrap_or_else(|| "<empty>".to_string());
            // Targeted part lives in its own IO half; the other stays empty.
            let Some(record) = self.record.as_ref() else {
                return ContentIoTexts {
                    input: text,
                    output: String::new(),
                };
            };
            let Some(part) = record.part_for_at(target) else {
                return ContentIoTexts {
                    input: text,
                    output: String::new(),
                };
            };
            return match part.io {
                sivtr_core::record::WorkPartIo::Input => ContentIoTexts {
                    input: text,
                    output: String::new(),
                },
                sivtr_core::record::WorkPartIo::Output => ContentIoTexts {
                    input: String::new(),
                    output: text,
                },
            };
        }

        let Some(record) = self.record.as_ref() else {
            return ContentIoTexts {
                input: "<empty>".to_string(),
                output: String::new(),
            };
        };
        if record.parts.is_empty() {
            return ContentIoTexts {
                input: "<empty>".to_string(),
                output: String::new(),
            };
        }
        let reading = matches!(mode, ContentViewMode::Reading);
        content_io_from_record(record, reading)
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
        let part = self.record.as_ref()?.part_for_at(target)?;
        if part.kind.is_structure() {
            return Some(raw_part_text(part));
        }
        Some(part.text.clone())
    }

    fn targeted_structured_text(&self, target: WorkAt) -> Option<String> {
        let WorkAt::Part { .. } = target else {
            return None;
        };
        let part = self.record.as_ref()?.part_for_at(target)?;
        Some(structured_part_text(part))
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
    pub(crate) content_scrolls: ContentScrolls,
    pub(crate) content_io_focus: ContentIoFocus,
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

pub(crate) struct WorkspaceFooterView<'a> {
    pub(crate) focus: WorkspaceFocus,
    pub(crate) show_help: bool,
    pub(crate) search: Option<&'a WorkspaceSearchView<'a>>,
    pub(crate) line_filter_input_open: bool,
    pub(crate) line_filter: Option<&'a str>,
    pub(crate) line_filter_error: Option<&'a str>,
    pub(crate) fullscreen: Option<WorkspaceFocus>,
    pub(crate) content_mode: ContentViewMode,
    pub(crate) content_selection: Option<ContentSelection>,
    pub(crate) current_ref: Option<&'a WorkRef>,
}
