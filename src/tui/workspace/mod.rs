//! Workspace browser UI: model, layout, help registry, render.

pub(crate) mod help;
pub(crate) mod layout;
pub(crate) mod model;
pub(crate) mod render;

pub(crate) use help::*;
pub(crate) use layout::*;
pub(crate) use model::*;
pub(crate) use render::*;

// Dual-pane + content text surface for browse.
pub(crate) use crate::tui::content::io::{
    search_match_half, ContentIoFocus, ContentIoFrame, ContentIoTexts, ContentScrolls,
};
pub(crate) use crate::tui::content::text::{workspace_content_io_texts, workspace_content_text};

#[cfg(test)]
mod tests;
