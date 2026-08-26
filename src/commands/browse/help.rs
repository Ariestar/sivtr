//! Help-panel + table-driven action dispatch.
//!
//! Key bindings live in `workspace_help_entries()`. This module only runs actions.

use anyhow::Result;
use ratatui::widgets::ListState;

use crate::tui::content::block::BlockText;
use crate::tui::content::view::ContentViewMode;
use crate::tui::terminal::suspend;
use crate::tui::workspace::{
    can_open_dialogue_vim, selected_count, selected_index, workspace_content_text, ContentIoFocus,
    ContentScrolls, ExpandedBlocks, WorkspaceDialogue, WorkspaceFocus, WorkspaceHelpAction,
    WorkspacePickedContent, WorkspaceSession, WorkspaceSource,
};
use sivtr_core::record::WorkAt;

use super::content::{
    dialogue_text_vim_view, workspace_picked_content,
    workspace_picked_content_for_copy_with_line_filter, workspace_picked_content_for_cursor_block,
    WorkspaceCopyShortcut,
};
use super::nav::{
    move_workspace_cursor_down, move_workspace_cursor_up, reset_workspace_after_source_change,
    reset_workspace_dialogue_state, shown_dialogue_idx, ContentBlockCursor,
};
use super::panes::ContentPane;
use super::selection::{apply_range_selection, select_sources, WorkspaceSourceSelection};
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
    source_state: &mut ListState,
    selected_sources: &mut [bool],
    selected_sessions: &mut Vec<bool>,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    // Content block-range anchor for `v` (half + block id of the first press).
    content_range_anchor: &mut Option<(ContentIoFocus, usize)>,
    content_scrolls: &mut ContentScrolls,
    content_io_focus: &mut ContentIoFocus,
    content_mode: &mut ContentViewMode,
    expanded: &mut ExpandedBlocks,
    content_input_lines: usize,
    content_output_lines: usize,
    // Which selected dialogue the content pane shows (multi-select paging).
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
    sessions: &[WorkspaceSession],
    dialogues: &[WorkspaceDialogue],
    session_idx: usize,
    dialogue_idx: usize,
    dialogue_count: usize,
    terminal: &mut crate::tui::terminal::Tui,
) -> Result<HelpDispatch> {
    match action {
        WorkspaceHelpAction::FocusSource => set_focus(
            focus,
            fullscreen,
            range_anchor,
            content_range_anchor,
            WorkspaceFocus::Source,
        ),
        WorkspaceHelpAction::FocusSessions => set_focus(
            focus,
            fullscreen,
            range_anchor,
            content_range_anchor,
            WorkspaceFocus::Sessions,
        ),
        WorkspaceHelpAction::FocusDialogues if dialogue_count > 0 => set_focus(
            focus,
            fullscreen,
            range_anchor,
            content_range_anchor,
            WorkspaceFocus::Dialogues,
        ),
        WorkspaceHelpAction::FocusContent if dialogue_count > 0 => set_focus(
            focus,
            fullscreen,
            range_anchor,
            content_range_anchor,
            WorkspaceFocus::Content,
        ),
        WorkspaceHelpAction::MoveUp => move_workspace_cursor_up(
            *focus,
            sources,
            sessions,
            dialogue_count,
            selected_sessions,
            source_state,
            session_state,
            dialogue_state,
            selected_dialogues,
            content_scrolls,
            *content_io_focus,
            content_cursor,
            content_blocks,
        ),
        WorkspaceHelpAction::MoveDown => move_workspace_cursor_down(
            *focus,
            sources,
            sessions,
            dialogue_count,
            selected_sessions,
            source_state,
            session_state,
            dialogue_state,
            selected_dialogues,
            content_scrolls,
            *content_io_focus,
            content_cursor,
            content_blocks,
        ),
        WorkspaceHelpAction::PreviousPane => {
            if let Some(next_focus) = focus.previous(dialogue_count) {
                set_focus(
                    focus,
                    fullscreen,
                    range_anchor,
                    content_range_anchor,
                    next_focus,
                );
            }
        }
        WorkspaceHelpAction::NextPane => {
            if let Some(next_focus) = focus.next(dialogue_count) {
                set_focus(
                    focus,
                    fullscreen,
                    range_anchor,
                    content_range_anchor,
                    next_focus,
                );
            }
        }
        WorkspaceHelpAction::ToggleSelection => match *focus {
            WorkspaceFocus::Source => {
                if toggle_list_row(
                    *focus,
                    selected_index(source_state),
                    selected_sources,
                    selected_sessions,
                    selected_dialogues,
                    range_anchor,
                    session_state,
                    dialogue_state,
                    content_scrolls,
                ) {
                    return Ok(HelpDispatch::Refresh);
                }
            }
            WorkspaceFocus::Sessions => {
                toggle_list_row(
                    *focus,
                    session_idx,
                    selected_sources,
                    selected_sessions,
                    selected_dialogues,
                    range_anchor,
                    session_state,
                    dialogue_state,
                    content_scrolls,
                );
            }
            WorkspaceFocus::Dialogues => {
                toggle_list_row(
                    *focus,
                    dialogue_idx,
                    selected_sources,
                    selected_sessions,
                    selected_dialogues,
                    range_anchor,
                    session_state,
                    dialogue_state,
                    content_scrolls,
                );
            }
            WorkspaceFocus::Content => {
                // Pane-native selection: Space marks the focused block for
                // batch copy, like Space toggles a list row. Multi-select
                // pages one dialogue at a time, so the shown dialogue owns
                // the mark regardless of the selection count.
                let shown = shown_dialogue_idx(selected_dialogues, *content_page, dialogue_idx);
                if let Some((half, block)) = content_cursor.focused(*content_io_focus) {
                    content_pane.toggle_mark(half, shown, block);
                }
            }
        },
        // Multi-select paging: J/K flip the content pane to the next /
        // previous selected dialogue. The redraw resets the fold state and
        // cursor when the shown dialogue changes; marks follow their
        // dialogue and stay, so a later copy can join pages.
        WorkspaceHelpAction::NextDialoguePage if *focus == WorkspaceFocus::Content => {
            let count = selected_count(selected_dialogues);
            if count > 1 {
                *content_page = (*content_page + 1).min(count.saturating_sub(1));
                content_scrolls.clear();
            }
        }
        WorkspaceHelpAction::PreviousDialoguePage if *focus == WorkspaceFocus::Content => {
            if selected_count(selected_dialogues) > 1 {
                *content_page = content_page.saturating_sub(1);
                content_scrolls.clear();
            }
        }
        WorkspaceHelpAction::SelectAllSources => {
            select_sources(sources, selected_sources, WorkspaceSourceSelection::All);
            reset_workspace_after_source_change(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scrolls,
            );
            return Ok(HelpDispatch::Refresh);
        }
        WorkspaceHelpAction::SelectAgentSources => {
            select_sources(sources, selected_sources, WorkspaceSourceSelection::Agents);
            reset_workspace_after_source_change(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scrolls,
            );
            return Ok(HelpDispatch::Refresh);
        }
        WorkspaceHelpAction::SelectTerminalSource => {
            select_sources(
                sources,
                selected_sources,
                WorkspaceSourceSelection::Terminal,
            );
            reset_workspace_after_source_change(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scrolls,
            );
            return Ok(HelpDispatch::Refresh);
        }
        WorkspaceHelpAction::RangeSelect => match *focus {
            // All list panes share one range-selection semantic: `v`
            // anchors, moves extend, `v` again selects the span. Only the
            // completing `v` (which changes selection) rebuilds panes below.
            WorkspaceFocus::Source => {
                let finishing = range_anchor.is_some();
                apply_range_selection(range_anchor, selected_sources, selected_index(source_state));
                if finishing {
                    reset_workspace_after_source_change(
                        session_state,
                        selected_sessions,
                        dialogue_state,
                        selected_dialogues,
                        range_anchor,
                        content_scrolls,
                    );
                    return Ok(HelpDispatch::Refresh);
                }
            }
            WorkspaceFocus::Sessions => {
                let finishing = range_anchor.is_some();
                apply_range_selection(range_anchor, selected_sessions, session_idx);
                if finishing {
                    reset_workspace_dialogue_state(0, dialogue_state, selected_dialogues);
                    content_scrolls.clear();
                }
            }
            WorkspaceFocus::Dialogues => {
                apply_range_selection(range_anchor, selected_dialogues, dialogue_idx);
            }
            WorkspaceFocus::Content => {
                // Block range: `v` anchors the cursor block in the current
                // half, moves extend, and a second `v` toggles marks across
                // the anchor..cursor span. Switching half re-anchors.
                if let Some((half, cursor_block)) = content_cursor.focused(*content_io_focus) {
                    match *content_range_anchor {
                        Some((anchor_half, anchor_block)) if anchor_half == half => {
                            let shown =
                                shown_dialogue_idx(selected_dialogues, *content_page, dialogue_idx);
                            let blocks = match half {
                                ContentIoFocus::Input => content_blocks.0,
                                ContentIoFocus::Output => content_blocks.1,
                            };
                            let anchor = blocks.iter().position(|b| b.id == anchor_block);
                            let cursor = blocks.iter().position(|b| b.id == cursor_block);
                            if let (Some(anchor), Some(cursor)) = (anchor, cursor) {
                                let (lo, hi) = (anchor.min(cursor), anchor.max(cursor));
                                for block in &blocks[lo..=hi] {
                                    content_pane.toggle_mark(half, shown, block.id);
                                }
                            }
                            *content_range_anchor = None;
                        }
                        _ => *content_range_anchor = Some((half, cursor_block)),
                    }
                }
            }
        },
        WorkspaceHelpAction::ToggleAllDialogues if *focus == WorkspaceFocus::Dialogues => {
            let select_all = selected_dialogues.iter().any(|selected| !selected);
            selected_dialogues.fill(select_all);
            *range_anchor = None;
        }
        WorkspaceHelpAction::OpenVim if can_open_dialogue_vim(*focus, dialogue_count) => {
            let view = dialogue_text_vim_view(workspace_content_text(
                dialogues,
                shown_dialogue_idx(selected_dialogues, *content_page, dialogue_idx),
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
            let lines = match *content_io_focus {
                ContentIoFocus::Input => content_input_lines,
                ContentIoFocus::Output => content_output_lines,
            };
            content_scrolls.set(*content_io_focus, lines.saturating_sub(1));
        }
        WorkspaceHelpAction::ToggleContentMode if *focus == WorkspaceFocus::Content => {
            *content_mode = content_mode.toggle();
        }
        WorkspaceHelpAction::ToggleContentIo if *focus == WorkspaceFocus::Content => {
            *content_io_focus = match *content_io_focus {
                ContentIoFocus::Input => ContentIoFocus::Output,
                ContentIoFocus::Output => ContentIoFocus::Input,
            };
            *content_range_anchor = None;
        }
        WorkspaceHelpAction::ToggleBlockFold if *focus == WorkspaceFocus::Content => {
            if *content_mode == ContentViewMode::Reading {
                if let Some((half, block)) = content_cursor.focused(*content_io_focus) {
                    expanded.toggle(half, block);
                    content_cursor.follow = true;
                }
            }
        }
        WorkspaceHelpAction::Copy => match *focus {
            WorkspaceFocus::Source => set_focus(
                focus,
                fullscreen,
                range_anchor,
                content_range_anchor,
                WorkspaceFocus::Sessions,
            ),
            WorkspaceFocus::Sessions if dialogue_count > 0 => set_focus(
                focus,
                fullscreen,
                range_anchor,
                content_range_anchor,
                WorkspaceFocus::Dialogues,
            ),
            WorkspaceFocus::Dialogues | WorkspaceFocus::Content => {
                return Ok(HelpDispatch::Picked(workspace_picked_content(
                    dialogues,
                    selected_dialogues,
                    dialogue_idx,
                    content_at,
                )));
            }
            WorkspaceFocus::Sessions => {}
        },
        WorkspaceHelpAction::CopyInput if dialogue_count > 0 => {
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy_with_line_filter(
                    dialogues,
                    selected_dialogues,
                    dialogue_idx,
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
                    selected_dialogues,
                    dialogue_idx,
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
            let shown = shown_dialogue_idx(selected_dialogues, *content_page, dialogue_idx);
            let block_id = content_cursor.get(*content_io_focus).unwrap_or(0);
            if let Some(picked) = workspace_picked_content_for_cursor_block(
                dialogues,
                selected_dialogues,
                shown,
                *content_io_focus,
                block_id,
                line_filter,
            ) {
                return Ok(HelpDispatch::Picked(picked));
            }
        }
        WorkspaceHelpAction::CopyCommand if dialogue_count > 0 => {
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy_with_line_filter(
                    dialogues,
                    selected_dialogues,
                    dialogue_idx,
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
            reset_workspace_after_source_change(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scrolls,
            );
        }
        WorkspaceHelpAction::BackOrCancel => match *focus {
            WorkspaceFocus::Source | WorkspaceFocus::Sessions => {
                anyhow::bail!(PICK_CANCELLED_MESSAGE)
            }
            WorkspaceFocus::Dialogues => {
                set_focus(
                    focus,
                    fullscreen,
                    range_anchor,
                    content_range_anchor,
                    WorkspaceFocus::Sessions,
                );
            }
            WorkspaceFocus::Content => {
                set_focus(
                    focus,
                    fullscreen,
                    range_anchor,
                    content_range_anchor,
                    WorkspaceFocus::Dialogues,
                );
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
#[allow(clippy::too_many_arguments)]
pub(super) fn toggle_list_row(
    focus: WorkspaceFocus,
    idx: usize,
    selected_sources: &mut [bool],
    selected_sessions: &mut Vec<bool>,
    selected_dialogues: &mut Vec<bool>,
    range_anchor: &mut Option<usize>,
    session_state: &mut ListState,
    dialogue_state: &mut ListState,
    content_scrolls: &mut ContentScrolls,
) -> bool {
    match focus {
        WorkspaceFocus::Source => {
            if let Some(selected) = selected_sources.get_mut(idx) {
                *selected = !*selected;
            }
            reset_workspace_after_source_change(
                session_state,
                selected_sessions,
                dialogue_state,
                selected_dialogues,
                range_anchor,
                content_scrolls,
            );
            true
        }
        WorkspaceFocus::Sessions => {
            if let Some(selected) = selected_sessions.get_mut(idx) {
                *selected = !*selected;
            }
            *range_anchor = None;
            reset_workspace_dialogue_state(0, dialogue_state, selected_dialogues);
            content_scrolls.clear();
            false
        }
        WorkspaceFocus::Dialogues => {
            if let Some(selected) = selected_dialogues.get_mut(idx) {
                *selected = !*selected;
            }
            *range_anchor = None;
            false
        }
        WorkspaceFocus::Content => false,
    }
}

pub(super) fn set_focus(
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    range_anchor: &mut Option<usize>,
    content_range_anchor: &mut Option<(ContentIoFocus, usize)>,
    next: WorkspaceFocus,
) {
    *focus = next;
    // Range selection is per-pane: leaving a pane discards its anchors.
    *range_anchor = None;
    *content_range_anchor = None;
    if fullscreen.is_some() {
        *fullscreen = Some(next);
    }
}
