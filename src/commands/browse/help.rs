//! Help-panel + table-driven action dispatch.
//!
//! Key bindings live in `workspace_help_entries()`. This module only runs actions.

use anyhow::Result;

use crate::tui::content::block::BlockText;
use crate::tui::content::view::ContentViewMode;
use crate::tui::terminal::suspend;
use crate::tui::workspace::{
    can_open_dialogue_vim, workspace_content_text, ContentIoFocus, ContentScrolls, ExpandedBlocks,
    Rows, WorkspaceDialogue, WorkspaceFocus, WorkspaceHelpAction, WorkspacePickedContent,
    WorkspaceSource,
};
use sivtr_core::record::WorkAt;

use super::content::{
    dialogue_text_vim_view, workspace_picked_content,
    workspace_picked_content_for_copy_with_line_filter, workspace_picked_content_for_cursor_block,
    WorkspaceCopyShortcut,
};
use super::nav::{
    invalidate_panes_below, move_workspace_cursor, shown_dialogue_idx, ContentBlockCursor,
};
use super::panes::ContentPane;
use super::selection::{select_sources, WorkspaceSourceSelection};
use super::vim::open_vim_view;
use super::PICK_CANCELLED_MESSAGE;

/// Result of dispatching a help-table action.
pub(super) enum HelpDispatch {
    Continue,
    Picked(WorkspacePickedContent),
    /// Caller must refresh session/dialogue load (needs SessionColumn).
    Refresh,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_workspace_help_action(
    action: WorkspaceHelpAction,
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    sources: &[WorkspaceSource],
    // Cursor, marks, and the live `v` anchor of all three list panes.
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
    content_io_focus: &mut ContentIoFocus,
    content_mode: &mut ContentViewMode,
    expanded: &mut ExpandedBlocks,
    // Which marked dialogue the content pane shows (multi-select paging).
    content_page: &mut usize,
    content_cursor: &mut ContentBlockCursor,
    content_pane: &mut ContentPane,
    content_blocks: (&[BlockText], &[BlockText]),
    show_help: &mut bool,
    show_search: &mut bool,
    search_query: &mut String,
    search_dirty: &mut bool,
    content_at: Option<WorkAt>,
    line_filter: Option<&str>,
    dialogues: &[WorkspaceDialogue],
    terminal: &mut crate::tui::terminal::Tui,
) -> Result<HelpDispatch> {
    let dialogue_count = rows.dialogues.len();
    match action {
        WorkspaceHelpAction::FocusSource => {
            set_focus(focus, fullscreen, rows, WorkspaceFocus::Source)
        }
        WorkspaceHelpAction::FocusSessions => {
            set_focus(focus, fullscreen, rows, WorkspaceFocus::Sessions)
        }
        WorkspaceHelpAction::FocusDialogues if dialogue_count > 0 => {
            set_focus(focus, fullscreen, rows, WorkspaceFocus::Dialogues)
        }
        WorkspaceHelpAction::FocusContent if dialogue_count > 0 => {
            set_focus(focus, fullscreen, rows, WorkspaceFocus::Content)
        }
        WorkspaceHelpAction::MoveUp | WorkspaceHelpAction::MoveDown => move_workspace_cursor(
            action == WorkspaceHelpAction::MoveUp,
            *focus,
            rows,
            content_scrolls,
            content_cursor,
            content_blocks,
        ),
        WorkspaceHelpAction::PreviousPane => {
            if let Some(next_focus) = focus.previous(dialogue_count) {
                set_focus(focus, fullscreen, rows, next_focus);
            }
        }
        WorkspaceHelpAction::NextPane => {
            if let Some(next_focus) = focus.next(dialogue_count) {
                set_focus(focus, fullscreen, rows, next_focus);
            }
        }
        WorkspaceHelpAction::ToggleSelection => match *focus {
            WorkspaceFocus::Content => {
                // Pane-native selection: Space marks the focused block for
                // batch copy, like Space toggles a list row. Multi-select
                // pages one dialogue at a time, so the shown dialogue owns
                // the mark regardless of the selection count.
                let shown = shown_dialogue_idx(&rows.dialogues, *content_page);
                if let Some(block) = content_cursor.get() {
                    content_pane.toggle_mark(shown, block);
                }
            }
            pane => {
                if toggle_list_row(pane, rows.cursor(pane), rows, content_scrolls) {
                    return Ok(HelpDispatch::Refresh);
                }
            }
        },
        // Multi-select paging: J/K flip the content pane to the next /
        // previous marked dialogue. The redraw resets the fold state and
        // cursor when the shown dialogue changes; marks follow their
        // dialogue and stay, so a later copy can join pages.
        WorkspaceHelpAction::NextDialoguePage if *focus == WorkspaceFocus::Content => {
            let count = rows.dialogues.marked();
            if count > 1 {
                *content_page = (*content_page + 1).min(count.saturating_sub(1));
                content_scrolls.clear();
            }
        }
        WorkspaceHelpAction::PreviousDialoguePage if *focus == WorkspaceFocus::Content => {
            if rows.dialogues.marked() > 1 {
                *content_page = content_page.saturating_sub(1);
                content_scrolls.clear();
            }
        }
        WorkspaceHelpAction::SelectAllSources
        | WorkspaceHelpAction::SelectAgentSources
        | WorkspaceHelpAction::SelectTerminalSource => {
            select_sources(
                sources,
                rows.source.mask_mut(),
                match action {
                    WorkspaceHelpAction::SelectAgentSources => WorkspaceSourceSelection::Agents,
                    WorkspaceHelpAction::SelectTerminalSource => WorkspaceSourceSelection::Terminal,
                    _ => WorkspaceSourceSelection::All,
                },
            );
            invalidate_panes_below(WorkspaceFocus::Source, rows, content_scrolls);
            return Ok(HelpDispatch::Refresh);
        }
        WorkspaceHelpAction::RangeSelect => match *focus {
            WorkspaceFocus::Content => {
                // The span covers block ids instead of list rows, and only
                // visible blocks are in it — a folded run is one unit, so
                // marking its hidden members too would copy them twice.
                if let Some(cursor_block) = content_cursor.get() {
                    if let Some(span) = rows.range(*focus, cursor_block) {
                        let shown = shown_dialogue_idx(&rows.dialogues, *content_page);
                        content_pane.toggle_mark_range(
                            shown,
                            content_blocks
                                .0
                                .iter()
                                .chain(content_blocks.1)
                                .map(|block| block.id)
                                .filter(|id| span.contains(id)),
                        );
                    }
                }
            }
            // Every list pane shares one range-selection semantic: `v`
            // anchors, moves extend, `v` again selects the span. Only the
            // completing `v` (which changes selection) rebuilds panes below.
            pane => {
                if rows.range_select(pane) && invalidate_panes_below(pane, rows, content_scrolls) {
                    return Ok(HelpDispatch::Refresh);
                }
            }
        },
        WorkspaceHelpAction::ToggleAllDialogues if *focus == WorkspaceFocus::Dialogues => {
            rows.dialogues.toggle_all();
            rows.close_ranges();
        }
        WorkspaceHelpAction::OpenVim if can_open_dialogue_vim(*focus, dialogue_count) => {
            let view = dialogue_text_vim_view(workspace_content_text(
                dialogues,
                shown_dialogue_idx(&rows.dialogues, *content_page),
                *content_mode,
                content_at,
            ));
            // A failed editor launch must not kill the picker: report it and keep running.
            suspend(terminal, || {
                if let Err(error) = open_vim_view(&view) {
                    eprintln!("sivtr: editor error: {error}");
                }
                Ok(())
            })??;
        }
        WorkspaceHelpAction::ScrollDown if *focus == WorkspaceFocus::Content => {
            content_scrolls.set(
                *content_io_focus,
                content_scrolls.get(*content_io_focus).saturating_add(10),
            );
        }
        WorkspaceHelpAction::ScrollUp if *focus == WorkspaceFocus::Content => {
            content_scrolls.set(
                *content_io_focus,
                content_scrolls.get(*content_io_focus).saturating_sub(10),
            );
        }
        WorkspaceHelpAction::ScrollContentTop if *focus == WorkspaceFocus::Content => {
            content_scrolls.set(*content_io_focus, 0);
        }
        WorkspaceHelpAction::ScrollContentBottom if *focus == WorkspaceFocus::Content => {
            let lines = content_pane.line_count(*content_io_focus);
            content_scrolls.set(*content_io_focus, lines.saturating_sub(1));
        }
        WorkspaceHelpAction::ToggleContentMode if *focus == WorkspaceFocus::Content => {
            *content_mode = content_mode.toggle();
        }
        WorkspaceHelpAction::ToggleContentIo if *focus == WorkspaceFocus::Content => {
            // Only which half scrolls changes; block ids span the dialogue, so
            // an open block range stays valid across the flip.
            *content_io_focus = match *content_io_focus {
                ContentIoFocus::Input => ContentIoFocus::Output,
                ContentIoFocus::Output => ContentIoFocus::Input,
            };
        }
        WorkspaceHelpAction::ToggleBlockFold if *focus == WorkspaceFocus::Content => {
            if *content_mode == ContentViewMode::Reading {
                if let Some(block) = content_cursor.get() {
                    expanded.toggle(block);
                    content_cursor.follow = true;
                }
            }
        }
        WorkspaceHelpAction::Copy => match *focus {
            WorkspaceFocus::Source => set_focus(focus, fullscreen, rows, WorkspaceFocus::Sessions),
            WorkspaceFocus::Sessions if dialogue_count > 0 => {
                set_focus(focus, fullscreen, rows, WorkspaceFocus::Dialogues)
            }
            WorkspaceFocus::Dialogues | WorkspaceFocus::Content => {
                return Ok(HelpDispatch::Picked(workspace_picked_content(
                    dialogues,
                    rows.dialogues.mask(),
                    rows.dialogues.cursor(),
                    content_at,
                )?));
            }
            WorkspaceFocus::Sessions => {}
        },
        WorkspaceHelpAction::CopyInput if dialogue_count > 0 => {
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy_with_line_filter(
                    dialogues,
                    rows.dialogues.mask(),
                    rows.dialogues.cursor(),
                    WorkspaceCopyShortcut::Input,
                    line_filter,
                    None,
                    *content_mode,
                )?,
            ));
        }
        WorkspaceHelpAction::CopyOutput if dialogue_count > 0 => {
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy_with_line_filter(
                    dialogues,
                    rows.dialogues.mask(),
                    rows.dialogues.cursor(),
                    WorkspaceCopyShortcut::Output,
                    line_filter,
                    None,
                    *content_mode,
                )?,
            ));
        }
        WorkspaceHelpAction::CopyBlock if dialogue_count > 0 => {
            // y copies the block under the content cursor (call + result
            // bodies); marked blocks take over in the picker beforehand.
            // The block id belongs to the *displayed* dialogue, so resolve
            // the shown index like the marked paths do, not the focused row.
            let shown = shown_dialogue_idx(&rows.dialogues, *content_page);
            let block_id = content_cursor.get().unwrap_or(0);
            if let Some(picked) = workspace_picked_content_for_cursor_block(
                dialogues,
                rows.dialogues.mask(),
                shown,
                block_id,
            ) {
                return Ok(HelpDispatch::Picked(picked));
            }
        }
        WorkspaceHelpAction::CopyCommand if dialogue_count > 0 => {
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy_with_line_filter(
                    dialogues,
                    rows.dialogues.mask(),
                    rows.dialogues.cursor(),
                    WorkspaceCopyShortcut::Command,
                    line_filter,
                    None,
                    *content_mode,
                )?,
            ));
        }
        WorkspaceHelpAction::ToggleFullscreen => {
            *fullscreen = toggle_fullscreen(*fullscreen, *focus);
        }
        WorkspaceHelpAction::ToggleHelp => {
            *show_help = !*show_help;
        }
        WorkspaceHelpAction::OpenSearch => {
            *show_help = false;
            *show_search = true;
            search_query.clear();
            *search_dirty = true;
            // Search replaces the session list wholesale: drop everything the
            // panes below the sources were showing.
            invalidate_panes_below(WorkspaceFocus::Source, rows, content_scrolls);
        }
        WorkspaceHelpAction::BackOrCancel => match *focus {
            WorkspaceFocus::Source | WorkspaceFocus::Sessions => {
                anyhow::bail!(PICK_CANCELLED_MESSAGE)
            }
            WorkspaceFocus::Dialogues => {
                set_focus(focus, fullscreen, rows, WorkspaceFocus::Sessions);
            }
            WorkspaceFocus::Content => {
                set_focus(focus, fullscreen, rows, WorkspaceFocus::Dialogues);
            }
        },
        WorkspaceHelpAction::Cancel => anyhow::bail!(PICK_CANCELLED_MESSAGE),
        WorkspaceHelpAction::Refresh => return Ok(HelpDispatch::Refresh),
        // Focus-gated arms that did not match: ignore.
        WorkspaceHelpAction::FocusDialogues
        | WorkspaceHelpAction::FocusContent
        | WorkspaceHelpAction::ToggleAllDialogues
        | WorkspaceHelpAction::OpenVim
        | WorkspaceHelpAction::ScrollDown
        | WorkspaceHelpAction::ScrollUp
        | WorkspaceHelpAction::ScrollContentTop
        | WorkspaceHelpAction::ScrollContentBottom
        | WorkspaceHelpAction::ToggleContentMode
        | WorkspaceHelpAction::ToggleContentIo
        | WorkspaceHelpAction::ToggleBlockFold
        | WorkspaceHelpAction::NextDialoguePage
        | WorkspaceHelpAction::PreviousDialoguePage
        | WorkspaceHelpAction::CopyInput
        | WorkspaceHelpAction::CopyOutput
        | WorkspaceHelpAction::CopyBlock
        | WorkspaceHelpAction::CopyCommand => {}
    }

    Ok(HelpDispatch::Continue)
}

pub(super) fn toggle_fullscreen(
    fullscreen: Option<WorkspaceFocus>,
    focus: WorkspaceFocus,
) -> Option<WorkspaceFocus> {
    if fullscreen == Some(focus) {
        None
    } else {
        Some(focus)
    }
}

/// Toggle the focused list row's selection mark — the single path shared by
/// the Space key and a dot-gutter click. `true` when panes below need a
/// refresh (a Source toggle reshapes the session/dialogue trees).
pub(super) fn toggle_list_row(
    focus: WorkspaceFocus,
    idx: usize,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
) -> bool {
    let Some(pane) = rows.pane_mut(focus) else {
        return false;
    };
    pane.toggle(idx);
    rows.close_ranges();
    invalidate_panes_below(focus, rows, content_scrolls)
}

pub(super) fn set_focus(
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    rows: &mut Rows,
    next: WorkspaceFocus,
) {
    *focus = next;
    // Range selection is per-pane: leaving a pane discards its anchor.
    rows.close_ranges();
    if fullscreen.is_some() {
        *fullscreen = Some(next);
    }
}
