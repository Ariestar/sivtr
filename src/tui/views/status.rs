use crate::app::{App, AppMode};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// Render the status bar at the bottom of the screen.
pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let mode_str = match app.mode {
        AppMode::Normal => " NORMAL ",
        AppMode::Insert => " INSERT ",
        AppMode::Visual => " VISUAL ",
        AppMode::VisualLine => " V-LINE ",
        AppMode::VisualBlock => " V-BLOCK ",
        AppMode::Search => " SEARCH ",
    };

    let mode_style = match app.mode {
        AppMode::Normal => Style::default().bg(Color::Blue).fg(Color::White).bold(),
        AppMode::Insert => Style::default().bg(Color::Green).fg(Color::Black).bold(),
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

    let status_segment = format!(" {status_text}");
    let middle_width = status_middle_width(
        area.width,
        mode_str,
        &status_segment,
        &total_lines,
        &position,
    );
    let middle_pad = " ".repeat(middle_width);

    let line = Line::from(vec![
        Span::styled(mode_str, mode_style),
        Span::styled(status_segment, status_style),
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

fn status_middle_width(
    area_width: u16,
    mode: &str,
    status: &str,
    total_lines: &str,
    position: &str,
) -> usize {
    let occupied = UnicodeWidthStr::width(mode)
        + UnicodeWidthStr::width(status)
        + UnicodeWidthStr::width(total_lines)
        + UnicodeWidthStr::width(position);
    usize::from(area_width).saturating_sub(occupied)
}

#[cfg(test)]
mod tests {
    use super::status_middle_width;

    #[test]
    fn status_spacing_uses_display_width_for_cjk_text() {
        assert_eq!(
            status_middle_width(40, " NORMAL ", " 中文错误", " 10 lines ", " 2:3 "),
            8
        );
    }

    #[test]
    fn status_spacing_saturates_when_content_is_wider_than_the_bar() {
        assert_eq!(
            status_middle_width(10, " NORMAL ", " 很长的错误信息", " 10 lines ", " 2:3 "),
            0
        );
    }
}
