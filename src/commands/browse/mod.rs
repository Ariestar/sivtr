//! Workspace browser: source catalog, on-demand load, and TUI picker.
//!
//! Product surface for bare `sivtr`, hotkey, and `copy --pick`. Returns
//! [`WorkspacePickedContent`]; callers decide how to export (clipboard, etc.).
//!
//! Pane data capability lives in [`crate::pane`] (`SlidingPane`). This module
//! owns loaders + picker orchestration only. `tui::pane` is chrome only.

mod content;
mod help;
mod load;
mod nav;
mod panes;
#[cfg(feature = "perf-benches")]
pub mod perf;
mod picker;
mod selection;
mod text;
mod vim;
mod visual;

pub(crate) use load::{workspace_source_catalog, SourceLoadState};
pub(crate) use picker::run as run_picker;
pub(crate) use text::{filter_lines_by_spec, record_text_to_pair, select_lines};

use anyhow::{anyhow, Context, Result};
use sivtr_core::ai::AgentProvider;
use std::io::{self, IsTerminal};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::tui::terminal::{
    finish as finish_tui, init as init_tui, panic_payload_message, restore as restore_tui,
    wait_for_enter,
};
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
    run_catalog(providers, select_remotes, initial_focus, true)
}

/// Like [`run`], but does not prompt for Enter after a recovered panic.
///
/// The Windows hotkey picker runs in its own console and already waits when it
/// reports errors (`show_pick_error_and_wait`); waiting here as well would make
/// the first keypress appear to be ignored by the panic prompt.
pub fn run_without_panic_wait(
    providers: &[AgentProvider],
    select_remotes: bool,
    initial_focus: WorkspaceFocus,
) -> Result<WorkspacePickedContent> {
    run_catalog(providers, select_remotes, initial_focus, false)
}

fn run_catalog(
    providers: &[AgentProvider],
    select_remotes: bool,
    initial_focus: WorkspaceFocus,
    wait_after_panic: bool,
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
        sources.iter().map(|_| SourceLoadState::idle()).collect();

    let mut terminal = init_tui()?;
    let result = run_picker_guarded(
        &mut terminal,
        sources,
        source_states,
        selected_sources,
        cwd,
        initial_focus,
        wait_after_panic,
    );
    finish_tui(&mut terminal, result)
}

/// Open the picker on an already-built session list for one source.
pub fn run_with_sessions(
    source: WorkspaceSource,
    sessions: Vec<crate::tui::workspace::WorkspaceSession>,
    initial_focus: WorkspaceFocus,
) -> Result<WorkspacePickedContent> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let loaded = sessions.len().max(1);
    let mut terminal = init_tui()?;
    let result = run_picker_guarded(
        &mut terminal,
        vec![source],
        vec![SourceLoadState::ready_from_sessions(sessions, loaded)],
        vec![true],
        cwd,
        initial_focus,
        true,
    );
    finish_tui(&mut terminal, result)
}

/// Run the picker and convert any panic into a reported error.
///
/// A panic inside the TUI must not leave the terminal in raw mode with the
/// alternate screen active. The unwind is caught and the terminal is restored
/// first; panics that skip this path are caught by the global panic hook
/// ([`crate::tui::panic`]) and by the `Tui` drop. The panic message is
/// printed before the error propagates.
///
/// `wait_after_panic` controls whether a recovered panic prompts for Enter: the
/// hotkey picker reports errors itself, so it passes `false` to avoid consuming
/// a keypress the outer handler is about to wait on.
fn run_picker_guarded(
    terminal: &mut crate::tui::terminal::Tui,
    sources: Vec<WorkspaceSource>,
    source_states: Vec<SourceLoadState>,
    selected_sources: Vec<bool>,
    cwd: std::path::PathBuf,
    initial_focus: WorkspaceFocus,
    wait_after_panic: bool,
) -> Result<WorkspacePickedContent> {
    // This guard recovers the panic and reports it itself, so the terminal-
    // restoring hook must not also emit the default "uncaught panic" report.
    let _suppress = crate::tui::panic::SuppressDefaultReport::enter();
    match catch_unwind(AssertUnwindSafe(|| {
        run_picker(
            terminal,
            sources,
            source_states,
            selected_sources,
            cwd,
            initial_focus,
        )
    })) {
        Ok(result) => result,
        Err(payload) => {
            let message = panic_payload_message(&payload);
            let restored = restore_tui(terminal).is_ok();
            eprintln!("sivtr: TUI panicked: {message}");
            // Only prompt after a successful restore: while raw mode is still
            // active a blocking read may never see a newline, and the later
            // finish_tui retry must get a chance to run and report the
            // cleanup failure.
            if restored && wait_after_panic && io::stdin().is_terminal() {
                wait_for_enter("press Enter to continue");
            }
            Err(anyhow!("TUI panicked: {message}"))
        }
    }
}

/// Shared cancel sentinel for picker Esc/q.
pub const PICK_CANCELLED_MESSAGE: &str = "Pick cancelled";

pub fn is_pick_cancelled(error: &anyhow::Error) -> bool {
    error.to_string() == PICK_CANCELLED_MESSAGE
}
