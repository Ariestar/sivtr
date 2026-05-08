use anyhow::Result;
use std::env;
use std::fs;

use sivtr_core::capture::scrollback;
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
        let prompt = env::var("SIVTR_LAST_PROMPT").unwrap_or_default();
        let command = env::var("SIVTR_LAST_COMMAND").unwrap_or_default();
        let command_id = env::var("SIVTR_LAST_COMMAND_ID").ok();
        let output = session::extract_output_from_snapshot(
            &prompt,
            &command,
            &current_lines,
            snapshot.width,
        );
        append_entry_from_output(
            &scrollback::session_log_path(),
            &scrollback::flush_state_path(),
            prompt,
            command,
            command_id,
            output,
        )?;
    }

    #[cfg(not(windows))]
    {
        let capture_path = scrollback::capture_file_path();
        if !capture_path.exists() {
            return Ok(());
        }

        let output = String::from_utf8_lossy(&fs::read(&capture_path)?).into_owned();
        let _ = fs::remove_file(&capture_path);

        if output.trim().is_empty() {
            return Ok(());
        }

        let prompt = env::var("SIVTR_LAST_PROMPT").unwrap_or_default();
        let command = env::var("SIVTR_LAST_COMMAND").unwrap_or_default();
        let command_id = env::var("SIVTR_LAST_COMMAND_ID").ok();
        let output = trim_trailing_prompt_artifact(output, &prompt);

        append_entry_from_output(
            &scrollback::session_log_path(),
            &scrollback::flush_state_path(),
            prompt,
            command,
            command_id,
            output,
        )?;
    }

    Ok(())
}

fn load_state_lossy(path: &std::path::Path) -> SessionState {
    session::load_state(path).unwrap_or_default()
}

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

fn append_entry_from_output(
    session_log_path: &std::path::Path,
    state_path: &std::path::Path,
    prompt: String,
    command: String,
    command_id: Option<String>,
    output: String,
) -> Result<()> {
    let mut state = load_state_lossy(state_path);

    if should_append_entry(&state, command_id.as_deref(), &command) {
        let entry = SessionEntry::new(prompt, command.clone(), output);
        session::append_entry(session_log_path, &entry)?;
        state.last_command_id = command_id;
        state.last_command = Some(command);
    }

    session::save_state(state_path, &state)
}

fn trim_trailing_prompt_artifact(output: String, prompt: &str) -> String {
    let prompt_last_line = SessionEntry::new(prompt, "", "")
        .prompt
        .lines()
        .last()
        .unwrap_or_default()
        .trim_end()
        .to_string();
    if prompt_last_line.is_empty() {
        return output;
    }

    let mut raw_lines: Vec<&str> = output.lines().collect();
    let Some(last_raw_line) = raw_lines.last().copied() else {
        return output;
    };

    if !last_raw_line.contains('\x1b') && !last_raw_line.contains('\r') {
        return output;
    }

    let last_plain_line = SessionEntry::new("", "", last_raw_line)
        .output
        .trim()
        .to_string();
    if last_plain_line.is_empty() || prompt_last_line.ends_with(&last_plain_line) {
        raw_lines.pop();
        return raw_lines.join("\n");
    }

    output
}

#[cfg(test)]
mod tests {
    use super::{append_entry_from_output, should_append_entry, trim_trailing_prompt_artifact};
    use sivtr_core::session::{self, SessionState};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sivtr-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn appends_structured_entry_for_capture_output() {
        let dir = unique_test_dir("flush");
        let log_path = dir.join("session_1.log");
        let state_path = dir.join("session_1.state");

        append_entry_from_output(
            &log_path,
            &state_path,
            "repo on main\n❯  ".to_string(),
            "cargo test".to_string(),
            Some("42".to_string()),
            "\u{1b}[32mok\u{1b}[0m\n".to_string(),
        )
        .unwrap();

        let entries = session::load_entries(&log_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "cargo test");
        assert_eq!(entries[0].prompt, "repo on main\n❯  ");
        assert_eq!(entries[0].output, "ok");
        assert_eq!(
            entries[0].output_ansi.as_deref(),
            Some("\u{1b}[32mok\u{1b}[0m")
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn skips_duplicate_command_id_when_state_matches() {
        let dir = unique_test_dir("flush-state");
        let log_path = dir.join("session_1.log");
        let state_path = dir.join("session_1.state");

        append_entry_from_output(
            &log_path,
            &state_path,
            "repo> ".to_string(),
            "cargo test".to_string(),
            Some("99".to_string()),
            "ok".to_string(),
        )
        .unwrap();
        append_entry_from_output(
            &log_path,
            &state_path,
            "repo> ".to_string(),
            "cargo test".to_string(),
            Some("99".to_string()),
            "still ok".to_string(),
        )
        .unwrap();

        let entries = session::load_entries(&log_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].output, "ok");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_empty_commands_even_without_command_id() {
        let state = SessionState::default();
        assert!(!should_append_entry(&state, None, ""));
        assert!(!should_append_entry(&state, Some("7"), "   "));
    }

    #[test]
    fn trims_trailing_prompt_artifact_from_zsh_capture() {
        let prompt = "jacob-NucBox-EVO-X2# ";
        let output = "hello from zsh\n\u{1b}[1m\u{1b}[7m#\u{1b}[27m\u{1b}[1m\u{1b}[0m \r \r";

        assert_eq!(
            trim_trailing_prompt_artifact(output.to_string(), prompt),
            "hello from zsh"
        );
    }
}
