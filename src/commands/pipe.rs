use anyhow::Result;
use sift_core::capture::pipe::read_stdin;
use sift_core::config::{SiftConfig, OpenMode};
use sift_core::parse;
use sift_core::buffer::Buffer;
use sift_core::export::editor;

use crate::app::App;
use super::browse;

/// Execute pipe mode: read from stdin, then open based on config.
pub fn execute() -> Result<()> {
    let raw = read_stdin()?;

    if raw.is_empty() {
        eprintln!("sift: no input received from stdin");
        return Ok(());
    }

    let config = SiftConfig::load().unwrap_or_default();

    match config.general.open_mode {
        OpenMode::Editor => {
            let ed = editor::resolve_editor_with_config(&config)?;
            eprintln!("sift: opening in {}", ed);
            editor::open_in_editor(&raw)?;
            Ok(())
        }
        OpenMode::Tui => {
            let lines = parse::parse_lines(&raw);
            let buffer = Buffer::new(lines);
            let mut app = App::new(buffer);
            app.config = config;
            browse::run_tui(&mut app)
        }
    }
}
