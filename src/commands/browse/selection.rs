//! Active-row selection and hierarchical refresh.
//!
//! Active rows = multi-selected rows if any, otherwise the focused row.
//! `R` reloads the next hierarchy level under those rows.

use ratatui::widgets::ListState;

use crate::tui::workspace::{selected_index, WorkspaceFocus, WorkspaceSession, WorkspaceSource};

use super::load::SessionColumn;
use crate::pane::Viewport;

/// Active rows: multi-select if any, otherwise the focused row.
pub(super) fn active_mask(selected: &[bool], focus_idx: usize, len: usize) -> Vec<bool> {
    assert_eq!(
        selected.len(),
        len,
        "selection mask length must match list length"
    );
    if selected.iter().any(|selected| *selected) {
        return selected.to_vec();
    }
    if len == 0 {
        return Vec::new();
    }
    let mut out = vec![false; len];
    out[focus_idx.min(len - 1)] = true;
    out
}

/// Refresh the next level under active rows of the focused pane.
///
/// | Focus     | Active rows        | Reloads                         |
/// |-----------|--------------------|---------------------------------|
/// | Source    | sources            | those sources (sessions update) |
/// | Sessions  | sessions           | parent sources (dialogues/records update) |
/// | Dialogues | sessions (parents) | parent sources (dialogue list update) |
/// | Content   | —                  | no-op (content is in-memory)    |
///
/// Dialogue content is derived from session records, so session/dialogue refresh
/// re-queries parent sources. There is no separate dialogue transport.
#[allow(clippy::too_many_arguments)]
pub(super) fn refresh_next_level(
    focus: WorkspaceFocus,
    selected_sources: &[bool],
    source_state: &ListState,
    sessions: &[WorkspaceSession],
    selected_sessions: &[bool],
    session_state: &ListState,
    sessions_pane: &mut SessionColumn,
    all_sessions: &mut Vec<WorkspaceSession>,
    search_dirty: &mut bool,
    viewport: Viewport,
) {
    let sources = sessions_pane.sources();
    let sources_to_reload = match focus {
        WorkspaceFocus::Source => active_mask(
            selected_sources,
            selected_index(source_state),
            sources.len(),
        ),
        WorkspaceFocus::Sessions | WorkspaceFocus::Dialogues => parent_source_mask(
            sources,
            sessions,
            &active_mask(
                selected_sessions,
                selected_index(session_state),
                sessions.len(),
            ),
        ),
        WorkspaceFocus::Content => return,
    };

    if !sources_to_reload.iter().any(|selected| *selected) {
        return;
    }

    sessions_pane.refresh(&sources_to_reload, viewport);
    // Meta list only; search rebuild (with bodies) happens on search_dirty in picker.
    *all_sessions = sessions_pane.collect(selected_sources);
    *search_dirty = true;
}

fn parent_source_mask(
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    active_sessions: &[bool],
) -> Vec<bool> {
    let mut parent = vec![false; sources.len()];
    for (session_idx, session) in sessions.iter().enumerate() {
        if !active_sessions.get(session_idx).copied().unwrap_or(false) {
            continue;
        }
        if let Some(source_idx) = sources.iter().position(|source| source == &session.source) {
            parent[source_idx] = true;
        }
    }
    parent
}

#[derive(Clone, Copy)]
pub(super) enum WorkspaceSourceSelection {
    All,
    Agents,
    Terminal,
}

pub(super) fn select_sources(
    sources: &[WorkspaceSource],
    selected_sources: &mut [bool],
    selection: WorkspaceSourceSelection,
) {
    assert_eq!(sources.len(), selected_sources.len());
    for (idx, source) in sources.iter().enumerate() {
        selected_sources[idx] = match selection {
            WorkspaceSourceSelection::All => true,
            WorkspaceSourceSelection::Agents => source.is_agent(),
            WorkspaceSourceSelection::Terminal => source.is_terminal(),
        };
    }
}

pub(super) fn has_selected_sessions(selected_sessions: &[bool]) -> bool {
    selected_sessions.iter().any(|selected| *selected)
}
