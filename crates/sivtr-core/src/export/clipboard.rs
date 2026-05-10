use anyhow::{Context, Result};
use arboard::Clipboard;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_LINUX_CLIPBOARD_HOLD_MS: u64 = 200;

fn parse_linux_clipboard_hold_ms(value: Option<&str>) -> u64 {
    value
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_LINUX_CLIPBOARD_HOLD_MS)
}

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    if let Ok(()) = copy_with_arboard(text) {
        return Ok(());
    }

    copy_with_external_clipboard(text)
}

fn copy_with_arboard(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new().context("Failed to open clipboard")?;

    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
    ))]
    {
        use arboard::SetExtLinux;

        // Keep clipboard ownership briefly so managers/paste targets can request data
        // before short-lived CLI processes exit.
        let hold_env = std::env::var("SIVTR_LINUX_CLIPBOARD_HOLD_MS").ok();
        let hold_ms = parse_linux_clipboard_hold_ms(hold_env.as_deref());

        if hold_ms > 0 {
            let deadline = Instant::now() + Duration::from_millis(hold_ms);
            clipboard
                .set()
                .wait_until(deadline)
                .text(text)
                .context("Failed to set clipboard")?;
            return Ok(());
        }
    }

    clipboard
        .set_text(text)
        .context("Failed to set clipboard")?;

    Ok(())
}

fn copy_with_external_clipboard(text: &str) -> Result<()> {
    for candidate in clipboard_command_candidates() {
        if try_copy_with_command(candidate, text).is_ok() {
            return Ok(());
        }
    }

    anyhow::bail!("Failed to copy clipboard via arboard and external clipboard commands")
}

fn try_copy_with_command(command: &[&str], text: &str) -> Result<()> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Empty clipboard command"))?;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to spawn clipboard command `{}`", command.join(" ")))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "Clipboard command `{}` exited with {status}",
            command.join(" ")
        )
    }
}

fn clipboard_command_candidates() -> Vec<&'static [&'static str]> {
    let mut candidates: Vec<&'static [&'static str]> = Vec::new();

    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
    ))]
    {
        candidates.push(&["wl-copy"]);
        candidates.push(&["xclip", "-selection", "clipboard", "-in"]);
        candidates.push(&["xsel", "--clipboard", "--input"]);
    }

    #[cfg(target_os = "macos")]
    {
        candidates.push(&["pbcopy"]);
    }

    #[cfg(windows)]
    {
        candidates.push(&["clip"]);
        candidates.push(&["clip.exe"]);
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::{parse_linux_clipboard_hold_ms, DEFAULT_LINUX_CLIPBOARD_HOLD_MS};

    #[test]
    fn parses_default_linux_clipboard_hold_ms_when_missing() {
        assert_eq!(
            parse_linux_clipboard_hold_ms(None),
            DEFAULT_LINUX_CLIPBOARD_HOLD_MS
        );
    }

    #[test]
    fn parses_default_linux_clipboard_hold_ms_when_invalid() {
        assert_eq!(
            parse_linux_clipboard_hold_ms(Some("not-a-number")),
            DEFAULT_LINUX_CLIPBOARD_HOLD_MS
        );
    }

    #[test]
    fn parses_linux_clipboard_hold_ms_from_numeric_values() {
        assert_eq!(parse_linux_clipboard_hold_ms(Some("0")), 0);
        assert_eq!(parse_linux_clipboard_hold_ms(Some("1200")), 1200);
        assert_eq!(parse_linux_clipboard_hold_ms(Some(" 350 ")), 350);
    }
}
