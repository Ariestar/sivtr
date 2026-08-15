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

/// Range-select rows: first `v` anchors at `idx`, the next completes the
/// span between anchor and `idx` (inverting the span if any row is unselected).
/// Shared by every list pane (Source/Sessions/Dialogues).
pub(super) fn apply_range_selection(
    range_anchor: &mut Option<usize>,
    selected: &mut [bool],
    idx: usize,
) {
    if let Some(anchor) = range_anchor.take() {
        let start = anchor.min(idx);
        let end = anchor.max(idx);
        let select = selected
            .get(start..=end)
            .map(|range| range.iter().any(|selected| !selected))
            .unwrap_or(true);
        for i in start..=end {
            if let Some(flag) = selected.get_mut(i) {
                *flag = select;
            }
        }
    } else {
        *range_anchor = Some(idx);
    }
}

#[cfg(test)]
mod tests {
    use super::apply_range_selection;

    #[test]
    fn first_v_anchors_second_v_selects_span() {
        let mut anchor = None;
        let mut selected = [false; 5];

        // First `v` only anchors; nothing is selected yet.
        apply_range_selection(&mut anchor, &mut selected, 4);
        assert_eq!(anchor, Some(4));
        assert!(!selected.iter().any(|flag| *flag));

        // Moving the cursor does not disturb the anchor.
        apply_range_selection(&mut anchor, &mut selected, 1);
        assert_eq!(anchor, None);
        // Span 1..=4 toggled on; row 0 untouched.
        assert!(!selected[0]);
        assert!(selected[1..].iter().all(|flag| *flag));
    }

    #[test]
    fn second_v_inverts_an_already_selected_span() {
        let mut anchor = None;
        let mut selected = [true, true, false, false, true];

        apply_range_selection(&mut anchor, &mut selected, 4);
        apply_range_selection(&mut anchor, &mut selected, 2);
        // Span 2..=4 is currently false/false/true (mixed) → select all.
        assert!(selected[2..].iter().all(|flag| *flag));

        apply_range_selection(&mut anchor, &mut selected, 4);
        apply_range_selection(&mut anchor, &mut selected, 2);
        // Span is now all true → deselect all.
        assert!(selected[0]);
        assert!(selected[2..=4].iter().all(|flag| !*flag));
    }
}
