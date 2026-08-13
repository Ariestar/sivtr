//! Part → dual Input/Output display text (per-block fold in reading mode).

use sivtr_core::record::{WorkAt, WorkRecord};

use crate::tui::content::block::render_half;
use crate::tui::content::io::{ContentIoTexts, ExpandedBlocks};
use crate::tui::content::view::ContentViewMode;
use crate::tui::workspace::model::WorkspaceDialogue;

/// Read mode folds every block to its `<:…:>` tag (structure blocks by
/// default, body blocks only when flipped); blocks listed in `expanded`
/// show their full body instead. Raw mode always shows full blocks (the
/// expand state only affects reading).
pub(crate) fn content_io_from_record(
    record: &WorkRecord,
    reading: bool,
    expanded: &ExpandedBlocks,
) -> ContentIoTexts {
    ContentIoTexts::new(
        render_half(record, true, reading, expanded),
        render_half(record, false, reading, expanded),
    )
}

pub(crate) fn workspace_content_text(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkAt>,
) -> String {
    workspace_content_io_texts(
        dialogues,
        selected_dialogues,
        highlighted_idx,
        mode,
        target,
        &ExpandedBlocks::default(),
    )
    .join_displayed()
}

/// Input / Output bodies for the dual content panes with per-block fold
/// state. Every workpart is a block; the segments stay attached to the pane
/// text so the content view can map displayed lines back to their block.
pub(crate) fn workspace_content_io_texts(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkAt>,
    expanded: &ExpandedBlocks,
) -> ContentIoTexts {
    if dialogues.is_empty() {
        return ContentIoTexts::new(Vec::new(), Vec::new());
    }

    let selected = selected_dialogues
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return dialogues
            .get(highlighted_idx)
            .map(|dialogue| dialogue.content_io_texts(mode, target, expanded))
            .unwrap_or_else(|| ContentIoTexts::new(Vec::new(), Vec::new()));
    }

    // Multi-select: join each dialogue's IO half separately, block by block.
    let mut input = Vec::new();
    let mut output = Vec::new();
    for dialogue_idx in selected {
        let Some(dialogue) = dialogues.get(dialogue_idx) else {
            continue;
        };
        let io = dialogue.content_io_texts(mode, None, expanded);
        input.extend(io.input_blocks);
        output.extend(io.output_blocks);
    }
    ContentIoTexts::new(input, output)
}

/// A line that opens or closes a structure block (`<:tool:…:>`,
/// `<:skill:…:>`, `<:thinking:>`, or the generic `<:structure:>` fallback).
/// Validates the full marker shape, not just the `<:` prefix, so plain text
/// that happens to start with `<:` is not treated as a marker.
pub(crate) fn is_structure_marker(line: &str) -> bool {
    let Some(inner) = line
        .strip_prefix("<:")
        .and_then(|rest| rest.strip_suffix(":>"))
    else {
        return false;
    };
    let kind = inner
        .strip_prefix('/')
        .unwrap_or(inner)
        .split(':')
        .next()
        .unwrap_or_default();
    matches!(kind, "tool" | "skill" | "thinking" | "structure")
}

#[cfg(test)]
mod tests {
    use super::content_io_from_record;
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

    fn tool_result_part(seq: usize, tool: &str, output: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: None,
                tool: Some(tool.to_string()),
                output: serde_json::json!({ "stdout": output }),
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
    fn reading_folds_runs_to_tags_and_expanded_runs_show_full() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        let output = &io.output;
        // The two consecutive tool calls fold into one run tag.
        assert!(output.contains("<:tool x2:>"));
        assert!(!output.contains("ls"));
        assert!(!output.contains("file"));

        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(ContentIoFocus::Output, 0);
        let io = content_io_from_record(&rec, true, &expanded);
        let output = &io.output;
        assert!(output.contains("<:tool:Bash call:>"));
        assert!(output.contains("<:/tool:Bash call:>"));
        assert!(output.contains("<:tool:Read call:>"));
        assert!(output.contains("ls"));
        assert!(output.contains("file"));
    }

