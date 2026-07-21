//! Help-panel + table-driven action dispatch.
//!
//! Key bindings live in `workspace_help_entries()`. This module only runs actions.

use anyhow::Result;
use ratatui::widgets::ListState;

use crate::tui::content::view::ContentViewMode;
use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use crate::tui::workspace::{
    can_open_dialogue_vim, selected_index, workspace_content_io_texts, workspace_content_text,
    workspace_layout, ContentIoFocus, ContentIoFrame, ContentScrolls, WorkspaceDialogue,
    WorkspaceFocus, WorkspaceHelpAction, WorkspacePickedContent, WorkspaceSession, WorkspaceSource,
};
use sivtr_core::record::WorkAt;

use super::content::{
    apply_dialogue_range_selection, dialogue_text_vim_view, workspace_picked_content,
    workspace_picked_content_for_copy_with_line_filter, WorkspaceCopyShortcut,
};
use super::nav::{
    move_workspace_cursor_down, move_workspace_cursor_up, reset_workspace_after_source_change,
    reset_workspace_dialogue_state, reset_workspace_search_state,
};
use super::selection::{select_sources, WorkspaceSourceSelection};
use super::vim::open_vim_view;
use super::visual::{enter_visual_select_mode, VisualSelectMode};
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
    content_scrolls: &mut ContentScrolls,
    content_io_focus: &mut ContentIoFocus,
    content_mode: &mut ContentViewMode,
    content_input_lines: usize,
    content_output_lines: usize,
    show_help: &mut bool,
    show_search: &mut bool,
    search_query: &mut String,
    search_dirty: &mut bool,
    visual_select_mode: &mut Option<VisualSelectMode>,
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
        WorkspaceHelpAction::FocusSource => set_focus(focus, fullscreen, WorkspaceFocus::Source),
        WorkspaceHelpAction::FocusSessions => {
            set_focus(focus, fullscreen, WorkspaceFocus::Sessions)
        }
        WorkspaceHelpAction::FocusDialogues if dialogue_count > 0 => {
            set_focus(focus, fullscreen, WorkspaceFocus::Dialogues)
        }
        WorkspaceHelpAction::FocusContent if dialogue_count > 0 => {
            set_focus(focus, fullscreen, WorkspaceFocus::Content)
        }
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
            range_anchor,
            content_scrolls,
            *content_io_focus,
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
            range_anchor,
            content_scrolls,
            *content_io_focus,
        ),
        WorkspaceHelpAction::PreviousPane => {
            if let Some(next_focus) = focus.previous(dialogue_count) {
                set_focus(focus, fullscreen, next_focus);
            }
        }
        WorkspaceHelpAction::NextPane => {
            if let Some(next_focus) = focus.next(dialogue_count) {
                set_focus(focus, fullscreen, next_focus);
            }
        }
        WorkspaceHelpAction::ToggleSelection => match *focus {
            WorkspaceFocus::Source => {
                let source_idx = selected_index(source_state);
                if let Some(selected) = selected_sources.get_mut(source_idx) {
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
                return Ok(HelpDispatch::Refresh);
            }
            WorkspaceFocus::Sessions => {
                if let Some(selected) = selected_sessions.get_mut(session_idx) {
                    *selected = !*selected;
                }
                reset_workspace_dialogue_state(0, dialogue_state, selected_dialogues, range_anchor);
                content_scrolls.clear();
            }
            WorkspaceFocus::Dialogues => {
                if let Some(selected) = selected_dialogues.get_mut(dialogue_idx) {
                    *selected = !*selected;
                }
                *range_anchor = None;
            }
            WorkspaceFocus::Content => {}
        },
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
        WorkspaceHelpAction::RangeSelect if *focus == WorkspaceFocus::Dialogues => {
            apply_dialogue_range_selection(range_anchor, selected_dialogues, dialogue_idx);
        }
        WorkspaceHelpAction::ToggleAllDialogues if *focus == WorkspaceFocus::Dialogues => {
            let select_all = selected_dialogues.iter().any(|selected| !selected);
            selected_dialogues.fill(select_all);
            *range_anchor = None;
        }
        WorkspaceHelpAction::OpenVim if can_open_dialogue_vim(*focus, dialogue_count) => {
            let view = dialogue_text_vim_view(workspace_content_text(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                *content_mode,
                content_at,
            ));
            restore_tui(terminal)?;
            open_vim_view(&view)?;
            *terminal = init_tui()?;
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
            content_scrolls.clear_half(*content_io_focus);
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
            *content_io_focus = content_io_focus.toggle();
        }
        WorkspaceHelpAction::VisualTextSelect if *focus == WorkspaceFocus::Content => {
            let size = terminal.size()?;
            let layout = workspace_layout(
                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                *focus,
                *fullscreen,
            );
            let io = workspace_content_io_texts(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                *content_mode,
                content_at,
            );
            let frame =
                ContentIoFrame::build(layout.content, &io, *content_mode, *content_io_focus);
            let active = frame.active(*content_io_focus, content_scrolls);
            enter_visual_select_mode(
                visual_select_mode,
                active.scroll,
                active.area,
                active.text,
                *content_mode,
            );
        }
        WorkspaceHelpAction::Copy => match *focus {
            WorkspaceFocus::Source => set_focus(focus, fullscreen, WorkspaceFocus::Sessions),
            WorkspaceFocus::Sessions if dialogue_count > 0 => {
                set_focus(focus, fullscreen, WorkspaceFocus::Dialogues)
            }
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
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy_with_line_filter(
                    dialogues,
                    selected_dialogues,
                    dialogue_idx,
                    WorkspaceCopyShortcut::Block,
                    line_filter,
                    None,
                    *content_mode,
                )?,
            ));
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
            reset_workspace_search_state(
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
                set_focus(focus, fullscreen, WorkspaceFocus::Sessions);
            }
            WorkspaceFocus::Content => {
                set_focus(focus, fullscreen, WorkspaceFocus::Dialogues);
            }
        },
        WorkspaceHelpAction::Cancel => anyhow::bail!(PICK_CANCELLED_MESSAGE),
        WorkspaceHelpAction::Refresh => return Ok(HelpDispatch::Refresh),
        // Focus-gated arms that did not match: ignore.
        WorkspaceHelpAction::FocusDialogues
        | WorkspaceHelpAction::FocusContent
        | WorkspaceHelpAction::RangeSelect
        | WorkspaceHelpAction::ToggleAllDialogues
        | WorkspaceHelpAction::OpenVim
        | WorkspaceHelpAction::ScrollDown
        | WorkspaceHelpAction::ScrollUp
        | WorkspaceHelpAction::ScrollContentTop
        | WorkspaceHelpAction::ScrollContentBottom
        | WorkspaceHelpAction::ToggleContentMode
        | WorkspaceHelpAction::ToggleContentIo
        | WorkspaceHelpAction::VisualTextSelect
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

pub(super) fn set_focus(
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    next: WorkspaceFocus,
) {
    *focus = next;
    if fullscreen.is_some() {
        *fullscreen = Some(next);
    }
}
