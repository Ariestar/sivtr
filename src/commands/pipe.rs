use anyhow::Result;
use sivtr_core::buffer::Buffer;
use sivtr_core::capture::pipe::read_stdin;
use sivtr_core::config::{OpenMode, SivtrConfig};
use sivtr_core::export::editor;
use sivtr_core::history::CaptureSource;
use sivtr_core::parse;

use super::{browse, capture_history};
use crate::app::App;

/// Execute pipe mode: read from stdin, then open based on config.
pub fn execute() -> Result<()> {
    let raw = read_stdin()?;

    if raw.is_empty() {
        eprintln!("sivtr: no input received from stdin");
        return Ok(());
    }

    let config = SivtrConfig::load().unwrap_or_default();
    if let Err(error) =
        capture_history::maybe_save_default(&config, &raw, None, CaptureSource::Pipe)
    {
        eprintln!("sivtr: failed to save history: {error:#}");
    }

    match config.general.open_mode {
        OpenMode::Editor => {
            let ed = editor::resolve_editor_with_config(&config)?;
            eprintln!("sivtr: opening in {ed}");
            editor::open_in_editor(&raw)?;
            Ok(())
        }
        OpenMode::Tui => {
            let lines = parse::parse_lines(&raw);
            let buffer = Buffer::new(lines);
            let mut app = App::new(buffer);
            app.config = config;
            browse::run_tui(&mut app, false)
        }
    }
}
