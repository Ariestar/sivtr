//! Compare two dialogues from the current terminal session.
//!
//! Terminal-only product surface: load goes through workset, but the address is
//! always the current terminal session (not agent sources).

use anyhow::{Context, Result};
use crossterm::terminal;
use similar::{ChangeTag, TextDiff};
use sivtr_core::record::{RecordTextMode, WorkRecord};

use crate::commands::memory::copy::load::{current_terminal_source, load_dialogues};
use crate::commands::select::{parse_selector, resolve_selector};

const MIN_SIDE_BY_SIDE_WIDTH: usize = 20;
const SIDE_BY_SIDE_OVERHEAD: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffTextMode {
    Output,
    Block,
    Input,
    Command,
}

impl DiffTextMode {
    fn record_text_mode(self) -> RecordTextMode {
        match self {
            Self::Output => RecordTextMode::Output,
            Self::Block => RecordTextMode::Combined,
            Self::Input => RecordTextMode::Input,
            Self::Command => RecordTextMode::Command,
        }
    }

    fn include_prompt(self) -> bool {
        matches!(self, Self::Block | Self::Input)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DiffRequest<'a> {
    pub left_selector: &'a str,
    pub right_selector: &'a str,
    pub mode: DiffTextMode,
    pub side_by_side: bool,
}

pub fn execute(request: DiffRequest<'_>) -> Result<()> {
    let left_selector = request.left_selector.trim();
    let right_selector = request.right_selector.trim();
    if left_selector.is_empty() || right_selector.is_empty() {
        anyhow::bail!("Selectors must not be empty.");
    }

    let source = current_terminal_source()?
        .context("No session log found. Run a command first, then try `sivtr diff` again.")?;
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let records = load_dialogues(&source, Some(&cwd))?;
    if records.is_empty() {
        anyhow::bail!("No command blocks recorded in the current session.");
    }

    let left_idx = resolve_single_selector(left_selector, records.len())?;
    let right_idx = resolve_single_selector(right_selector, records.len())?;

    let left_text = record_text(&records[left_idx], request.mode);
    let right_text = record_text(&records[right_idx], request.mode);

    if left_text == right_text {
        println!("sivtr: no differences");
        return Ok(());
    }

    if request.side_by_side {
        let width = detect_output_width();
        let rendered = render_side_by_side(&left_text, &right_text, width);
        print!("{rendered}");
    } else {
        let rendered = render_unified_diff(left_selector, right_selector, &left_text, &right_text);
        print!("{rendered}");
    }

    Ok(())
}

fn record_text(record: &WorkRecord, mode: DiffTextMode) -> String {
    record
        .copy_text(mode.record_text_mode(), mode.include_prompt())
        .plain
}

fn resolve_single_selector(selector: &str, total: usize) -> Result<usize> {
    let parsed = parse_selector(selector)?;
    let indices = resolve_selector(&parsed, total)?;
    match indices.as_slice() {
        [idx] => Ok(*idx),
        [] => anyhow::bail!("Selector `{selector}` did not match any command block."),
        _ => anyhow::bail!(
            "Selector `{selector}` resolved to multiple command blocks. `sivtr diff` requires a single selector."
        ),
    }
}

fn render_unified_diff(
    left_selector: &str,
    right_selector: &str,
    left: &str,
    right: &str,
) -> String {
    TextDiff::from_lines(left, right)
        .unified_diff()
        .header(left_selector, right_selector)
        .to_string()
}

fn render_side_by_side(left: &str, right: &str, total_width: usize) -> String {
    let diff = TextDiff::from_lines(left, right);
    let rows = build_side_by_side_rows(&diff);
    let (left_width, right_width) = compute_columns(total_width);

    let mut out = String::new();
    for row in rows {
        let left_cell = truncate_and_pad(&row.left, left_width);
        let right_cell = truncate_to_width(&row.right, right_width);
        out.push_str(&format!(
            "{} {} | {} {}\n",
            row.left_mark, left_cell, row.right_mark, right_cell
        ));
    }

    out
}

fn detect_output_width() -> usize {
    terminal::size()
        .map(|(w, _)| usize::from(w))
        .unwrap_or(120)
        .max(MIN_SIDE_BY_SIDE_WIDTH)
}

fn compute_columns(total_width: usize) -> (usize, usize) {
    let width = total_width.max(MIN_SIDE_BY_SIDE_WIDTH);
    let content = width.saturating_sub(SIDE_BY_SIDE_OVERHEAD).max(2);
    let left = content / 2;
    let right = content - left;
    (left.max(1), right.max(1))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SideBySideRow {
    left_mark: char,
    right_mark: char,
    left: String,
    right: String,
}

fn build_side_by_side_rows(diff: &TextDiff<'_, '_, str>) -> Vec<SideBySideRow> {
    let mut rows = Vec::new();
    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let value = change.value().trim_end_matches('\n').to_string();
            match change.tag() {
                ChangeTag::Equal => rows.push(SideBySideRow {
                    left_mark: ' ',
                    right_mark: ' ',
                    left: value.clone(),
                    right: value,
                }),
                ChangeTag::Delete => rows.push(SideBySideRow {
                    left_mark: '-',
                    right_mark: ' ',
                    left: value,
                    right: String::new(),
                }),
                ChangeTag::Insert => rows.push(SideBySideRow {
                    left_mark: ' ',
                    right_mark: '+',
                    left: String::new(),
                    right: value,
                }),
            }
        }
    }
    // Merge adjacent delete+insert into replace marks when possible.
    let mut merged = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if i + 1 < rows.len()
            && rows[i].left_mark == '-'
            && rows[i].right.is_empty()
            && rows[i + 1].right_mark == '+'
            && rows[i + 1].left.is_empty()
        {
            merged.push(SideBySideRow {
                left_mark: '~',
                right_mark: '~',
                left: rows[i].left.clone(),
                right: rows[i + 1].right.clone(),
            });
            i += 2;
        } else {
            merged.push(rows[i].clone());
            i += 1;
        }
    }
    merged
}

fn truncate_and_pad(text: &str, width: usize) -> String {
    let truncated = truncate_to_width(text, width);
    format!("{truncated:<width$}")
}

fn truncate_to_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = unicode_width(ch);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

fn unicode_width(ch: char) -> usize {
    // Keep side-by-side simple; treat non-ascii as width 1 (matches prior behavior closely enough).
    let _ = ch;
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_single_selector() {
        assert_eq!(resolve_single_selector("1", 5).unwrap(), 4);
        assert_eq!(resolve_single_selector("3", 5).unwrap(), 2);
    }

    #[test]
    fn rejects_multi_block_selector_for_diff() {
        assert!(resolve_single_selector("2..4", 10).is_err());
    }

    #[test]
    fn computes_non_zero_columns_for_small_width() {
        let (l, r) = compute_columns(10);
        assert!(l >= 1 && r >= 1);
    }

    #[test]
    fn renders_unified_diff_headers_from_selectors() {
        let rendered = render_unified_diff("1", "2", "a\n", "b\n");
        assert!(rendered.contains("--- 1"));
        assert!(rendered.contains("+++ 2"));
    }

    #[test]
    fn builds_side_by_side_rows_with_replace_marks() {
        let diff = TextDiff::from_lines("old\n", "new\n");
        let rows = build_side_by_side_rows(&diff);
        assert!(rows
            .iter()
            .any(|row| row.left_mark == '~' || row.left_mark == '-'));
    }
}
