//! List-pane row state: cursor, marks, and the live range anchor.
//!
//! Three panes (Source / Sessions / Dialogues) are lists of rows. Each keeps a
//! cursor, a mark mask, and its own `v` anchor, and they never move apart:
//! every row key needs cursor and mask both, and a change in the row count
//! clamps the one, resizes the other, and voids the anchor. [`ListPane`] holds
//! them together — which is what lets the row helpers drop their `len`
//! parameter, since the mask's length *is* the pane's row count.
//!
//! [`Rows`] holds the three panes plus Content's block anchor, so "the focused
//! pane's rows" is one lookup instead of a four-arm match at every call site.

use std::ops::RangeInclusive;

use ratatui::widgets::ListState;

use super::model::{selected_indices, WorkspaceFocus};
use crate::pane::Selection;
use crate::workset::WorkSet;

/// Rows a pane-wide action applies to: every marked row in row order, or the
/// cursor row alone when nothing is marked. `len` is the row count, so a stale
/// cursor clamps to the last row and an empty pane yields nothing. The one
/// answer to "which rows does this act on" — refresh, copy, and the dialogue
/// projection all ask it, the last two against masks that are not a pane's
/// (dialogue rows against materialized dialogues, session rows against a
/// search result), which is why it stays a free function.
pub(crate) fn active_rows(marked: &[bool], cursor: usize, len: usize) -> Vec<usize> {
    let rows = selected_indices(marked);
    if !rows.is_empty() {
        return rows;
    }
    match len {
        0 => Vec::new(),
        len => vec![cursor.min(len - 1)],
    }
}

/// One `v` press over `row`: opens a range and yields nothing, or closes the
/// one already open and yields the span it covers. The single range rule — list
/// rows and Content block ids both go through it.
fn range_step(anchor: &mut Option<usize>, row: usize) -> Option<RangeInclusive<usize>> {
    match anchor.take() {
        Some(start) => Some(start.min(row)..=start.max(row)),
        None => {
            *anchor = Some(row);
            None
        }
    }
}

/// One list pane's cursor, row marks, and open `v` anchor.
#[derive(Default)]
pub(crate) struct ListPane {
    state: ListState,
    marks: Selection,
    anchor: Option<usize>,
}

impl ListPane {
    /// A pane over `marks` — the caller already knows which rows start marked
    /// (source presets); its length is the row count.
    pub(crate) fn with_marks(marks: Vec<bool>) -> Self {
        let mut pane = Self {
            state: ListState::default(),
            marks: marks.into(),
            anchor: None,
        };
        pane.clamp();
        pane
    }

    pub(crate) fn len(&self) -> usize {
        self.marks.mask().len()
    }

    /// Cursor row, clamped to the row count (0 when the pane is empty).
    pub(crate) fn cursor(&self) -> usize {
        self.state
            .selected()
            .unwrap_or(0)
            .min(self.len().saturating_sub(1))
    }

    /// Move the cursor to `idx`, clamped; an empty pane has no cursor.
    pub(crate) fn select(&mut self, idx: usize) {
        let len = self.len();
        self.state
            .select((len > 0).then(|| idx.min(len.saturating_sub(1))));
    }

    /// Scroll offset of the rendered list.
    pub(crate) fn offset(&self) -> usize {
        self.state.offset()
    }

    /// List state for the paint; the widget owns the offset.
    pub(crate) fn state(&self) -> &ListState {
        &self.state
    }

    pub(crate) fn mask(&self) -> &[bool] {
        self.marks.mask()
    }

    /// Mask for the policies that set flags in place (source presets).
    pub(crate) fn mask_mut(&mut self) -> &mut [bool] {
        self.marks.mask_mut()
    }

    pub(crate) fn marked(&self) -> usize {
        self.marks.count()
    }

    pub(crate) fn has_marks(&self) -> bool {
        self.marks.any()
    }

    /// [`active_rows`] for this pane.
    pub(crate) fn active(&self) -> Vec<usize> {
        active_rows(self.mask(), self.cursor(), self.len())
    }

    /// [`Self::active`] as a mask, for the transports that reload by mask.
    pub(crate) fn active_mask(&self) -> Vec<bool> {
        let mut mask = vec![false; self.len()];
        for row in self.active() {
            if let Some(flag) = mask.get_mut(row) {
                *flag = true;
            }
        }
        mask
    }

    pub(crate) fn toggle(&mut self, idx: usize) {
        self.marks.toggle(idx);
    }

    /// Mark every row, or clear them all when they are already marked — the
    /// same one-state rule a `v` range follows, over the whole pane.
    pub(crate) fn toggle_all(&mut self) {
        let len = self.len();
        self.marks.toggle_ids(0..len);
    }

