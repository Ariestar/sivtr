use anyhow::Result;
use sift_core::capture::subprocess;
use sift_core::config::{SiftConfig, OpenMode};
use sift_core::parse;
use sift_core::buffer::Buffer;
use sift_core::export::editor;

use crate::app::App;
use super::browse;

/// Execute a command, capture its output, then open based on config.
pub fn execute(command: &str, args: &[String]) -> Result<()> {
    eprintln!("sift: running `{} {}`", command, args.join(" "));

    let result = subprocess::run_and_capture(command, args)?;

    match result.exit_code {
        Some(0) => eprintln!("sift: command exited successfully"),
        Some(code) => eprintln!("sift: command exited with code {}", code),
        None => eprintln!("sift: command was terminated by signal"),
    }

    if result.combined.is_empty() {
        eprintln!("sift: no output captured");
        return Ok(());
    }

    let config = SiftConfig::load().unwrap_or_default();

    match config.general.open_mode {
        OpenMode::Editor => {
            let ed = editor::resolve_editor_with_config(&config)?;
            eprintln!("sift: opening in {}", ed);
            editor::open_in_editor(&result.combined)?;
            Ok(())
        }
        OpenMode::Tui => {
            let lines = parse::parse_lines(&result.combined);
            let buffer = Buffer::new(lines);
            let mut app = App::new(buffer);
            app.config = config;
            browse::run_tui(&mut app)
        }
    }
}
