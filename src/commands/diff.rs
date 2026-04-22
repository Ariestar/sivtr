use anyhow::{Context, Result};
use crossterm::terminal;
use similar::{ChangeTag, TextDiff};
use sivtr_core::capture::scrollback;
use sivtr_core::session;

use crate::command_blocks::{CommandBlockTextMode, ParsedCommandBlock};
use crate::commands::command_block_selector::{parse_selector, resolve_selector};

const MIN_SIDE_BY_SIDE_WIDTH: usize = 20;
const SIDE_BY_SIDE_OVERHEAD: usize = 7;

#[derive(Clone, Copy, Debug)]
pub struct DiffRequest<'a> {
    pub left_selector: &'a str,
    pub right_selector: &'a str,
    pub mode: CommandBlockTextMode,
    pub side_by_side: bool,
}

pub fn execute(request: DiffRequest<'_>) -> Result<()> {
    let left_selector = request.left_selector.trim();
    let right_selector = request.right_selector.trim();
    if left_selector.is_empty() || right_selector.is_empty() {
        anyhow::bail!("Selectors must not be empty.");
    }

    let log_path = scrollback::session_log_path();
    if !log_path.exists() {
        anyhow::bail!("No session log found. Run a command first, then try `sivtr diff` again.");
    }

    let entries = session::load_entries(&log_path).context("Failed to read session log")?;
    if entries.is_empty() {
        anyhow::bail!("No command blocks recorded in the current session.");
    }

    let blocks: Vec<ParsedCommandBlock> = entries
        .iter()
        .map(ParsedCommandBlock::from_session_entry)
        .collect();
    if blocks.is_empty() {
        anyhow::bail!("No command blocks recorded in the current session.");
    }

    let left_idx = resolve_single_selector(left_selector, blocks.len())?;
    let right_idx = resolve_single_selector(right_selector, blocks.len())?;

    let left_text = blocks[left_idx].text_for_mode(request.mode);
    let right_text = blocks[right_idx].text_for_mode(request.mode);

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

fn resolve_single_selector(selector: &str, total: usize) -> Result<usize> {
    let parsed = parse_selector(selector)?;
    let indices = resolve_selector(parsed, total)?;
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
    let changes: Vec<(ChangeTag, String)> = diff
        .iter_all_changes()
        .map(|change| (change.tag(), normalize_change_value(change.value())))
        .collect();

    let mut rows = Vec::new();
    let mut i = 0usize;
    while i < changes.len() {
        match changes[i].0 {
            ChangeTag::Equal => {
                let line = changes[i].1.clone();
                rows.push(SideBySideRow {
                    left_mark: ' ',
                    right_mark: ' ',
                    left: line.clone(),
                    right: line,
                });
                i += 1;
            }
            ChangeTag::Delete => {
                let mut deleted = Vec::new();
                while i < changes.len() && changes[i].0 == ChangeTag::Delete {
                    deleted.push(changes[i].1.clone());
                    i += 1;
                }

                let mut inserted = Vec::new();
                while i < changes.len() && changes[i].0 == ChangeTag::Insert {
                    inserted.push(changes[i].1.clone());
                    i += 1;
                }

                let span = deleted.len().max(inserted.len());
                for row_idx in 0..span {
                    let left_line = deleted.get(row_idx).cloned().unwrap_or_default();
                    let right_line = inserted.get(row_idx).cloned().unwrap_or_default();
                    let (left_mark, right_mark) = if !left_line.is_empty() && !right_line.is_empty()
                    {
                        ('~', '~')
                    } else if !left_line.is_empty() {
                        ('-', ' ')
                    } else {
                        (' ', '+')
                    };
                    rows.push(SideBySideRow {
                        left_mark,
                        right_mark,
                        left: left_line,
                        right: right_line,
                    });
                }
            }
            ChangeTag::Insert => {
                while i < changes.len() && changes[i].0 == ChangeTag::Insert {
                    rows.push(SideBySideRow {
                        left_mark: ' ',
                        right_mark: '+',
                        left: String::new(),
                        right: changes[i].1.clone(),
                    });
                    i += 1;
                }
            }
        }
    }

    rows
}

fn normalize_change_value(value: &str) -> String {
    value
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string()
}

fn truncate_and_pad(text: &str, width: usize) -> String {
    let mut truncated = truncate_to_width(text, width);
    let current = truncated.chars().count();
    if current < width {
        truncated.push_str(&" ".repeat(width - current));
    }
    truncated
}

fn truncate_to_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let len = text.chars().count();
    if len <= width {
        return text.to_string();
    }

    if width <= 3 {
        return ".".repeat(width);
    }

    let mut out = String::with_capacity(width);
    for ch in text.chars().take(width - 3) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::{
        build_side_by_side_rows, compute_columns, render_unified_diff, resolve_single_selector,
    };
    use similar::TextDiff;

    #[test]
    fn resolves_single_selector() {
        assert_eq!(resolve_single_selector("1", 3).unwrap(), 2);
    }

    #[test]
    fn rejects_multi_block_selector_for_diff() {
        let err = resolve_single_selector("1..2", 3).unwrap_err();
        assert!(err.to_string().contains("requires a single selector"));
    }

    #[test]
    fn renders_unified_diff_headers_from_selectors() {
        let out = render_unified_diff("1", "2", "a\n", "b\n");
        assert!(out.contains("--- 1"));
        assert!(out.contains("+++ 2"));
    }

    #[test]
    fn builds_side_by_side_rows_with_replace_marks() {
        let diff = TextDiff::from_lines("a\nb\n", "a\nc\n");
        let rows = build_side_by_side_rows(&diff);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].left_mark, ' ');
        assert_eq!(rows[1].left_mark, '~');
        assert_eq!(rows[1].right_mark, '~');
    }

    #[test]
    fn computes_non_zero_columns_for_small_width() {
        let (left, right) = compute_columns(8);
        assert!(left > 0);
        assert!(right > 0);
    }
}
