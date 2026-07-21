//! Shared terminal UI primitives for the workspace browser.
//!
//! Product orchestration lives in `commands/browse`. Pane **data** capability
//! (sliding window / ensure) lives in [`crate::pane`]. This area only renders
//! workspace models and chrome (`pane` module here = borders / titles / lists).

pub mod content_io;
pub mod content_markdown;
pub mod content_view;
pub mod pane;
pub mod terminal;
pub mod theme;
pub mod workspace;
pub mod workspace_search;
