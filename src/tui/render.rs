use ratatui::prelude::*;
use ratatui::layout::{Constraint, Direction, Layout};
use crate::app::{App, AppMode};
use super::views;

/// Main render function — lays out and draws all views.
pub fn render(app: &App, frame: &mut Frame) {
    let size = frame.area();

    // Layout: main content area + status bar (1 line) + optional search bar (1 line)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1), // status bar
            Constraint::Length(if app.mode == AppMode::Search { 1 } else { 0 }),
        ])
        .split(size);

    // Render main browse view
    views::browse::render(app, chunks[0], frame.buffer_mut());

    // Render status bar
    views::status::render(app, chunks[1], frame.buffer_mut());

    // Render search bar if in search mode
    if app.mode == AppMode::Search {
        views::search::render(app, chunks[2], frame.buffer_mut());
    }
}
