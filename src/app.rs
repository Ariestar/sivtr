use sivtr_core::buffer::Buffer;
use sivtr_core::config::SivtrConfig;
use sivtr_core::search::SearchState;
use sivtr_core::search::matcher;
use sivtr_core::buffer::cursor::Cursor;
use sivtr_core::selection::{Selection, SelectionMode};
use sivtr_core::export;

use anyhow::Result;

/// Application mode 鈥?maps to the TUI state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    /// Normal browsing mode (hjkl navigation).
    Normal,
    /// Character-wise visual selection.
    Visual,
    /// Line-wise visual selection.
    VisualLine,
    /// Block/column visual selection.
    VisualBlock,
    /// Search input mode.
    Search,
}

/// Status message shown at the bottom.
#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub is_error: bool,
}

/// Main application state.
pub struct App {
    pub buffer: Buffer,
    pub mode: AppMode,
    pub config: SivtrConfig,
    pub search_state: Option<SearchState>,
    pub search_input: String,
    pub status: Option<StatusMessage>,
    pub should_quit: bool,
    /// Flag: the event loop should suspend TUI and open the editor.
    pub pending_editor: bool,
    /// Pending Vim-style prefix state (currently only `g`).
    pub pending_g: bool,
    pub mouse_anchor: Option<Cursor>,
    pub mouse_mode: SelectionMode,
    pub mouse_dragged: bool,
}

impl App {
    pub fn new(buffer: Buffer) -> Self {
        Self {
            buffer,
            mode: AppMode::Normal,
            config: SivtrConfig::default(),
            search_state: None,
            search_input: String::new(),
            status: None,
            should_quit: false,
            pending_editor: false,
            pending_g: false,
            mouse_anchor: None,
            mouse_mode: SelectionMode::Visual,
            mouse_dragged: false,
        }
    }

    /// Enter visual selection mode.
    pub fn enter_visual(&mut self, mode: SelectionMode) {
        let anchor = self.buffer.cursor;
        self.enter_visual_from(mode, anchor);
    }

    pub fn enter_visual_from(&mut self, mode: SelectionMode, anchor: Cursor) {
        self.buffer.selection = Some(Selection::new(mode, anchor));
        self.mode = match mode {
            SelectionMode::Visual => AppMode::Visual,
            SelectionMode::VisualLine => AppMode::VisualLine,
            SelectionMode::VisualBlock => AppMode::VisualBlock,
        };
    }

    /// Exit selection mode back to normal.
    pub fn exit_visual(&mut self) {
        self.buffer.selection = None;
        self.mode = AppMode::Normal;
        self.cancel_mouse_selection();
    }

    /// Yank (copy) the current selection to the system clipboard.
    pub fn yank_selection(&mut self) -> Result<()> {
        if let Some(ref sel) = self.buffer.selection {
            let text = sivtr_core::selection::extract::extract_selection(
                &self.buffer,
                sel,
                &self.buffer.cursor,
            );
            export::clipboard::copy_to_clipboard(&text)?;
            let line_count = text.lines().count();
            self.status = Some(StatusMessage {
                text: format!("{} lines yanked to clipboard", line_count),
                is_error: false,
            });
        }
        self.exit_visual();
        Ok(())
    }

    /// Enter search mode.
    pub fn enter_search(&mut self) {
        self.mode = AppMode::Search;
        self.search_input.clear();
    }

    /// Execute the current search.
    pub fn execute_search(&mut self) {
        if self.search_input.is_empty() {
            self.mode = AppMode::Normal;
            return;
        }

        let matches = matcher::find_all(&self.buffer.lines, &self.search_input, false);
        let count = matches.len();

        let mut state = SearchState::new(self.search_input.clone(), matches);

        // Jump to first match at or after current cursor
        state.jump_to_row(self.buffer.cursor.row);

        // Move cursor to the current match
        if let Some(m) = state.current_match() {
            self.buffer.cursor.row = m.row;
            self.buffer.ensure_cursor_visible_pub();
        }

        self.status = Some(StatusMessage {
            text: format!("[{}/{}] matches", state.current.map(|i| i + 1).unwrap_or(0), count),
            is_error: count == 0,
        });

        self.search_state = Some(state);
        self.mode = AppMode::Normal;
    }

    /// Jump to next search match.
    pub fn search_next(&mut self) {
        if let Some(ref mut state) = self.search_state {
            state.next();
            if let Some(m) = state.current_match() {
                self.buffer.cursor.row = m.row;
                self.buffer.ensure_cursor_visible_pub();
            }
            self.update_search_status();
        }
    }

    /// Jump to previous search match.
    pub fn search_prev(&mut self) {
        if let Some(ref mut state) = self.search_state {
            state.prev();
            if let Some(m) = state.current_match() {
                self.buffer.cursor.row = m.row;
                self.buffer.ensure_cursor_visible_pub();
            }
            self.update_search_status();
        }
    }

    /// Cancel search input.
    pub fn cancel_search(&mut self) {
        self.search_input.clear();
        self.mode = AppMode::Normal;
    }

    /// Request opening the editor (actual launch is handled by the event loop).
    pub fn request_editor(&mut self) {
        self.pending_editor = true;
    }

    pub fn swap_selection_anchor(&mut self) {
        if let Some(ref mut selection) = self.buffer.selection {
            std::mem::swap(&mut selection.anchor, &mut self.buffer.cursor);
            self.buffer.ensure_cursor_visible_pub();
        }
    }

    pub fn clear_pending_prefixes(&mut self) {
        self.pending_g = false;
    }

    pub fn begin_mouse_selection(&mut self, anchor: Cursor, mode: SelectionMode) {
        self.mouse_anchor = Some(anchor);
        self.mouse_mode = mode;
        self.mouse_dragged = false;
        self.clear_pending_prefixes();
    }

    pub fn update_mouse_selection(&mut self, cursor: Cursor) {
        self.buffer.set_cursor(cursor.row, cursor.col);
        if let Some(anchor) = self.mouse_anchor {
            if !self.mouse_dragged {
                self.enter_visual_from(self.mouse_mode, anchor);
                self.mouse_dragged = true;
            } else {
                self.buffer.ensure_cursor_visible_pub();
            }
        }
    }

    pub fn cancel_mouse_selection(&mut self) {
        self.mouse_anchor = None;
        self.mouse_dragged = false;
    }

    pub fn finish_mouse_selection(&mut self) {
        if !self.mouse_dragged && matches!(self.mode, AppMode::Visual | AppMode::VisualLine | AppMode::VisualBlock) {
            self.exit_visual();
        } else {
            self.cancel_mouse_selection();
        }
    }

    /// Get the text to send to the editor: selection if active, otherwise full buffer.
    pub fn get_content_for_editor(&self) -> String {
        if let Some(ref sel) = self.buffer.selection {
            sivtr_core::selection::extract::extract_selection(
                &self.buffer,
                sel,
                &self.buffer.cursor,
            )
        } else {
            self.buffer
                .lines
                .iter()
                .map(|l| l.content.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    fn update_search_status(&mut self) {
        if let Some(ref state) = self.search_state {
            self.status = Some(StatusMessage {
                text: format!(
                    "[{}/{}] '{}'",
                    state.current.map(|i| i + 1).unwrap_or(0),
                    state.match_count(),
                    state.pattern
                ),
                is_error: false,
            });
        }
    }
}