//! Cursor movement, list clamps, pane resets, and link open.

use anyhow::Result;
use ratatui::widgets::ListState;
use std::process::Command;

use crate::tui::workspace::{
    selected_index, ContentIoFocus, ContentScrolls, WorkspaceFocus, WorkspaceSession,
    WorkspaceSource,
};

use super::selection::has_selected_sessions;

pub(super) fn open_link_target(target: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(target).spawn()?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target).spawn()?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(target).spawn()?;
    }

    Ok(())
}

/// Keyboard/mouse cursor over content blocks, one position per half (each
/// half keeps its own, like the session/dialogue lists). `follow` asks the
/// picker to keep the cursor block visible on the next redraw; keyboard
/// moves set it, clicks do not (a clicked line is already visible).
#[derive(Default)]
pub(super) struct ContentBlockCursor {
    pub(super) input: Option<usize>,
    pub(super) output: Option<usize>,
    pub(super) follow: bool,
}

impl ContentBlockCursor {
    pub(super) fn get(&self, half: ContentIoFocus) -> Option<usize> {
        match half {
            ContentIoFocus::Input => self.input,
            ContentIoFocus::Output => self.output,
        }
    }

    pub(super) fn set(&mut self, half: ContentIoFocus, block: usize) {
        match half {
            ContentIoFocus::Input => self.input = Some(block),
            ContentIoFocus::Output => self.output = Some(block),
        }
    }

    pub(super) fn clear(&mut self) {
        self.input = None;
        self.output = None;
        self.follow = false;
    }

    /// `(half, block)` of the focused half, for the view highlight.
    pub(super) fn focused(&self, focus: ContentIoFocus) -> Option<(ContentIoFocus, usize)> {
        self.get(focus).map(|block| (focus, block))
    }
}

pub(super) fn reset_workspace_after_source_change(
    session_state: &mut ListState,
    selected_sessions: &mut Vec<bool>,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scrolls: &mut ContentScrolls,
) {
    session_state.select(None);
    selected_sessions.clear();
    dialogue_state.select(None);
    selected_dialogues.clear();
    *range_anchor = None;
    content_scrolls.clear();
}

pub(super) fn resize_workspace_dialogue_selection(
    dialogue_count: usize,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
) {
    selected_dialogues.clear();
    selected_dialogues.resize(dialogue_count, false);
    *range_anchor = None;
}

