//! Active-row selection and hierarchical refresh.
//!
//! Active rows = multi-selected rows if any, otherwise the focused row.
//! `R` reloads the next hierarchy level under those rows.

use ratatui::widgets::ListState;

use crate::tui::workspace::{
    selected_index, selected_indices, WorkspaceFocus, WorkspaceSession, WorkspaceSource,
};

use super::load::SessionColumn;
use super::nav::RangeAnchor;
use crate::pane::{toggle_row_ids, Viewport};

/// Rows a pane-wide action applies to: every selected row in row order, or
/// the focused row alone when nothing is selected. `len` is the pane's row
/// count, so a stale focus clamps to the last row and an empty pane yields
/// nothing. The one answer to "which rows does this act on" — refresh, copy,
/// and dialogue projection all ask it.
pub(super) fn active_rows(selected: &[bool], focus_idx: usize, len: usize) -> Vec<usize> {
    let rows = selected_indices(selected);
    if !rows.is_empty() {
        return rows;
    }
    match len {
        0 => Vec::new(),
        len => vec![focus_idx.min(len - 1)],
    }
}

/// [`active_rows`] as a mask, for the transports that reload by mask.
fn active_mask(selected: &[bool], focus_idx: usize, len: usize) -> Vec<bool> {
    let mut mask = vec![false; len];
    for row in active_rows(selected, focus_idx, len) {
        if let Some(flag) = mask.get_mut(row) {
            *flag = true;
        }
    }
    mask
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
            &active_rows(
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
    All,
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
            WorkspaceSourceSelection::All => true,
            WorkspaceSourceSelection::Agents => source.is_agent(),
            WorkspaceSourceSelection::Terminal => source.is_terminal(),
        };
    }
    for flag in &mut selected_sources[overlap..] {
        *flag = false;
    }
}

pub(super) fn has_selected_sessions(selected_sessions: &[bool]) -> bool {
    selected_sessions.iter().any(|selected| *selected)
}

/// The selection mask `focus` owns, or `None` for Content — it marks blocks
/// instead of rows. One lookup for every row-selection key (Space, `v`), so
/// the panes stop repeating "which mask goes with which focus".
pub(super) fn focused_mask<'a>(
    focus: WorkspaceFocus,
    selected_sources: &'a mut [bool],
    selected_sessions: &'a mut [bool],
    selected_dialogues: &'a mut [bool],
) -> Option<&'a mut [bool]> {
    match focus {
        WorkspaceFocus::Source => Some(selected_sources),
        WorkspaceFocus::Sessions => Some(selected_sessions),
        WorkspaceFocus::Dialogues => Some(selected_dialogues),
        WorkspaceFocus::Content => None,
    }
}

/// Range-select rows: first `v` anchors at `idx`, the next completes the
/// span between anchor and `idx`. `true` when this press completed a span,
/// so the caller can rebuild the panes below it. The span rule itself is
/// [`toggle_row_ids`], shared with the content pane's block range.
/// Shared by every list pane (Source/Sessions/Dialogues).
pub(super) fn apply_range_selection(
    range_anchor: &mut RangeAnchor,
    pane: WorkspaceFocus,
    selected: &mut [bool],
    idx: usize,
) -> bool {
    let Some(span) = range_anchor.span(pane, idx) else {
        return false;
    };
    // Reject out-of-bounds endpoints before iterating: a stray large
    // index must not walk a near-empty mask for the whole span.
    if *span.end() < selected.len() {
        toggle_row_ids(selected, span);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::nav::RangeAnchor;
    use super::apply_range_selection;
    use crate::tui::workspace::WorkspaceFocus;

    const PANE: WorkspaceFocus = WorkspaceFocus::Dialogues;

    #[test]
    fn first_v_anchors_second_v_selects_span() {
        let mut anchor = RangeAnchor::default();
        let mut selected = [false; 5];

        // First `v` only anchors; nothing is selected yet.
        assert!(!apply_range_selection(&mut anchor, PANE, &mut selected, 4));
        assert_eq!(anchor.get(PANE), Some(4));
        assert!(!selected.iter().any(|flag| *flag));

        // Moving the cursor does not disturb the anchor.
        assert!(apply_range_selection(&mut anchor, PANE, &mut selected, 1));
        assert_eq!(anchor.get(PANE), None);
        // Span 1..=4 toggled on; row 0 untouched.
        assert!(!selected[0]);
        assert!(selected[1..].iter().all(|flag| *flag));
    }

    #[test]
    fn a_range_never_completes_against_another_panes_anchor() {
        let mut anchor = RangeAnchor::default();
        let mut selected = [false; 5];

        apply_range_selection(&mut anchor, WorkspaceFocus::Sessions, &mut selected, 4);
        // The Dialogues pane sees no open range: its `v` anchors instead of
        // completing the session pane's span.
        assert!(!apply_range_selection(&mut anchor, PANE, &mut selected, 1));
        assert!(!selected.iter().any(|flag| *flag));
        assert_eq!(anchor.get(WorkspaceFocus::Sessions), None);
        assert_eq!(anchor.get(PANE), Some(1));
    }

    #[test]
    fn second_v_inverts_an_already_selected_span() {
        let mut anchor = RangeAnchor::default();
        let mut selected = [true, true, false, false, true];

        apply_range_selection(&mut anchor, PANE, &mut selected, 4);
        apply_range_selection(&mut anchor, PANE, &mut selected, 2);
        // Span 2..=4 is currently false/false/true (mixed) → select all.
        assert!(selected[2..].iter().all(|flag| *flag));

        apply_range_selection(&mut anchor, PANE, &mut selected, 4);
        apply_range_selection(&mut anchor, PANE, &mut selected, 2);
        // Span is now all true → deselect all.
        assert!(selected[0]);
        assert!(selected[2..=4].iter().all(|flag| !*flag));
    }
}
