use anyhow::Result;
use std::fs;
use std::io::Write;

use sivtr_core::capture::scrollback;
use sivtr_core::parse::ansi::strip_ansi;

/// Flush: read console buffer, append new content to session.log.
/// Called by the shell prompt hook after each command.
/// Must NEVER fail or print anything 鈥?the prompt depends on it.
pub fn execute() -> Result<()> {
    if let Err(_) = do_flush() {
        // Silently ignore all errors 鈥?never break the user's prompt
    }
    Ok(())
}

fn do_flush() -> Result<()> {
    #[cfg(windows)]
    {
        let raw = scrollback::capture_console_buffer()?;
        if raw.trim().is_empty() {
            return Ok(());
        }

        let current_lines: Vec<&str> = raw.lines().collect();

        // Load previous state (stripped last lines from previous flush)
        let state_path = scrollback::flush_state_path();
        let prev_tail: Vec<String> = if state_path.exists() {
            fs::read_to_string(&state_path)?
                .lines()
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        };

        // Find where new content starts by matching previous tail in current buffer
        let new_start = find_new_start(&current_lines, &prev_tail);

        if new_start < current_lines.len() {
            let log_path = scrollback::session_log_path();
            fs::create_dir_all(log_path.parent().unwrap())?;

            // Record command boundary (byte offset) before writing
            let log_size = if log_path.exists() {
                fs::metadata(&log_path)?.len()
            } else {
                0
            };
            let boundaries_path = log_path.with_extension("boundaries");
            let mut bf = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&boundaries_path)?;
            writeln!(bf, "{}", log_size)?;

            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)?;

            for line in &current_lines[new_start..] {
                writeln!(file, "{}", line)?;
            }
        }

        // Save state: stripped version of last 5 lines
        let tail_start = current_lines.len().saturating_sub(5);
        let state: String = current_lines[tail_start..]
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n");
        fs::create_dir_all(state_path.parent().unwrap())?;
        fs::write(&state_path, state)?;
    }

    #[cfg(not(windows))]
    {
        // Non-Windows: no-op for now (tmux/zellij have native capture)
    }

    Ok(())
}

/// Find the index in `current` where new (unseen) content begins,
/// by matching `prev_tail` (stripped lines) against stripped current lines.
#[cfg(windows)]
fn find_new_start(current: &[&str], prev_tail: &[String]) -> usize {
    if prev_tail.is_empty() {
        return 0;
    }

    let last_prev = &prev_tail[prev_tail.len() - 1];

    // Search from end backwards for a matching sequence
    for i in (0..current.len()).rev() {
        let stripped = strip_ansi(current[i]);
        if stripped == *last_prev {
            // Verify more lines match
            let mut ok = true;
            for j in 1..prev_tail.len() {
                if i < j {
                    break;
                }
                let prev_idx = prev_tail.len() - 1 - j;
                let stripped_j = strip_ansi(current[i - j]);
                if stripped_j != prev_tail[prev_idx] {
                    ok = false;
                    break;
                }
            }
            if ok {
                return i + 1; // new content starts after the match
            }
        }
    }

    // No overlap found 鈥?everything is new
    0
}
