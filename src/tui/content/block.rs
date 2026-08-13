//! Content blocks: every workpart is a foldable block.
//!
//! A block is the smallest unit the content pane highlights, navigates, and
//! folds: one workpart, or a ToolCall + ToolResult pair with the same call id
//! (they read as one tool invocation). Structure blocks default to their
//! `<:…:>` tag; body blocks default to their full text — one fold model, no
//! structure-only special cases.

use sivtr_core::record::{WorkPart, WorkPartData, WorkPartKind, WorkRecord};

use crate::tui::content::io::{ContentIoFocus, ExpandedBlocks};

/// A foldable content block: the parts it owns and the kind that drives its
/// fold default and collapsed tag (the first part's kind).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    /// Indices into the record's parts, in display order.
    pub(crate) parts: Vec<usize>,
    pub(crate) kind: WorkPartKind,
}

impl Block {
    pub(crate) fn is_structure(&self) -> bool {
        self.kind.is_structure()
    }

    /// Full body: every part formatted as in the current content text.
    pub(crate) fn body(&self, record: &WorkRecord) -> String {
        self.parts
            .iter()
            .map(|&idx| {
                let part = &record.parts[idx];
                if part.kind().is_structure() {
                    sivtr_core::record::format_work_part(part)
                } else {
                    part.text().into_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Collapsed tag: the structure marker (with tool description when
    /// present) for structure blocks, `<:kind:>` for body blocks.
    pub(crate) fn fold_label(&self, record: &WorkRecord) -> String {
        fold_label_for_part(&record.parts[self.parts[0]])
    }
}

/// Partition one IO half's parts into blocks: a ToolCall followed by a
/// ToolResult with the same call id folds into one block; anything else is
/// one part per block.
pub(crate) fn half_blocks(record: &WorkRecord, input: bool) -> Vec<Block> {
    let parts: Vec<usize> = record
        .parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.kind().is_input() == input)
        .map(|(idx, _)| idx)
        .collect();

    let mut blocks = Vec::new();
    let mut idx = 0usize;
    while idx < parts.len() {
        let first = parts[idx];
        let kind = record.parts[first].kind();
        let group_end = if matches!(kind, WorkPartKind::ToolCall) {
            parts
                .get(idx + 1)
                .filter(|&&next| {
                    matches!(record.parts[next].kind(), WorkPartKind::ToolResult)
                        && part_call_id(&record.parts[next]) == part_call_id(&record.parts[first])
                })
                .map_or(idx, |_| idx + 1)
        } else {
            idx
        };
        blocks.push(Block {
            parts: parts[idx..=group_end].to_vec(),
            kind,
        });
        idx = group_end + 1;
    }
    blocks
}

/// Collapsed tag for one part: the structure marker with the tool
/// description when present, or a plain `<:kind:>` tag for body parts.
pub(crate) fn fold_label_for_part(part: &WorkPart) -> String {
    if part.kind().is_structure() {
        let marker = part
            .kind()
            .as_agent_block_kind()
            .and_then(|kind| kind.open_marker(part.label()))
            .unwrap_or_else(|| "<:structure:>".to_string());
        match tool_description(part) {
            Some(description) => match marker.strip_suffix(":>") {
                Some(base) => format!("{base}: {description}:>"),
                None => marker,
            },
            None => marker,
        }
    } else {
        format!("<:{}:>", fold_kind_name(part.kind()))
    }
}

fn fold_kind_name(kind: WorkPartKind) -> &'static str {
    match kind {
        WorkPartKind::Prompt => "prompt",
        WorkPartKind::Command => "command",
        WorkPartKind::User => "user",
        WorkPartKind::Assistant => "assistant",
        WorkPartKind::Output => "output",
        WorkPartKind::Error => "error",
        // Structure kinds keep their own markers above; unreachable here.
        WorkPartKind::ToolCall
        | WorkPartKind::ToolResult
        | WorkPartKind::Skill
        | WorkPartKind::Thinking => "body",
    }
}

/// Human description from a tool call's input (`description` field), if any,
/// truncated to fit the tag line.
fn tool_description(part: &WorkPart) -> Option<String> {
    let WorkPartData::ToolCall { input, .. } = &part.data else {
        return None;
    };
    let description = input
        .get("description")
        .and_then(serde_json::Value::as_str)?;
    let description = description.trim();
    if description.is_empty() {
        return None;
    }
    const MAX: usize = 40;
    let mut truncated: String = description.chars().take(MAX).collect();
    if description.chars().count() > MAX {
        truncated.push('…');
    }
    Some(truncated)
}

fn part_call_id(part: &WorkPart) -> Option<&str> {
    match &part.data {
        WorkPartData::ToolCall { call_id, .. } | WorkPartData::ToolResult { call_id, .. } => {
            call_id.as_deref()
        }
        _ => None,
    }
}

/// Render one IO half's blocks to their display segments: the full body when
/// shown, the collapsed tag otherwise. The pane text is the segments joined
/// with a blank line between blocks.
pub(crate) fn render_half(
    record: &WorkRecord,
    input: bool,
    reading: bool,
    expanded: &ExpandedBlocks,
) -> Vec<String> {
    let focus = if input {
        ContentIoFocus::Input
    } else {
        ContentIoFocus::Output
    };
    half_blocks(record, input)
        .into_iter()
        .enumerate()
        .map(|(idx, block)| {
            let shown = !reading || expanded.expanded(focus, idx, block.is_structure());
            if shown {
                block.body(record)
            } else {
                block.fold_label(record)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::content::io::ExpandedBlocks;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource, WorkTime,
        RECORD_SCHEMA_VERSION,
    };

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

    fn tool_result_part(seq: usize, tool: &str, call_id: Option<&str>, output: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: call_id.map(str::to_string),
                tool: Some(tool.to_string()),
                output: serde_json::json!({ "stdout": output }),
            },
        }
    }

    fn user_part(seq: usize, content: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::User {
                content: content.to_string(),
            },
        }
    }

    fn record(parts: Vec<WorkPart>) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: "session".to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "cmd".to_string(),
            parts,
        }
    }

