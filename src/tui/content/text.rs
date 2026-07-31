//! Part → dual Input/Output display text (structure fold in reading mode).

use sivtr_core::record::{WorkAt, WorkRecord};

use crate::tui::content::io::ContentIoTexts;
use crate::tui::content::view::ContentViewMode;
use crate::tui::workspace::model::WorkspaceDialogue;

pub(crate) fn content_io_from_record(record: &WorkRecord, reading: bool) -> ContentIoTexts {
    ContentIoTexts {
        input: io_body_text(record, reading, true),
        output: io_body_text(record, reading, false),
    }
}

fn io_body_text(record: &WorkRecord, reading: bool, input: bool) -> String {
    let parts: Vec<&sivtr_core::record::WorkPart> = record
        .parts
        .iter()
        .filter(|part| part.kind().is_input() == input)
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    if reading {
        structured_parts_text(&parts)
    } else {
        raw_parts_text(&parts)
    }
}

/// Reading: dialogue in order; each adjacent structure run folds into a
/// per-channel summary at its position (tools / mcp / skills / thinking).
fn structured_parts_text(parts: &[&sivtr_core::record::WorkPart]) -> String {
    let mut chunks = Vec::new();
    let mut run: Vec<&sivtr_core::record::WorkPart> = Vec::new();
    let flush = |run: &mut Vec<&sivtr_core::record::WorkPart>, chunks: &mut Vec<String>| {
        if run.is_empty() {
            return;
        }
        let fold = collapse_structure_markers(run);
        run.clear();
        if !fold.is_empty() {
            chunks.push(fold);
        }
    };
    for part in parts {
        if part.kind().is_structure() {
            run.push(part);
            continue;
        }
        flush(&mut run, &mut chunks);
        chunks.push(part.text().into_owned());
    }
    flush(&mut run, &mut chunks);
    chunks.join("\n\n")
}

fn raw_parts_text(parts: &[&sivtr_core::record::WorkPart]) -> String {
    parts
        .iter()
        .map(|part| sivtr_core::record::format_work_part(part))
        .collect::<Vec<_>>()
        .join("\n\n")
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

/// Per-channel summary of a structure run: tools / mcp / skills / thinking.
/// Tool results are dropped (a call implies its result).
fn collapse_structure_markers(parts: &[&sivtr_core::record::WorkPart]) -> String {
    use sivtr_core::record::WorkPartKind;

    let mut tools: Vec<(String, usize)> = Vec::new();
    let mut mcp: Vec<(String, usize)> = Vec::new();
    let mut skills: Vec<(String, usize)> = Vec::new();
    let mut thinking = 0usize;

    for part in parts {
        match part.kind() {
            WorkPartKind::ToolCall => {
                let label = part.label().unwrap_or("tool");
                match label.strip_prefix("mcp__") {
                    Some(rest) => bump(&mut mcp, rest.rsplit("__").next().unwrap_or(rest)),
                    None => bump(&mut tools, label),
                }
            }
            WorkPartKind::Skill => bump(&mut skills, part.label().unwrap_or("skill")),
            WorkPartKind::Thinking => thinking += 1,
            _ => {}
        }
    }

    let mut lines = Vec::new();
    if let Some(line) = channel_line("tools", &tools) {
        lines.push(line);
    }
    if let Some(line) = channel_line("mcp", &mcp) {
        lines.push(line);
    }
    if let Some(line) = channel_line("skills", &skills) {
        lines.push(line);
    }
    if thinking > 0 {
        lines.push(format!("thinking{}", count_suffix(thinking)));
    }
    lines.join("\n")
}

fn bump(counts: &mut Vec<(String, usize)>, name: &str) {
    if let Some((_, count)) = counts.iter_mut().find(|(existing, _)| existing == name) {
        *count += 1;
    } else {
        counts.push((name.to_string(), 1));
    }
}

fn channel_line(label: &str, counts: &[(String, usize)]) -> Option<String> {
    if counts.is_empty() {
        return None;
    }
    let names = counts
        .iter()
        .map(|(name, count)| format!("{name}{}", count_suffix(*count)))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("{label}: {names}"))
}

fn count_suffix(count: usize) -> String {
    if count > 1 {
        format!(" x{count}")
    } else {
        String::new()
    }
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

/// Input / Output bodies for the dual content panes.
pub(crate) fn workspace_content_io_texts(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkAt>,
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
            .map(|dialogue| dialogue.content_io_texts(mode, target))
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
        let io = dialogue.content_io_texts(mode, None);
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
