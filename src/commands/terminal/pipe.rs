//! Read stdin, save history, open in editor.

use anyhow::Result;
use sivtr_core::capture::pipe::read_stdin;
use sivtr_core::config::SivtrConfig;
use sivtr_core::export::editor;
use sivtr_core::history::CaptureSource;

use super::history;

/// Pipe mode: read stdin, optionally save history, open editor.
pub fn execute() -> Result<()> {
    let raw = read_stdin()?;

    if raw.is_empty() {
        eprintln!("sivtr: no input received from stdin");
        return Ok(());
    }

    let config = SivtrConfig::load().unwrap_or_default();
    if let Err(error) = history::maybe_save_default(&config, &raw, None, CaptureSource::Pipe) {
        eprintln!("sivtr: failed to save history: {error:#}");
    }

    let ed = editor::resolve_editor_with_config(&config)?;
    eprintln!("sivtr: opening in {ed}");
    editor::open_in_editor(&raw)?;
    Ok(())
}
