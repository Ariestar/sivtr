use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use crate::app::{App, AppMode};
use sivtr_core::buffer::cursor::Cursor;
use sivtr_core::selection::SelectionMode;

const LINE_NUMBER_WIDTH: usize = 6;

/// Handle a single crossterm event, updating the App state.
pub fn handle_event(app: &mut App) -> Result<()> {
    match event::read()? {
        Event::Key(key) => {
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
        Event::Mouse(mouse) => handle_mouse_event(app, mouse),
        Event::Resize(_, _) => {}
        _ => {}
    }
    Ok(())
}

fn handle_normal_mode(app: &mut App, key: KeyEvent) -> Result<()> {
    if handle_gg_prefix(app, key) {
        return Ok(());
    }

    app.clear_pending_prefixes();

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('e') => app.request_editor(),
        KeyCode::Char('/') => app.enter_search(),
        KeyCode::Char('n') => app.search_next(),
        KeyCode::Char('N') if key.modifiers.contains(KeyModifiers::SHIFT) => app.search_prev(),
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.enter_visual(SelectionMode::VisualBlock);
        }
        KeyCode::Char('V') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.enter_visual(SelectionMode::VisualLine);
        }
        KeyCode::Char('v') => app.enter_visual(SelectionMode::Visual),
        _ => {
            handle_motion_key(app, key);
        }
    }
    Ok(())
}

fn handle_visual_mode(app: &mut App, key: KeyEvent) -> Result<()> {
    if handle_gg_prefix(app, key) {
        return Ok(());
    }

    app.clear_pending_prefixes();

    match key.code {
        KeyCode::Esc => app.exit_visual(),
        KeyCode::Char('y') => app.yank_selection()?,
        KeyCode::Char('e') => app.request_editor(),
        KeyCode::Char('o') => app.swap_selection_anchor(),
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.enter_visual_from(SelectionMode::VisualBlock, current_anchor(app));
        }
        KeyCode::Char('V') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.enter_visual_from(SelectionMode::VisualLine, current_anchor(app));
        }
        KeyCode::Char('v') => app.enter_visual_from(SelectionMode::Visual, current_anchor(app)),
        _ => handle_motion_key(app, key),
    }

    Ok(())
}

fn handle_search_mode(app: &mut App, key: KeyEvent) {
    app.clear_pending_prefixes();
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

fn handle_motion_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.buffer.cursor_down(1),
        KeyCode::Char('k') | KeyCode::Up => app.buffer.cursor_up(1),
        KeyCode::Char('h') | KeyCode::Left => app.buffer.cursor_left(1),
        KeyCode::Char('l') | KeyCode::Right => app.buffer.cursor_right(1),
        KeyCode::Char('0') | KeyCode::Home => app.buffer.cursor_line_start(),
        KeyCode::Char('^') => app.buffer.cursor_first_nonblank(),
        KeyCode::Char('$') | KeyCode::End => app.buffer.cursor_line_end(),
        KeyCode::Char('H') if key.modifiers.contains(KeyModifiers::SHIFT) => app.buffer.cursor_view_top(),
        KeyCode::Char('M') if key.modifiers.contains(KeyModifiers::SHIFT) => app.buffer.cursor_view_middle(),
        KeyCode::Char('L') if key.modifiers.contains(KeyModifiers::SHIFT) => app.buffer.cursor_view_bottom(),
        KeyCode::Char('G') if key.modifiers.contains(KeyModifiers::SHIFT) => app.buffer.cursor_bottom(),
        KeyCode::PageDown => app.buffer.page_down(),
        KeyCode::PageUp => app.buffer.page_up(),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.buffer.half_page_down()
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.buffer.half_page_up()
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => app.buffer.page_down(),
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => app.buffer.page_up(),
        _ => {}
    }
}

fn handle_gg_prefix(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers == KeyModifiers::NONE && key.code == KeyCode::Char('g') {
        if app.pending_g {
            app.buffer.cursor_top();
            app.pending_g = false;
        } else {
            app.pending_g = true;
        }
        return true;
    }
    false
}

fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    app.clear_pending_prefixes();

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(cursor) = mouse_to_cursor(app, mouse.column, mouse.row) {
                app.buffer.set_cursor(cursor.row, cursor.col);
                let mode = if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                    SelectionMode::VisualBlock
                } else {
                    SelectionMode::Visual
                };
                app.begin_mouse_selection(cursor, mode);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(cursor) = mouse_to_cursor(app, mouse.column, mouse.row) {
                app.update_mouse_selection(cursor);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(cursor) = mouse_to_cursor(app, mouse.column, mouse.row) {
                if app.mouse_anchor.is_some() && app.mouse_dragged {
                    app.update_mouse_selection(cursor);
                } else {
                    app.buffer.set_cursor(cursor.row, cursor.col);
                }
            }
            app.finish_mouse_selection();
        }
        MouseEventKind::ScrollUp => app.buffer.cursor_up(3),
        MouseEventKind::ScrollDown => app.buffer.cursor_down(3),
        _ => {}
    }
}

fn mouse_to_cursor(app: &App, column: u16, row: u16) -> Option<Cursor> {
    let row = row as usize;
    if row >= app.buffer.viewport.height || app.buffer.line_count() == 0 {
        return None;
    }

    let absolute_row = (app.buffer.viewport.offset + row).min(app.buffer.line_count().saturating_sub(1));
    let content_col = (column as usize).saturating_sub(LINE_NUMBER_WIDTH);
    Some(Cursor::new(absolute_row, content_col))
}

fn current_anchor(app: &App) -> Cursor {
    app.buffer
        .selection
        .as_ref()
        .map(|selection| selection.anchor)
        .unwrap_or(app.buffer.cursor)
}