    #[test]
    fn raw_mode_ignores_expand_state() {
        let rec = record(vec![tool_part(1, "Bash", "ls")]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(ContentIoFocus::Output, 0);
        let io = content_io_from_record(&rec, false, &expanded);
        assert!(io.output.contains("<:tool:Bash call:>"));
        assert!(io.output.contains("ls"));
        assert!(io.output.contains("<:/tool:Bash call:>"));
    }

    #[test]
    fn tool_call_and_result_fold_into_one_run() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_result_part(2, "Bash", "ok"),
            tool_part(3, "Read", "file"),
        ]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        let output = &io.output;
        // Collapsed: the call+result pair and the Read call fold into one
        // run tag; the result payload stays hidden.
        assert!(output.contains("<:tool x2:>"));
        assert!(!output.contains("<:tool:Bash result:>"));
        assert!(!output.contains("ok"));
        assert!(!output.contains("file"));

        // Expanding the run reveals the result inside the same block, with
        // the second call on the next line (no blank line between).
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(ContentIoFocus::Output, 0);
        let io = content_io_from_record(&rec, true, &expanded);
        let output = &io.output;
        assert!(output.contains("<:tool:Bash result:>"));
        assert!(output.contains("ok"));
        assert!(output.contains("<:/tool:Bash result:>"));
        assert!(output.contains("<:tool:Read call:>"));
        assert!(output.contains("file"));
    }

    #[test]
    fn fold_label_shows_tool_description_when_present() {
        let rec = record(vec![WorkPart {
            seq: 1,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: serde_json::json!({
                    "command": "git diff",
                    "description": "Review working tree",
                }),
            },
        }]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        assert!(io
            .output
            .contains("<:tool:Bash call: Review working tree:>"));

        // Long descriptions are truncated to fit the tag line.
        let rec = record(vec![WorkPart {
            seq: 1,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: serde_json::json!({
                    "command": "git diff",
                    "description": "Review working tree and diff size for this change, then summarize",
                }),
            },
        }]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        let tag = io.output.find("<:tool:Bash call: ").expect("tag");
        let line = &io.output[tag..];
        assert!(line.starts_with("<:tool:Bash call: Review working tree and diff size for th…:>"));
        assert!(!line.starts_with("<:tool:Bash call: Review working tree and diff size for this"));

        // No description: the plain tag stays.
        let rec = record(vec![tool_part(1, "Bash", "ls")]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        assert!(io.output.contains("<:tool:Bash call:>"));
    }

    #[test]
    fn fold_label_normalizes_multiline_descriptions_to_one_tag_line() {
        let rec = record(vec![WorkPart {
            seq: 1,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: serde_json::json!({
                    "command": "git diff",
                    "description": "line one\nline two",
                }),
            },
        }]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        // The tag stays a single line: internal whitespace collapses.
        let mut lines = io.output.lines();
        assert_eq!(lines.next(), Some("<:tool:Bash call: line one line two:>"));
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn id_less_call_and_result_group_only_for_the_same_tool() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_result_part(2, "Read", "ok"),
        ]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        // Different tools without call ids are separate blocks, so the
        // result keeps its own tag instead of hiding inside the Bash block.
        let output = &io.output;
        assert!(output.contains("<:tool:Bash call:>"));
        assert!(output.contains("<:tool:Read result:>"));
    }

    #[test]
    fn distinct_call_ids_never_group() {
        let rec = record(vec![
            WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("a".to_string()),
                    tool: Some("Bash".to_string()),
                    input: serde_json::json!({ "command": "ls" }),
                },
            },
            WorkPart {
                seq: 2,
                occurred_at: None,
                data: WorkPartData::ToolResult {
                    call_id: Some("b".to_string()),
                    tool: Some("Bash".to_string()),
                    output: serde_json::json!({ "stdout": "ok" }),
                },
            },
        ]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        let output = &io.output;
        assert!(output.contains("<:tool:Bash call:>"));
        assert!(output.contains("<:tool:Bash result:>"));
    }
}
