//! Dialogue construction, content copy, line filter, and search targeting.

use anyhow::{Context, Result};
use crossterm::event::KeyCode;

use crate::commands::select::CommandSelection;
use crate::tui::content::block::{dialogue_blocks, Block};
use crate::tui::content::view::{line_count, ContentViewMode};
use crate::tui::search::{WorkspaceSearchMatch, WorkspaceSearchOutput};
use crate::tui::workspace::{
    WorkspaceDialogue, WorkspacePickedContent, WorkspaceSession, WorkspaceSource,
};
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};

use super::panes::ContentPane;
use super::text::filter_lines_by_spec;
use super::vim::{VimBlock, VimView};

#[derive(Clone, Copy)]
pub(super) enum WorkspaceCopyShortcut {
    Displayed,
    Input,
    Output,
    Command,
}

/// Source a copy is attributed to: the first picked dialogue's.
pub(super) fn picked_source(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
) -> Option<WorkspaceSource> {
    dialogues
        .get(*picked.first()?)
        .map(|dialogue| dialogue.source.clone())
}

pub(super) fn workspace_picked_content_for_copy_with_line_filter(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
    shortcut: WorkspaceCopyShortcut,
    line_filter: Option<&str>,
    target: Option<WorkAt>,
    content_mode: ContentViewMode,
) -> Result<WorkspacePickedContent> {
    let source = picked_source(dialogues, picked).context("copy needs at least one dialogue")?;
    let display_target = (picked.len() == 1
        && matches!(shortcut, WorkspaceCopyShortcut::Displayed))
    .then_some(target)
    .flatten();
    let units = picked
        .iter()
        .filter_map(|&idx| dialogues.get(idx))
        .map(|dialogue| match shortcut {
            WorkspaceCopyShortcut::Displayed => dialogue.display_unit(content_mode, display_target),
            WorkspaceCopyShortcut::Input => dialogue.copy.input.clone(),
            WorkspaceCopyShortcut::Output => dialogue.copy.output.clone(),
            WorkspaceCopyShortcut::Command => dialogue.copy.command.clone(),
        })
        .collect::<Vec<_>>();
    let units = apply_workspace_line_filter(units, line_filter)?;
    let selection = CommandSelection::RecentExplicit((1..=units.len()).collect());
    Ok(WorkspacePickedContent {
        source,
        units,
        selection,
    })
}

#[cfg(test)]
pub(super) fn workspace_picked_content_for_copy(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
    shortcut: WorkspaceCopyShortcut,
) -> WorkspacePickedContent {
    workspace_picked_content_for_copy_with_line_filter(
        dialogues,
        picked,
        shortcut,
        None,
        None,
        ContentViewMode::Reading,
    )
    .expect("workspace copy without a line filter should not fail")
}

pub(super) fn workspace_picked_content_with_line_filter(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
    line_filter: Option<&str>,
    target: Option<WorkAt>,
) -> Result<WorkspacePickedContent> {
    workspace_picked_content_for_copy_with_line_filter(
        dialogues,
        picked,
        WorkspaceCopyShortcut::Displayed,
        line_filter,
        target,
        ContentViewMode::Reading,
    )
}

pub(super) fn workspace_picked_content(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
    target: Option<WorkAt>,
) -> Result<WorkspacePickedContent> {
    workspace_picked_content_with_line_filter(dialogues, picked, None, target)
}

/// Picked content from the content pane's marked blocks: every selected
/// block's full body (regardless of fold state), joined in display order.
/// Marks follow their dialogue (multi-select paging keeps them), so all
/// marked dialogues contribute, in selection order. Dialogues without a
/// record are skipped, keeping the blocks already collected. `None` when
/// nothing is marked.
pub(super) fn workspace_picked_content_for_marked_blocks(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
    content_pane: &ContentPane,
) -> Option<WorkspacePickedContent> {
    let mut texts = Vec::new();
    for &dialogue_idx in picked {
        let Some(record) = dialogues
            .get(dialogue_idx)
            .and_then(|dialogue| dialogue.record.as_ref())
        else {
            continue;
        };
        let (input_blocks, output_blocks) = dialogue_blocks(record);
        for block in input_blocks.iter().chain(&output_blocks) {
            collect_marked_blocks(block, dialogue_idx, content_pane, record, &mut texts);
        }
    }
    picked_for_texts(dialogues, picked, texts)
}

/// Push every marked block's body. A run's body already spans its members,
/// so a marked run closes the branch: marking the run and a member of it —
/// which an expanded run lets a click or a `v` span do — must not copy that
/// member twice. Members are only collected on their own when the run itself
/// is unmarked.
fn collect_marked_blocks(
    block: &Block,
    dialogue_idx: usize,
    content_pane: &ContentPane,
    record: &WorkRecord,
    texts: &mut Vec<String>,
) {
    if content_pane
        .marked(dialogue_idx)
        .get(block.id)
        .copied()
        .unwrap_or(false)
    {
        texts.push(block.body(record));
        return;
    }
    for child in &block.children {
        collect_marked_blocks(child, dialogue_idx, content_pane, record, texts);
    }
}

/// Copy the block under the content cursor: y without marked blocks joins
/// just that block's call + result bodies, not the whole dialogue. The copy
/// is attributed to the dialogue the block belongs to.
pub(super) fn workspace_picked_content_for_cursor_block(
    dialogues: &[WorkspaceDialogue],
    dialogue_idx: usize,
    block_id: usize,
) -> Option<WorkspacePickedContent> {
    let dialogue = dialogues.get(dialogue_idx)?;
    let record = dialogue.record.as_ref()?;
    let (input_blocks, output_blocks) = dialogue_blocks(record);
    let block = input_blocks
        .iter()
        .chain(&output_blocks)
        .find_map(|block| find_block(block, block_id))?;
    picked_for_texts(dialogues, &[dialogue_idx], vec![block.body(record)])
}

