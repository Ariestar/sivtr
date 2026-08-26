//! Content blocks: every semantic work atom is a foldable block.
//!
//! A block is the smallest unit the content pane highlights, navigates, and
//! folds: one workpart, or a ToolCall + ToolResult semantic atom (they read as
//! one tool invocation). Consecutive structure blocks fold
//! into one run block that collapses to a single `kind xN` tag; expanding a
//! run reveals its members below the tag, one call per line, each still
//! folded and expandable in turn — two fold levels. Structure blocks default
//! to their `<:…:>` tag; body blocks default to their full text — one fold
//! model, no structure-only special cases.

use sivtr_core::record::{work_atoms, WorkPart, WorkPartData, WorkPartKind, WorkRecord};

use crate::tui::content::io::{ContentIoFocus, ExpandedBlocks};
use crate::tui::content::tool::{part_body_text, tool_display_name, tool_tag_for_part};

/// A foldable content block: the parts it owns, the kind that drives its
/// fold default and collapsed tag (the first part's kind), and — for runs —
/// the member blocks revealed when the run is expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    /// Stable identity within one IO half (DFS pre-order), used by the fold
    /// state and the content cursor; id 0 is the first block of the half.
    pub(crate) id: usize,
    /// Indices into the record's parts, in display order.
    pub(crate) parts: Vec<usize>,
    pub(crate) kind: WorkPartKind,
    /// Member blocks of a run; empty for leaves.
    pub(crate) children: Vec<Block>,
}

/// One rendered segment of a half's display text: a block's collapsed tag
/// or full body. `tight` joins the next segment with a single newline
/// instead of a blank line — members of one run read as a single series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlockText {
    pub(crate) id: usize,
    pub(crate) text: String,
    pub(crate) tight: bool,
    /// Kind of the block's first part; drives the dot-gutter color.
    pub(crate) kind: WorkPartKind,
}

/// Largest block id plus one for the given ids, for mask sizing. Shared by
/// the content layout and the block-selection masks.
pub(crate) fn marked_mask_len(ids: impl IntoIterator<Item = usize>) -> usize {
    ids.into_iter().max().map_or(0, |max| max + 1)
}

impl Block {
    /// A leaf (non-run) block; ids are assigned by `assign_ids` afterwards.
    fn leaf(parts: Vec<usize>, kind: WorkPartKind) -> Self {
        Block {
            id: 0,
            parts,
            kind,
            children: Vec::new(),
        }
    }

