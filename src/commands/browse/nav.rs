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

pub(super) fn reset_workspace_search_state(
    session_state: &mut ListState,
    selected_sessions: &mut Vec<bool>,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    content_scrolls: &mut ContentScrolls,
) {
    reset_workspace_after_source_change(
        session_state,
        selected_sessions,
        dialogue_state,
        selected_dialogues,
        range_anchor,
        content_scrolls,
    );
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
            let next = content_scrolls.get(content_io_focus).saturating_sub(1);
            content_scrolls.set(content_io_focus, next);
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
            let next = content_scrolls.get(content_io_focus).saturating_add(1);
            content_scrolls.set(content_io_focus, next);
        }
    }
}

pub(super) fn row_list_index(area: ratatui::layout::Rect, row: u16, len: usize) -> Option<usize> {
    let row = row.checked_sub(area.y.saturating_add(1))? as usize;
    (row < len).then_some(row)
}

pub(super) fn source_list_index(
    area: ratatui::layout::Rect,
    column: u16,
    row: u16,
    sources: &[WorkspaceSource],
    vertical: bool,
) -> Option<usize> {
    if vertical {
        // List panel: one source per row (same as sessions/dialogues).
        return row_list_index(area, row, sources.len());
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
