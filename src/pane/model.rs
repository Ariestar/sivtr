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
