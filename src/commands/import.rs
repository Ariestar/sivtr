use anyhow::Result;
use sivtr_core::buffer::Buffer;
use sivtr_core::capture::scrollback;
use sivtr_core::config::{OpenMode, SivtrConfig};
use sivtr_core::export::editor;
use sivtr_core::parse;

use super::browse;
use crate::app::App;
use crate::command_blocks;

/// Open the current session log.
pub fn execute() -> Result<()> {
    match scrollback::read_session_log()? {
        Some(raw) => {
            if raw.trim().is_empty() {
                eprintln!("sivtr: session log is empty");
                return Ok(());
            }

            let config = SivtrConfig::load().unwrap_or_default();

            match config.general.open_mode {
                OpenMode::Editor => {
                    let ed = editor::resolve_editor_with_config(&config)?;
                    eprintln!("sivtr: opening session log in {ed}");
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
                    app.command_blocks =
                        command_blocks::load_from_session_log()?.unwrap_or_default();
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
