use std::io::IsTerminal;
use std::str::FromStr;

use anyhow::{Context, Result};
use sivtr_core::record::{WorkPart, WorkRecord, WorkRef, WorkRefTarget};

use crate::cli::ShowArgs;
use crate::commands::workset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkSetOutputFormat {
    Full,
    Compact,
    Timeline,
    Md,
    Refs,
    WorkSet,
}

impl FromStr for WorkSetOutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "full" => Ok(Self::Full),
            "compact" => Ok(Self::Compact),
            "timeline" => Ok(Self::Timeline),
            "md" | "markdown" => Ok(Self::Md),
            "refs" => Ok(Self::Refs),
            "workset" | "json" => Ok(Self::WorkSet),
            _ => Err(format!(
                "unknown show output format `{value}`; expected full, timeline, compact, md, refs, or workset"
            )),
        }
    }
}

impl std::fmt::Display for WorkSetOutputFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Full => "full",
            Self::Compact => "compact",
            Self::Timeline => "timeline",
            Self::Md => "md",
            Self::Refs => "refs",
            Self::WorkSet => "workset",
        })
    }
}

struct AnchorItem<'a> {
    anchor: WorkRef,
    record: &'a WorkRecord,
}

struct ShowItem {
    ref_: WorkRef,
    content: String,
}

pub fn default_output_format() -> WorkSetOutputFormat {
    if std::io::stdout().is_terminal() {
        WorkSetOutputFormat::Full
    } else {
        WorkSetOutputFormat::WorkSet
    }
}

pub fn resolve_output_format(
    format: Option<WorkSetOutputFormat>,
    full: bool,
    refs: bool,
    json: bool,
) -> WorkSetOutputFormat {
    if full {
        WorkSetOutputFormat::Full
    } else if refs {
        WorkSetOutputFormat::Refs
    } else if json {
        WorkSetOutputFormat::WorkSet
    } else {
        format.unwrap_or_else(default_output_format)
    }
}

pub fn execute(args: &ShowArgs) -> Result<()> {
    let set = resolve_target(args)?;
    print_workset(
        &set,
        resolve_output_format(args.format, args.full, args.refs, args.json),
    )
}

fn resolve_target(args: &ShowArgs) -> Result<workset::WorkSet> {
    Ok(workset::load_source(&args.source, args.cwd.as_deref())?.into_workset())
}

pub fn print_workset(set: &workset::WorkSet, format: WorkSetOutputFormat) -> Result<()> {
    match format {
        WorkSetOutputFormat::Full => print_full(set)?,
        WorkSetOutputFormat::WorkSet => {
            println!("{}", serde_json::to_string_pretty(set)?);
        }
        WorkSetOutputFormat::Compact => print_compact(set)?,
        WorkSetOutputFormat::Timeline => print_timeline(set)?,
        WorkSetOutputFormat::Md => print_markdown(set)?,
        WorkSetOutputFormat::Refs => print_refs(set),
    }
    Ok(())
}

fn anchor_items(set: &workset::WorkSet) -> Result<Vec<AnchorItem<'_>>> {
    set.anchors()
        .into_iter()
        .map(|anchor| {
            let record = workset::record_for_anchor(&set.records, &anchor)
                .with_context(|| format!("No record found for anchor `{anchor}`"))?;
            Ok(AnchorItem { anchor, record })
        })
        .collect()
}

fn print_full(set: &workset::WorkSet) -> Result<()> {
    let items = anchor_items(set)?
        .into_iter()
        .map(|item| {
            let content = item
                .record
                .content_for_target(item.anchor.target())
                .with_context(|| format!("No content found for ref `{}`", item.anchor))?;
            Ok(ShowItem {
                ref_: item.anchor,
                content,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    print_show_items(items);
    Ok(())
}

fn print_show_items(items: Vec<ShowItem>) {
    let multi = items.len() > 1;
    for item in items {
        if multi {
            println!("--- {} ---", item.ref_);
        }
        print!("{}", item.content);
        if !item.content.ends_with('\n') {
            println!();
        }
    }
}

fn print_compact(set: &workset::WorkSet) -> Result<()> {
    for item in anchor_items(set)? {
        println!(
            "{}  {:<8}  {}",
            short_time(item.record),
            source_label(item.record),
            anchor_summary(item.record, &item.anchor)
        );
    }
    Ok(())
}

fn print_timeline(set: &workset::WorkSet) -> Result<()> {
    let mut previous_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
    for item in anchor_items(set)? {
        let timestamp =
            anchor_timestamp(item.record, &item.anchor).and_then(sivtr_core::time::parse_timestamp);
        if let (Some(previous), Some(current)) = (previous_timestamp, timestamp) {
            let gap_minutes = (current - previous).num_minutes();
            if gap_minutes >= 15 {
                println!("          -- gap {gap_minutes}m --");
            }
        }
        if timestamp.is_some() {
            previous_timestamp = timestamp;
        }

        println!(
            "{}  {:<8}  {:<28}  {}",
            short_time(item.record),
            source_label(item.record),
            item.anchor,
            anchor_summary(item.record, &item.anchor)
        );
    }
    Ok(())
}

fn print_markdown(set: &workset::WorkSet) -> Result<()> {
    for item in anchor_items(set)? {
        println!(
            "- **{}** `{}` {}",
            short_time(item.record),
            item.anchor,
            escape_markdown_title(&anchor_summary(item.record, &item.anchor))
        );
    }
    Ok(())
}

fn print_refs(set: &workset::WorkSet) {
    for anchor in set.anchors() {
        println!("{anchor}");
    }
}

fn anchor_summary(record: &WorkRecord, anchor: &WorkRef) -> String {
    match anchor.target() {
        WorkRefTarget::Record => record.title.clone(),
        WorkRefTarget::Line(_) => record
            .content_for_target(anchor.target())
            .map(|text| summary_text(&text))
            .unwrap_or_else(|| record.title.clone()),
        WorkRefTarget::Part { .. } => record
            .part_for_target(anchor.target())
            .map(part_summary)
            .unwrap_or_else(|| record.title.clone()),
    }
}

fn part_summary(part: &WorkPart) -> String {
    let label = part
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .map(|label| format!("{} ", summary_text(label)))
        .unwrap_or_default();
    format!("{}{}", label, summary_text(&part.text))
}

fn anchor_timestamp<'a>(record: &'a WorkRecord, anchor: &WorkRef) -> Option<&'a str> {
    match anchor.target() {
        WorkRefTarget::Part { .. } => record
            .part_for_target(anchor.target())
            .and_then(|part| part.occurred_at.as_deref())
            .or_else(|| record.time.primary_at()),
        WorkRefTarget::Record | WorkRefTarget::Line(_) => record.time.primary_at(),
    }
}

pub fn summary_text(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = compact.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    let summary = trimmed.chars().take(96).collect::<String>();
    if trimmed.chars().count() > 96 {
        format!("{summary}...")
    } else {
        summary
    }
}

fn short_time(record: &WorkRecord) -> String {
    record
        .time
        .primary_at()
        .and_then(sivtr_core::time::parse_timestamp)
        .map(|timestamp| {
            timestamp
                .with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "--:--:--".to_string())
}

fn source_label(record: &WorkRecord) -> &'static str {
    match record.kind {
        sivtr_core::record::WorkRecordKind::TerminalCommand => "terminal",
        sivtr_core::record::WorkRecordKind::ChatTurn => record
            .work_ref
            .provider()
            .map(|provider| provider.command_name())
            .unwrap_or("agent"),
    }
}

fn escape_markdown_title(title: &str) -> String {
    title.replace('[', "\\[").replace(']', "\\]")
}
