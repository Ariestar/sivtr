use crate::app::{App, AppMode};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Render the status bar at the bottom of the screen.
pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let mode_str = match app.mode {
        AppMode::Normal => " NORMAL ",
        AppMode::Visual => " VISUAL ",
        AppMode::VisualLine => " V-LINE ",
        AppMode::VisualBlock => " V-BLOCK ",
        AppMode::Search => " SEARCH ",
    };

    let mode_style = match app.mode {
        AppMode::Normal => Style::default().bg(Color::Blue).fg(Color::White).bold(),
        AppMode::Visual | AppMode::VisualLine | AppMode::VisualBlock => {
            Style::default().bg(Color::Magenta).fg(Color::White).bold()
        }
        AppMode::Search => Style::default().bg(Color::Yellow).fg(Color::Black).bold(),
    };

    let position = format!(
        " {}:{} ",
        app.buffer.cursor.row + 1,
        app.buffer.cursor.col + 1
    );

    let total_lines = format!(" {} lines ", app.buffer.line_count());

    let status_text = app
        .status
        .as_ref()
        .map(|s| s.text.clone())
        .unwrap_or_default();

    let status_style = if app.status.as_ref().map(|s| s.is_error).unwrap_or(false) {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Gray)
    };

    // Calculate spacing
    let left_len = mode_str.len() + status_text.len();
    let right_len = position.len() + total_lines.len();
    let middle_width = (area.width as usize).saturating_sub(left_len + right_len);
    let middle_pad = " ".repeat(middle_width);

    let line = Line::from(vec![
        Span::styled(mode_str, mode_style),
        Span::styled(format!(" {}", status_text), status_style),
        Span::raw(middle_pad),
        Span::styled(total_lines, Style::default().fg(Color::DarkGray)),
        Span::styled(
            position,
            Style::default().bg(Color::DarkGray).fg(Color::White),
        ),
    ]);

    let paragraph = Paragraph::new(line);
    paragraph.render(area, buf);
}
