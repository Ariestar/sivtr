//! Help-panel action dispatch.

use anyhow::Result;
use ratatui::widgets::ListState;

use crate::tui::content_view::ContentViewMode;
use crate::tui::terminal::{init as init_tui, restore as restore_tui};
use crate::tui::workspace::{
    can_open_dialogue_vim, selected_index, workspace_content_io_texts, workspace_content_text,
    workspace_layout, ContentIoFocus, ContentIoFrame, ContentScrolls, WorkspaceDialogue,
    WorkspaceFocus, WorkspaceHelpAction, WorkspacePickedContent, WorkspaceSession, WorkspaceSource,
};
use sivtr_core::record::WorkAt;

use super::content::{
    dialogue_text_vim_view, workspace_picked_content, workspace_picked_content_for_copy,
    WorkspaceCopyShortcut,
};
use super::nav::{
    move_workspace_cursor_down, move_workspace_cursor_up, reset_workspace_after_source_change,
    reset_workspace_dialogue_state, reset_workspace_search_state,
};
use super::content::apply_dialogue_range_selection;
use super::selection::{select_sources, WorkspaceSourceSelection};
use super::vim::open_vim_view;
use super::visual::{enter_visual_select_mode, VisualSelectMode};
use super::PICK_CANCELLED_MESSAGE;

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
    show_search: &mut bool,
    search_query: &mut String,
    search_dirty: &mut bool,
    visual_select_mode: &mut Option<VisualSelectMode>,
    content_at: Option<WorkAt>,
    sessions: &[WorkspaceSession],
    dialogues: &[WorkspaceDialogue],
    session_idx: usize,
    dialogue_idx: usize,
    dialogue_count: usize,
    terminal: &mut crate::tui::terminal::Tui,
) -> Result<Option<WorkspacePickedContent>> {
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
            _ => {}
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
        WorkspaceHelpAction::ToggleContentMode if *focus == WorkspaceFocus::Content => {
            *content_mode = content_mode.toggle();
        }
        WorkspaceHelpAction::VisualTextSelect if *focus == WorkspaceFocus::Content => {
            let size = terminal.size()?;
            let layout = workspace_layout(
                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                *focus,
                *fullscreen,
            );
            {
                let io = workspace_content_io_texts(
                    dialogues,
                    selected_dialogues,
                    dialogue_idx,
                    *content_mode,
                    content_at,
                );
                let frame = ContentIoFrame::build(layout.content, &io, *content_mode);
                let active = frame.active(*content_io_focus, content_scrolls);
                enter_visual_select_mode(
                    visual_select_mode,
                    active.scroll,
                    active.area,
                    active.text,
                    *content_mode,
                );
            }
        }
        WorkspaceHelpAction::Copy => match *focus {
            WorkspaceFocus::Source => set_focus(focus, fullscreen, WorkspaceFocus::Sessions),
            WorkspaceFocus::Sessions if dialogue_count > 0 => {
                set_focus(focus, fullscreen, WorkspaceFocus::Dialogues)
            }
            WorkspaceFocus::Dialogues | WorkspaceFocus::Content => {
                return Ok(Some(workspace_picked_content(
                    dialogues,
                    selected_dialogues,
                    dialogue_idx,
                    content_at,
                )));
            }
            WorkspaceFocus::Sessions => {}
        },
        WorkspaceHelpAction::CopyInput if dialogue_count > 0 => {
            return Ok(Some(workspace_picked_content_for_copy(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                WorkspaceCopyShortcut::Input,
            )));
        }
        WorkspaceHelpAction::CopyOutput if dialogue_count > 0 => {
            return Ok(Some(workspace_picked_content_for_copy(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                WorkspaceCopyShortcut::Output,
            )));
        }
        WorkspaceHelpAction::CopyBlock if dialogue_count > 0 => {
            return Ok(Some(workspace_picked_content_for_copy(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                WorkspaceCopyShortcut::Block,
            )));
        }
        WorkspaceHelpAction::CopyCommand if dialogue_count > 0 => {
            return Ok(Some(workspace_picked_content_for_copy(
                dialogues,
                selected_dialogues,
                dialogue_idx,
                WorkspaceCopyShortcut::Command,
            )));
        }
        WorkspaceHelpAction::ToggleFullscreen => {
            *fullscreen = toggle_fullscreen(*fullscreen, *focus);
        }
        WorkspaceHelpAction::CloseHelp => {}
        WorkspaceHelpAction::OpenSearch => {
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
        WorkspaceHelpAction::Cancel => anyhow::bail!(PICK_CANCELLED_MESSAGE),
        WorkspaceHelpAction::Refresh => {
            // Help path cannot refresh without the load pump; keyboard R handles it.
        }
        _ => {}
    }

    Ok(None)
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