/// Depth-first block lookup: run members live nested in `children`, and the
/// content cursor may sit on either a run or one of its members.
fn find_block(block: &Block, id: usize) -> Option<&Block> {
    if block.id == id {
        return Some(block);
    }
    block
        .children
        .iter()
        .find_map(|child| find_block(child, id))
}

/// One copy unit from already-collected block bodies.
fn picked_for_texts(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
    texts: Vec<String>,
) -> Option<WorkspacePickedContent> {
    if texts.is_empty() {
        return None;
    }
    let plain = texts.join("\n\n");
    let source = picked_source(dialogues, picked)?;
    Some(WorkspacePickedContent {
        source,
        units: vec![crate::tui::workspace::TextPair {
            ansi: plain.clone(),
            plain,
        }],
        selection: CommandSelection::RecentExplicit(vec![1]),
    })
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

/// Apply a bracketed paste to the line filter with the same character policy
/// as typed input (digits, `:`, `,`). Clipboard content is often copied with a
/// trailing newline or other stray characters; appending it verbatim would make
/// the later `filter_lines_by_spec` parse fail and exit the picker. Matching the
/// typed path, the error is cleared once usable characters land.
pub(super) fn handle_line_filter_paste(
    text: &str,
    line_filter: &mut String,
    line_filter_error: &mut Option<String>,
) {
    let filtered: String = text
        .chars()
        .filter(|ch| matches!(ch, '0'..='9' | ':' | ','))
        .collect();
    if !filtered.is_empty() {
        line_filter.push_str(&filtered);
        *line_filter_error = None;
    }
}

pub(super) fn workspace_search_target_ref<'a>(
    sessions: &'a [WorkspaceSession],
    matched: &WorkspaceSearchMatch,
    records: &dyn Fn(&WorkspaceSession) -> Option<&'a [sivtr_core::record::WorkRecord]>,
) -> Option<WorkRef> {
    let session = sessions.get(matched.session_index)?;
    records(session)?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::browse::panes::{ContentCtx, ContentPane};
    use crate::tui::content::io::{ContentIoFocus, ExpandedBlocks};
    use crate::tui::workspace::{WorkspaceCopyParts, WorkspaceSource};
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef,
        WorkSource, WorkTime,
    };

    fn record(title: &str, tool: &str, command: &str, index: usize) -> WorkRecord {
        let mut record = WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "test", index + 1),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: "test".to_string(),
                canonical_id: Some("test-session-0123456789abcdef".to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: title.to_string(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::User {
                    content: "user".to_string(),
                },
            }],
        };
        record.parts.push(WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: Some("c1".to_string()),
                tool: Some(tool.to_string()),
                input: serde_json::json!({ "command": command }),
            },
        });
        record
    }

    fn dialogue(record: WorkRecord) -> WorkspaceDialogue {
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            record: Some(record),
            copy: WorkspaceCopyParts::default(),
        }
    }

    #[test]
    fn marked_blocks_copy_joins_every_selected_dialogue() {
        let a = dialogue(record("A", "Bash", "ls", 0));
        let b = dialogue(record("B", "Bash", "git status", 1));
        let dialogues = [a, b];
        let mut pane = ContentPane::default();
        // Multi-select paging ensures each dialogue in turn, keeping its
        // marks; both end up owned by their dialogue.
        for idx in 0..2 {
            pane.ensure(ContentCtx {
                dialogues: &dialogues,
                highlighted_idx: idx,
                mode: ContentViewMode::Reading,
                target: None,
                area: ratatui::layout::Rect::new(0, 0, 60, 20),
                io_focus: ContentIoFocus::Output,
                expanded: &ExpandedBlocks::default(),
            });
            // Dialogue-global ids: the input user block is 0, so the output
            // tool block is 1.
            pane.toggle_mark(idx, 1);
        }

        let picked = workspace_picked_content_for_marked_blocks(&dialogues, &[0, 1], &pane)
            .expect("marked blocks across two dialogues");
        let joined: Vec<String> = picked.units.iter().map(|unit| unit.plain.clone()).collect();
        let all = joined.join("\n");
        assert!(
            all.contains("$ ls"),
            "dialogue A marked block missing: {all}"
        );
        assert!(
            all.contains("$ git status"),
            "dialogue B marked block missing: {all}"
        );
    }

    #[test]
    fn a_marked_run_copies_its_members_once() {
        let mut base = record("A", "Bash", "ls", 0);
        // A second consecutive tool call folds both into one run: block 1 is
        // the run, blocks 2 and 3 its members.
        base.parts.push(WorkPart {
            seq: 3,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: Some("c2".to_string()),
                tool: Some("Bash".to_string()),
                input: serde_json::json!({ "command": "git status" }),
            },
        });
        let dialogues = [dialogue(base)];
        let mut pane = ContentPane::default();
        pane.ensure(ContentCtx {
            dialogues: &dialogues,
            highlighted_idx: 0,
            mode: ContentViewMode::Reading,
            target: None,
            area: ratatui::layout::Rect::new(0, 0, 60, 20),
            io_focus: ContentIoFocus::Output,
            expanded: &ExpandedBlocks::default(),
        });
        // An expanded run shows its header and its members, so a `v` span or
        // two clicks can mark both; the run's body already spans the member.
        pane.toggle_mark(0, 1);
        pane.toggle_mark(0, 2);

        let picked = workspace_picked_content_for_marked_blocks(&dialogues, &[0], &pane)
            .expect("marked run");
        let all = picked
            .units
            .iter()
            .map(|unit| unit.plain.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(all.matches("$ ls").count(), 1, "member copied twice: {all}");
    }
}
