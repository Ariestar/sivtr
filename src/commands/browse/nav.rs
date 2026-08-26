//! Cursor movement, list clamps, pane resets, and link open.

use anyhow::Result;
use ratatui::widgets::ListState;
use std::process::Command;

use crate::tui::content::block::BlockText;
use crate::tui::workspace::{
    selected_index, selected_indices, ContentIoFocus, ContentScrolls, WorkspaceFocus,
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

/// Keyboard/mouse cursor over content blocks, one position per dialogue
/// across both IO halves (block ids are dialogue-global). `follow` asks the
/// picker to keep the cursor block visible on the next redraw; keyboard
/// moves set it, clicks do not (a clicked line is already visible).
#[derive(Default)]
pub(super) struct ContentBlockCursor {
    pub(super) block: Option<usize>,
    pub(super) follow: bool,
}

impl ContentBlockCursor {
    pub(super) fn get(&self) -> Option<usize> {
        self.block
    }

    pub(super) fn set(&mut self, block: usize) {
        self.block = Some(block);
    }

    pub(super) fn clear(&mut self) {
        self.block = None;
        self.follow = false;
    }

    /// `(half, block)` of the cursor, for the view highlight and block
    /// operations. The half is the one whose displayed segments own the
    /// cursor id; a block hidden by a fold has no half and no highlight.
    pub(super) fn focused(
        &self,
        blocks: (&[BlockText], &[BlockText]),
    ) -> Option<(ContentIoFocus, usize)> {
        let block = self.block?;
        let half = if blocks.0.iter().any(|segment| segment.id == block) {
            ContentIoFocus::Input
        } else if blocks.1.iter().any(|segment| segment.id == block) {
            ContentIoFocus::Output
        } else {
            return None;
        };
        Some((half, block))
    }
}

/// The live `v` range anchor: which pane opened the range, and the row it
/// started at (a list row index, or a content block id). Only one range can
/// be open at a time — moving focus discards it — so one anchor serves every
/// pane, and tagging it with its pane is what keeps a stale anchor from
/// completing a span somewhere else.
#[derive(Clone, Copy, Default)]
pub(super) struct RangeAnchor(Option<(WorkspaceFocus, usize)>);

impl RangeAnchor {
    /// Row the open range started at, when it belongs to `pane`.
    pub(super) fn get(&self, pane: WorkspaceFocus) -> Option<usize> {
        self.0.and_then(|(open, row)| (open == pane).then_some(row))
    }

    pub(super) fn clear(&mut self) {
        self.0 = None;
    }

    /// Drop the anchor only if `pane` owns it (its rows just changed meaning).
    pub(super) fn clear_pane(&mut self, pane: WorkspaceFocus) {
        if self.get(pane).is_some() {
            self.clear();
        }
    }

    /// One `v` press on `row` of `pane`: opens the range and yields nothing,
    /// or closes the one already open there and yields the span it covers.
    pub(super) fn span(
        &mut self,
        pane: WorkspaceFocus,
        row: usize,
    ) -> Option<std::ops::RangeInclusive<usize>> {
        match self.0.take() {
            Some((open, anchor)) if open == pane => Some(anchor.min(row)..=anchor.max(row)),
            _ => {
                self.0 = Some((pane, row));
                None
            }
        }
    }
}

/// Index of the dialogue the content pane shows: the `page`-th selected
/// dialogue when several are selected, otherwise the focused row. `page`
/// is clamped to the current selection count.
pub(super) fn shown_dialogue_idx(
    selected_dialogues: &[bool],
    page: usize,
    dialogue_idx: usize,
) -> usize {
    let selected = selected_indices(selected_dialogues);
    selected
        .get(page.min(selected.len().saturating_sub(1)))
        .copied()
        .unwrap_or(dialogue_idx)
}

/// Discard whatever the panes right of `focus` derived from its selection,
/// after that selection changed. `true` when the change reaches back to the
/// sources, so the caller must reload sessions.
pub(super) fn invalidate_panes_below(
    focus: WorkspaceFocus,
    session_state: &mut ListState,
    selected_sessions: &mut Vec<bool>,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut RangeAnchor,
    content_scrolls: &mut ContentScrolls,
) -> bool {
    match focus {
        WorkspaceFocus::Source => {
            session_state.select(None);
            selected_sessions.clear();
            dialogue_state.select(None);
            selected_dialogues.clear();
            range_anchor.clear();
            content_scrolls.clear();
            true
        }
        WorkspaceFocus::Sessions => {
            reset_workspace_dialogue_state(0, dialogue_state, selected_dialogues);
            content_scrolls.clear();
            false
        }
        // Dialogues feed the content pane, which rebuilds from the shown
        // dialogue every redraw; Content has nothing to its right.
        WorkspaceFocus::Dialogues | WorkspaceFocus::Content => false,
    }
}

pub(super) fn resize_workspace_dialogue_selection(
    dialogue_count: usize,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut RangeAnchor,
) {
    selected_dialogues.clear();
    selected_dialogues.resize(dialogue_count, false);
    range_anchor.clear();
}

pub(super) fn clamp_list_state(state: &mut ListState, len: usize) {
    let selected = if len == 0 {
        None
    } else {
        Some(selected_index(state).min(len.saturating_sub(1)))
    };
    state.select(selected);
}

/// Move the focused pane's cursor one row (`up` or down). Every list pane
/// follows one rule: clamp to the row count, and a move that does not change
/// the row does nothing — so bumping the first or last row never resets the
/// panes below it. Content moves its block cursor instead.
#[allow(clippy::too_many_arguments)]
pub(super) fn move_workspace_cursor(
    up: bool,
    focus: WorkspaceFocus,
    source_count: usize,
    session_count: usize,
    dialogue_count: usize,
    selected_sessions: &[bool],
    source_state: &mut ListState,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    content_scrolls: &mut ContentScrolls,
    content_cursor: &mut ContentBlockCursor,
    content_blocks: (&[BlockText], &[BlockText]),
) {
    let (state, len) = match focus {
        WorkspaceFocus::Source => (&mut *source_state, source_count),
        WorkspaceFocus::Sessions => (&mut *session_state, session_count),
        WorkspaceFocus::Dialogues => (&mut *dialogue_state, dialogue_count),
        WorkspaceFocus::Content => {
            move_content_cursor(up, content_cursor, content_blocks);
            return;
        }
    };
    if len == 0 {
        state.select(None);
        return;
    }
    let current = selected_index(state);
    let next = if up {
        current.saturating_sub(1)
    } else {
        (current + 1).min(len - 1)
    };
    if next == current {
        return;
    }
    state.select(Some(next));
    // A row change invalidates what the panes to the right derived from it.
    match focus {
        WorkspaceFocus::Sessions => {
            if !has_selected_sessions(selected_sessions) {
                reset_workspace_dialogue_state(0, dialogue_state, selected_dialogues);
            }
            content_scrolls.clear();
        }
        WorkspaceFocus::Dialogues => content_scrolls.clear(),
        _ => {}
    }
}

/// Move the content cursor across the whole dialogue: input blocks then
/// output blocks form one continuous sequence, so j/k flows over the half
/// boundary. The walk follows the *visible* block sequence (the rendered
/// segments), so folds — which change which blocks are shown — resolve the
/// cursor id against the current segments and clamp to the nearest visible
/// block.
fn move_content_cursor(
    up: bool,
    cursor: &mut ContentBlockCursor,
    blocks: (&[BlockText], &[BlockText]),
) {
    let (input_blocks, output_blocks) = blocks;
    let input_len = input_blocks.len();
    let total = input_len + output_blocks.len();
    if total == 0 {
        return;
    }
    let ids: Vec<_> = input_blocks
        .iter()
        .chain(output_blocks)
        .map(|block| block.id)
        .collect();
    let current_id = cursor.get();
    let position = current_id.and_then(|id| ids.iter().position(|visible| *visible == id));
    let next = match (current_id, position) {
        (None, _) => 0,
        (Some(_), Some(pos)) if up => pos.saturating_sub(1),
        (Some(_), Some(pos)) => (pos + 1).min(total - 1),
        (Some(id), None) if up => ids.iter().rposition(|visible| *visible < id).unwrap_or(0),
        (Some(id), None) => ids
            .iter()
            .position(|visible| *visible > id)
            .unwrap_or(total - 1),
    };
    cursor.set(ids[next]);
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

/// Is `column` inside a list row's selection-dot gutter (rows render
/// `{dot} ` at `area.x`, two columns wide)?
pub(super) fn dot_gutter_hit(area: ratatui::layout::Rect, column: u16) -> bool {
    column <= area.x.saturating_add(1)
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
) {
    dialogue_state.select((dialogue_count > 0).then_some(0));
    selected_dialogues.clear();
    selected_dialogues.resize(dialogue_count, false);
}

#[cfg(test)]
mod tests {
    use super::{move_content_cursor, row_list_index, shown_dialogue_idx, ContentBlockCursor};
    use crate::tui::content::block::BlockText;
    use ratatui::layout::Rect;
    use sivtr_core::record::WorkPartKind;

    fn block(id: usize) -> BlockText {
        BlockText {
            id,
            text: String::new(),
            tight: false,
            kind: WorkPartKind::Output,
        }
    }

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

    #[test]
    fn shown_dialogue_idx_falls_back_to_focused_row_without_selection() {
        assert_eq!(shown_dialogue_idx(&[false, false], 0, 1), 1);
    }

    #[test]
    fn shown_dialogue_idx_pages_through_the_selected_dialogues() {
        let selected = [false, true, false, true, true];
        // Page 0..3 maps onto the 2nd, 4th, and 5th dialogues.
        assert_eq!(shown_dialogue_idx(&selected, 0, 0), 1);
        assert_eq!(shown_dialogue_idx(&selected, 1, 0), 3);
        assert_eq!(shown_dialogue_idx(&selected, 2, 0), 4);
        // A page past the end clamps to the last selected dialogue.
        assert_eq!(shown_dialogue_idx(&selected, 9, 0), 4);
    }

    #[test]
    fn hidden_cursor_moves_to_the_nearest_visible_block() {
        let blocks = [block(0), block(1), block(4)];
        let mut cursor = ContentBlockCursor::default();
        cursor.set(3);

        move_content_cursor(true, &mut cursor, (&[], &blocks));
        assert_eq!(cursor.get(), Some(1));

        cursor.set(3);
        move_content_cursor(false, &mut cursor, (&[], &blocks));
        assert_eq!(cursor.get(), Some(4));
    }
}
