mod entry;

use anyhow::Result;
use std::path::PathBuf;

use crate::workspace;

pub use entry::{
    append_entry, load_entries, load_state, render_entries, render_entries_ansi, render_entry,
    render_entry_ansi, render_input, save_state, SessionEntry, SessionState,
};

/// Session log for the terminal running this process, when it is inside a repo.
pub fn current_log_path() -> Result<Option<PathBuf>> {
    workspace::current_terminal_log_path()
}
