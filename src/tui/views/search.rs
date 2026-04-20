use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::App;

/// Render the search input bar at the bottom of the screen.
pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let cursor_style = Style::default().fg(Color::Yellow);
    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled("/", cursor_style),
        Span::raw(&app.search_input),
        Span::styled("_", Style::default().fg(Color::Yellow)),
    ]));
    paragraph.render(area, buf);
}
