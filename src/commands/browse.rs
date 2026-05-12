use anyhow::{bail, Result};
use sivtr_core::config::SivtrConfig;
use sivtr_core::export::editor;
use sivtr_core::history::{store::CaptureSource, HistoryStore};

use crate::app::{App, StatusMessage};
use crate::tui;

/// Shared TUI event loop with external editor support.
/// Used by both `pipe` and `run` commands.
pub fn run_tui(app: &mut App, start_at_bottom: bool) -> Result<()> {
    ensure_tui_stdout()?;

    let mut terminal = tui::terminal::init()?;
    let size = terminal.size()?;
    app.buffer
        .resize(size.width as usize, size.height.saturating_sub(2) as usize);
    if start_at_bottom {
        app.buffer.cursor_bottom();
    }

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

        // Handle pending editor request: suspend TUI 鈫?launch editor 鈫?resume TUI
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
                    eprintln!("sivtr: editor error: {e}");
                    app.status = Some(StatusMessage {
                        text: format!("Editor error: {e}"),
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

pub(crate) fn record_history(
    config: &SivtrConfig,
    content: &str,
    command: Option<&str>,
    source: CaptureSource,
) {
    if let Err(error) = try_record_history(config, content, command, source) {
        eprintln!("sivtr: failed to save history: {error:#}");
    }
}

fn ensure_tui_stdout() -> Result<()> {
    if atty::is(atty::Stream::Stdout) {
        return Ok(());
    }

    bail!(
        "sivtr: TUI mode requires an interactive terminal\n  hint: run inside a terminal or set `general.open_mode = \"editor\"` in config"
    )
}

fn try_record_history(
    config: &SivtrConfig,
    content: &str,
    command: Option<&str>,
    source: CaptureSource,
) -> Result<()> {
    if !config.history.auto_save || content.trim().is_empty() {
        return Ok(());
    }

    let store = HistoryStore::open_default()?;
    try_record_history_with_store(&store, config, content, command, source)
}

fn try_record_history_with_store(
    store: &HistoryStore,
    config: &SivtrConfig,
    content: &str,
    command: Option<&str>,
    source: CaptureSource,
) -> Result<()> {
    if !config.history.auto_save || content.trim().is_empty() {
        return Ok(());
    }

    store.insert(content, command, source)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ensure_tui_stdout, try_record_history_with_store};
    use sivtr_core::config::SivtrConfig;
    use sivtr_core::history::{store::CaptureSource, HistoryStore};

    #[test]
    fn records_history_when_auto_save_is_enabled() {
        let store = HistoryStore::open_memory().unwrap();
        let config = SivtrConfig::default();

        try_record_history_with_store(
            &store,
            &config,
            "hello world",
            Some("echo hello"),
            CaptureSource::Run,
        )
        .unwrap();

        let entries = store.list_recent(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command.as_deref(), Some("echo hello"));
        assert_eq!(entries[0].source, "run");
    }

    #[test]
    fn skips_history_when_auto_save_is_disabled() {
        let store = HistoryStore::open_memory().unwrap();
        let mut config = SivtrConfig::default();
        config.history.auto_save = false;

        try_record_history_with_store(
            &store,
            &config,
            "hello world",
            Some("echo hello"),
            CaptureSource::Run,
        )
        .unwrap();

        let entries = store.list_recent(10).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn rejects_non_interactive_tui_stdout() {
        let error = ensure_tui_stdout().unwrap_err();

        assert!(error
            .to_string()
            .contains("TUI mode requires an interactive terminal"));
    }
}
