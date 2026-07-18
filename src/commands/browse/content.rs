//! Dialogue construction, content copy, line filter, and search targeting.

use anyhow::Result;
use crossterm::event::KeyCode;

use crate::commands::select::CommandSelection;
use crate::tui::content_view::{content_view_line_count, line_count, ContentViewMode};
use crate::tui::workspace::{
    workspace_content_text, WorkspaceDialogue, WorkspacePickedContent, WorkspaceSession,
};
use crate::tui::workspace_search::{WorkspaceSearchMatch, WorkspaceSearchOutput};
use sivtr_core::record::{WorkAt, WorkRef};

use super::text::{filter_lines_by_spec, record_to_copy_parts};
use super::vim::{VimBlock, VimView};

#[derive(Clone, Copy)]
pub(super) enum WorkspaceCopyShortcut {
    Displayed,
    Input,
    Output,
    Block,
    Command,
}

pub(super) fn workspace_picked_content_for_copy_with_line_filter(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    shortcut: WorkspaceCopyShortcut,
    line_filter: Option<&str>,
    target: Option<WorkAt>,
    content_mode: ContentViewMode,
) -> Result<WorkspacePickedContent> {
    let selected_indices = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();
    let picked_indices = if selected_indices.is_empty() {
        vec![dialogue_idx]
    } else {
        selected_indices
    };
    let source_idx = picked_indices[0];
    let display_target = (picked_indices.len() == 1
        && matches!(shortcut, WorkspaceCopyShortcut::Displayed))
    .then_some(target)
    .flatten();
    let units = picked_indices
        .into_iter()
        .filter_map(|idx| dialogues.get(idx))
        .map(|dialogue| match shortcut {
            WorkspaceCopyShortcut::Displayed => dialogue.display_unit(content_mode, display_target),
            WorkspaceCopyShortcut::Input => dialogue.copy.input.clone(),
            WorkspaceCopyShortcut::Output => dialogue.copy.output.clone(),
            WorkspaceCopyShortcut::Block => dialogue.copy.block.clone(),
            WorkspaceCopyShortcut::Command => dialogue.copy.command.clone(),
        })
        .collect::<Vec<_>>();
    let units = apply_workspace_line_filter(units, line_filter)?;
    let selection = CommandSelection::RecentExplicit((1..=units.len()).collect());
    Ok(WorkspacePickedContent {
        source: dialogues[source_idx].source.clone(),
        units,
        selection,
    })
}

pub(super) fn workspace_picked_content_for_copy(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    shortcut: WorkspaceCopyShortcut,
) -> WorkspacePickedContent {
    workspace_picked_content_for_copy_with_line_filter(
        dialogues,
        selected_dialogues,
        dialogue_idx,
        shortcut,
        None,
        None,
        ContentViewMode::Reading,
    )
    .expect("workspace copy without a line filter should not fail")
}

pub(super) fn workspace_picked_content_with_line_filter(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    line_filter: Option<&str>,
    target: Option<WorkAt>,
) -> Result<WorkspacePickedContent> {
    workspace_picked_content_for_copy_with_line_filter(
        dialogues,
        selected_dialogues,
        dialogue_idx,
        WorkspaceCopyShortcut::Displayed,
        line_filter,
        target,
        ContentViewMode::Reading,
    )
}

pub(super) fn workspace_picked_content(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    target: Option<WorkAt>,
) -> WorkspacePickedContent {
    workspace_picked_content_with_line_filter(
        dialogues,
        selected_dialogues,
        dialogue_idx,
        None,
        target,
    )
    .expect("workspace copy without a line filter should not fail")
}

pub(super) fn line_filter_spec(line_filter: &str) -> Option<&str> {
    (!line_filter.is_empty()).then_some(line_filter)
}

pub(super) fn apply_workspace_line_filter(
    units: Vec<crate::tui::workspace::TextPair>,
    line_filter: Option<&str>,
) -> Result<Vec<crate::tui::workspace::TextPair>> {
    let Some(spec) = line_filter else {
        return Ok(units);
    };

    units
        .into_iter()
        .map(|unit| filter_lines_by_spec(&unit, spec))
        .collect()
}

