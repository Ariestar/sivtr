//! Clipboard sink + post-projection filters.

use anyhow::{Context, Result};
use regex::Regex;

use crate::commands::browse::{
    filter_lines_by_spec, select_lines, PickedContent, WorkspacePickProjection,
};
use crate::output;
use crate::tui::workspace::TextPair;

use super::plan::CopyFilters;

/// Export TUI-picked content to the clipboard.
///
/// Product surfaces (bare `sivtr`, hotkey) own the browse call; copy only sinks.
pub fn export_picked(
    picked: &PickedContent,
    print_full: bool,
    regex: Option<&str>,
    lines: Option<&str>,
    ansi: bool,
) -> Result<()> {
    let (units, label) = picked_units(picked)?;
    finish_units(
        &units,
        &CopyFilters {
            print: print_full,
            ansi,
            regex: regex.map(str::to_string),
            lines: lines.map(str::to_string),
            prompt: None,
            cwd: None,
        },
        &label,
    )
    .context("export picked content")
}

pub(crate) fn picked_units(picked: &PickedContent) -> Result<(Vec<TextPair>, String)> {
    match picked {
        PickedContent::Text { source, units } => Ok((units.clone(), source.label())),
        PickedContent::WorkSet {
            source,
            set,
            projection,
        } => {
            let projection = match projection {
                WorkspacePickProjection::Whole => super::plan::Projection::Both,
                WorkspacePickProjection::Input => super::plan::Projection::Input,
                WorkspacePickProjection::Output => super::plan::Projection::Output,
                WorkspacePickProjection::Command => super::plan::Projection::Command,
                WorkspacePickProjection::Parts => {
                    super::plan::Projection::Exact(sivtr_core::record::WorkAt::Whole)
                }
            };
            let anchors = set.anchors();
            let units = anchors
                .iter()
                .map(|anchor| {
                    let record = set
                        .records
                        .iter()
                        .find(|record| record.work_ref.whole() == anchor.whole())
                        .with_context(|| {
                            format!("picked record `{}` is not materialized", anchor.whole())
                        })?;
                    let projection = if matches!(
                        projection,
                        super::plan::Projection::Both
                            | super::plan::Projection::Exact(sivtr_core::record::WorkAt::Whole)
                    ) && anchor.at != sivtr_core::record::WorkAt::Whole
                    {
                        super::plan::Projection::Exact(anchor.at)
                    } else {
                        projection
                    };
                    super::project::project_record(record, projection, None)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((units, source.label()))
        }
    }
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
    use crate::commands::browse::{PickedContent, WorkspacePickProjection};
    use crate::commands::memory::workset::WorkSet;
    use crate::tui::workspace::WorkspaceSource;
    use sivtr_core::session::SessionEntry;
    use std::path::Path;

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

    #[test]
    fn workset_pick_projects_its_records() {
        let record = sivtr_core::record::WorkRecord::terminal(
            &SessionEntry::new("PS C:\\repo>", "cargo test", "ok"),
            Path::new("current"),
            0,
        )
        .expect("test record");
        let picked = PickedContent::WorkSet {
            source: WorkspaceSource::terminal(),
            set: WorkSet::with_anchors("current", vec![record.clone()], vec![record.work_ref]),
            projection: WorkspacePickProjection::Command,
        };

        let (units, label) = picked_units(&picked).expect("projects workset");
        assert_eq!(label, "terminal");
        assert_eq!(units[0].plain, "cargo test");
    }
}
