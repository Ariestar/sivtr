//! Text helpers shared by the browser (line filters, record → copy parts).

use anyhow::{Context, Result};
use sivtr_core::ai::AgentSelection;
use sivtr_core::record::{RecordTextMode, WorkRecord};

use crate::tui::workspace::{TextPair, WorkspaceCopyParts};

pub fn filter_lines_by_spec(text: &TextPair, spec: &str) -> Result<TextPair> {
    let lines: Vec<&str> = text.plain.lines().collect();
    let mut selected = Vec::new();

    for part in spec
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let range = part.split_once(':');

        if let Some((start, end)) = range {
            let start = parse_line_number(start)?;
            let end = parse_line_number(end)?;
            if start == 0 || end == 0 {
                anyhow::bail!("Line ranges are 1-based. Example: `10:20`.");
            }
            let (start, end) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            for idx in start..=end {
                if lines.get(idx - 1).is_some() {
                    selected.push(idx - 1);
                }
            }
        } else {
            let idx = parse_line_number(part)?;
            if idx == 0 {
                anyhow::bail!("Line numbers are 1-based. Example: `1,3,8:12`.");
            }
            if lines.get(idx - 1).is_some() {
                selected.push(idx - 1);
            }
        }
    }

    Ok(select_lines(text, &selected))
}

pub fn select_lines(text: &TextPair, indices: &[usize]) -> TextPair {
    let plain_lines: Vec<&str> = text.plain.lines().collect();
    let ansi_lines: Vec<&str> = text.ansi.lines().collect();
    let mut plain_selected = Vec::new();
    let mut ansi_selected = Vec::new();

    for &idx in indices {
        if let Some(line) = plain_lines.get(idx) {
            plain_selected.push((*line).to_string());
            ansi_selected.push(ansi_lines.get(idx).copied().unwrap_or(line).to_string());
        }
    }

    TextPair {
        plain: plain_selected.join("\n"),
        ansi: ansi_selected.join("\n"),
    }
}

fn parse_line_number(value: &str) -> Result<usize> {
    value.parse::<usize>().with_context(|| {
        format!("Invalid line number `{value}`. Use `N`, `A:B`, or comma-separated lists.")
    })
}

pub fn record_to_copy_parts(
    record: &WorkRecord,
    selection_mode: AgentSelection,
) -> WorkspaceCopyParts {
    match selection_mode {
        AgentSelection::LastBlocks(_) | AgentSelection::All => WorkspaceCopyParts::from_block(
            record_text_to_pair(record.copy_text(RecordTextMode::Combined, false)),
        ),
        _ => WorkspaceCopyParts::from(record.copy_parts(false)),
    }
}

pub fn record_text_to_pair(text: sivtr_core::record::RecordText) -> TextPair {
    let ansi = text.ansi.unwrap_or_else(|| text.plain.clone());
    TextPair {
        plain: text.plain,
        ansi,
    }
}

impl From<sivtr_core::record::WorkRecordCopyParts> for WorkspaceCopyParts {
    fn from(parts: sivtr_core::record::WorkRecordCopyParts) -> Self {
        WorkspaceCopyParts {
            input: record_text_to_pair(parts.input),
            output: record_text_to_pair(parts.output),
            block: record_text_to_pair(parts.block),
            command: record_text_to_pair(parts.command),
        }
    }
}
