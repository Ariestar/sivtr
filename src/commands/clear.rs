use anyhow::Result;
use std::fs;

use sivtr_core::capture::scrollback;

/// Clear the current session's log and state files.
pub fn execute() -> Result<()> {
    let log = scrollback::session_log_path();
    let state = scrollback::flush_state_path();

    let mut cleared = false;

    if log.exists() {
        fs::remove_file(&log)?;
        cleared = true;
    }
    for f in [&state] {
        if f.exists() {
            let _ = fs::remove_file(f);
        }
    }

    if cleared {
        eprintln!("sivtr: session cleared ({})", log.display());
    } else {
        eprintln!("sivtr: no session to clear");
    }

    Ok(())
}
