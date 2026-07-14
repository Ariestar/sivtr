use anyhow::{anyhow, Context, Result};
#[cfg(windows)]
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{buffer::CellDiffOption, prelude::*};
use std::io::{self, Stdout};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Initialize the terminal for TUI rendering.
pub fn init() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Draw one TUI frame.
///
/// Windows terminals can leave stale trailing cells behind when Ratatui's incremental diff
/// replaces CJK or other wide glyphs. Force every rendered cell into the update without issuing a
/// physical screen clear, then hold the update until the complete frame is ready.
pub fn draw<F>(terminal: &mut Tui, render: F) -> Result<()>
where
    F: FnOnce(&mut Frame),
{
    #[cfg(windows)]
    {
        execute!(terminal.backend_mut(), BeginSynchronizedUpdate)
            .context("Failed to begin synchronized terminal update")?;

        let draw_result = (|| -> Result<()> {
            synchronize_terminal_size(terminal, || Ok(CrosstermBackend::new(io::stdout())))
                .context("Failed to synchronize the Windows terminal size")?;
            draw_terminal_frame(terminal, true, render)
                .context("Failed to draw a full Windows terminal frame")
        })();
        let end_result = execute!(terminal.backend_mut(), EndSynchronizedUpdate)
            .context("Failed to end synchronized terminal update");

        match (draw_result, end_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(draw_error), Ok(())) => Err(draw_error),
            (Ok(()), Err(end_error)) => Err(end_error),
            (Err(draw_error), Err(end_error)) => Err(anyhow!(
                "{draw_error:#}; additionally failed to end synchronized terminal update: {end_error:#}"
            )),
        }
    }

    #[cfg(not(windows))]
    {
        draw_terminal_frame(terminal, false, render).context("Failed to draw terminal frame")
    }
}

fn synchronize_terminal_size<B, F>(
    terminal: &mut Terminal<B>,
    create_backend: F,
) -> std::result::Result<bool, B::Error>
where
    B: Backend,
    F: FnOnce() -> std::result::Result<B, B::Error>,
{
    let backend_size = terminal.backend().size()?;
    let viewport_size = terminal.get_frame().area().as_size();
    if backend_size == viewport_size {
        return Ok(false);
    }

    *terminal = Terminal::new(create_backend()?)?;
    Ok(true)
}

fn draw_terminal_frame<B, F>(
    terminal: &mut Terminal<B>,
    full_redraw: bool,
    render: F,
) -> std::result::Result<(), B::Error>
where
    B: Backend,
    F: FnOnce(&mut Frame),
{
    terminal.draw(|frame| {
        render(frame);
        if full_redraw {
            for cell in &mut frame.buffer_mut().content {
                cell.set_diff_option(CellDiffOption::AlwaysUpdate);
            }
        }
    })?;
    Ok(())
}

/// Restore the terminal to its original state.
pub fn restore(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, text::Line, widgets::Paragraph, Terminal};

    use super::draw_terminal_frame;

    #[test]
    fn full_redraw_overwrites_stale_wide_glyphs_without_clearing() {
        let backend = TestBackend::with_lines([
            "旧帧中文残留 ABC    ",
            "한글 stale 😀      ",
            "should disappear   ",
        ]);
        let mut terminal = Terminal::new(backend).expect("terminal");

        draw_terminal_frame(&mut terminal, true, |frame| {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from("[x] 新会话 中文 한글 😀"),
                    Line::from("[ ] second row"),
                ]),
                frame.area(),
            );
        })
        .expect("first full redraw");
        draw_terminal_frame(&mut terminal, true, |frame| {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from("[ ] 新会话 中文 한글 😀"),
                    Line::from("[x] second row"),
                ]),
                frame.area(),
            );
        })
        .expect("second full redraw");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "[");
        assert_eq!(buffer[(1, 0)].symbol(), " ");
        assert_eq!(buffer[(2, 0)].symbol(), "]");
        assert_eq!(buffer[(0, 1)].symbol(), "[");
        assert_eq!(buffer[(1, 1)].symbol(), "x");
        assert!(buffer.content[40..].iter().all(|cell| cell.symbol() == " "));
    }

    #[test]
    fn full_redraw_marks_every_single_width_cell_for_output() {
        let backend = TestBackend::new(8, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");

        draw_terminal_frame(&mut terminal, true, |frame| {
            frame.render_widget(Paragraph::new("text"), frame.area());
        })
        .expect("full redraw");

        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .all(|cell| cell.diff_option == ratatui::buffer::CellDiffOption::AlwaysUpdate));
    }

    #[test]
    fn incremental_draw_keeps_the_existing_non_windows_policy() {
        let backend = TestBackend::new(12, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");

        draw_terminal_frame(&mut terminal, false, |frame| {
            frame.render_widget(Paragraph::new("plain frame"), frame.area());
        })
        .expect("incremental redraw");

        terminal
            .backend()
            .assert_buffer_lines(["plain frame ", "            "]);
    }

    #[test]
    fn resize_recreates_buffers_without_using_terminal_clear() {
        let backend = TestBackend::new(8, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.backend_mut().resize(14, 4);

        let recreated =
            super::synchronize_terminal_size(&mut terminal, || Ok(TestBackend::new(14, 4)))
                .expect("resize synchronization");

        assert!(recreated);
        assert_eq!(terminal.get_frame().area().as_size(), (14, 4).into());
    }
}
