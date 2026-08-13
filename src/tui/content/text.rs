//! Part → dual Input/Output display text (structure fold in reading mode).

use sivtr_core::record::{WorkAt, WorkRecord};

use crate::tui::content::io::{ContentIoTexts, ExpandedBlocks};
use crate::tui::content::view::ContentViewMode;
use crate::tui::workspace::model::WorkspaceDialogue;

/// Read mode folds every structure part to its `<:…:>` tag; parts listed in
/// `expanded` show their full block instead. Raw mode always shows full
/// blocks (the expand state only affects reading).
pub(crate) fn content_io_from_record_expanded(
    record: &WorkRecord,
    reading: bool,
    expanded: &ExpandedBlocks,
) -> ContentIoTexts {
    ContentIoTexts {
        input: io_body_text(record, reading, true, &expanded.input),
        output: io_body_text(record, reading, false, &expanded.output),
    }
}

fn io_body_text(
    record: &WorkRecord,
    reading: bool,
    input: bool,
    expanded: &std::collections::HashSet<usize>,
) -> String {
    let parts: Vec<&sivtr_core::record::WorkPart> = record
        .parts
        .iter()
        .filter(|part| part.kind().is_input() == input)
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    let mut block = 0usize;
    let mut chunks = Vec::new();
    for part in &parts {
        if part.kind().is_structure() {
            let idx = block;
            block += 1;
            // Raw mode always shows full blocks; read mode folds to the tag
            // unless the block was expanded.
            let show_full = if reading {
                expanded.contains(&idx)
            } else {
                true
            };
            if show_full {
                chunks.push(sivtr_core::record::format_work_part(part));
            } else {
                chunks.push(structure_fold_label(part));
            }
        } else {
            chunks.push(part.text().into_owned());
        }
    }
    chunks.join("\n\n")
}

pub(crate) fn structured_part_text(part: &sivtr_core::record::WorkPart) -> String {
    if part.kind().is_structure() {
        structure_fold_label(part)
    } else {
        part.text().into_owned()
    }
}

fn structure_fold_label(part: &sivtr_core::record::WorkPart) -> String {
    part.kind()
        .as_agent_block_kind()
        .and_then(|kind| kind.open_marker(part.label()))
        .unwrap_or_else(|| "<:structure:>".to_string())
}

pub(crate) fn workspace_content_text(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkAt>,
) -> String {
    workspace_content_io_texts(dialogues, selected_dialogues, highlighted_idx, mode, target)
        .join_displayed()
}

/// Input / Output bodies for the dual content panes (default expand state).
pub(crate) fn workspace_content_io_texts(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkAt>,
) -> ContentIoTexts {
    workspace_content_io_texts_expanded(
        dialogues,
        selected_dialogues,
        highlighted_idx,
        mode,
        target,
        &ExpandedBlocks::default(),
    )
}

/// Input / Output bodies for the dual content panes with per-block expansion.
pub(crate) fn workspace_content_io_texts_expanded(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkAt>,
    expanded: &ExpandedBlocks,
) -> ContentIoTexts {
    if dialogues.is_empty() {
        return ContentIoTexts {
            input: "<empty>".to_string(),
            output: String::new(),
        };
    }

    let selected = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return dialogues
            .get(highlighted_idx)
            .map(|dialogue| dialogue.content_io_texts_expanded(mode, target, expanded))
            .unwrap_or_else(|| ContentIoTexts {
                input: "<empty>".to_string(),
                output: String::new(),
            });
    }

    // Multi-select: join each dialogue's IO half separately.
    let mut input = Vec::new();
    let mut output = Vec::new();
    for dialogue_idx in selected {
        let Some(dialogue) = dialogues.get(dialogue_idx) else {
            continue;
        };
        let io = dialogue.content_io_texts_expanded(mode, None, expanded);
        if !io.input.trim().is_empty() {
            input.push(io.input);
        }
        if !io.output.trim().is_empty() {
            output.push(io.output);
        }
    }
    ContentIoTexts {
        input: if input.is_empty() {
            String::new()
        } else {
            input.join("\n\n")
        },
        output: if output.is_empty() {
            String::new()
        } else {
            output.join("\n\n")
        },
    }
}

/// Index of the structure block whose open marker sits on `line` of the
/// displayed text, if any. Every block contributes exactly one open marker
/// line — collapsed (tag only) or expanded — so the marker ordinal is the
/// block index.
pub(crate) fn structure_block_index(text: &str, line: usize) -> Option<usize> {
    let mut block = 0usize;
    for (idx, text_line) in text.lines().enumerate() {
        if is_structure_open_marker(text_line) {
            if idx == line {
                return Some(block);
            }
            block += 1;
        }
    }
    None
}

fn is_structure_open_marker(line: &str) -> bool {
    line.starts_with("<:") && !line.starts_with("<:/")
}

#[cfg(test)]
mod tests {
    use super::{content_io_from_record_expanded, structure_block_index};
    use crate::tui::content::io::{ContentIoFocus, ExpandedBlocks};
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{WorkPart, WorkPartData, WorkRecord, WorkRef};

    fn tool_part(seq: usize, tool: &str, input: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some(tool.to_string()),
                input: serde_json::json!({ "command": input }),
            },
        }
    }

    fn record(parts: Vec<WorkPart>) -> WorkRecord {
        WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
            kind: sivtr_core::record::WorkRecordKind::ChatTurn,
            source: sivtr_core::record::WorkSource {
                channel: sivtr_core::record::WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: sivtr_core::record::WorkSessionRef {
                id: "session".to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: sivtr_core::record::WorkTime::default(),
            status: None,
            title: "cmd".to_string(),
            parts,
        }
    }

    #[test]
    fn reading_folds_to_tags_and_expanded_blocks_show_full() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let io = content_io_from_record_expanded(&rec, true, &ExpandedBlocks::default());
        let output = &io.output;
        assert!(output.contains("<:tool:Bash call:>"));
        assert!(output.contains("<:tool:Read call:>"));
        assert!(!output.contains("ls"));
        assert!(!output.contains("file"));

        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(ContentIoFocus::Output, 1);
        let io = content_io_from_record_expanded(&rec, true, &expanded);
        let output = &io.output;
        assert!(output.contains("<:tool:Read call:>"));
        assert!(output.contains("<:/tool:Read call:>"));
        assert!(output.contains("file"));
        assert!(!output.contains("ls"));
    }

    #[test]
    fn raw_mode_ignores_expand_state() {
        let rec = record(vec![tool_part(1, "Bash", "ls")]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(ContentIoFocus::Output, 0);
        let io = content_io_from_record_expanded(&rec, false, &expanded);
        assert!(io.output.contains("<:tool:Bash call:>"));
        assert!(io.output.contains("ls"));
        assert!(io.output.contains("<:/tool:Bash call:>"));
    }

    #[test]
    fn block_index_maps_displayed_markers() {
        let text = "<:tool:A call:>\nbody\n\n<:tool:B call:>";
        assert_eq!(structure_block_index(text, 0), Some(0));
        assert_eq!(structure_block_index(text, 3), Some(1));
        assert_eq!(structure_block_index(text, 1), None);
    }
}
