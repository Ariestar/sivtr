//! Part → dual Input/Output display text (structure fold in reading mode).

use sivtr_core::record::{WorkAt, WorkRecord};

use crate::tui::content::io::ContentIoTexts;
use crate::tui::content::view::ContentViewMode;
use crate::tui::workspace::model::WorkspaceDialogue;

pub(crate) fn content_io_from_record(record: &WorkRecord, reading: bool) -> ContentIoTexts {
    use sivtr_core::record::WorkPartIo;
    ContentIoTexts {
        input: io_body_text(record, reading, WorkPartIo::Input),
        output: io_body_text(record, reading, WorkPartIo::Output),
    }
}

fn io_body_text(
    record: &WorkRecord,
    reading: bool,
    io: sivtr_core::record::WorkPartIo,
) -> String {
    let parts: Vec<&sivtr_core::record::WorkPart> =
        record.parts.iter().filter(|part| part.io == io).collect();
    if parts.is_empty() {
        return String::new();
    }
    if reading {
        structured_parts_text(&parts)
    } else {
        raw_parts_text(&parts)
    }
}

/// Reading: dialogue in order; structure folded.
/// Identical markers in this IO half count as `xN` (no adjacency requirement).
fn structured_parts_text(parts: &[&sivtr_core::record::WorkPart]) -> String {
    let structure: Vec<&sivtr_core::record::WorkPart> = parts
        .iter()
        .copied()
        .filter(|part| part.kind.is_structure())
        .collect();
    let fold = (!structure.is_empty()).then(|| collapse_structure_markers(&structure));

    let mut chunks = Vec::new();
    let mut fold_emitted = false;
    for part in parts {
        if part.kind.is_structure() {
            if !fold_emitted {
                if let Some(fold) = fold.as_ref() {
                    chunks.push(fold.clone());
                }
                fold_emitted = true;
            }
            continue;
        }
        chunks.push(part.text.clone());
    }
    chunks.join("\n\n")
}

fn raw_parts_text(parts: &[&sivtr_core::record::WorkPart]) -> String {
    parts
        .iter()
        .map(|part| raw_part_text(part))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn structured_part_text(part: &sivtr_core::record::WorkPart) -> String {
    if part.kind.is_structure() {
        structure_fold_label(part)
    } else {
        part.text.clone()
    }
}

pub(crate) fn raw_part_text(part: &sivtr_core::record::WorkPart) -> String {
    if part.kind.is_structure() {
        return part
            .kind
            .as_agent_block_kind()
            .map(|kind| {
                sivtr_core::ai::format_structured_block(
                    kind,
                    part.label.as_deref(),
                    part.text.trim(),
                )
            })
            .unwrap_or_else(|| part.text.clone());
    }
    part.text.clone()
}

fn structure_fold_label(part: &sivtr_core::record::WorkPart) -> String {
    part.kind
        .as_agent_block_kind()
        .and_then(|kind| kind.open_marker(part.label.as_deref()))
        .unwrap_or_else(|| "<:structure:>".to_string())
}

/// One line of original markers; identical labels become `label xN`.
fn collapse_structure_markers(parts: &[&sivtr_core::record::WorkPart]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for part in parts {
        let label = structure_fold_label(part);
        if let Some((_, count)) = counts.iter_mut().find(|(existing, _)| *existing == label) {
            *count += 1;
        } else {
            counts.push((label, 1));
        }
    }
    counts
        .into_iter()
        .map(|(label, count)| {
            if count == 1 {
                label
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
