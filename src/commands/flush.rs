use anyhow::Result;

#[cfg(windows)]
use std::env;

#[cfg(windows)]
use sivtr_core::capture::scrollback;
#[cfg(windows)]
use sivtr_core::session::{self, SessionEntry, SessionState};

/// Flush: read console buffer, append new content to session.log.
/// Called by the shell prompt hook after each command.
/// Must NEVER fail or print anything 鈥?the prompt depends on it.
pub fn execute() -> Result<()> {
    if do_flush().is_err() {
        // Silently ignore all errors 鈥?never break the user's prompt
    }
    Ok(())
}

fn do_flush() -> Result<()> {
    #[cfg(windows)]
    {
        let snapshot = scrollback::capture_console_buffer()?;
        if snapshot.content.trim().is_empty() {
            return Ok(());
        }

        let current_lines: Vec<&str> = snapshot.content.lines().collect();

        let state_path = scrollback::flush_state_path();
        let mut state = load_state_lossy(&state_path);

        let prompt = env::var("SIVTR_LAST_PROMPT").unwrap_or_default();
        let command = env::var("SIVTR_LAST_COMMAND").unwrap_or_default();
        let command_id = env::var("SIVTR_LAST_COMMAND_ID").ok();
        let output = session::extract_output_from_snapshot(
            &prompt,
            &command,
            &current_lines,
            snapshot.width,
        );

        if should_append_entry(&state, command_id.as_deref(), &command) {
            let entry = SessionEntry::new(prompt, command.clone(), output);
            session::append_entry(&scrollback::session_log_path(), &entry)?;
            state.last_command_id = command_id;
            state.last_command = Some(command);
        }

        session::save_state(&state_path, &state)?;
    }

    #[cfg(not(windows))]
    {
        // Non-Windows: no-op for now (tmux/zellij have native capture)
    }

    Ok(())
}

#[cfg(windows)]
fn load_state_lossy(path: &std::path::Path) -> SessionState {
    session::load_state(path).unwrap_or_default()
}

#[cfg(windows)]
fn should_append_entry(state: &SessionState, command_id: Option<&str>, command: &str) -> bool {
    if command.trim().is_empty() {
        return false;
    }

    if let Some(command_id) = command_id {
        if state.last_command_id.as_deref() == Some(command_id) {
            return false;
        }
        return true;
    }

    state.last_command.as_deref() != Some(command)
}