pub(super) fn handle_line_filter_key(
    key: KeyCode,
    dialogue_count: usize,
    line_filter_input_open: &mut bool,
    line_filter: &mut String,
    line_filter_error: &mut Option<String>,
) -> bool {
    if *line_filter_input_open {
        match key {
            KeyCode::Char(ch) if matches!(ch, '0'..='9' | ':' | ',') => {
                line_filter.push(ch);
                *line_filter_error = None;
                return true;
            }
            KeyCode::Backspace => {
                *line_filter_error = None;
                if line_filter.pop().is_none() {
                    *line_filter_input_open = false;
                }
                return true;
            }
            KeyCode::Esc => {
                *line_filter_input_open = false;
                line_filter.clear();
                *line_filter_error = None;
                return true;
            }
            _ => {}
        }
    }

    match key {
        KeyCode::Char(':') if dialogue_count > 0 => {
            *line_filter_input_open = true;
            *line_filter_error = None;
            true
        }
        KeyCode::Esc if line_filter_error.is_some() => {
            *line_filter_error = None;
            true
        }
        _ => false,
    }
}

pub(super) fn workspace_content_line_count(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    target: Option<WorkAt>,
    content_area: ratatui::layout::Rect,
    mode: ContentViewMode,
) -> usize {
    let text = workspace_content_text(dialogues, selected_dialogues, highlighted_idx, mode, target);
    content_view_line_count(content_area, &text, mode)
}

pub(super) fn apply_dialogue_range_selection(
    range_anchor: &mut Option<usize>,
    selected_dialogues: &mut [bool],
    dialogue_idx: usize,
) {
    if let Some(anchor) = range_anchor.take() {
        let start = anchor.min(dialogue_idx);
        let end = anchor.max(dialogue_idx);
        let select = selected_dialogues
            .get(start..=end)
            .map(|range| range.iter().any(|selected| !selected))
            .unwrap_or(true);
        for idx in start..=end {
            if let Some(selected) = selected_dialogues.get_mut(idx) {
                *selected = select;
            }
        }
    } else {
        *range_anchor = Some(dialogue_idx);
    }
}

pub(super) fn workspace_dialogues_for_sessions(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
) -> Vec<WorkspaceDialogue> {
    let selected_indices = selected_sessions
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();
    let session_indices = if selected_indices.is_empty() {
        vec![session_idx]
    } else {
        selected_indices
    };

    session_indices
        .into_iter()
        .filter_map(|idx| sessions.get(idx))
        .flat_map(|session| {
            session
                .records
                .iter()
                .map(|record| WorkspaceDialogue {
                    source: session.source.clone(),
                    work_ref: Some(record.work_ref.clone()),
                    title: record.title.clone(),
                    record: Some(record.clone()),
                    copy: record_to_copy_parts(record, sivtr_core::ai::AgentSelection::LastTurn),
                })
                .collect::<Vec<_>>()
                .into_iter()
        })
        .collect()
}

pub(super) fn workspace_search_target_ref(
    sessions: &[WorkspaceSession],
    matched: &WorkspaceSearchMatch,
) -> Option<WorkRef> {
    sessions
        .get(matched.session_index)?
        .records
        .get(matched.dialogue_index)
        .map(|record| record.work_ref.with_at(matched.at))
}

pub(super) fn active_workspace_content_at(
    search_has_query: bool,
    search_output: &WorkspaceSearchOutput,
    search_cursor: usize,
    session_idx: usize,
    selected_dialogues: &[bool],
    dialogue_idx: usize,
) -> Option<WorkAt> {
    if !search_has_query || selected_dialogues.iter().any(|selected| *selected) {
        return None;
    }

    let matched = search_output.matches.get(search_cursor)?;
    (matched.session_index == session_idx && matched.dialogue_index == dialogue_idx)
        .then_some(matched.at)
}

#[cfg(test)]
pub(super) fn workspace_dialogue_vim_view(dialogue: &WorkspaceDialogue) -> VimView {
    dialogue_text_vim_view(dialogue.content_text(ContentViewMode::Reading, None))
}

pub(super) fn dialogue_text_vim_view(text: String) -> VimView {
    let end = line_count(&text).max(1);
    VimView {
        blocks: vec![VimBlock {
            start: 1,
            end,
            input_start: 1,
            input_end: end,
            output_start: 1,
            output_end: end,
            block_text: text.clone(),
            input_text: text.clone(),
            output_text: text.clone(),
            command_text: String::new(),
        }],
        raw: text,
    }
}
