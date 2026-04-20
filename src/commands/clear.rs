use anyhow::Result;
use std::fs;

use sift_core::capture::scrollback;

/// Clear the current session's log and state files.
pub fn execute() -> Result<()> {
    let log = scrollback::session_log_path();
    let state = scrollback::flush_state_path();
    let boundaries = log.with_extension("boundaries");

    let mut cleared = false;

    if log.exists() {
        fs::remove_file(&log)?;
        cleared = true;
    }
    for f in [&state, &boundaries] {
        if f.exists() {
            let _ = fs::remove_file(f);
        }
    }

    if cleared {
        eprintln!("sift: session cleared ({})", log.display());
    } else {
        eprintln!("sift: no session to clear");
    }

    Ok(())
}
