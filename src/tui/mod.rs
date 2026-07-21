//! Shared terminal UI primitives for the workspace browser.
//!
//! Product orchestration lives in `commands/browse`. Pane **data** capability
//! (sliding window / ensure) lives in [`crate::pane`]. This area only renders
//! workspace models and chrome (`pane` module here = borders / titles / lists).

pub mod content;
pub mod pane;
pub mod search;
pub mod terminal;
pub mod theme;
pub mod workspace;

// Historical paths kept for browse imports.
pub use content::io as content_io;
pub use content::markdown as content_markdown;
pub use content::view as content_view;
pub use search as workspace_search;
