//! Unified product pane surface.
//!
//! New panes plug in by implementing [`Pane`]:
//! 1. Own a [`super::SlidingPane`] (engine).
//! 2. Map domain data → rows inside `ensure` (only place that knows the data source).
//! 3. Call SlidingPane `ensure_*` / `apply_*` — never reimplement growth policy.
//! 4. Optional async: spawn work from `ensure`, finish in `poll`.
//!
//! The picker only does:
//! ```text
//! pane.poll();
//! pane.ensure(ctx, &PaneInput { viewport, focus, selected, .. });
//! view = pane rows
//! ```

use super::Viewport;

/// Per-frame ensure input shared by every pane.
#[derive(Clone, Debug)]
pub struct PaneInput {
    pub viewport: Viewport,
    /// Focused row index in this pane's list.
    pub focus: usize,
    /// Multi-select mask (length may lag `len()`; empty = focus-only keep).
    pub selected: Vec<bool>,
    pub neighbor_radius: usize,
    /// Force meta refresh (`R` / context rebuild).
    pub force: bool,
}

impl PaneInput {
    pub fn new(viewport: Viewport, focus: usize) -> Self {
        Self {
            viewport,
            focus,
            selected: Vec::new(),
            neighbor_radius: 1,
            force: false,
        }
    }

    pub fn with_selected(mut self, selected: Vec<bool>) -> Self {
        self.selected = selected;
        self
    }

    pub fn with_neighbors(mut self, radius: usize) -> Self {
        self.neighbor_radius = radius;
        self
    }
}

/// Unified content-pane contract. Browse (and future panes) implement this;
/// orchestration never branches on pane kind for ensure/poll.
pub trait Pane {
    /// Frame-local domain context (sessions, sources, document, …).
    type Ctx<'a>;

    /// Grow meta / hydrate bodies / evict for this frame.
    /// Returns `true` when rows or body residency changed.
    fn ensure(&mut self, ctx: Self::Ctx<'_>, input: &PaneInput) -> bool;

    /// Drain async work. Default: nothing.
    fn poll(&mut self) -> bool {
        false
    }

    fn len(&self) -> usize;

    fn is_fetching(&self) -> bool {
        false
    }
}
