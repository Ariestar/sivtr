use anyhow::Result;
use sivtr_core::buffer::Buffer;
use sivtr_core::capture::subprocess;
use sivtr_core::config::{OpenMode, SivtrConfig};
use sivtr_core::export::editor;
use sivtr_core::parse;

use super::browse;
use crate::app::App;

/// Execute a command, capture its output, then open based on config.
pub fn execute(command: &str, args: &[String]) -> Result<()> {
    eprintln!("sivtr: running `{} {}`", command, args.join(" "));

    let result = subprocess::run_and_capture(command, args)?;

    match result.exit_code {
        Some(0) => eprintln!("sivtr: command exited successfully"),
        Some(code) => eprintln!("sivtr: command exited with code {code}"),
        None => eprintln!("sivtr: command was terminated by signal"),
    }

    if result.combined.is_empty() {
        eprintln!("sivtr: no output captured");
        return Ok(());
    }

    let config = SivtrConfig::load().unwrap_or_default();

    match config.general.open_mode {
        OpenMode::Editor => {
            let ed = editor::resolve_editor_with_config(&config)?;
            eprintln!("sivtr: opening in {ed}");
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
