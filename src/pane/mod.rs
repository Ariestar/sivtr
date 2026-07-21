//! Pane data capability: ordered sliding windows with viewport ensure.
//!
//! Layering:
//! - **`pane`** (this module): pure data window + unified [`Pane`] contract.
//! - **`tui::pane`**: chrome only (border / title / list paint).
//! - **`commands/browse`**: product panes implementing [`Pane`].
//!
//! Dynamic loading is a property of [`SlidingPane`]. Product panes only fulfill
//! requests; they do not reimplement growth policy.

mod model;
mod sliding;
mod store;

pub use model::{Pane, PaneInput};
pub use sliding::{MetaNeed, SlidingPane};
pub use store::{keep_keys, StorePhase, Viewport, WindowRow, FETCH_CEILING, FETCH_FLOOR};
