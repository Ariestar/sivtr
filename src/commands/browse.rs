use anyhow::Result;
use sift_core::export::editor;

use crate::app::{App, StatusMessage};
use crate::tui;

/// Shared TUI event loop with external editor support.
/// Used by both `pipe` and `run` commands.
pub fn run_tui(app: &mut App) -> Result<()> {
    let mut terminal = tui::terminal::init()?;
    let size = terminal.size()?;
    app.buffer
        .resize(size.width as usize, size.height.saturating_sub(2) as usize);

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            app.buffer
                .resize(area.width as usize, area.height.saturating_sub(2) as usize);
            tui::render::render(app, frame);
        })?;

        tui::event::handle_event(app)?;

        if app.should_quit {
            break;
        }

        // Handle pending editor request: suspend TUI → launch editor → resume TUI
        if app.pending_editor {
            app.pending_editor = false;

            let content = app.get_content_for_editor();

            // Suspend TUI
            tui::terminal::restore(&mut terminal)?;

            // Launch editor
            match editor::open_in_editor(&content) {
                Ok(_) => {
                    app.status = Some(StatusMessage {
                        text: "Editor closed".to_string(),
                        is_error: false,
                    });
                }
                Err(e) => {
                    eprintln!("sift: editor error: {}", e);
                    app.status = Some(StatusMessage {
                        text: format!("Editor error: {}", e),
                        is_error: true,
                    });
                }
            }

            // Exit visual mode if we were in one
            app.exit_visual();

            // Resume TUI
            terminal = tui::terminal::init()?;
        }
    }

    tui::terminal::restore(&mut terminal)?;
    Ok(())
}
