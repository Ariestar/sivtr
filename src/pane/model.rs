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

/// Native multi-select: the row mask of one pane, with the toggle /
/// range-toggle API shared by every selectable pane. Panes own a `Selection`
/// when their rows are selectable and hand the mask to the keep policy via
/// [`PaneInput::selected`]; consumers (render, copy) read the mask. The live
/// range anchor is not part of it — only one range is open at a time across
/// every pane, so it lives once in the picker instead of per mask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    mask: Vec<bool>,
}

impl Selection {
    /// Resize to a new row count, dropping marks that fall out of range.
    pub fn resize(&mut self, len: usize) {
        self.mask.resize(len, false);
    }

    /// Row mask: `mask[i]` is true when row `i` is selected.
    pub fn mask(&self) -> &[bool] {
        &self.mask
    }

    pub fn count(&self) -> usize {
        self.mask.iter().filter(|flag| **flag).count()
    }

    /// Toggle one row; out-of-range rows are ignored (the mask is resized
    /// when the pane's rows change).
    pub fn toggle(&mut self, idx: usize) {
        if let Some(flag) = self.mask.get_mut(idx) {
            *flag = !*flag;
        }
    }

    /// Range-select the given rows through [`toggle_row_ids`].
    pub fn toggle_ids(&mut self, ids: impl Iterator<Item = usize> + Clone) {
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

    /// Bring the pane in line with this frame's data and viewport.
    fn ensure(&mut self, ctx: Self::Ctx<'_>, input: &PaneInput<'_>);

    /// Drain finished async work; `true` when it changed the pane.
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
    use super::{toggle_row_ids, Selection};

    #[test]
    fn row_ids_toggle_to_one_state_over_gaps() {
        let mut mask = [false, true, false, false, false];
        // Mixed span selects every listed row; ids need not be contiguous and
        // out-of-range ids are ignored.
        toggle_row_ids(&mut mask, [1, 3, 9].into_iter());
        assert_eq!(mask, [false, true, false, true, false]);
        // A fully selected span clears instead.
        toggle_row_ids(&mut mask, [1, 3].into_iter());
        assert_eq!(mask, [false, false, false, false, false]);
    }

    #[test]
    fn selection_resize_drops_marks_out_of_range() {
        let mut selection = Selection::default();
        selection.resize(4);
        selection.toggle(3);
        assert_eq!(selection.count(), 1);
        selection.resize(2);
        assert_eq!(selection.count(), 0);
        selection.toggle(1);
        selection.resize(3);
        assert_eq!(selection.mask(), &[false, true, false]);
    }
}
