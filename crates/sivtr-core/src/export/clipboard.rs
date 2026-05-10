use anyhow::{Context, Result};
use arboard::Clipboard;
use std::time::{Duration, Instant};

const DEFAULT_LINUX_CLIPBOARD_HOLD_MS: u64 = 200;

fn parse_linux_clipboard_hold_ms(value: Option<&str>) -> u64 {
    value
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_LINUX_CLIPBOARD_HOLD_MS)
}

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
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
