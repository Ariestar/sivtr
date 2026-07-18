//! Open the current terminal session log in the editor.

use anyhow::Result;
use sivtr_core::capture::scrollback;
use sivtr_core::config::SivtrConfig;
use sivtr_core::export::editor;
use sivtr_core::history::CaptureSource;

use super::history;

/// Open the current session log in the editor (optionally record history).
pub fn execute() -> Result<()> {
    match scrollback::read_session_log()? {
        Some(raw) => {
            if raw.trim().is_empty() {
                eprintln!("sivtr: session log is empty");
                return Ok(());
            }

            let config = SivtrConfig::load().unwrap_or_default();
            if let Err(error) = history::maybe_save_default(
                &config,
                &raw,
                Some("sivtr import"),
                CaptureSource::Import,
            ) {
                eprintln!("sivtr: failed to save history: {error:#}");
            }

            let ed = editor::resolve_editor_with_config(&config)?;
            eprintln!("sivtr: opening session log in {ed}");
            editor::open_in_editor(&raw)?;
            Ok(())
        }
        None => {
            eprintln!("sivtr: no session log found");
            eprintln!("  hint: run `sivtr init <shell>` then restart your terminal");
            Ok(())
        }
    }
}
