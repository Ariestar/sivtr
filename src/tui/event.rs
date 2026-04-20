use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crate::app::{App, AppMode};
use sift_core::selection::SelectionMode;

/// Handle a single crossterm event, updating the App state.
pub fn handle_event(app: &mut App) -> Result<()> {
    if let Event::Key(key) = event::read()? {
        // Only handle key press events (ignore release/repeat — fixes double-move on Windows)
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        match app.mode {
            AppMode::Normal => handle_normal_mode(app, key)?,
            AppMode::Visual | AppMode::VisualLine | AppMode::VisualBlock => {
                handle_visual_mode(app, key)?;
            }
            AppMode::Search => handle_search_mode(app, key),
        }
    }
    Ok(())
}

fn handle_normal_mode(app: &mut App, key: KeyEvent) -> Result<()> {
    match (key.modifiers, key.code) {
        // Quit
        (_, KeyCode::Char('q')) => app.should_quit = true,

        // Navigation
        (_, KeyCode::Char('j')) | (_, KeyCode::Down) => app.buffer.cursor_down(1),
        (_, KeyCode::Char('k')) | (_, KeyCode::Up) => app.buffer.cursor_up(1),
        (_, KeyCode::Char('h')) | (_, KeyCode::Left) => app.buffer.cursor_left(1),
        (_, KeyCode::Char('l')) | (_, KeyCode::Right) => app.buffer.cursor_right(1),

        // Page navigation
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => app.buffer.half_page_down(),
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => app.buffer.half_page_up(),

        // Jump to top/bottom
        (_, KeyCode::Char('g')) => app.buffer.cursor_top(), // simplified: single 'g' goes to top
        (KeyModifiers::SHIFT, KeyCode::Char('G')) => app.buffer.cursor_bottom(),

        // Enter visual modes (order matters: check modifiers first)
        (KeyModifiers::CONTROL, KeyCode::Char('v')) => {
            app.enter_visual(SelectionMode::VisualBlock);
        }
        (KeyModifiers::SHIFT, KeyCode::Char('V')) => {
            app.enter_visual(SelectionMode::VisualLine);
        }
        (KeyModifiers::NONE, KeyCode::Char('v')) => app.enter_visual(SelectionMode::Visual),

        // Open in external editor (full buffer)
        (_, KeyCode::Char('e')) => app.request_editor(),

        // Search
        (_, KeyCode::Char('/')) => app.enter_search(),
        (_, KeyCode::Char('n')) => app.search_next(),
        (KeyModifiers::SHIFT, KeyCode::Char('N')) => app.search_prev(),

        _ => {}
    }
    Ok(())
}

fn handle_visual_mode(app: &mut App, key: KeyEvent) -> Result<()> {
    match (key.modifiers, key.code) {
        // Cancel selection
        (_, KeyCode::Esc) => app.exit_visual(),

        // Yank selection
        (_, KeyCode::Char('y')) => app.yank_selection()?,

        // Open selection in external editor
        (_, KeyCode::Char('e')) => app.request_editor(),

        // Navigation within selection
        (_, KeyCode::Char('j')) | (_, KeyCode::Down) => app.buffer.cursor_down(1),
        (_, KeyCode::Char('k')) | (_, KeyCode::Up) => app.buffer.cursor_up(1),
        (_, KeyCode::Char('h')) | (_, KeyCode::Left) => app.buffer.cursor_left(1),
        (_, KeyCode::Char('l')) | (_, KeyCode::Right) => app.buffer.cursor_right(1),

        (KeyModifiers::CONTROL, KeyCode::Char('d')) => app.buffer.half_page_down(),
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => app.buffer.half_page_up(),

        (KeyModifiers::SHIFT, KeyCode::Char('G')) => app.buffer.cursor_bottom(),
        (_, KeyCode::Char('g')) => app.buffer.cursor_top(),

        _ => {}
    }
    Ok(())
}

fn handle_search_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => app.execute_search(),
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Backspace => {
            app.search_input.pop();
        }
        KeyCode::Char(c) => {
            app.search_input.push(c);
        }
        _ => {}
    }
}
