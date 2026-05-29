use anyhow::{bail, Context, Result};
use serde::Serialize;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{semantic_search, WorkLinkKind, WorkRecord, WorkRef};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use crate::cli::{
    WorkCommand, WorkLinksArgs, WorkPartsArgs, WorkRecordsArgs, WorkSemanticArgs, WorkSessionsArgs,
};
use crate::commands::filter;
use crate::commands::records::current_work_record_index;
use crate::commands::show;
use crate::commands::work_json::{session_meta, WorkJsonSessionMeta};
use crate::commands::workset::{self, WorkSet};

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

#[derive(Serialize)]
struct WorkSessionsJsonOutput {
    cwd: String,
    sessions: Vec<WorkSessionListItem>,
}

pub fn execute(command: &WorkCommand) -> Result<()> {
    match &command.action {
        crate::cli::WorkSubcommand::Sessions(args) => execute_sessions(args),
        crate::cli::WorkSubcommand::Records(args) => execute_records(args),
        crate::cli::WorkSubcommand::Parts(args) => execute_parts(args),
        crate::cli::WorkSubcommand::Semantic(args) => execute_semantic(args),
        crate::cli::WorkSubcommand::Links(args) => execute_links(args),
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
    let mut set = if let Ok(marker) = args.source.parse::<WorkSessionMarker>() {
        records_for_session_marker(&cwd, &marker)?
    } else {
        let source = workset::load_source(&args.source, Some(&cwd))?;
        let (records, anchors) = source.into_parts();
        let record_anchors = anchors
            .into_iter()
            .map(|anchor| anchor.record_ref())
            .collect::<Vec<_>>();
        WorkSet::with_anchors(
            cwd.display().to_string(),
            workset::records_for_anchors(&records, &record_anchors),
            record_anchors,
        )
    };
    set.save_last()?;
    if let Some(name) = args.save.as_deref() {
        set.save_as(name)?;
    }
    show::print_workset(
        &set,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

fn execute_parts(args: &WorkPartsArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let source = workset::load_source(&args.source, Some(&cwd))?;
    let (records, anchors) = source.into_parts();
    let mut set = filter::apply_parts(
        cwd,
        records,
        anchors,
        filter::FilterSpec::from_work_parts_args(args)?,
    )?;
    set.save_last()?;
    if let Some(name) = args.save.as_deref() {
        set.save_as(name)?;
    }
    show::print_workset(
        &set,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

fn execute_semantic(args: &WorkSemanticArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let filter_tags = args.tag.clone();
    let providers = AgentProvider::all()
        .iter()
        .map(|spec| spec.provider)
        .collect::<Vec<_>>();
    let records = current_work_record_index(&providers, &cwd, None)?;
    let results = semantic_search(records.records(), &args.query, args.limit, |record| {
        tags_match_record(&record.parts, &filter_tags)
    });
    if results.is_empty() {
        println!(
            "No semantically relevant records found for `{}`",
            args.query
        );
        return Ok(());
    }

    let anchors: Vec<WorkRef> = results.iter().map(|result| result.record_ref.clone()).collect();
    let matched = workset::records_for_anchors(records.records(), &anchors);
    let set = WorkSet::with_anchors(cwd.display().to_string(), matched, anchors);
    set.save_last()?;

    for result in &results {
        let record = records
            .resolve(&result.record_ref)
            .with_context(|| format!("Record not found: {}", result.record_ref))?;
        eprintln!(
            "{}  [{:<5}]  (score: {}, terms: {})",
            result.record_ref,
            record.kind_label(),
            result.score,
            result.matched_terms.join(", "),
        );
    }

    show::print_workset(&set, show::resolve_output_format(None, false, false, true))
}

fn execute_links(args: &WorkLinksArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let providers = AgentProvider::all()
        .iter()
        .map(|spec| spec.provider)
        .collect::<Vec<_>>();
    let records = current_work_record_index(&providers, &cwd, None)?;
    let mut links = records.infer_links();

    if let Some(kind) = &args.kind {
        let wanted = match kind.to_ascii_lowercase().as_str() {
            "caused_by" => WorkLinkKind::CausedBy,
            "follows_up" => WorkLinkKind::FollowsUp,
            "references" => WorkLinkKind::References,
            _ => bail!("Unknown link kind `{kind}`; expected caused_by, follows_up, or references"),
        };
        links.retain(|link| link.kind == wanted);
    }
    if let Some(ref_prefix) = &args.ref_ {
        links.retain(|link| {
            link.from.to_string().starts_with(ref_prefix)
                || link.to.to_string().starts_with(ref_prefix)
        });
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&links)?);
        return Ok(());
    }
    if links.is_empty() {
        println!("No causal links found.");
        return Ok(());
    }

    for link in &links {
        let kind = serde_json::to_value(link.kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{:?}", link.kind));
        println!("{}  ->  {}  [{}]", link.from, link.to, kind);
    }
    Ok(())
}

fn records_for_session_marker(cwd: &Path, marker: &WorkSessionMarker) -> Result<WorkSet> {
    let providers = marker.providers();
    let records = current_work_record_index(&providers, cwd, None)?;
    let selected = records
        .records()
        .iter()
        .filter(|record| marker.matches_record(record))
        .cloned()
        .collect::<Vec<_>>();

    if selected.is_empty() {
        bail!("No records found for `{marker}`");
    }

    Ok(WorkSet::new(cwd.display().to_string(), selected))
}

fn tags_match(filter: &[String], tags: &[String]) -> bool {
<<<<<<< HEAD
    filter.iter().all(|tag| tags.iter().any(|part_tag| part_tag == tag))
=======
    filter.iter().all(|t| tags.iter().any(|pt| pt == t))
}

fn resolve_cwd(cwd: Option<&Path>) -> Result<std::path::PathBuf> {
    Ok(cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?))
}

fn execute_semantic(args: &WorkSemanticArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let filter_tags = args.tag.clone();
    let records = current_work_record_index(
        &AgentProvider::all()
            .iter()
            .map(|s| s.provider)
            .collect::<Vec<_>>(),
        &cwd,
        None,
    )?;
    let results = semantic_search(records.records(), &args.query, args.limit, |record| {
        tags_match_record(&record.parts, &filter_tags)
    });
    if results.is_empty() {
        println!(
            "No semantically relevant records found for `{}`",
            args.query
        );
        return Ok(());
    }
    let anchors: Vec<WorkRef> = results.iter().map(|r| r.record_ref.clone()).collect();
    let matched = workset::records_for_anchors(records.records(), &anchors);
    let mut set = WorkSet::with_anchors(cwd.display().to_string(), matched, anchors);
    set.save_last()?;
    for result in &results {
        let record = records
            .resolve(&result.record_ref)
            .with_context(|| format!("Record not found: {}", result.record_ref))?;
        eprintln!(
            "{}  [{:<5}]  (score: {}, terms: {})",
            result.record_ref,
            record.kind_label(),
            result.score,
            result.matched_terms.join(", "),
        );
    }
    show::print_workset(&set, show::resolve_output_format(None, false, false, true))?;
    Ok(())
}

fn execute_links(args: &WorkLinksArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let records = current_work_record_index(
        &AgentProvider::all()
            .iter()
            .map(|s| s.provider)
            .collect::<Vec<_>>(),
        &cwd,
        None,
    )?;
    let mut links = records.infer_links();
    if let Some(kind) = &args.kind {
        let wanted = match kind.to_lowercase().as_str() {
            "caused_by" => WorkLinkKind::CausedBy,
            "follows_up" => WorkLinkKind::FollowsUp,
            "references" => WorkLinkKind::References,
            _ => bail!("Unknown link kind `{kind}`; expected caused_by, follows_up, or references"),
        };
        links.retain(|l| l.kind == wanted);
    }
    if let Some(ref_prefix) = &args.ref_ {
        links.retain(|l| {
            l.from.to_string().starts_with(ref_prefix) || l.to.to_string().starts_with(ref_prefix)
        });
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&links)?);
        return Ok(());
    }
    if links.is_empty() {
        println!("No causal links found.");
        return Ok(());
    }
    for link in &links {
        let kind_str = serde_json::to_value(link.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{:?}", link.kind));
        println!("{}  →  {}  [{}]", link.from, link.to, kind_str);
    }
    Ok(())
>>>>>>> f65cdcc (fix: resolve all six architecture issues)
}

fn tags_match_record(parts: &[sivtr_core::record::WorkPart], filter: &[String]) -> bool {
    if filter.is_empty() {
        return true;
    }
    parts.iter().any(|part| tags_match(filter, &part.tags))
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

fn session_group_key(record: &WorkRecord, marker: &WorkSessionMarker) -> String {
    let session_id = record
        .session
        .canonical_id
        .as_deref()
        .unwrap_or(marker.session.as_str());
    format!("{}/{}", marker.source_name(), session_id)
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

impl WorkSessionMarker {
    fn from_record(record: &WorkRecord) -> Self {
        match &record.work_ref {
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
        WorkChannel, WorkPart, WorkPartIo, WorkPartKind, WorkRecordKind, WorkSessionRef,
        WorkSource, WorkTime,
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
    fn groups_sessions_with_canonical_session_metadata() {
        let record = test_record("codex/alpha/2", "title", Some("2026-05-24T12:00:00Z"));

        let items = build_session_items(&[record]);

        assert_eq!(items[0].session_meta.display_id, "alpha");
        assert_eq!(
            items[0].session_meta.canonical_id.as_deref(),
            Some("alpha-session-0123456789abcdef")
        );
    }

    fn test_record(reference: &str, title: &str, timestamp: Option<&str>) -> WorkRecord {
        let work_ref: WorkRef = reference.parse().unwrap();
        WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
            kind: if matches!(work_ref, WorkRef::Terminal { .. }) {
                WorkRecordKind::TerminalCommand
            } else {
                WorkRecordKind::ChatTurn
            },
            source: WorkSource {
                channel: if matches!(work_ref, WorkRef::Terminal { .. }) {
                    WorkChannel::Terminal
                } else {
                    WorkChannel::Chat
                },
                provider: work_ref
                    .provider()
                    .map(|provider| provider.command_name().to_string()),
            },
            session: WorkSessionRef {
                id: work_ref.session().to_string(),
                canonical_id: Some(format!("{}-session-0123456789abcdef", work_ref.session())),
                path: None,
            },
            work_ref,
            cwd: None,
            time: WorkTime::from_components(timestamp.map(str::to_string), None, None),
            status: None,
            title: title.to_string(),
            parts: vec![
                WorkPart {
                    io: WorkPartIo::Input,
                    kind: WorkPartKind::UserMessage,
                    index: 1,
                    occurred_at: timestamp.map(str::to_string),
                    label: None,
                    text: "user prompt".to_string(),
                    ansi: None,
                    tags: Vec::new(),
                },
                WorkPart {
                    io: WorkPartIo::Output,
                    kind: WorkPartKind::AssistantMessage,
                    index: 1,
                    occurred_at: timestamp.map(str::to_string),
                    label: None,
                    text: "assistant reply".to_string(),
                    ansi: None,
                    tags: Vec::new(),
                },
            ],
        }
    }
}
