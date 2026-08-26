//! Clipboard sink + post-projection filters.

use anyhow::{Context, Result};
use regex::Regex;

use crate::commands::browse::{filter_lines_by_spec, select_lines};
use crate::output;
use crate::tui::workspace::{TextPair, WorkspacePickedContent};

use super::plan::CopyFilters;

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
    finish_units(
        &picked.units,
        &CopyFilters {
            print: print_full,
            ansi,
            regex: regex.map(str::to_string),
            lines: lines.map(str::to_string),
            prompt: None,
            cwd: None,
        },
        &picked.source.label(),
    )
}

/// Join every unit that carries text and sink it, naming `label` in both the
/// empty warning and the success line. The single path both copy surfaces —
/// the CLI plan and the TUI pick — end on.
pub(super) fn finish_units(units: &[TextPair], filters: &CopyFilters, label: &str) -> Result<()> {
    let kept: Vec<TextPair> = units
        .iter()
        .filter(|unit| !unit.plain.trim().is_empty())
        .cloned()
        .collect();
    if kept.is_empty() {
        output::warning(format!("selected {label} content is empty"));
        return Ok(());
    }
    let count = kept.len();
    finish_text(
        join_text_pairs(&kept),
        filters,
        &format!("copied {count} item(s) from {label} to clipboard"),
    )
}

fn finish_text(mut text: TextPair, filters: &CopyFilters, success_message: &str) -> Result<()> {
    if let Some(pattern) = filters.regex.as_deref() {
        text = filter_lines_by_regex(&text, pattern)?;
    }
    if let Some(spec) = filters.lines.as_deref() {
        text = filter_lines_by_spec(&text, spec)?;
    }
    let body = if filters.ansi { text.ansi } else { text.plain };
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

fn join_text_pairs(pairs: &[TextPair]) -> TextPair {
    TextPair {
        plain: pairs
            .iter()
            .map(|pair| pair.plain.as_str())
            .collect::<Vec<_>>()
            .join(
                "

",
            ),
        ansi: pairs
            .iter()
            .map(|pair| pair.ansi.as_str())
            .collect::<Vec<_>>()
            .join(
                "

",
            ),
    }
}

fn filter_lines_by_regex(text: &TextPair, pattern: &str) -> Result<TextPair> {
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
