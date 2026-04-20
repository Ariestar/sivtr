use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::*;

use crate::app::{App, AppMode};

use super::views;

const LINE_NUMBER_WIDTH: u16 = 6;

/// Main render function lays out and draws all views.
pub fn render(app: &App, frame: &mut Frame) {
    let size = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(if app.mode == AppMode::Search { 1 } else { 0 }),
        ])
        .split(size);

    views::browse::render(app, chunks[0], frame.buffer_mut());
    views::status::render(app, chunks[1], frame.buffer_mut());

    if app.mode == AppMode::Search {
        views::search::render(app, chunks[2], frame.buffer_mut());
        let search_x = chunks[2]
            .x
            .saturating_add(1 + app.search_input.chars().count() as u16);
        frame.set_cursor_position(Position::new(
            search_x.min(chunks[2].right().saturating_sub(1)),
            chunks[2].y,
        ));
    } else {
        let cursor_y = chunks[0].y.saturating_add(
            app.buffer
                .cursor
                .row
                .saturating_sub(app.buffer.viewport.offset) as u16,
        );
        let cursor_x = chunks[0]
            .x
            .saturating_add(LINE_NUMBER_WIDTH)
            .saturating_add(app.buffer.cursor.col as u16);
        frame.set_cursor_position(Position::new(
            cursor_x.min(chunks[0].right().saturating_sub(1)),
            cursor_y.min(chunks[0].bottom().saturating_sub(1)),
        ));
    }
}
