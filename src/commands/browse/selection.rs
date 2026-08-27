//! Hierarchical refresh and the source-preset selections.
//!
//! Active rows = marked rows if any, otherwise the cursor row (see
//! [`crate::tui::workspace::active_rows`]). `R` reloads the next hierarchy
//! level under those rows.

use crate::tui::workspace::{Rows, WorkspaceFocus, WorkspaceSession, WorkspaceSource};

use super::load::SessionColumn;
use crate::pane::Viewport;

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
pub(super) fn refresh_next_level(
    focus: WorkspaceFocus,
    rows: &Rows,
    sessions: &[WorkspaceSession],
    sessions_pane: &mut SessionColumn,
    all_sessions: &mut Vec<WorkspaceSession>,
    search_dirty: &mut bool,
    viewport: Viewport,
) {
    let sources = sessions_pane.sources();
    let sources_to_reload = match focus {
        WorkspaceFocus::Source => rows.source.active_mask(),
        WorkspaceFocus::Sessions | WorkspaceFocus::Dialogues => {
            parent_source_mask(sources, sessions, &rows.sessions.active())
        }
        WorkspaceFocus::Content => return,
    };

    if !sources_to_reload.iter().any(|selected| *selected) {
        return;
    }

    sessions_pane.refresh(&sources_to_reload, viewport);
    // Meta list only; search rebuild (with bodies) happens on search_dirty in picker.
    *all_sessions = sessions_pane.collect(rows.source.mask());
    *search_dirty = true;
}

fn parent_source_mask(
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    active_sessions: &[usize],
) -> Vec<bool> {
    let mut parent = vec![false; sources.len()];
    for session in active_sessions.iter().filter_map(|row| sessions.get(*row)) {
        if let Some(source_idx) = sources.iter().position(|source| source == &session.source) {
            parent[source_idx] = true;
        }
    }
    parent
}

#[derive(Clone, Copy)]
pub(super) enum WorkspaceSourceSelection {
    Agents,
    Terminal,
}

pub(super) fn select_sources(
    sources: &[WorkspaceSource],
    selected_sources: &mut [bool],
    selection: WorkspaceSourceSelection,
) {
    // The mask is normally built from `sources`, but a length mismatch must
    // not index out of bounds in release builds (debug_assert! is compiled
    // out): apply over the overlap and clear any stale flags beyond it.
    let overlap = sources.len().min(selected_sources.len());
    for (idx, source) in sources.iter().take(overlap).enumerate() {
        selected_sources[idx] = match selection {
            WorkspaceSourceSelection::Agents => source.is_agent(),
            WorkspaceSourceSelection::Terminal => source.is_terminal(),
        };
    }
    for flag in &mut selected_sources[overlap..] {
        *flag = false;
    }
}

pub(super) fn source_records(
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    session_records: &[Vec<sivtr_core::record::WorkRecord>],
    source_idx: usize,
) -> Vec<sivtr_core::record::WorkRecord> {
    let Some(source) = sources.get(source_idx) else {
        return Vec::new();
    };
    sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| &session.source == source)
        .flat_map(|(idx, _)| session_records.get(idx).into_iter().flatten().cloned())
        .collect()
}

/// Materialize records covered by marked source/session scopes. Scope masks
/// drive loading; once bodies arrive, the canonical WorkSet owns the choice.
pub(super) fn resolve_loaded_scopes(
    rows: &mut Rows,
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    sessions_pane: &SessionColumn,
) {
    for (session_idx, session) in sessions.iter().enumerate() {
        let source_idx = sources.iter().position(|source| source == &session.source);
        let source_marked = source_idx
            .and_then(|idx| rows.source.mask().get(idx))
            .copied()
            .unwrap_or(false);
        let session_marked = rows
            .sessions
            .mask()
            .get(session_idx)
            .copied()
            .unwrap_or(false);
        if source_marked || session_marked {
            if let Some(records) = sessions_pane.body_for(session) {
                rows.selection.include_records(records.iter().cloned());
            }
        }
    }
}
