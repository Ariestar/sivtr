//! Unified product pane surface.
//!
//! New panes implement [`Pane`]:
//! 1. Own a [`super::SlidingPane`].
//! 2. Map domain data inside `ensure` only.
//! 3. Call SlidingPane `ensure_*` / `apply_*` — never reimplement growth.
//! 4. Optional async: spawn in `ensure`, finish in `poll`.

use super::Viewport;

/// Per-frame ensure input. `selected` is borrowed for the frame.
#[derive(Clone, Copy, Debug)]
pub struct PaneInput<'a> {
    pub viewport: Viewport,
    pub focus: usize,
    /// Empty = focus-only keep.
    pub selected: &'a [bool],
    pub neighbor_radius: usize,
    pub force: bool,
}

/// Set the listed rows to one state: select them all when any of them is
/// unselected, clear them all otherwise. This is the range-selection rule
/// every pane uses for `v` and range clicks, and the only place it lives.
/// The ids need not be contiguous — the content pane's visible blocks skip
/// the ids a fold is hiding — and out-of-range ids are ignored.
pub fn toggle_row_ids(mask: &mut [bool], ids: impl Iterator<Item = usize> + Clone) {
    let select = ids
        .clone()
        .any(|id| mask.get(id).is_some_and(|flag| !*flag));
    for id in ids {
        if let Some(flag) = mask.get_mut(id) {
            *flag = select;
        }
    }
}

/// Native multi-select: a boolean mask plus a range anchor, with one
/// toggle / range-toggle / clear API shared by every pane (source, session,
/// dialogue, and content blocks). Panes own a `Selection` when their rows
/// are selectable and hand the mask to the keep policy via
/// [`PaneInput::selected`]; consumers (render, copy) read the mask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    mask: Vec<bool>,
    anchor: Option<usize>,
}

impl Selection {
    pub fn new(len: usize) -> Self {
        Self {
            mask: vec![false; len],
            anchor: None,
        }
    }

    /// Resize to a new row count, dropping marks that fall out of range.
    pub fn resize(&mut self, len: usize) {
        self.mask.resize(len, false);
        self.anchor = None;
    }

    /// Row mask: `mask[i]` is true when row `i` is selected.
    pub fn mask(&self) -> &[bool] {
        &self.mask
    }

    pub fn len(&self) -> usize {
        self.mask.len()
    }

    pub fn is_empty(&self) -> bool {
        !self.mask.contains(&true)
    }

    pub fn count(&self) -> usize {
        self.mask.iter().filter(|flag| **flag).count()
    }

    pub fn clear(&mut self) {
        self.mask.fill(false);
        self.anchor = None;
    }

    /// Toggle one row; out-of-range rows are ignored (the mask is resized
    /// when the pane's rows change) and never update the anchor.
    pub fn toggle(&mut self, idx: usize) {
        if let Some(flag) = self.mask.get_mut(idx) {
            *flag = !*flag;
            self.anchor = Some(idx);
        }
    }

    /// Range-select the given rows through [`toggle_row_ids`], leaving the
    /// anchor on the last of them.
    pub fn toggle_ids(&mut self, ids: impl Iterator<Item = usize> + Clone) {
        self.anchor = ids.clone().last();
        toggle_row_ids(&mut self.mask, ids);
    }
}

impl<'a> PaneInput<'a> {
    pub fn new(viewport: Viewport, focus: usize) -> Self {
        Self {
            viewport,
            focus,
            selected: &[],
            neighbor_radius: 1,
            force: false,
        }
    }

    pub fn with_selected(mut self, selected: &'a [bool]) -> Self {
        self.selected = selected;
        self
    }

    pub fn with_neighbors(mut self, radius: usize) -> Self {
        self.neighbor_radius = radius;
        self
    }
}

/// Product pane contract.
#[allow(clippy::len_without_is_empty)]
pub trait Pane {
    type Ctx<'a>;

    fn ensure(&mut self, ctx: Self::Ctx<'_>, input: &PaneInput<'_>) -> bool;

    fn poll(&mut self) -> bool {
        false
    }

    fn len(&self) -> usize;

    fn is_fetching(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;

    #[test]
    fn selection_toggles_and_counts_rows() {
        let mut selection = Selection::new(3);
        assert!(selection.is_empty());
        selection.toggle(1);
        assert!(!selection.is_empty());
        assert_eq!(selection.count(), 1);
        assert_eq!(selection.mask(), &[false, true, false]);
        // Toggling again clears the row.
        selection.toggle(1);
        assert!(selection.is_empty());
        // Out-of-range toggles are ignored.
        selection.toggle(9);
        assert!(selection.is_empty());
    }

    #[test]
    fn selection_range_toggles_every_listed_row_to_one_state() {
        let mut selection = Selection::new(5);
        selection.toggle(1);
        selection.toggle_ids(1..=3);
        assert_eq!(selection.mask(), &[false, true, true, true, false]);
        // Second range over the same rows clears them.
        selection.toggle_ids(1..=3);
        assert_eq!(selection.mask(), &[false, false, false, false, false]);
        // Gaps are allowed (folded blocks are skipped) and out-of-range ids
        // are ignored.
        selection.toggle_ids([0, 3, 9].into_iter());
        assert_eq!(selection.mask(), &[true, false, false, true, false]);
    }

    #[test]
    fn selection_resize_drops_marks_out_of_range() {
        let mut selection = Selection::new(4);
        selection.toggle(3);
        selection.resize(2);
        assert!(selection.is_empty());
        selection.toggle(1);
        selection.resize(3);
        assert_eq!(selection.mask(), &[false, true, false]);
    }
}