pub(super) fn clamp_list_state(state: &mut ListState, len: usize) {
    let selected = if len == 0 {
        None
    } else {
        Some(selected_index(state).min(len.saturating_sub(1)))
    };
    state.select(selected);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn move_workspace_cursor_up(
    focus: WorkspaceFocus,
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    dialogue_count: usize,
    selected_sessions: &[bool],
    source_state: &mut ListState,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scrolls: &mut ContentScrolls,
    content_io_focus: ContentIoFocus,
    content_cursor: &mut ContentBlockCursor,
    content_block_counts: (usize, usize),
) {
    match focus {
        WorkspaceFocus::Source => {
            let next = selected_index(source_state).saturating_sub(1);
            source_state.select((!sources.is_empty()).then_some(next));
        }
        WorkspaceFocus::Sessions => {
            let next = selected_index(session_state).saturating_sub(1);
            if next != selected_index(session_state) {
                session_state.select((!sessions.is_empty()).then_some(next));
                if !has_selected_sessions(selected_sessions) {
                    reset_workspace_dialogue_state(
                        0,
                        dialogue_state,
                        selected_dialogues,
                        range_anchor,
                    );
                }
                content_scrolls.clear();
            }
        }
        WorkspaceFocus::Dialogues => {
            let next = selected_index(dialogue_state).saturating_sub(1);
            dialogue_state.select((dialogue_count > 0).then_some(next));
            content_scrolls.clear();
        }
        WorkspaceFocus::Content => {
            move_content_cursor(true, content_cursor, content_block_counts, content_io_focus);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn move_workspace_cursor_down(
    focus: WorkspaceFocus,
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    dialogue_count: usize,
    selected_sessions: &[bool],
    source_state: &mut ListState,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scrolls: &mut ContentScrolls,
    content_io_focus: ContentIoFocus,
    content_cursor: &mut ContentBlockCursor,
    content_block_counts: (usize, usize),
) {
    match focus {
        WorkspaceFocus::Source => {
            let current = selected_index(source_state);
            let next = (current + 1).min(sources.len().saturating_sub(1));
            source_state.select((!sources.is_empty()).then_some(next));
        }
        WorkspaceFocus::Sessions => {
            let current = selected_index(session_state);
            let next = (current + 1).min(sessions.len().saturating_sub(1));
            if next != current {
                session_state.select((!sessions.is_empty()).then_some(next));
                if !has_selected_sessions(selected_sessions) {
                    reset_workspace_dialogue_state(
                        0,
                        dialogue_state,
                        selected_dialogues,
                        range_anchor,
                    );
                }
                content_scrolls.clear();
            }
        }
        WorkspaceFocus::Dialogues => {
            let current = selected_index(dialogue_state);
            let next = (current + 1).min(dialogue_count.saturating_sub(1));
            dialogue_state.select((dialogue_count > 0).then_some(next));
            content_scrolls.clear();
        }
        WorkspaceFocus::Content => {
            move_content_cursor(
                false,
                content_cursor,
                content_block_counts,
                content_io_focus,
            );
        }
    }
}

/// Move the content block cursor within the focused half, clamped like a
/// list selection; the next redraw keeps the cursor block visible.
fn move_content_cursor(
    up: bool,
    cursor: &mut ContentBlockCursor,
    counts: (usize, usize),
    focus: ContentIoFocus,
) {
    let count = match focus {
        ContentIoFocus::Input => counts.0,
        ContentIoFocus::Output => counts.1,
    };
    if count == 0 {
        return;
    }
    let next = match cursor.get(focus) {
        Some(idx) if up => idx.saturating_sub(1),
        Some(idx) => (idx + 1).min(count - 1),
        None => 0,
    };
    cursor.set(focus, next);
    cursor.follow = true;
}

pub(super) fn row_list_index(
    area: ratatui::layout::Rect,
    row: u16,
    len: usize,
    offset: usize,
) -> Option<usize> {
    let row = row.checked_sub(area.y.saturating_add(1))? as usize;
    let index = row.saturating_add(offset);
    (index < len).then_some(index)
}

pub(super) fn source_list_index(
    area: ratatui::layout::Rect,
    column: u16,
    row: u16,
    sources: &[WorkspaceSource],
    vertical: bool,
    offset: usize,
) -> Option<usize> {
    if vertical {
        // List panel: one source per row (same as sessions/dialogues).
        return row_list_index(area, row, sources.len(), offset);
    }
    // Compact strip: single content row, labels laid out left→right.
    if row != area.y.saturating_add(1)
        || column <= area.x
        || column >= area.x.saturating_add(area.width)
    {
        return None;
    }
    let mut cursor = area.x.saturating_add(1);
    for (idx, source) in sources.iter().enumerate() {
        if idx > 0 {
            cursor = cursor.saturating_add(2);
        }
        let width = source.label().len() as u16 + 4;
        if column >= cursor && column < cursor.saturating_add(width) {
            return Some(idx);
        }
        cursor = cursor.saturating_add(width);
    }
    None
}

pub(super) fn reset_workspace_dialogue_state(
    dialogue_count: usize,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
) {
    dialogue_state.select((dialogue_count > 0).then_some(0));
    selected_dialogues.clear();
    selected_dialogues.resize(dialogue_count, false);
    *range_anchor = None;
}

#[cfg(test)]
mod tests {
    use super::row_list_index;
    use ratatui::layout::Rect;

    #[test]
    fn row_list_index_includes_scroll_offset() {
        let area = Rect::new(0, 0, 30, 10);
        // Row 1 is the first content row below the panel title.
        assert_eq!(row_list_index(area, 1, 100, 0), Some(0));
        // A scrolled list maps the clicked row onto the offset index.
        assert_eq!(row_list_index(area, 1, 100, 50), Some(50));
        assert_eq!(row_list_index(area, 4, 100, 50), Some(53));
        // Indices beyond the list end are rejected.
        assert_eq!(row_list_index(area, 4, 100, 98), None);
        // The title row is not a selectable row.
        assert_eq!(row_list_index(area, 0, 100, 0), None);
    }
}
