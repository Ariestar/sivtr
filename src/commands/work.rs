use anyhow::{bail, Context, Result};
use serde::Serialize;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkPartIo, WorkPartKind, WorkRecord, WorkRef};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use crate::cli::{
    WorkCommand, WorkPartFilterArg, WorkPartsArgs, WorkRecordsArgs, WorkSessionsArgs,
};
use crate::commands::records::current_work_record_index;
use crate::commands::work_json::{
    session_meta, target_meta, WorkJsonSessionMeta, WorkJsonTargetMeta,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkSessionSource {
    Terminal,
    Agent(AgentProvider),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkSessionMarker {
    source: WorkSessionSource,
    session: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WorkSessionListItem {
    #[serde(rename = "ref")]
    ref_: String,
    source: String,
    session: String,
    session_meta: WorkJsonSessionMeta,
    timestamp: Option<String>,
    record_count: usize,
    title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WorkRecordListItem {
    #[serde(rename = "ref")]
    ref_: String,
    kind: String,
    session: WorkJsonSessionMeta,
    target: WorkJsonTargetMeta,
    timestamp: Option<String>,
    input_parts: usize,
    output_parts: usize,
    title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WorkPartListItem {
    #[serde(rename = "ref")]
    ref_: String,
    session: WorkJsonSessionMeta,
    target: WorkJsonTargetMeta,
    io: WorkPartIo,
    kind: WorkPartKind,
    index: usize,
    timestamp: Option<String>,
    label: Option<String>,
    summary: String,
}

#[derive(Serialize)]
struct WorkSessionsJsonOutput {
    cwd: String,
    sessions: Vec<WorkSessionListItem>,
}

#[derive(Serialize)]
struct WorkRecordsJsonOutput {
    cwd: String,
    session: String,
    records: Vec<WorkRecordListItem>,
}

#[derive(Serialize)]
struct WorkPartsJsonOutput {
    cwd: String,
    record: String,
    io: &'static str,
    parts: Vec<WorkPartListItem>,
}

pub fn execute(command: &WorkCommand) -> Result<()> {
    match &command.action {
        crate::cli::WorkSubcommand::Sessions(args) => execute_sessions(args),
        crate::cli::WorkSubcommand::Records(args) => execute_records(args),
        crate::cli::WorkSubcommand::Parts(args) => execute_parts(args),
    }
}

fn execute_sessions(args: &WorkSessionsArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let providers = args.provider.providers();
    let records = current_work_record_index(&providers, &cwd, None)?;
    let items = build_session_items(records.records());

    if args.json {
        let output = WorkSessionsJsonOutput {
            cwd: cwd.display().to_string(),
            sessions: items,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if items.is_empty() {
        println!("No workspace sessions found");
        return Ok(());
    }

    for item in &items {
        println!("{}", format_session_item(item));
    }
    Ok(())
}

fn execute_records(args: &WorkRecordsArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let marker = args.session.parse::<WorkSessionMarker>()?;
    let providers = marker.providers();
    let records = current_work_record_index(&providers, &cwd, None)?;
    let items = records
        .records()
        .iter()
        .filter(|record| marker.matches_record(record))
        .map(record_item)
        .collect::<Vec<_>>();

    if args.json {
        let output = WorkRecordsJsonOutput {
            cwd: cwd.display().to_string(),
            session: marker.to_string(),
            records: items,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if items.is_empty() {
        println!("No records found for `{marker}`");
        return Ok(());
    }

    for item in &items {
        println!("{}", format_record_item(item));
    }
    Ok(())
}

fn execute_parts(args: &WorkPartsArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let input_ref: WorkRef = args.record.parse()?;
    let record_ref = input_ref.record_ref();
    let providers = record_ref
        .provider()
        .map(|provider| vec![provider])
        .unwrap_or_default();
    let records = current_work_record_index(&providers, &cwd, None)?;
    let record = records
        .resolve(&record_ref)
        .with_context(|| format!("No record found for ref `{}`", args.record))?;
    let items = build_part_items(record, args.io);

    if args.json {
        let output = WorkPartsJsonOutput {
            cwd: cwd.display().to_string(),
            record: record_ref.to_string(),
            io: args.io.as_str(),
            parts: items,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if items.is_empty() {
        println!("No {} parts found for `{}`", args.io.label(), record_ref);
        return Ok(());
    }

    for item in &items {
        println!("{}", format_part_item(item));
    }
    Ok(())
}

fn resolve_cwd(cwd: Option<&Path>) -> Result<std::path::PathBuf> {
    Ok(cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?))
}

fn build_session_items(records: &[WorkRecord]) -> Vec<WorkSessionListItem> {
    let mut items: Vec<WorkSessionListItem> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();

    for record in records {
        let marker = WorkSessionMarker::from_record(record);
        let group_key = session_group_key(record, &marker);
        if let Some(position) = positions.get(&group_key).copied() {
            items[position].record_count += 1;
            continue;
        }
        let display_ref = marker.to_string();
        positions.insert(group_key, items.len());
        items.push(WorkSessionListItem {
            ref_: display_ref,
            source: marker.source_name().to_string(),
            session: marker.session,
            session_meta: session_meta(record),
            timestamp: record.time.primary_at().map(str::to_string),
            record_count: 1,
            title: record.title.clone(),
        });
    }

    items
}

fn record_item(record: &WorkRecord) -> WorkRecordListItem {
    let (input_parts, output_parts) = part_counts(record);
    WorkRecordListItem {
        ref_: record.session.work_ref.to_string(),
        kind: record.kind_label().to_string(),
        session: session_meta(record),
        target: target_meta(record, record.session.work_ref.target()),
        timestamp: record.time.primary_at().map(str::to_string),
        input_parts,
        output_parts,
        title: record.title.clone(),
    }
}

fn session_group_key(record: &WorkRecord, marker: &WorkSessionMarker) -> String {
    let session_id = record
        .session
        .canonical_id
        .as_deref()
        .unwrap_or(marker.session.as_str());
    format!("{}/{}", marker.source_name(), session_id)
}

fn build_part_items(record: &WorkRecord, io: WorkPartFilterArg) -> Vec<WorkPartListItem> {
    record
        .parts
        .iter()
        .filter(|part| io.matches(part.io))
        .map(|part| WorkPartListItem {
            ref_: record
                .session
                .work_ref
                .with_part(part.io, part.index_in_record)
                .to_string(),
            session: session_meta(record),
            target: target_meta(
                record,
                record
                    .session
                    .work_ref
                    .with_part(part.io, part.index_in_record)
                    .target(),
            ),
            io: part.io,
            kind: part.kind,
            index: part.index_in_record,
            timestamp: part.occurred_at.clone(),
            label: part.label.clone(),
            summary: summary_text(&part.text),
        })
        .collect()
}

fn part_counts(record: &WorkRecord) -> (usize, usize) {
    record
        .parts
        .iter()
        .fold((0, 0), |(input, output), part| match part.io {
            WorkPartIo::Input => (input + 1, output),
            WorkPartIo::Output => (input, output + 1),
        })
}

fn format_session_item(item: &WorkSessionListItem) -> String {
    format_marker_line(
        &item.ref_,
        &[
            format!("records {}", item.record_count),
            timestamp_tag(item.timestamp.as_deref()),
        ],
        &item.title,
    )
}

fn format_record_item(item: &WorkRecordListItem) -> String {
    format_marker_line(
        &item.ref_,
        &[
            format!("parts i={} o={}", item.input_parts, item.output_parts),
            timestamp_tag(item.timestamp.as_deref()),
        ],
        &item.title,
    )
}

fn format_part_item(item: &WorkPartListItem) -> String {
    let mut tags = vec![
        format!("{} {}", io_name(item.io), kind_name(item.kind)),
        format!("n={}", item.index),
        timestamp_tag(item.timestamp.as_deref()),
    ];
    if let Some(label) = item
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
    {
        tags.push(format!("label {}", summary_text(label)));
    }
    format_marker_line(&item.ref_, &tags, &item.summary)
}

fn format_marker_line(marker: &str, tags: &[String], summary: &str) -> String {
    let mut line = marker.to_string();
    for tag in tags.iter().filter(|tag| !tag.is_empty()) {
        line.push_str("  [");
        line.push_str(tag);
        line.push(']');
    }
    if !summary.trim().is_empty() {
        line.push_str("  ");
        line.push_str(summary);
    }
    line
}

fn timestamp_tag(timestamp: Option<&str>) -> String {
    timestamp.unwrap_or("unknown-time").to_string()
}

fn summary_text(text: &str) -> String {
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

fn io_name(io: WorkPartIo) -> &'static str {
    match io {
        WorkPartIo::Input => "input",
        WorkPartIo::Output => "output",
    }
}

fn kind_name(kind: WorkPartKind) -> &'static str {
    match kind {
        WorkPartKind::Prompt => "prompt",
        WorkPartKind::Command => "command",
        WorkPartKind::UserMessage => "user_message",
        WorkPartKind::AssistantMessage => "assistant_message",
        WorkPartKind::ToolCall => "tool_call",
        WorkPartKind::ToolOutput => "tool_output",
        WorkPartKind::Text => "text",
        WorkPartKind::Error => "error",
    }
}

impl WorkSessionMarker {
    fn from_record(record: &WorkRecord) -> Self {
        match &record.session.work_ref {
            WorkRef::Terminal { session, .. } => Self {
                source: WorkSessionSource::Terminal,
                session: session.clone(),
            },
            WorkRef::Agent {
                provider, session, ..
            } => Self {
                source: WorkSessionSource::Agent(*provider),
                session: session.clone(),
            },
        }
    }

    fn providers(&self) -> Vec<AgentProvider> {
        match self.source {
            WorkSessionSource::Terminal => Vec::new(),
            WorkSessionSource::Agent(provider) => vec![provider],
        }
    }

    fn matches_record(&self, record: &WorkRecord) -> bool {
        self.source == Self::from_record(record).source && record.session.matches_id(&self.session)
    }

    fn source_name(&self) -> &'static str {
        match self.source {
            WorkSessionSource::Terminal => "terminal",
            WorkSessionSource::Agent(provider) => provider.command_name(),
        }
    }
}

impl fmt::Display for WorkSessionMarker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.source_name(), self.session)
    }
}

impl std::str::FromStr for WorkSessionMarker {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let parts = value
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.len() != 2 {
            bail!(
                "Invalid session ref `{value}`; expected terminal/<session> or <provider>/<session>"
            );
        }

        let source = if parts[0].eq_ignore_ascii_case("terminal") {
            WorkSessionSource::Terminal
        } else {
            WorkSessionSource::Agent(AgentProvider::from_command_name(parts[0]).ok_or_else(
                || {
                    anyhow::anyhow!(
                        "Invalid session ref `{value}`; unknown source `{}`",
                        parts[0]
                    )
                },
            )?)
        };

        Ok(Self {
            source,
            session: parts[1].to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{
        WorkChannel, WorkOutcome, WorkPart, WorkPayload, WorkRecordKind, WorkSessionRef,
        WorkSource, WorkStatus, WorkText, WorkTime,
    };

    #[test]
    fn parses_session_refs() {
        let terminal: WorkSessionMarker = "terminal/session_123".parse().unwrap();
        assert_eq!(terminal.to_string(), "terminal/session_123");

        let agent: WorkSessionMarker = "codex/abcdef12".parse().unwrap();
        assert_eq!(agent.to_string(), "codex/abcdef12");

        assert!("codex/abcdef12/3".parse::<WorkSessionMarker>().is_err());
        assert!("unknown/abcdef12".parse::<WorkSessionMarker>().is_err());
    }

    #[test]
    fn groups_sessions_in_latest_record_order() {
        let records = vec![
            test_record(
                "codex/alpha/2",
                "latest title",
                Some("2026-05-24T12:00:00Z"),
            ),
            test_record(
                "terminal/shell/1",
                "shell title",
                Some("2026-05-24T11:00:00Z"),
            ),
            test_record("codex/alpha/1", "older title", Some("2026-05-24T10:00:00Z")),
        ];

        let items = build_session_items(&records);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].ref_, "codex/alpha");
        assert_eq!(items[0].record_count, 2);
        assert_eq!(items[0].title, "latest title");
        assert_eq!(items[1].ref_, "terminal/shell");
    }

    #[test]
    fn builds_part_items_with_structured_refs() {
        let record = test_record("codex/alpha/2", "title", Some("2026-05-24T12:00:00Z"));

        let parts = build_part_items(&record, WorkPartFilterArg::Output);

        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].ref_, "codex/alpha/2/o/1");
        assert_eq!(
            parts[0].session.canonical_id.as_deref(),
            Some("alpha-session-0123456789abcdef")
        );
        assert_eq!(parts[0].target.type_, "part");
        assert_eq!(
            parts[0].ref_,
            parts[0].target.part.as_ref().expect("part metadata").ref_
        );
        assert_eq!(parts[0].summary, "assistant reply");
    }

    #[test]
    fn groups_sessions_with_canonical_session_metadata() {
        let record = test_record("codex/alpha/2", "title", Some("2026-05-24T12:00:00Z"));

        let items = build_session_items(&[record]);

        assert_eq!(items[0].session_meta.display_id, "alpha");
        assert_eq!(
            items[0].session_meta.canonical_id.as_deref(),
            Some("alpha-session-0123456789abcdef")
        );
    }

    #[test]
    fn session_markers_match_canonical_ids() {
        let marker: WorkSessionMarker = "codex/alpha-session-0123456789abcdef".parse().unwrap();
        let record = test_record("codex/alpha/2", "title", Some("2026-05-24T12:00:00Z"));

        assert!(marker.matches_record(&record));
    }

    #[test]
    fn keeps_distinct_canonical_sessions_separate_when_display_ids_collide() {
        let mut first = test_record("codex/alpha/2", "first", Some("2026-05-24T12:00:00Z"));
        first.session.canonical_id = Some("session-aaaa1111".to_string());
        let mut second = test_record("codex/alpha/1", "second", Some("2026-05-24T11:00:00Z"));
        second.session.canonical_id = Some("session-bbbb2222".to_string());

        let items = build_session_items(&[first, second]);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].record_count, 1);
        assert_eq!(items[1].record_count, 1);
    }

    fn test_record(ref_id: &str, title: &str, timestamp: Option<&str>) -> WorkRecord {
        let work_ref: WorkRef = ref_id.parse().unwrap();
        WorkRecord {
            schema_version: 1,
            id: ref_id.to_string(),
            kind: WorkRecordKind::ChatTurn,
            work_ref: work_ref.clone(),
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: "alpha".to_string(),
                canonical_id: Some("alpha-session-0123456789abcdef".to_string()),
                path: None,
                index: 1,
                work_ref,
            },
            cwd: None,
            time: WorkTime {
                started_at: timestamp.map(str::to_string),
                ended_at: timestamp.map(str::to_string),
                duration_ms: None,
            },
            status: WorkStatus {
                outcome: WorkOutcome::Unknown,
                exit_code: None,
            },
            title: title.to_string(),
            text: WorkText {
                input: Some("user prompt".to_string()),
                output: Some("assistant reply".to_string()),
                combined: "user prompt\nassistant reply".to_string(),
            },
            parts: vec![
                WorkPart {
                    io: WorkPartIo::Input,
                    kind: WorkPartKind::UserMessage,
                    index_in_record: 1,
                    occurred_at: timestamp.map(str::to_string),
                    label: Some("user".to_string()),
                    text: "user prompt".to_string(),
                    ansi: None,
                },
                WorkPart {
                    io: WorkPartIo::Output,
                    kind: WorkPartKind::AssistantMessage,
                    index_in_record: 1,
                    occurred_at: timestamp.map(str::to_string),
                    label: Some("assistant".to_string()),
                    text: "assistant reply".to_string(),
                    ansi: None,
                },
            ],
            payload: WorkPayload::ChatTurn {
                user: "user prompt".to_string(),
                assistant: "assistant reply".to_string(),
                messages: Vec::new(),
            },
        }
    }
}
