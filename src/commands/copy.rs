use anyhow::{Result, Context};
use std::fs;

use sift_core::capture::scrollback;
use sift_core::parse::ansi::strip_ansi;

/// Copy the output of the Nth-last command to clipboard.
pub fn execute(n: usize) -> Result<()> {
    let log_path = scrollback::session_log_path();
    let boundaries_path = log_path.with_extension("boundaries");

    if !log_path.exists() || !boundaries_path.exists() {
        eprintln!("sift: no session log found");
        eprintln!("  hint: run `sift init powershell` to start recording");
        return Ok(());
    }

    let content = fs::read_to_string(&log_path)
        .context("Failed to read session log")?;
    let boundaries: Vec<usize> = fs::read_to_string(&boundaries_path)?
        .lines()
        .filter_map(|l| l.parse().ok())
        .collect();

    if boundaries.is_empty() {
        eprintln!("sift: no commands recorded yet");
        return Ok(());
    }

    let total = boundaries.len();
    if n == 0 || n > total {
        eprintln!("sift: only {} commands recorded", total);
        return Ok(());
    }

    // Extract the Nth-last command block
    let start = boundaries[total - n];
    let end = if n > 1 {
        boundaries[total - n + 1]
    } else {
        content.len()
    };

    let block = &content[start..end];
    let clean = strip_ansi(block).trim().to_string();

    if clean.is_empty() {
        eprintln!("sift: command output is empty");
        return Ok(());
    }

    arboard::Clipboard::new()
        .context("Failed to open clipboard")?
        .set_text(&clean)
        .context("Failed to set clipboard")?;

    // Show preview (first 3 lines)
    let preview: Vec<&str> = clean.lines().take(3).collect();
    let line_count = clean.lines().count();
    for line in &preview {
        eprintln!("  {}", line);
    }
    if line_count > 3 {
        eprintln!("  ... ({} lines total)", line_count);
    }
    eprintln!("sift: copied to clipboard");

    Ok(())
}
