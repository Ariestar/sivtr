//! Cursor movement, list clamps, pane resets, and link open.

use anyhow::Result;
use std::process::Command;

use crate::tui::content::block::BlockText;
use crate::tui::workspace::{
    ContentIoFocus, ContentScrolls, ListPane, Rows, WorkspaceFocus, WorkspaceSource,
};

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

/// Index of the dialogue the content pane shows: the `page`-th marked
/// dialogue when several are marked, otherwise the cursor row — the pane's
/// own "which rows does this act on" answer, paged. `page` is clamped to
/// that row count.
pub(super) fn shown_dialogue_idx(dialogues: &ListPane, page: usize) -> usize {
    let rows = dialogues.active();
    rows.get(page.min(rows.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0)
}

/// Discard whatever the panes right of `focus` derived from its selection,
/// after that selection changed. `true` when the change reaches back to the
/// sources, so the caller must reload sessions.
pub(super) fn invalidate_panes_below(
    focus: WorkspaceFocus,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
) -> bool {
    match focus {
        WorkspaceFocus::Source => {
            rows.sessions.reset(0);
            rows.dialogues.reset(0);
            rows.close_ranges();
            content_scrolls.clear();
            true
        }
        WorkspaceFocus::Sessions => {
            rows.dialogues.reset(0);
            content_scrolls.clear();
            false
        }
        // Dialogues feed the content pane, which rebuilds from the shown
        // dialogue every redraw; Content has nothing to its right.
        WorkspaceFocus::Dialogues | WorkspaceFocus::Content => false,
    }
}

/// Discard whatever the panes right of `focus` derived from the list row the
/// cursor just left. Both ways a cursor lands on a new row share this rule: a
/// j/k step (and the wheel, which steps) and a click's absolute jump.
///
/// Distinct from [`invalidate_panes_below`], which answers the same question
/// after a *selection* change: a selected pane's list spans every marked row,
/// so moving the cursor inside it changes nothing below.
pub(super) fn invalidate_after_cursor_move(
    focus: WorkspaceFocus,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
) {
    match focus {
        WorkspaceFocus::Sessions => {
            // The dialogue list spans every marked session, so it only
            // follows the cursor row while nothing is marked.
            if !rows.sessions.has_marks() {
                rows.dialogues.reset(0);
            }
            content_scrolls.clear();
        }
        // A different dialogue is on screen: its content starts at the top.
        WorkspaceFocus::Dialogues => content_scrolls.clear(),
        // Source feeds sessions through its selection, not its cursor;
        // Content has no list row.
        WorkspaceFocus::Source | WorkspaceFocus::Content => {}
    }
}

/// Move the focused pane's cursor one row (`up` or down). Every list pane
/// follows one rule: clamp to the row count, and a move that does not change
/// the row does nothing — so bumping the first or last row never resets the
/// panes below it. Content moves its block cursor instead.
pub(super) fn move_workspace_cursor(
    up: bool,
    focus: WorkspaceFocus,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
    content_cursor: &mut ContentBlockCursor,
    content_blocks: (&[BlockText], &[BlockText]),
) {
    if focus == WorkspaceFocus::Content {
        move_content_cursor(up, content_cursor, content_blocks);
        return;
    }
    if rows.pane_mut(focus).is_some_and(|pane| pane.step(up)) {
        invalidate_after_cursor_move(focus, rows, content_scrolls);
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

#[cfg(test)]
mod tests {
    use super::{move_content_cursor, row_list_index, shown_dialogue_idx, ContentBlockCursor};
    use crate::tui::content::block::BlockText;
    use crate::tui::workspace::ListPane;
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
    fn shown_dialogue_idx_falls_back_to_the_cursor_row_without_marks() {
        let mut pane = ListPane::with_marks(vec![false, false]);
        pane.select(1);
        assert_eq!(shown_dialogue_idx(&pane, 0), 1);
    }

    #[test]
    fn shown_dialogue_idx_pages_through_the_marked_dialogues() {
        let pane = ListPane::with_marks(vec![false, true, false, true, true]);
        // Page 0..3 maps onto the 2nd, 4th, and 5th dialogues.
        assert_eq!(shown_dialogue_idx(&pane, 0), 1);
        assert_eq!(shown_dialogue_idx(&pane, 1), 3);
        assert_eq!(shown_dialogue_idx(&pane, 2), 4);
        // A page past the end clamps to the last marked dialogue.
        assert_eq!(shown_dialogue_idx(&pane, 9), 4);
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