    /// Full body of a leaf: every part formatted as in the current content
    /// text, members joining on adjacent lines.
    pub(crate) fn body(&self, record: &WorkRecord) -> String {
        self.parts
            .iter()
            .map(|&idx| part_body_text(&record.parts[idx]))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Collapsed tag: `<:kind xN:>` for a run. Members list every kind in
    /// order with its count, repeats collapsed to `kind xN` and singles as
    /// the bare kind — `<:bash, thinking, read:>` — instead of a `+` mashup.
    pub(crate) fn fold_label(&self, record: &WorkRecord) -> String {
        if !self.children.is_empty() {
            let mut kinds: Vec<(String, usize)> = Vec::new();
            for child in &self.children {
                let name = match &record.parts[child.parts[0]].data {
                    WorkPartData::ToolCall { tool, .. } | WorkPartData::ToolResult { tool, .. } => {
                        tool_display_name(tool.as_deref().unwrap_or_default())
                    }
                    _ => kind_name(child.kind).to_string(),
                };
                match kinds.iter_mut().find(|(kind, _)| *kind == name) {
                    Some((_, count)) => *count += 1,
                    None => kinds.push((name, 1)),
                }
            }
            let label = kinds
                .into_iter()
                .map(|(name, count)| {
                    if count > 1 {
                        format!("{name} x{count}")
                    } else {
                        name
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("<:{label}:>")
        } else {
            fold_label_for_part(&record.parts[self.parts[0]])
        }
    }
}

/// Short display name for a kind: body tags use it (`<:user:>`), structure
/// runs use it for the count label (`<:tool x2:>`).
fn kind_name(kind: WorkPartKind) -> &'static str {
    match kind {
        WorkPartKind::Prompt => "prompt",
        WorkPartKind::Command => "command",
        WorkPartKind::User => "user",
        WorkPartKind::Assistant => "assistant",
        WorkPartKind::ToolCall | WorkPartKind::ToolResult => "tool",
        WorkPartKind::Skill => "skill",
        WorkPartKind::Thinking => "thinking",
        WorkPartKind::Output => "output",
        WorkPartKind::Error => "error",
    }
}

/// Partition one IO half's semantic atoms into display blocks. Tool call/result
/// pairing comes from the core atom model; consecutive structure atoms fold into
/// one run for compact reading. Blocks get stable DFS pre-order ids so fold
/// state and cursor survive folds.
pub(crate) fn half_blocks(record: &WorkRecord, input: bool) -> Vec<Block> {
    let index_by_seq: std::collections::HashMap<usize, usize> = record
        .parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.kind().is_input() == input)
        .map(|(index, part)| (part.seq, index))
        .collect();
    let units: Vec<Block> = work_atoms(record, input)
        .into_iter()
        .map(|atom| {
            Block::leaf(
                atom.part_seqs
                    .into_iter()
                    .filter_map(|seq| index_by_seq.get(&seq).copied())
                    .collect(),
                atom.kind,
            )
        })
        .filter(|block| !block.parts.is_empty())
        .collect();

    // Consecutive structure units fold into one run, whatever their kinds.
    let mut blocks: Vec<Block> = Vec::new();
    for unit in units {
        let merges =
            unit.kind.is_structure() && blocks.last().is_some_and(|last| last.kind.is_structure());
        if merges {
            let last = blocks.last_mut().expect("run block exists");
            if last.children.is_empty() {
                // Promote the leaf into a run holding itself as first member.
                last.children
                    .push(Block::leaf(last.parts.clone(), last.kind));
            }
            last.parts.extend(unit.parts.iter().copied());
            last.children.push(unit);
        } else {
            blocks.push(unit);
        }
    }

    // Stable DFS pre-order ids for the fold state and the content cursor.
    let mut next = 0usize;
    for block in &mut blocks {
        assign_ids(block, &mut next);
    }
    blocks
}

fn assign_ids(block: &mut Block, next: &mut usize) {
    block.id = *next;
    *next += 1;
    for child in &mut block.children {
        assign_ids(child, next);
    }
}

/// Collapsed tag for one part: the per-tool tag (`<:read: path:>`) for known
/// tools, otherwise the structure marker with the tool description when
/// present, or a plain `<:kind:>` tag for body parts.
pub(crate) fn fold_label_for_part(part: &WorkPart) -> String {
    if let Some(tag) = tool_tag_for_part(part) {
        return tag;
    }
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
        format!("<:{}:>", kind_name(part.kind()))
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
    // Normalize internal whitespace so a multi-line description still folds
    // to a single tag line (block layout assumes one line per tag).
    let description: String = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return None;
    }
    const MAX: usize = 40;
    Some(crate::tui::content::truncate_chars(&description, MAX))
}

/// Render one IO half's blocks to their display segments, in display order:
/// a block's full body when shown, its collapsed tag otherwise. Runs always
/// show the aggregate tag; expanding a run reveals its members below it,
/// each still folded, joined on adjacent lines.
pub(crate) fn render_half(
    record: &WorkRecord,
    input: bool,
    reading: bool,
    expanded: &ExpandedBlocks,
) -> Vec<BlockText> {
    let focus = if input {
        ContentIoFocus::Input
    } else {
        ContentIoFocus::Output
    };
    let mut out = Vec::new();
    for block in half_blocks(record, input) {
        out.extend(render_block(record, &block, reading, focus, expanded));
    }
    out
}

fn render_block(
    record: &WorkRecord,
    block: &Block,
    reading: bool,
    focus: ContentIoFocus,
    expanded: &ExpandedBlocks,
) -> Vec<BlockText> {
    let mut segs = Vec::new();
    if !reading {
        // Raw mode: every block shows its full body; runs expand flat.
        if block.children.is_empty() {
            segs.push(BlockText {
                id: block.id,
                text: block.body(record),
                tight: false,
                kind: block.kind,
            });
        } else {
            for child in &block.children {
                segs.extend(render_block(record, child, reading, focus, expanded));
            }
        }
    } else if block.children.is_empty() {
        // Leaf: body or collapsed tag by the block's fold default.
        let shown = expanded.expanded(focus, block.id, block.kind.is_structure());
        segs.push(BlockText {
            id: block.id,
            text: if shown {
                block.body(record)
            } else {
                block.fold_label(record)
            },
            tight: false,
            kind: block.kind,
        });
    } else {
        // Run: the aggregate tag stays as the group header; expanding the
        // run reveals its members below it, each still folded.
        let shown = expanded.expanded(focus, block.id, true);
        segs.push(BlockText {
            id: block.id,
            text: block.fold_label(record),
            tight: false, // rewritten below: all but the last segment join tight
            kind: block.kind,
        });
        if shown {
            for child in &block.children {
                segs.extend(render_block(record, child, reading, focus, expanded));
            }
        }
    }
    // Members of one run join on adjacent lines; the last segment closes
    // the group with the usual blank line.
    if !block.children.is_empty() {
        let last = segs.len().saturating_sub(1);
        for (i, seg) in segs.iter_mut().enumerate() {
            seg.tight = i < last;
        }
    }
    segs
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
                start_line: None,
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

    fn thinking_part(seq: usize, content: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::Thinking {
                content: content.to_string(),
            },
        }
    }

