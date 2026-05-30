use anyhow::{Context, Result};
use sivtr_core::capture::scrollback;
use sivtr_core::workspace;
use std::fs;

use crate::output;

/// Clear the current session's log and state files.
pub fn execute(clear_all: bool) -> Result<()> {
    if clear_all {
        let removed = clear_all_sessions()?;
        if removed > 0 {
            output::success(format!("cleared {removed} workspace session tree(s)"));
        } else {
            output::info("no session files to clear");
        }
        return Ok(());
    }

    let Some(log) = scrollback::session_log_path()? else {
        output::info("no current terminal session to clear");
        return Ok(());
    };
    let state = log.with_extension("state");
    let capture = log.with_extension("capture");

    let mut cleared = false;

    if log.exists() {
        fs::remove_file(&log)?;
        cleared = true;
    }
    for f in [&state, &capture] {
        if f.exists() {
            let _ = fs::remove_file(f);
        }
    }

    if cleared {
        output::success("cleared current terminal session");
        output::detail("path", log.display());
    } else {
        output::info("no current terminal session to clear");
    }

    Ok(())
}

fn clear_all_sessions() -> Result<usize> {
    let workspaces_dir = workspace::data_dir().join("workspaces");
    if workspaces_dir.exists() {
        fs::remove_dir_all(&workspaces_dir).context("Failed to clear workspace session files")?;
        return Ok(1);
    }

    Ok(0)
}
