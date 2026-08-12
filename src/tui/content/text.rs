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
/// marker-only line at its position, identical markers counting as `xN`.
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

/// One line of original markers; identical labels become `label xN`.
/// Tool results are dropped (a call marker implies its result).
fn collapse_structure_markers(parts: &[&sivtr_core::record::WorkPart]) -> String {
    use sivtr_core::ai::AgentBlockKind;
    let mut counts: Vec<(String, usize)> = Vec::new();
    for part in parts {
        if part.kind().as_agent_block_kind() == Some(AgentBlockKind::ToolOutput) {
            continue;
        }
        let label = structure_fold_label(part);
        if let Some((_, count)) = counts.iter_mut().find(|(existing, _)| *existing == label) {
            *count += 1;
        } else {
            counts.push((label, 1));
        }
    }
    counts
        .iter()
        .map(|(label, count)| {
            if *count == 1 {
                label.clone()
            } else {
                format!("{label} x{count}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
