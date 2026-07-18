use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::*;
use unicode_width::UnicodeWidthStr;

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
        let search_x = search_cursor_x(chunks[2], &app.search_input);
        frame.set_cursor_position(Position::new(
            search_x.min(chunks[2].right().saturating_sub(1)),
            chunks[2].y,
        ));
    } else {
        let cursor_col = if app.mode == AppMode::VisualBlock {
            app.buffer.preferred_col()
        } else {
            app.buffer.cursor.col
        };
        let cursor_y = chunks[0].y.saturating_add(
            app.buffer
                .cursor
                .row
                .saturating_sub(app.buffer.viewport.offset) as u16,
        );
        let cursor_x = chunks[0]
            .x
            .saturating_add(LINE_NUMBER_WIDTH)
            .saturating_add(cursor_col as u16);
        frame.set_cursor_position(Position::new(
            cursor_x.min(chunks[0].right().saturating_sub(1)),
            cursor_y.min(chunks[0].bottom().saturating_sub(1)),
        ));
    }
}

fn search_cursor_x(area: Rect, input: &str) -> u16 {
    let input_width = u16::try_from(UnicodeWidthStr::width(input)).unwrap_or(u16::MAX);
    area.x
        .saturating_add(1)
        .saturating_add(input_width)
        .min(area.right().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::search_cursor_x;

    #[test]
    fn search_cursor_uses_display_width_for_cjk_input() {
        let area = Rect::new(5, 2, 20, 1);

        assert_eq!(search_cursor_x(area, "中文"), 10);
        assert_eq!(search_cursor_x(area, "abc"), 9);
    }

    #[test]
    fn search_cursor_is_clamped_to_the_prompt_area() {
        let area = Rect::new(5, 2, 6, 1);

        assert_eq!(search_cursor_x(area, "很长的中文查询"), 10);
    }
}
