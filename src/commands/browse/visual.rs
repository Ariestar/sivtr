//! Visual content selection and mouse scroll helpers.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::widgets::ListState;

use crate::commands::select::CommandSelection;
use crate::tui::content::io::ContentIoFrame;
use crate::tui::content::view::{
    clamp_content_position, content_block_at, content_position_in_text_row, content_text_area,
    selected_content_text, ContentPosition, ContentSelection, ContentSelectionKind,
    ContentViewMode,
};
use crate::tui::workspace::{
    ContentIoFocus, ContentScrolls, WorkspaceDialogue, WorkspaceFocus, WorkspacePickedContent,
    WorkspaceSession, WorkspaceSource,
};

use super::content::workspace_picked_content;
use super::nav::{move_workspace_cursor_down, move_workspace_cursor_up};

/// Lines per wheel notch: web-like scroll steps (lists move selection by the
/// same amount, content by the same line count).
pub(super) const MOUSE_SCROLL_LINES: usize = 3;
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

/// Mouse-down anchor recorded before any dragging. A pure click (down and
/// up with no drag event) never becomes a text selection: the anchor is
/// only promoted to `VisualSelectMode` by the first actual drag.
pub(super) struct MouseSelectionStart {
    pub(super) anchor: ContentPosition,
    pub(super) kind: ContentSelectionKind,
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
        KeyCode::Esc => return Ok(None),
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
    ensure_visual_cursor_visible(mode, content_area, content_scroll);
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
    ensure_visual_cursor_visible(mode, content_area, content_scroll);
}

pub(super) fn ensure_visual_cursor_visible(
    mode: &VisualSelectMode,
    content_area: ratatui::layout::Rect,
    content_scroll: &mut usize,
) {
    let text_area = content_text_area(content_area);
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
/// A click (down then up with no drag) only moves focus and highlights the
/// block under the cursor; the selection starts on the first real drag.
/// Returns `true` when the event was consumed by selection handling.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_content_mouse_select(
    visual_select_mode: &mut Option<VisualSelectMode>,
    mouse_down: &mut Option<MouseSelectionStart>,
    kind: MouseEventKind,
    modifiers: KeyModifiers,
    column: u16,
    row: u16,
    content: VisualContentContext<'_>,
    // When true, left-down on content may arm a selection even if mode is None.
    allow_start: bool,
) -> bool {
    match kind {
        MouseEventKind::Down(MouseButton::Left) if allow_start || visual_select_mode.is_some() => {
            let Some(position) = content_position_in_text_row(
                content.area,
                content.text,
                content.scroll,
                content.mode,
                column,
                row,
            ) else {
                // Outside content: drop an armed/pending selection so list
                // panes can take the click. Keep consuming only while a drag
                // is in progress.
                if visual_select_mode.as_ref().is_some_and(|m| m.dragging) {
                    return true;
                }
                *visual_select_mode = None;
                *mouse_down = None;
                return false;
            };
            match visual_select_mode.as_mut() {
                // An active selection (keyboard `v` mode) re-anchors on click.
                Some(mode) => {
                    mode.selection.anchor = position;
                    mode.selection.cursor = position;
                    mode.selection.kind = mouse_selection_kind(modifiers);
                    mode.dragging = true;
                }
                // Otherwise arm the drag: a pure click below never selects.
                None => {
                    *mouse_down = Some(MouseSelectionStart {
                        anchor: position,
                        kind: mouse_selection_kind(modifiers),
                    });
                }
            }
            true
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // First real drag promotes the armed click into a selection.
            if let Some(start) = mouse_down.take() {
                let mut selection = ContentSelection {
                    anchor: start.anchor,
                    cursor: start.anchor,
                    kind: start.kind,
                };
                if let Some(position) = content_position_in_text_row(
                    content.area,
                    content.text,
                    content.scroll,
                    content.mode,
                    column,
                    row,
                ) {
                    selection.cursor = position;
                }
                *visual_select_mode = Some(VisualSelectMode {
                    selection,
                    dragging: true,
                });
                return true;
            }
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
            if let Some(mode) = visual_select_mode.as_mut() {
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
                // A drag that lands back on its anchor clears the selection.
                if mode.selection.anchor == mode.selection.cursor {
                    *visual_select_mode = None;
                }
                return true;
            }
            // Pure click: never selected, just let the caller handle the
            // block highlight / fold toggle.
            if mouse_down.take().is_some() {
                return true;
            }
            false
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
    let base = workspace_picked_content(dialogues, selected_dialogues, dialogue_idx, None);
    let source = base.source;
    let plain = selected_content_text(content_area, text, content_mode, selection);
    WorkspacePickedContent {
        source,
        units: vec![crate::tui::workspace::TextPair {
            ansi: plain.clone(),
            plain,
        }],
        selection: CommandSelection::RecentExplicit(vec![1]),
        // A visual text range can cut through a part and therefore has no
        // exact atomic anchor. Clipboard callers still receive the text;
        // publication selection rejects this path instead of widening it to
        // the whole dialogue.
        anchors: Vec::new(),
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
    content_scrolls: &mut ContentScrolls,
    content_io_focus: ContentIoFocus,
    content_cursor: &mut super::nav::ContentBlockCursor,
    content_frame: &ContentIoFrame,
) {
    if focus == WorkspaceFocus::Content {
        // The wheel keeps smooth line scrolling; j/k navigates blocks. The
        // block cursor snaps to the first visible block so the highlight
        // always shows where the viewport is.
        let scroll = content_scrolls.get(content_io_focus);
        let next = if scroll_up {
            scroll.saturating_sub(MOUSE_SCROLL_LINES)
        } else {
            scroll.saturating_add(MOUSE_SCROLL_LINES)
        };
        let layout = content_frame.layout(content_io_focus);
        let next = next.min(layout.lines.len().saturating_sub(1));
        content_scrolls.set(content_io_focus, next);
        if let Some(block) = content_block_at(layout, next) {
            content_cursor.set(content_io_focus, block);
        }
        return;
    }
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
                content_scrolls,
                content_io_focus,
                content_cursor,
                content_frame.texts.block_slices(),
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
                content_scrolls,
                content_io_focus,
                content_cursor,
                content_frame.texts.block_slices(),
            );
        }
    }
}
