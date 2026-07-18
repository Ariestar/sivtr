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

    let (program, extra_args) = parse_editor_command(&editor)?;

    // Spawn editor
    let status = Command::new(&program)
        .args(&extra_args)
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to launch editor '{editor}'"))?;

    if !status.success() {
        anyhow::bail!("Editor '{editor}' exited with {status}");
    }

    // Read back content
    let result = std::fs::read_to_string(&path).context("Failed to read back from temp file")?;

    Ok(result)
}

/// Parse a configured editor command while preserving quoted paths and arguments.
pub fn parse_editor_command(command: &str) -> Result<(String, Vec<String>)> {
    let command = command.trim();
    let mut parts = split_editor_command(command)?;
    if parts.is_empty() {
        anyhow::bail!("Empty editor command");
    }

    let program = parts.remove(0);
    if program.is_empty() {
        anyhow::bail!("Empty editor command program");
    }
    Ok((program, parts))
}

#[cfg(not(windows))]
fn split_editor_command(command: &str) -> Result<Vec<String>> {
    shell_words::split(command)
        .with_context(|| format!("Invalid editor command quoting: `{command}`"))
}

#[cfg(windows)]
fn split_editor_command(command: &str) -> Result<Vec<String>> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, slice};
    use winapi::um::{shellapi::CommandLineToArgvW, winbase::LocalFree};

    if command.trim().is_empty() {
        return Ok(Vec::new());
    }

    let wide = command
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut count = 0;
    let argv = unsafe { CommandLineToArgvW(wide.as_ptr(), &mut count) };
    if argv.is_null() {
        anyhow::bail!(
            "Invalid editor command `{command}`: {}",
            std::io::Error::last_os_error()
        );
    }

    let mut parts = Vec::with_capacity(count as usize);
    for &argument in unsafe { slice::from_raw_parts(argv, count as usize) } {
        let mut len = 0;
        while unsafe { *argument.add(len) } != 0 {
            len += 1;
        }
        let value = unsafe { slice::from_raw_parts(argument, len) };
        parts.push(OsString::from_wide(value).to_string_lossy().into_owned());
    }
    unsafe {
        LocalFree(argv.cast());
    }

    Ok(parts)
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

#[cfg(test)]
mod tests {
    use super::parse_editor_command;

    #[test]
    fn parses_quoted_windows_editor_path_and_arguments() {
        let (program, args) =
            parse_editor_command(r#"  "C:\Program Files\Neovim\bin\nvim.exe" --clean  "#).unwrap();

        assert_eq!(program, r#"C:\Program Files\Neovim\bin\nvim.exe"#);
        assert_eq!(args, vec!["--clean"]);
    }

    #[test]
    fn rejects_an_explicitly_empty_editor_program() {
        assert!(parse_editor_command(r#""" --clean"#).is_err());
    }
}
