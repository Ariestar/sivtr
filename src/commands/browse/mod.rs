//! Workspace browser: source catalog, on-demand load, and TUI picker.
//!
//! Product surface for bare `sivtr`, hotkey, and `copy --pick`. Returns
//! [`WorkspacePickedContent`]; callers decide how to export (clipboard, etc.).

mod content;
mod help;
mod load;
mod nav;
mod picker;
mod selection;
mod text;
mod vim;
mod visual;
pub(crate) use load::{workspace_source_catalog, SourceLoadState};
pub(crate) use picker::run as run_picker;
pub(crate) use text::{filter_lines_by_spec, record_text_to_pair, select_lines};

use anyhow::{Context, Result};
use sivtr_core::ai::AgentProvider;

use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use crate::tui::workspace::{WorkspaceFocus, WorkspacePickedContent, WorkspaceSource};

/// Run the workspace browser.
///
/// Catalog = local + mounts. `select_remotes` only sets the initial selection mask.
/// Loads run in the background; the picker draws immediately.
pub fn run(
    providers: &[AgentProvider],
    select_remotes: bool,
    initial_focus: WorkspaceFocus,
) -> Result<WorkspacePickedContent> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let sources = workspace_source_catalog(providers, &cwd)?;
    if sources.is_empty() {
        anyhow::bail!("No terminal or AI sources configured");
    }

    let selected_sources: Vec<bool> = sources
        .iter()
        .map(|source| select_remotes || !source.is_remote())
        .collect();
    let source_states: Vec<SourceLoadState> =
        sources.iter().map(|_| SourceLoadState::Idle).collect();

    let mut terminal = init_tui()?;
    let result = run_picker(
        &mut terminal,
        sources,
        source_states,
        selected_sources,
        cwd,
        initial_focus,
    );
    restore_tui(&mut terminal)?;
    result
}

/// Open the picker on an already-built session list for one source.
pub fn run_with_sessions(
    source: WorkspaceSource,
    sessions: Vec<crate::tui::workspace::WorkspaceSession>,
    initial_focus: WorkspaceFocus,
) -> Result<WorkspacePickedContent> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let mut terminal = init_tui()?;
    let result = run_picker(
        &mut terminal,
        vec![source],
        vec![SourceLoadState::Ready(sessions)],
        vec![true],
        cwd,
        initial_focus,
    );
    restore_tui(&mut terminal)?;
    result
}

/// Shared cancel sentinel for picker Esc/q.
pub const PICK_CANCELLED_MESSAGE: &str = "Pick cancelled";

pub fn is_pick_cancelled(error: &anyhow::Error) -> bool {
    error.to_string() == PICK_CANCELLED_MESSAGE
}
