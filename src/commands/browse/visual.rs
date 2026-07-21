//! Visual content selection and mouse scroll helpers.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::widgets::ListState;

use crate::commands::select::CommandSelection;
use crate::tui::content::view::{
    clamp_content_position, content_position_in_text_row, content_text_area, selected_content_text,
    ContentPosition, ContentSelection, ContentSelectionKind, ContentViewMode,
};
use crate::tui::workspace::{
    selected_index, ContentIoFocus, ContentScrolls, WorkspaceDialogue, WorkspaceFocus,
    WorkspacePickedContent, WorkspaceSession, WorkspaceSource,
};

use super::content::workspace_picked_content;
use super::nav::{move_workspace_cursor_down, move_workspace_cursor_up};

const MOUSE_SCROLL_LINES: usize = 3;
#[derive(Clone, Copy)]
pub(super) struct VisualSelectMode {
    pub(super) selection: ContentSelection,
    pub(super) dragging: bool,
}

pub(super) struct VisualContentContext<'a> {
    pub(super) area: ratatui::layout::Rect,
    pub(super) text: &'a str,
    pub(super) mode: ContentViewMode,
    pub(super) scroll: usize,
}

pub(super) fn enter_visual_select_mode(
    visual_select_mode: &mut Option<VisualSelectMode>,
    content_scroll: &mut usize,
    content_area: ratatui::layout::Rect,
    text: &str,
    mode: ContentViewMode,
) {
    let position = clamp_content_position(
        content_area,
        text,
        mode,
        ContentPosition {
            line: *content_scroll,
            column: 0,
        },
    );
    *content_scroll = position.line;
    *visual_select_mode = Some(VisualSelectMode {
        selection: ContentSelection {
            anchor: position,
            cursor: position,
            kind: ContentSelectionKind::Linear,
        },
        dragging: false,
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_visual_select_key(
    key: KeyCode,
    modifiers: KeyModifiers,
    mode: &mut VisualSelectMode,
    content_area: ratatui::layout::Rect,
    text: &str,
    content_mode: ContentViewMode,
    content_scroll: &mut usize,
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
) -> Result<Option<WorkspacePickedContent>> {
    match key {
        KeyCode::Esc | KeyCode::Char('v') => return Ok(None),
        KeyCode::Enter | KeyCode::Char('y') => {
            return Ok(Some(workspace_picked_content_for_visual_selection(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                content_area,
                text,
                content_mode,
                mode.selection,
            )));
        }
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(Some(workspace_picked_content_for_visual_selection(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                content_area,
                text,
                content_mode,
                mode.selection,
            )));
        }
        KeyCode::Left | KeyCode::Char('h') => move_visual_cursor(
            mode,
            content_area,
            text,
            content_mode,
            content_scroll,
            -1,
            0,
        ),
        KeyCode::Right | KeyCode::Char('l') => {
            move_visual_cursor(mode, content_area, text, content_mode, content_scroll, 1, 0)
        }
        KeyCode::Up | KeyCode::Char('k') => move_visual_cursor(
            mode,
            content_area,
            text,
            content_mode,
            content_scroll,
            0,
            -1,
        ),
        KeyCode::Down | KeyCode::Char('j') => {
            move_visual_cursor(mode, content_area, text, content_mode, content_scroll, 0, 1)
        }
        KeyCode::Home | KeyCode::Char('0') => {
            mode.selection.cursor.column = 0;
        }
        KeyCode::End | KeyCode::Char('$') => {
            mode.selection.cursor = clamp_content_position(
                content_area,
                text,
                content_mode,
                ContentPosition {
                    line: mode.selection.cursor.line,
                    column: usize::MAX,
                },
            );
        }
        KeyCode::PageDown | KeyCode::Char('d')
            if key == KeyCode::PageDown || modifiers.contains(KeyModifiers::CONTROL) =>
        {
            move_visual_cursor(
                mode,
                content_area,
                text,
                content_mode,
                content_scroll,
                0,
                10,
            )
        }
        KeyCode::PageUp | KeyCode::Char('u')
            if key == KeyCode::PageUp || modifiers.contains(KeyModifiers::CONTROL) =>
        {
            move_visual_cursor(
                mode,
                content_area,
                text,
                content_mode,
                content_scroll,
                0,
                -10,
            )
        }
        _ => {}
    }
    ensure_visual_cursor_visible(mode, content_area, text, content_mode, content_scroll);
    Ok(None)
}

pub(super) fn move_visual_cursor(
    mode: &mut VisualSelectMode,
    content_area: ratatui::layout::Rect,
    text: &str,
    content_mode: ContentViewMode,
    content_scroll: &mut usize,
    column_delta: isize,
    line_delta: isize,
) {
    let cursor = mode.selection.cursor;
    let line = cursor.line.saturating_add_signed(line_delta);
    let column = cursor.column.saturating_add_signed(column_delta);
    mode.selection.cursor = clamp_content_position(
        content_area,
        text,
        content_mode,
        ContentPosition { line, column },
    );
    ensure_visual_cursor_visible(mode, content_area, text, content_mode, content_scroll);
}

pub(super) fn ensure_visual_cursor_visible(
    mode: &VisualSelectMode,
    content_area: ratatui::layout::Rect,
    text: &str,
    content_mode: ContentViewMode,
    content_scroll: &mut usize,
) {
    let text_area = content_text_area(content_area, text, content_mode);
    let height = text_area.height as usize;
    if height == 0 {
        return;
    }
    let cursor_line = mode.selection.cursor.line;
    if cursor_line < *content_scroll {
        *content_scroll = cursor_line;
    } else if cursor_line >= content_scroll.saturating_add(height) {
        *content_scroll = cursor_line.saturating_add(1).saturating_sub(height);
    }
}

/// Start or update content mouse selection.
///
/// Free drag works without first pressing `v`. Ctrl+drag forces block selection.
/// Returns `true` when the event was consumed by selection handling.
pub(super) fn handle_content_mouse_select(
    visual_select_mode: &mut Option<VisualSelectMode>,
    kind: MouseEventKind,
    modifiers: KeyModifiers,
    column: u16,
    row: u16,
    content: VisualContentContext<'_>,
    // When true, left-down on content may start a selection even if mode is None.
    allow_start: bool,
) -> bool {
    let in_content = content_position_in_text_row(
        content.area,
        content.text,
        content.scroll,
        content.mode,
        column,
        row,
    )
    .is_some();

    match kind {
        MouseEventKind::Down(MouseButton::Left) if allow_start || visual_select_mode.is_some() => {
            if !in_content {
                // Outside content: drop free selection so list panes can take the click.
                // Keep consuming only while a drag is in progress.
                if visual_select_mode.as_ref().is_some_and(|m| m.dragging) {
                    return true;
                }
                *visual_select_mode = None;
                return false;
            }
            let Some(position) = content_position_in_text_row(
                content.area,
                content.text,
                content.scroll,
                content.mode,
                column,
                row,
            ) else {
                return false;
            };
            *visual_select_mode = Some(VisualSelectMode {
                selection: ContentSelection {
                    anchor: position,
                    cursor: position,
                    kind: mouse_selection_kind(modifiers),
                },
                dragging: true,
            });
            true
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            let Some(mode) = visual_select_mode.as_mut() else {
                return false;
            };
            if !mode.dragging {
                return true;
            }
            if let Some(position) = content_position_in_text_row(
                content.area,
                content.text,
                content.scroll,
                content.mode,
                column,
                row,
            ) {
                mode.selection.cursor = position;
                if modifiers.contains(KeyModifiers::CONTROL) {
                    mode.selection.kind = ContentSelectionKind::Block;
                }
            }
            true
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let Some(mode) = visual_select_mode.as_mut() else {
                return false;
            };
            if let Some(position) = content_position_in_text_row(
                content.area,
                content.text,
                content.scroll,
                content.mode,
                column,
                row,
            ) {
                mode.selection.cursor = position;
            }
            mode.dragging = false;
            // Pure click (no drag range) clears selection so list clicks stay light.
            if mode.selection.anchor == mode.selection.cursor {
                *visual_select_mode = None;
            }
            true
        }
        _ => false,
    }
}

pub(super) fn mouse_selection_kind(modifiers: KeyModifiers) -> ContentSelectionKind {
    if modifiers.contains(KeyModifiers::CONTROL) {
        ContentSelectionKind::Block
    } else {
        ContentSelectionKind::Linear
    }
}

pub(super) fn workspace_picked_content_for_visual_selection(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    content_area: ratatui::layout::Rect,
    text: &str,
    content_mode: ContentViewMode,
    selection: ContentSelection,
) -> WorkspacePickedContent {
    let source = workspace_picked_content(dialogues, selected_dialogues, dialogue_idx, None).source;
    let plain = selected_content_text(content_area, text, content_mode, selection);
    WorkspacePickedContent {
        source,
        units: vec![crate::tui::workspace::TextPair {
            ansi: plain.clone(),
            plain,
        }],
        selection: CommandSelection::RecentExplicit(vec![1]),
    }
}

pub(super) fn scroll_list_state_up(state: &mut ListState) {
    for _ in 0..MOUSE_SCROLL_LINES {
        state.select(Some(selected_index(state).saturating_sub(1)));
    }
}

pub(super) fn scroll_list_state_down(state: &mut ListState, len: usize) {
    for _ in 0..MOUSE_SCROLL_LINES {
        let next = (selected_index(state) + 1).min(len.saturating_sub(1));
        state.select((len > 0).then_some(next));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_workspace_mouse_scroll(
    focus: WorkspaceFocus,
    scroll_up: bool,
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
    for _ in 0..MOUSE_SCROLL_LINES {
        if scroll_up {
            move_workspace_cursor_up(
                focus,
                sources,
                sessions,
                dialogue_count,
                selected_sessions,
                source_state,
                session_state,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scrolls,
                content_io_focus,
            );
        } else {
            move_workspace_cursor_down(
                focus,
                sources,
                sessions,
                dialogue_count,
                selected_sessions,
                source_state,
                session_state,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scrolls,
                content_io_focus,
            );
        }
    }
}

