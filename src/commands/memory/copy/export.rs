//! Clipboard sink + post-projection filters.

use anyhow::{Context, Result};
use regex::Regex;

use crate::commands::browse::{filter_lines_by_spec, select_lines};
use crate::output;
use crate::tui::workspace::{TextPair, WorkspacePickedContent};

use super::plan::{CopyFilters, DialogueSelect};
use crate::commands::select::resolve_selector;

/// Export TUI-picked content to the clipboard.
///
/// Product surfaces (bare `sivtr`, hotkey) own the browse call; copy only sinks.
pub fn export_picked(
    picked: &WorkspacePickedContent,
    print_full: bool,
    regex: Option<&str>,
    lines: Option<&str>,
    ansi: bool,
) -> Result<()> {
    let empty = format!("selected {} content is empty", picked.source.label());
    let success = format!("copied {} content to clipboard", picked.source.label());
    finish_units(
        &picked.units,
        &picked.selection,
        &CopyFilters {
            print: print_full,
            ansi,
            regex: regex.map(str::to_string),
            lines: lines.map(str::to_string),
            prompt: None,
            cwd: None,
        },
        &empty,
        &success,
    )
}

pub(super) fn finish_units(
    units: &[TextPair],
    selection: &DialogueSelect,
    filters: &CopyFilters,
    empty_message: &str,
    success_message: &str,
) -> Result<()> {
    let indices = resolve_selector(selection, units.len())?;
    let selected: Vec<TextPair> = indices
        .iter()
        .filter_map(|idx| units.get(*idx).cloned())
        .filter(|unit| !unit.plain.trim().is_empty())
        .collect();
    if selected.is_empty() {
        output::warning(empty_message);
        return Ok(());
    }
    let text = join_text_pairs(&selected, "\n\n");
    finish_text(text, filters, success_message)
}

pub(super) fn finish_text_pairs(
    pairs: &[TextPair],
    filters: &CopyFilters,
    success_message: &str,
) -> Result<()> {
    if pairs.is_empty() {
        output::warning("selected content is empty");
        return Ok(());
    }
    finish_text(join_text_pairs(pairs, "\n\n"), filters, success_message)
}

pub(super) fn finish_text(
    mut text: TextPair,
    filters: &CopyFilters,
    success_message: &str,
) -> Result<()> {
    if let Some(pattern) = filters.regex.as_deref() {
        text = filter_lines_by_regex(&text, pattern)?;
    }
    if let Some(spec) = filters.lines.as_deref() {
        text = filter_lines_by_spec(&text, spec)?;
    }
    let body = if filters.ansi {
        text.ansi
    } else {
        text.plain
    };
    let body = body.trim();
    if body.is_empty() {
        output::warning("filters removed everything");
        output::hint("loosen `--regex` or `--lines`, or copy without filters");
        return Ok(());
    }
    sivtr_core::export::clipboard::copy_to_clipboard(body)?;
    if filters.print {
        for line in body.lines() {
            output::plain(format!("  {line}"));
        }
    }
    output::success(success_message);
    Ok(())
}

pub(super) fn join_text_pairs(pairs: &[TextPair], separator: &str) -> TextPair {
    TextPair {
        plain: pairs
            .iter()
            .map(|pair| pair.plain.as_str())
            .collect::<Vec<_>>()
            .join(separator),
        ansi: pairs
            .iter()
            .map(|pair| pair.ansi.as_str())
            .collect::<Vec<_>>()
            .join(separator),
    }
}

pub(super) fn filter_lines_by_regex(text: &TextPair, pattern: &str) -> Result<TextPair> {
    let regex = Regex::new(pattern)
        .with_context(|| format!("Invalid regex `{pattern}`. Check the pattern syntax."))?;
    let indices = text
        .plain
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| regex.is_match(line).then_some(idx))
        .collect::<Vec<_>>();
    Ok(select_lines(text, &indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_regex() {
        let filtered = filter_lines_by_regex(
            &TextPair {
                plain: "a\nwarn: b\nc".to_string(),
                ansi: "a\nwarn: b\nc".to_string(),
            },
            "warn",
        )
        .unwrap();
        assert_eq!(filtered.plain, "warn: b");
    }

    #[test]
    fn filters_ansi_by_plain_regex_matches() {
        let filtered = filter_lines_by_regex(
            &TextPair {
                plain: "a\nwarn: b\nc".to_string(),
                ansi: "a\n\x1b[31mwarn: b\x1b[0m\nc".to_string(),
            },
            "warn",
        )
        .unwrap();
        assert_eq!(filtered.ansi, "\x1b[31mwarn: b\x1b[0m");
    }
}