    /// Step the cursor one row (`up` or down); `false` when it did not move,
    /// so bumping the first or last row never invalidates the panes below.
    pub(crate) fn step(&mut self, up: bool) -> bool {
        let len = self.len();
        if len == 0 {
            self.state.select(None);
            return false;
        }
        let current = self.cursor();
        let next = if up {
            current.saturating_sub(1)
        } else {
            (current + 1).min(len - 1)
        };
        if next == current {
            return false;
        }
        self.state.select(Some(next));
        true
    }

    /// Normalize the cursor against the current row count: an empty pane has
    /// none, a non-empty pane always has one (row 0 before the first move).
    fn clamp(&mut self) {
        let cursor = self.cursor();
        self.select(cursor);
    }

    /// The row count changed under the pane: marks no longer name the same
    /// rows, so drop them, clamp the cursor, and void an open range.
    pub(crate) fn fit(&mut self, len: usize) {
        if self.len() == len {
            self.clamp();
            return;
        }
        self.marks.reset(len);
        self.clamp();
        self.anchor = None;
    }

    /// The pane was rebuilt from its parent's selection: nothing it held still
    /// means anything, and the cursor starts at the first row.
    pub(crate) fn reset(&mut self, len: usize) {
        self.marks.reset(len);
        self.state.select((len > 0).then_some(0));
        self.anchor = None;
    }

    /// `v` over list rows: first press anchors at the cursor, the next marks
    /// the span between anchor and cursor to one state. `true` when this press
    /// completed a span, so the caller can rebuild the panes below it.
    #[cfg(test)]
    pub(crate) fn range_select(&mut self) -> bool {
        let cursor = self.cursor();
        let Some(span) = range_step(&mut self.anchor, cursor) else {
            return false;
        };
        // Reject out-of-bounds endpoints before iterating: a stray large index
        // must not walk a near-empty mask for the whole span.
        if *span.end() < self.len() {
            self.marks.toggle_ids(span);
        }
        true
    }
}

/// The three list panes and Content's block-range anchor.
///
/// Every list pane owns its own `v` anchor, so a stale one can never complete a
/// span in another pane; only one is ever open at a time because moving focus
/// closes them all. Content marks blocks rather than rows, so its anchor is a
/// block id and lives here instead of in a pane.
pub(crate) struct Rows {
    pub(crate) source: ListPane,
    pub(crate) sessions: ListPane,
    pub(crate) dialogues: ListPane,
    /// The only content-selection state. List and block marks are derived
    /// views; cursor/range state never decides what copy or MCP receives.
    pub(crate) selection: WorkSet,
    content_anchor: Option<usize>,
}

impl Default for Rows {
    fn default() -> Self {
        Self {
            source: ListPane::default(),
            sessions: ListPane::default(),
            dialogues: ListPane::default(),
            selection: WorkSet::new(".", Vec::new()),
            content_anchor: None,
        }
    }
}

impl Rows {
    /// The pane `focus` names, or `None` for Content — it has no rows, it
    /// marks blocks.
    pub(crate) fn pane(&self, focus: WorkspaceFocus) -> Option<&ListPane> {
        match focus {
            WorkspaceFocus::Source => Some(&self.source),
            WorkspaceFocus::Sessions => Some(&self.sessions),
            WorkspaceFocus::Dialogues => Some(&self.dialogues),
            WorkspaceFocus::Content => None,
        }
    }

    pub(crate) fn pane_mut(&mut self, focus: WorkspaceFocus) -> Option<&mut ListPane> {
        match focus {
            WorkspaceFocus::Source => Some(&mut self.source),
            WorkspaceFocus::Sessions => Some(&mut self.sessions),
            WorkspaceFocus::Dialogues => Some(&mut self.dialogues),
            WorkspaceFocus::Content => None,
        }
    }

    fn anchor_mut(&mut self, focus: WorkspaceFocus) -> &mut Option<usize> {
        match focus {
            WorkspaceFocus::Source => &mut self.source.anchor,
            WorkspaceFocus::Sessions => &mut self.sessions.anchor,
            WorkspaceFocus::Dialogues => &mut self.dialogues.anchor,
            WorkspaceFocus::Content => &mut self.content_anchor,
        }
    }

    /// Row an open range started at — the range highlight, and the endpoint the
    /// next `v` completes against.
    pub(crate) fn range_start(&self, focus: WorkspaceFocus) -> Option<usize> {
        self.pane(focus)
            .map_or(self.content_anchor, |pane| pane.anchor)
    }

    pub(crate) fn close_range(&mut self, focus: WorkspaceFocus) {
        *self.anchor_mut(focus) = None;
    }

    pub(crate) fn close_ranges(&mut self) {
        self.source.anchor = None;
        self.sessions.anchor = None;
        self.dialogues.anchor = None;
        self.content_anchor = None;
    }

