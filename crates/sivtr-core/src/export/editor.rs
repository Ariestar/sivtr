use anyhow::{Context, Result};
use std::io::Write;
use std::process::Command;

use crate::config::SivtrConfig;

/// Known editors to try, in order of preference.
const FALLBACK_EDITORS: &[&str] = &["hx", "nvim", "vim", "vi", "nano", "notepad"];

/// Resolve which editor to use.
/// Priority: config file `editor.command` > auto-detect from PATH.
pub fn resolve_editor() -> Result<String> {
    resolve_editor_with_config(&SivtrConfig::load().unwrap_or_default())
}

/// Resolve editor using a pre-loaded config.
pub fn resolve_editor_with_config(config: &SivtrConfig) -> Result<String> {
    // 1. Check config file setting
    if !config.editor.command.is_empty() {
        return Ok(config.editor.command.clone());
    }

    // 2. Auto-detect from PATH
    for &name in FALLBACK_EDITORS {
        if which_exists(name) {
            return Ok(name.to_string());
        }
    }

    anyhow::bail!(
        "No editor found. Set editor.command in config file \
         (run `sivtr config init` to create one)"
    )
}

/// Open text content in an external editor.
///
/// Writes content to a temp file, opens the editor, waits for it to exit,
/// then reads back the (potentially modified) content and cleans up.
///
/// Returns the content after editing (may be unchanged).
pub fn open_in_editor(content: &str) -> Result<String> {
    let editor = resolve_editor()?;

    // Create temp file
    let mut tmp = tempfile::Builder::new()
        .prefix("sivtr-")
        .suffix(".txt")
        .tempfile()
        .context("Failed to create temp file")?;

    tmp.write_all(content.as_bytes())
        .context("Failed to write to temp file")?;
    tmp.flush()?;

    let path = tmp.path().to_path_buf();

    // Parse editor command (handles cases like "code --wait")
    let parts: Vec<&str> = editor.split_whitespace().collect();
    let (program, extra_args) = parts.split_first().context("Empty editor command")?;

    // Spawn editor
    let status = Command::new(program)
        .args(extra_args)
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to launch editor '{}'", editor))?;

    if !status.success() {
        anyhow::bail!("Editor '{}' exited with {}", editor, status);
    }

    // Read back content
    let result = std::fs::read_to_string(&path).context("Failed to read back from temp file")?;

    Ok(result)
}

/// Check if a command exists on PATH.
fn which_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        Command::new("where")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