    fn assistant_part(seq: usize, content: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::Assistant {
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
        // The matching pair and the following Read call fold into one tool run.
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, WorkPartKind::ToolCall);
        assert_eq!(blocks[0].parts, vec![0, 1, 2]);
        assert_eq!(blocks[0].children.len(), 2);
    }

    #[test]
    fn parallel_calls_pair_results_by_call_id_across_interleaving() {
        // The ACP stream interleaves parallel calls: call 0, call 1, then
        // result 0, result 1. Adjacency pairing would leave all four as
        // separate blocks; call-id pairing folds each call with its result.
        let rec = record(vec![
            WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("c0".to_string()),
                    tool: Some("read_file".to_string()),
                    input: serde_json::json!({ "target_file": "a.rs" }),
                },
            },
            WorkPart {
                seq: 2,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("c1".to_string()),
                    tool: Some("read_file".to_string()),
                    input: serde_json::json!({ "target_file": "b.rs" }),
                },
            },
            tool_result_part(3, "read_file", Some("c0"), "a body"),
            tool_result_part(4, "read_file", Some("c1"), "b body"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);
        // Each member owns its call and the matching result, in call order.
        assert_eq!(blocks[0].children[0].parts, vec![0, 2]);
        assert_eq!(blocks[0].children[1].parts, vec![1, 3]);
    }

    #[test]
    fn orphan_results_stay_separate() {
        // A result whose call id never opened, and an id-less result with no
        // preceding id-less call, both stand alone.
        let rec = record(vec![
            tool_result_part(1, "Bash", Some("gone"), "no matching call"),
            tool_result_part(2, "Bash", None, "no call id"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);
        assert_eq!(blocks[0].children[0].parts, vec![0]);
        assert_eq!(blocks[0].children[1].parts, vec![1]);
    }

    #[test]
    fn idless_result_pairs_with_nearest_idless_call() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_result_part(2, "Bash", None, "ok"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, WorkPartKind::ToolCall);
        assert_eq!(blocks[0].parts, vec![0, 1]);
    }

    #[test]
    fn consecutive_same_kind_structure_parts_fold_to_one_run() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);
        assert_eq!(blocks[0].fold_label(&rec), "<:bash, read:>");
    }

    #[test]
    fn mixed_structure_kinds_fold_into_one_run() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            thinking_part(2, "reasoning"),
            tool_part(3, "Read", "file"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 3);
        assert_eq!(blocks[0].fold_label(&rec), "<:bash, thinking, read:>");
        // A body part after the series starts a new block in the same
        // (output) half: the run keeps its members, the body joins after.
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            thinking_part(2, "reasoning"),
            assistant_part(3, "answer"),
        ]);
        assert_eq!(half_blocks(&rec, false).len(), 2);
        assert!(half_blocks(&rec, true).is_empty());
    }

    #[test]
    fn ids_follow_dfs_preorder() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            thinking_part(2, "reasoning"),
            tool_part(3, "Read", "file"),
        ]);
        let blocks = half_blocks(&rec, false);
        // run id 0, members 1..=3.
        assert_eq!(blocks[0].id, 0);
        let ids: Vec<usize> = blocks[0].children.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn body_parts_default_to_full_text_and_structure_to_tag() {
        let rec = record(vec![user_part(1, "question"), tool_part(2, "Bash", "ls")]);
        let expanded = ExpandedBlocks::default();
        let input = render_half(&rec, true, true, &expanded);
        let output = render_half(&rec, false, true, &expanded);
        // Body block shows its text; the tool block folds to its tag.
        assert_eq!(texts(input), vec!["question"]);
        assert_eq!(texts(output), vec!["<:bash: ls:>"]);
    }

    #[test]
    fn body_block_folds_to_kind_tag_when_flipped() {
        let rec = record(vec![user_part(1, "question")]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(ContentIoFocus::Input, 0);
        assert_eq!(
            texts(render_half(&rec, true, true, &expanded)),
            vec!["<:user:>"]
        );
    }

    #[test]
    fn raw_mode_shows_every_block_full() {
        let rec = record(vec![user_part(1, "question"), tool_part(2, "Bash", "ls")]);
        let expanded = ExpandedBlocks::default();
        let input = render_half(&rec, true, false, &expanded);
        let output = render_half(&rec, false, false, &expanded);
        assert_eq!(texts(input), vec!["question"]);
        assert_eq!(output[0].text, "$ ls");
    }

    #[test]
    fn body_block_body_uses_plain_text() {
        let rec = record(vec![user_part(1, "hello\nworld")]);
        assert_eq!(half_blocks(&rec, true)[0].body(&rec), "hello\nworld");
    }

    #[test]
    fn expanding_a_run_reveals_members_as_folded_lines() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let mut expanded = ExpandedBlocks::default();
        // Folded: the run collapses to its tag.
        let folded = render_half(&rec, false, true, &expanded);
        assert_eq!(texts(folded), vec!["<:bash, read:>"]);
        // Expanded: the tag stays as the group header, members below it as
        // folded lines, joined without blank lines (tight).
        expanded.toggle(ContentIoFocus::Output, 0);
        let shown = render_half(&rec, false, true, &expanded);
        assert_eq!(
            texts(shown.clone()),
            vec!["<:bash, read:>", "<:bash: ls:>", "<:tool:Read call:>"]
        );
        // Members join on adjacent lines; only the last segment closes the
        // group with a blank line.
        assert!(shown[0].tight && shown[1].tight && !shown[2].tight);
    }

    #[test]
    fn run_member_expands_to_its_own_body() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(ContentIoFocus::Output, 0); // run open
        expanded.toggle(ContentIoFocus::Output, 1); // first member open
        let shown = render_half(&rec, false, true, &expanded);
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[0].text, "<:bash, read:>");
        assert_eq!(shown[1].text, "$ ls");
        assert_eq!(shown[2].text, "<:tool:Read call:>");
    }

    /// Segment texts for compact assertions.
    fn texts(segs: Vec<BlockText>) -> Vec<String> {
        segs.into_iter().map(|seg| seg.text).collect()
    }
}
