use anyhow::Result;
use sivtr_core::capture::scrollback;
use sivtr_core::config::{SivtrConfig, OpenMode};
use sivtr_core::parse;
use sivtr_core::buffer::Buffer;
use sivtr_core::export::editor;

use crate::app::App;
use super::browse;

/// Capture current terminal scrollback and open it.
pub fn execute() -> Result<()> {
    match scrollback::capture_scrollback()? {
        Some(raw) => {
            if raw.trim().is_empty() {
                eprintln!("sivtr: captured scrollback is empty");
                return Ok(());
            }

            let config = SivtrConfig::load().unwrap_or_default();

            match config.general.open_mode {
                OpenMode::Editor => {
                    let ed = editor::resolve_editor_with_config(&config)?;
                    eprintln!("sivtr: opening scrollback in {}", ed);
                    editor::open_in_editor(&raw)?;
                    Ok(())
                }
                OpenMode::Tui => {
                    let lines = parse::parse_lines(&raw);
                    let mut buffer = Buffer::new(lines);
                    // Start at the bottom (most recent output)
                    buffer.cursor_bottom();
                    let mut app = App::new(buffer);
                    app.config = config;
                    browse::run_tui(&mut app)
                }
            }
        }
        None => {
            eprintln!("sivtr: no session log found");
            eprintln!("  hint: run `sivtr init <shell>` then restart your terminal");
            Ok(())
        }
    }
}
