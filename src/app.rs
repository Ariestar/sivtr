use crate::command_blocks::{CommandBlockSpan, CopyTarget, SelectTarget};
use sivtr_core::buffer::cursor::Cursor;
use sivtr_core::buffer::Buffer;
use sivtr_core::config::SivtrConfig;
use sivtr_core::export;
use sivtr_core::search::matcher;
use sivtr_core::search::SearchState;
use sivtr_core::selection::{Selection, SelectionMode};

use anyhow::Result;

/// Application mode 鈥?maps to the TUI state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    /// Normal browsing mode (hjkl navigation).
    Normal,
    /// Insert mode (read-only typing state).
    Insert,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPrefix {
    G,
    LeftBracket,
    RightBracket,
    M,
    My,
    Mv,
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
    pub pending_prefix: Option<PendingPrefix>,
    pub mouse_anchor: Option<Cursor>,
    pub mouse_mode: SelectionMode,
    pub mouse_dragged: bool,
    pub command_blocks: Vec<CommandBlockSpan>,
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
            pending_prefix: None,
            mouse_anchor: None,
            mouse_mode: SelectionMode::Visual,
            mouse_dragged: false,
            command_blocks: Vec::new(),
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
                &self.selection_cursor(),
            );
            export::clipboard::copy_to_clipboard(&text)?;
            let line_count = text.lines().count();
            self.status = Some(StatusMessage {
                text: format!("{line_count} lines yanked to clipboard"),
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

    /// Enter insert mode.
    pub fn enter_insert(&mut self) {
        self.mode = AppMode::Insert;
        self.clear_pending_prefixes();
    }

    /// Exit insert mode back to normal.
    pub fn exit_insert(&mut self) {
        self.mode = AppMode::Normal;
        self.clear_pending_prefixes();
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
            text: format!(
                "[{}/{}] matches",
                state.current.map(|i| i + 1).unwrap_or(0),
                count
            ),
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
        self.pending_prefix = None;
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
        if !self.mouse_dragged
            && matches!(
                self.mode,
                AppMode::Visual | AppMode::VisualLine | AppMode::VisualBlock
            )
        {
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
                &self.selection_cursor(),
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

    pub fn selection_cursor(&self) -> Cursor {
        let mut cursor = self.buffer.cursor;
        if matches!(self.mode, AppMode::VisualBlock) {
            cursor.col = self.buffer.preferred_col();
        }
        cursor
    }

    pub fn jump_prev_command_block(&mut self) {
        let Some(current) = self.current_command_block_index() else {
            self.set_status("No command blocks found", true);
            return;
        };
        let target = current.saturating_sub(1);
        if target == current {
            self.set_status("Already at the first command block", false);
            return;
        }
        self.buffer
            .set_cursor(self.command_blocks[target].line_start, 0);
        self.set_status("Jumped to previous command block", false);
    }

    pub fn jump_next_command_block(&mut self) {
        let Some(current) = self.current_command_block_index() else {
            self.set_status("No command blocks found", true);
            return;
        };
        let target = (current + 1).min(self.command_blocks.len().saturating_sub(1));
        if target == current {
            self.set_status("Already at the last command block", false);
            return;
        }
        self.buffer
            .set_cursor(self.command_blocks[target].line_start, 0);
        self.set_status("Jumped to next command block", false);
    }

    pub fn copy_current_command_target(&mut self, target: CopyTarget) -> Result<()> {
        let Some(block) = self.current_command_block().cloned() else {
            self.set_status("No command block at cursor", true);
            return Ok(());
        };

        let Some(text) = block.text_for(target) else {
            self.set_status("Current command block has no matching content", true);
            return Ok(());
        };

        export::clipboard::copy_to_clipboard(&text)?;
        let label = match target {
            CopyTarget::Block => "command block",
            CopyTarget::Input => "command input",
            CopyTarget::Output => "command output",
            CopyTarget::Command => "bare command",
        };
        self.set_status(format!("Copied current {label}"), false);
        Ok(())
    }

    pub fn select_current_command_target(&mut self, target: SelectTarget) {
        let Some(block) = self.current_command_block().cloned() else {
            self.set_status("No command block at cursor", true);
            return;
        };

        let Some((start, end)) = block.line_range_for(target) else {
            self.set_status("Current command block has no matching section", true);
            return;
        };

        self.buffer.set_cursor(end, 0);
        self.enter_visual_from(SelectionMode::VisualLine, Cursor::new(start, 0));

        let label = match target {
            SelectTarget::Block => "command block",
            SelectTarget::Input => "command input",
            SelectTarget::Output => "command output",
        };
        self.set_status(format!("Selected current {label}"), false);
    }

    fn current_command_block(&self) -> Option<&CommandBlockSpan> {
        let idx = self.current_command_block_index()?;
        self.command_blocks.get(idx)
    }

    fn current_command_block_index(&self) -> Option<usize> {
        if self.command_blocks.is_empty() {
            return None;
        }

        let row = self.buffer.cursor.row;
        if let Some((idx, _)) = self
            .command_blocks
            .iter()
            .enumerate()
            .find(|(_, block)| row >= block.line_start && row <= block.line_end)
        {
            return Some(idx);
        }

        self.command_blocks
            .iter()
            .enumerate()
            .rfind(|(_, block)| block.line_start <= row)
            .map(|(idx, _)| idx)
            .or(Some(0))
    }

    fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status = Some(StatusMessage {
            text: text.into(),
            is_error,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_blocks::{CommandBlockSpan, ParsedCommandBlock};
    use sivtr_core::buffer::line::Line;

    fn make_line(content: &str) -> Line {
        Line {
            content: content.to_string(),
            display_widths: sivtr_core::parse::unicode::compute_display_widths(content),
            styles: Vec::new(),
        }
    }

    fn make_app() -> App {
        let buffer = Buffer::new(vec![
            make_line("PS C:\\repo> git status"),
            make_line("clean"),
            make_line("PS C:\\repo> cargo test"),
            make_line("ok"),
        ]);
        let mut app = App::new(buffer);
        app.command_blocks = vec![
            CommandBlockSpan {
                line_start: 0,
                line_end: 1,
                input_line_range: Some((0, 0)),
                output_line_range: Some((1, 1)),
                parsed: ParsedCommandBlock {
                    input_with_prompt: "PS C:\\repo> git status".to_string(),
                    input_without_prompt: "git status".to_string(),
                    output: "clean".to_string(),
                    command: "git status".to_string(),
                },
            },
            CommandBlockSpan {
                line_start: 2,
                line_end: 3,
                input_line_range: Some((2, 2)),
                output_line_range: Some((3, 3)),
                parsed: ParsedCommandBlock {
                    input_with_prompt: "PS C:\\repo> cargo test".to_string(),
                    input_without_prompt: "cargo test".to_string(),
                    output: "ok".to_string(),
                    command: "cargo test".to_string(),
                },
            },
        ];
        app
    }

    #[test]
    fn jumps_between_command_blocks() {
        let mut app = make_app();
        app.buffer.set_cursor(0, 0);
        app.jump_next_command_block();
        assert_eq!(app.buffer.cursor.row, 2);

        app.jump_prev_command_block();
        assert_eq!(app.buffer.cursor.row, 0);
    }

    #[test]
    fn selects_current_command_input_range() {
        let mut app = make_app();
        app.buffer.set_cursor(3, 0);
        app.select_current_command_target(SelectTarget::Input);

        assert_eq!(app.mode, AppMode::VisualLine);
        let selection = app.buffer.selection.expect("selection should exist");
        assert_eq!(selection.anchor.row, 2);
        assert_eq!(app.buffer.cursor.row, 2);
    }

    #[test]
    fn selects_current_command_block_range() {
        let mut app = make_app();
        app.buffer.set_cursor(3, 0);
        app.select_current_command_target(SelectTarget::Block);

        assert_eq!(app.mode, AppMode::VisualLine);
        let selection = app.buffer.selection.expect("selection should exist");
        assert_eq!(selection.anchor.row, 2);
        assert_eq!(app.buffer.cursor.row, 3);
    }

    #[test]
    fn enters_and_exits_insert_mode() {
        let mut app = make_app();
        assert_eq!(app.mode, AppMode::Normal);

        app.enter_insert();
        assert_eq!(app.mode, AppMode::Insert);

        app.exit_insert();
        assert_eq!(app.mode, AppMode::Normal);
    }
}