    /// One `v` press on `row` of `focus`. Rows are list rows for the three list
    /// panes, block ids for Content.
    pub(crate) fn range(
        &mut self,
        focus: WorkspaceFocus,
        row: usize,
    ) -> Option<RangeInclusive<usize>> {
        range_step(self.anchor_mut(focus), row)
    }

    /// [`ListPane::range_select`] for the focused list pane; Content marks
    /// blocks through [`Self::range`] instead.
    #[cfg(test)]
    pub(crate) fn range_select(&mut self, focus: WorkspaceFocus) -> bool {
        self.pane_mut(focus).is_some_and(ListPane::range_select)
    }
}

#[cfg(test)]
mod tests {
    use super::{ListPane, Rows};
    use crate::tui::workspace::WorkspaceFocus;

    const PANE: WorkspaceFocus = WorkspaceFocus::Dialogues;

    fn rows_of(marks: Vec<bool>) -> Rows {
        Rows {
            dialogues: ListPane::with_marks(marks),
            ..Rows::default()
        }
    }

    #[test]
    fn cursor_is_clamped_to_the_row_count() {
        let mut pane = ListPane::with_marks(vec![false; 3]);
        // A non-empty pane always has a cursor, before any move.
        assert_eq!(pane.cursor(), 0);
        pane.select(9);
        assert_eq!(pane.cursor(), 2);
        // Steps stop at the ends and report that they changed nothing.
        assert!(!pane.step(false));
        assert!(pane.step(true));
        assert_eq!(pane.cursor(), 1);
    }

    #[test]
    fn active_rows_are_the_marks_or_the_cursor_alone() {
        let mut pane = ListPane::with_marks(vec![false; 4]);
        pane.select(2);
        assert_eq!(pane.active(), vec![2]);
        pane.toggle(0);
        pane.toggle(3);
        assert_eq!(pane.active(), vec![0, 3]);
        assert_eq!(pane.active_mask(), vec![true, false, false, true]);
    }

    #[test]
    fn fit_keeps_the_rows_and_reset_starts_over() {
        let mut pane = ListPane::with_marks(vec![false; 4]);
        pane.select(3);
        pane.toggle(3);
        // Same length: marks and cursor survive.
        pane.fit(4);
        assert_eq!(pane.marked(), 1);
        assert_eq!(pane.cursor(), 3);
        // Shorter list: marks named other rows, so they go; the cursor clamps.
        pane.fit(2);
        assert_eq!(pane.marked(), 0);
        assert_eq!(pane.cursor(), 1);
        // Reset returns to the first row of a freshly built list.
        pane.reset(5);
        assert_eq!(pane.cursor(), 0);
        assert_eq!(pane.len(), 5);
        // An empty pane has no cursor at all.
        pane.reset(0);
        assert_eq!(pane.state().selected(), None);
    }

    #[test]
    fn fit_voids_a_range_anchored_into_the_old_rows() {
        let mut rows = rows_of(vec![false; 5]);
        rows.dialogues.select(4);
        assert!(!rows.range_select(PANE));
        // The list changed shape under the anchor: it no longer names row 4.
        rows.dialogues.fit(2);
        assert_eq!(rows.range_start(PANE), None);
    }

    #[test]
    fn toggle_all_marks_then_clears_every_row() {
        let mut pane = ListPane::with_marks(vec![false, true, false]);
        pane.toggle_all();
        assert_eq!(pane.marked(), 3);
        pane.toggle_all();
        assert_eq!(pane.marked(), 0);
    }

    #[test]
    fn first_v_anchors_second_v_selects_span() {
        let mut rows = rows_of(vec![false; 5]);
        rows.dialogues.select(4);

        // First `v` only anchors; nothing is marked yet.
        assert!(!rows.range_select(PANE));
        assert_eq!(rows.range_start(PANE), Some(4));
        assert!(!rows.dialogues.has_marks());

        // Moving the cursor does not disturb the anchor.
        rows.dialogues.select(1);
        assert!(rows.range_select(PANE));
        assert_eq!(rows.range_start(PANE), None);
        // Span 1..=4 marked; row 0 untouched.
        assert_eq!(rows.dialogues.mask(), &[false, true, true, true, true]);
    }

    #[test]
    fn second_v_inverts_an_already_marked_span() {
        let mut pane = ListPane::with_marks(vec![true, true, false, false, true]);
        let span = |pane: &mut ListPane, from: usize, to: usize| {
            pane.select(from);
            pane.range_select();
            pane.select(to);
            pane.range_select();
        };
        span(&mut pane, 4, 2);
        // Span 2..=4 was false/false/true (mixed) → mark all.
        assert_eq!(pane.mask()[2..], [true, true, true]);
        span(&mut pane, 4, 2);
        // Span is now all marked → clear it.
        assert!(pane.mask()[0]);
        assert_eq!(pane.mask()[2..], [false, false, false]);
    }
}