    #[test]
    fn tool_call_with_matching_result_folds_into_one_block() {
        let rec = record(vec![
            WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("c1".to_string()),
                    tool: Some("Bash".to_string()),
                    input: serde_json::json!({ "command": "ls" }),
                },
            },
            tool_result_part(2, "Bash", Some("c1"), "ok"),
            tool_part(3, "Read", "file"),
            user_part(4, "question"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, WorkPartKind::ToolCall);
        assert_eq!(blocks[0].parts, vec![0, 1]);
        assert_eq!(blocks[1].parts, vec![2]);
    }

    #[test]
    fn body_parts_default_to_full_text_and_structure_to_tag() {
        let rec = record(vec![user_part(1, "question"), tool_part(2, "Bash", "ls")]);
        let expanded = ExpandedBlocks::default();
        let input = render_half(&rec, true, true, &expanded);
        let output = render_half(&rec, false, true, &expanded);
        // Body block shows its text; the tool block folds to its tag.
        assert_eq!(input, vec!["question"]);
        assert_eq!(output, vec!["<:tool:Bash call:>"]);
    }

    #[test]
    fn body_block_folds_to_kind_tag_when_flipped() {
        let rec = record(vec![user_part(1, "question")]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(ContentIoFocus::Input, 0);
        assert_eq!(render_half(&rec, true, true, &expanded), vec!["<:user:>"]);
    }

    #[test]
    fn raw_mode_shows_every_block_full() {
        let rec = record(vec![user_part(1, "question"), tool_part(2, "Bash", "ls")]);
        let expanded = ExpandedBlocks::default();
        let input = render_half(&rec, true, false, &expanded);
        let output = render_half(&rec, false, false, &expanded);
        assert_eq!(input, vec!["question"]);
        assert!(output[0].contains("<:tool:Bash call:>"));
        assert!(output[0].contains("ls"));
        assert!(output[0].contains("<:/tool:Bash call:>"));
    }

    #[test]
    fn body_block_body_uses_plain_text() {
        let rec = record(vec![user_part(1, "hello\nworld")]);
        assert_eq!(half_blocks(&rec, true)[0].body(&rec), "hello\nworld");
    }
}
