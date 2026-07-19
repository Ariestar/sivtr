use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sivtr_core::ai::{AgentProvider, AgentSessionProvider};
use sivtr_core::record::{
    WorkAt, WorkOutcome, WorkPart, WorkPartIo, WorkPartKind, WorkRecord, WorkRecordKind, WorkRef,
};

use crate::cli::{
    FilterArgs, SearchArgs, SearchFieldArg, SearchSortArg, SearchStatusArg, WorkPartFilterArg,
    WorkPartsArgs,
};
use crate::commands::memory::show;
use crate::commands::memory::time_filter::{build_time_range, TimeRange};
use crate::commands::memory::workset::{self, WorkSet};

/// Default search bound when neither `--latest` nor `--limit` is set.
const SEARCH_DEFAULT_LATEST: usize = 5;

struct MatchedAnchor<'a> {
    record: &'a WorkRecord,
    anchor: WorkRef,
    sort_ref: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FilterMode {
    #[default]
    Anchors,
    Parts,
}

/// Unified filter for local and remote query. One type for CLI and wire.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    #[serde(default)]
    mode: FilterMode,
    /// Uncompiled pattern; applied with `(?i)` when matching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    match_regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exclude_regex: Option<String>,
    #[serde(default)]
    in_field: SearchFieldArg,
    #[serde(default)]
    io: WorkPartFilterArg,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<crate::cli::WorkPartKindArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<SearchStatusArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    until: Option<String>,
    /// Client-only; forced false when applied on a remote peer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    exclude_current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pre_sort: Option<SearchSortArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    post_sort: Option<SearchSortArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
}

impl Filter {
    pub fn from_search_args(args: &SearchArgs) -> Result<Self> {
        let mut filter = common_bounds(
            args.match_.as_deref(),
            args.exclude.as_deref(),
            args.in_field,
            args.kind,
            args.status,
            args.exit_code,
            args.min_duration.as_deref(),
            args.max_duration.as_deref(),
            args.since.as_deref(),
            args.until.as_deref(),
            args.last.as_deref(),
            args.exclude_current,
            args.latest,
            Some(args.sort),
            args.limit,
        )?;
        // Search always bounds: default latest=5 when neither latest nor limit set.
        if filter.latest.is_none() && filter.limit.is_none() {
            filter.latest = Some(SEARCH_DEFAULT_LATEST);
        }
        filter.mode = FilterMode::Anchors;
        filter.io = WorkPartFilterArg::All;
        filter.pre_sort = Some(SearchSortArg::Newest);
        if filter.post_sort.is_none() {
            filter.post_sort = Some(args.sort);
        }
        Ok(filter)
    }

    pub fn from_filter_args(args: &FilterArgs) -> Result<Self> {
        let mut filter = common_bounds(
            args.match_.as_deref(),
            args.exclude.as_deref(),
            args.in_field,
            args.kind,
            args.status,
            args.exit_code,
            args.min_duration.as_deref(),
            args.max_duration.as_deref(),
            args.since.as_deref(),
            args.until.as_deref(),
            args.last.as_deref(),
            args.exclude_current,
            args.latest,
            args.sort,
            args.limit,
        )?;
        filter.mode = if args.parts {
            FilterMode::Parts
        } else {
            FilterMode::Anchors
        };
        filter.io = args.io;
        filter.pre_sort = args.latest.map(|_| SearchSortArg::Newest);
        Ok(filter)
    }

    pub fn from_work_parts_args(args: &WorkPartsArgs) -> Result<Self> {
        Ok(Self {
            mode: FilterMode::Parts,
            match_regex: args.match_.clone(),
            exclude_regex: None,
            in_field: SearchFieldArg::Content,
            io: args.io,
            kind: args.kind,
            status: None,
            exit_code: None,
            min_duration_ms: None,
            max_duration_ms: None,
            since: None,
            until: None,
            exclude_current: false,
            pre_sort: None,
            latest: None,
            post_sort: None,
            limit: None,
        })
    }

    /// Keep every loaded anchor (show/nav/zoom).
    pub fn none() -> Self {
        Self::default()
    }

    /// Browse session list: newest-first, bounded by `latest` sessions/records after sort.
    ///
    /// Used only as a first-page / expand window for TUI catalog loads. Full-text
    /// search still uses [`Filter::from_search_args`] and remains WorkRecord-based.
    pub fn browse_session_page(latest: usize) -> Self {
        Self {
            mode: FilterMode::Anchors,
            pre_sort: Some(SearchSortArg::Newest),
            latest: Some(latest.max(1)),
            post_sort: Some(SearchSortArg::Newest),
            ..Self::default()
        }
    }

    /// Drop client-only flags before applying on a remote peer.
    pub fn for_remote_peer(&self) -> Self {
        let mut f = self.clone();
        f.exclude_current = false;
        f
    }

    fn time_range(&self) -> Result<Option<TimeRange>> {
        match (&self.since, &self.until) {
            (None, None) => Ok(None),
            (since, until) => {
                let since = since
                    .as_deref()
                    .map(|s| {
                        DateTime::parse_from_rfc3339(s)
                            .map(|dt| dt.with_timezone(&Utc))
                            .with_context(|| format!("Invalid filter.since: {s}"))
                    })
                    .transpose()?;
                let until = until
                    .as_deref()
                    .map(|s| {
                        DateTime::parse_from_rfc3339(s)
                            .map(|dt| dt.with_timezone(&Utc))
                            .with_context(|| format!("Invalid filter.until: {s}"))
                    })
                    .transpose()?;
                Ok(Some(TimeRange { since, until }))
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn common_bounds(
    match_: Option<&str>,
    exclude: Option<&str>,
    in_field: SearchFieldArg,
    kind: Option<crate::cli::WorkPartKindArg>,
    status: Option<SearchStatusArg>,
    exit_code: Option<i32>,
    min_duration: Option<&str>,
    max_duration: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
    last: Option<&str>,
    exclude_current: bool,
    latest: Option<usize>,
    sort: Option<SearchSortArg>,
    limit: Option<usize>,
) -> Result<Filter> {
    let min_duration_ms = parse_duration_ms_filter(min_duration, "--min-duration")?;
    let max_duration_ms = parse_duration_ms_filter(max_duration, "--max-duration")?;
    validate_duration_bounds(min_duration_ms, max_duration_ms)?;
    let (time_range, _) = build_time_range(since, until, last, Utc::now())?;
    Ok(Filter {
        mode: FilterMode::Anchors,
        match_regex: match_.map(str::to_string),
        exclude_regex: exclude.map(str::to_string),
        in_field,
        io: WorkPartFilterArg::All,
        kind,
        status,
        exit_code,
        min_duration_ms,
        max_duration_ms,
        since: time_range
            .as_ref()
            .and_then(|r| r.since)
            .map(|t| t.to_rfc3339()),
        until: time_range
            .as_ref()
            .and_then(|r| r.until)
            .map(|t| t.to_rfc3339()),
        exclude_current,
        pre_sort: None,
        latest,
        post_sort: sort,
        limit,
    })
}

pub fn execute(args: &FilterArgs) -> Result<()> {
    let set = run(args)?;
    show::print_workset(
        &set,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

/// Load, filter, and optionally save a WorkSet without printing.
pub fn run(args: &FilterArgs) -> Result<WorkSet> {
    let mut set = workset::query(
        &args.source,
        Filter::from_filter_args(args)?,
        args.cwd.as_deref(),
    )?;
    set.save_last()?;
    if let Some(name) = args.save.as_deref() {
        set.save_as(name)?;
    }
    Ok(set)
}

pub(crate) fn apply(
    cwd: PathBuf,
    records: Vec<WorkRecord>,
    anchors: Vec<WorkRef>,
    filter: Filter,
) -> Result<WorkSet> {
    let regex = compile_regex(filter.match_regex.as_deref())?;
    let exclude_regex = compile_regex(filter.exclude_regex.as_deref())?;
    let time_range = filter.time_range()?;
    let providers = providers_for_records(&records);
    let excluded_sessions = if filter.exclude_current {
        current_agent_session_paths(&providers, &cwd)?
    } else {
        HashSet::new()
    };

    let mut matches = anchors
        .iter()
        .filter_map(|anchor| {
            let record = workset::record_for_anchor(&records, anchor)?;
            Some((record, anchor))
        })
        .filter(|(record, _)| {
            record_matches_metadata(record, &filter, time_range.as_ref(), &excluded_sessions)
        })
        .flat_map(|(record, anchor)| matching_anchors(record, anchor, &filter, regex.as_ref()))
        .filter(|matched| !match_excluded(matched, exclude_regex.as_ref()))
        .collect::<Vec<_>>();

    if let Some(sort) = filter.pre_sort {
        sort_results(&mut matches, sort);
    }
    let mut anchors = dedup_matches(matches);
    if let Some(latest) = filter.latest {
        anchors.truncate(latest);
    }
    if let Some(sort) = filter.post_sort {
        sort_anchor_results(&mut anchors, &records, sort);
    }
    if let Some(limit) = filter.limit {
        anchors.truncate(limit);
    }

    let selected_records = workset::records_for_anchors(&records, &anchors);
    Ok(WorkSet::with_anchors(
        cwd.display().to_string(),
        selected_records,
        anchors,
    ))
}

fn providers_for_records(records: &[WorkRecord]) -> Vec<AgentProvider> {
    let mut providers = Vec::new();
    for record in records {
        if let Some(provider) = record.work_ref.provider() {
            if !providers.contains(&provider) {
                providers.push(provider);
            }
        }
    }
    providers
}

fn record_matches_metadata(
    record: &WorkRecord,
    filter: &Filter,
    time_range: Option<&TimeRange>,
    excluded_sessions: &HashSet<PathBuf>,
) -> bool {
    !excluded_session_matches(record, excluded_sessions)
        && status_matches(
            filter.status,
            record
                .status
                .as_ref()
                .map(|status| status.outcome)
                .unwrap_or(WorkOutcome::Unknown),
        )
        && exit_code_matches(
            filter.exit_code,
            record.status.as_ref().and_then(|status| status.exit_code),
        )
        && duration_matches(
            filter.min_duration_ms,
            filter.max_duration_ms,
            record.time.duration_ms,
        )
        && time_range.is_none_or(|range| range.contains_record_time(record.time.primary_at()))
}

fn status_matches(status: Option<SearchStatusArg>, outcome: WorkOutcome) -> bool {
    match status {
        Some(SearchStatusArg::Success) => outcome == WorkOutcome::Success,
        Some(SearchStatusArg::Failure) => outcome == WorkOutcome::Failure,
        Some(SearchStatusArg::Unknown) => outcome == WorkOutcome::Unknown,
        None => true,
    }
}

fn exit_code_matches(expected: Option<i32>, actual: Option<i32>) -> bool {
    expected.is_none_or(|expected| actual == Some(expected))
}

fn duration_matches(min: Option<u64>, max: Option<u64>, actual: Option<u64>) -> bool {
    if min.is_none() && max.is_none() {
        return true;
    }

    let Some(actual) = actual else {
        return false;
    };

    min.is_none_or(|min| actual >= min) && max.is_none_or(|max| actual <= max)
}

fn compile_regex(value: Option<&str>) -> Result<Option<Regex>> {
    value
        .map(|query| Regex::new(&format!("(?i){query}")))
        .transpose()
        .context("Invalid filter regex")
}

fn validate_duration_bounds(min: Option<u64>, max: Option<u64>) -> Result<()> {
    if let (Some(min), Some(max)) = (min, max) {
        if min > max {
            bail!("--min-duration must be less than or equal to --max-duration");
        }
    }
    Ok(())
}

fn parse_duration_ms_filter(value: Option<&str>, label: &str) -> Result<Option<u64>> {
    value
        .map(|value| parse_duration_ms(value).with_context(|| format!("Invalid {label}: {value}")))
        .transpose()
}

fn parse_duration_ms(value: &str) -> Result<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("duration is empty");
    }

    let number_end = trimmed
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()
        .ok_or_else(|| anyhow::anyhow!("duration must start with a number"))?;
    let amount = trimmed[..number_end]
        .parse::<u64>()
        .context("duration amount must be an unsigned integer")?;
    let unit = trimmed[number_end..].trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "ms" | "msec" | "msecs" | "millisecond" | "milliseconds" => 1,
        "s" | "sec" | "secs" | "second" | "seconds" => 1_000,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600_000,
        _ => bail!("unsupported duration unit `{unit}`"),
    };
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("duration is too large"))
}

fn matching_anchors<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    filter: &Filter,
    regex: Option<&Regex>,
) -> Vec<MatchedAnchor<'a>> {
    match filter.mode {
        FilterMode::Anchors => match anchor.at {
            WorkAt::Whole => record_anchor_matches(record, anchor, filter, regex),
            WorkAt::Line(line) => line_anchor_matches(record, anchor, filter, regex, line),
            WorkAt::Part { .. } => part_anchor_matches(record, anchor, filter, regex),
        },
        FilterMode::Parts => part_anchors_for(record, anchor, filter, regex),
    }
}

fn record_anchor_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    filter: &Filter,
    regex: Option<&Regex>,
) -> Vec<MatchedAnchor<'a>> {
    if matches!(
        filter.in_field,
        SearchFieldArg::Title | SearchFieldArg::Session
    ) {
        return (filter.kind.is_none() && meta_matches(record, filter.in_field, regex))
            .then(|| matched(record, anchor.clone()))
            .into_iter()
            .collect();
    }

    let matched_meta = filter.kind.is_none()
        && filter.in_field == SearchFieldArg::All
        && meta_matches(record, SearchFieldArg::All, regex);
    let matched_part = record
        .parts
        .iter()
        .any(|part| part_matches_filters(part, filter, regex));
    (matched_meta || matched_part)
        .then(|| matched(record, anchor.clone()))
        .into_iter()
        .collect()
}

fn line_anchor_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    filter: &Filter,
    regex: Option<&Regex>,
    line: usize,
) -> Vec<MatchedAnchor<'a>> {
    let Some(text) = record.content_for_at(WorkAt::Line(line)) else {
        return Vec::new();
    };
    if matches!(
        filter.in_field,
        SearchFieldArg::Title | SearchFieldArg::Session
    ) {
        return (filter.kind.is_none() && meta_matches(record, filter.in_field, regex))
            .then(|| matched(record, anchor.clone()))
            .into_iter()
            .collect();
    }
    regex
        .is_none_or(|regex| regex.is_match(&text))
        .then(|| matched(record, anchor.clone()))
        .into_iter()
        .collect()
}

fn part_anchor_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    filter: &Filter,
    regex: Option<&Regex>,
) -> Vec<MatchedAnchor<'a>> {
    let Some(part) = record.part_for_at(anchor.at) else {
        return Vec::new();
    };
    part_matches_filters(part, filter, regex)
        .then(|| matched(record, anchor.clone()))
        .into_iter()
        .collect()
}

fn part_anchors_for<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    filter: &Filter,
    regex: Option<&Regex>,
) -> Vec<MatchedAnchor<'a>> {
    match anchor.at {
        WorkAt::Part { .. } => part_anchor_matches(record, anchor, filter, regex),
        WorkAt::Whole | WorkAt::Line(_) => record
            .parts
            .iter()
            .filter(|part| part_matches_filters(part, filter, regex))
            .map(|part| matched(record, record.work_ref.with_part(part.io, part.index)))
            .collect(),
    }
}

fn matched(record: &WorkRecord, anchor: WorkRef) -> MatchedAnchor<'_> {
    MatchedAnchor {
        record,
        sort_ref: anchor.to_string(),
        anchor,
    }
}

fn part_matches_filters(part: &WorkPart, filter: &Filter, regex: Option<&Regex>) -> bool {
    if !filter.io.matches(part.io) {
        return false;
    }
    if filter.kind.is_some_and(|kind| !kind.matches(part.kind)) {
        return false;
    }
    if !part_field_matches(part, filter.in_field) {
        return false;
    }
    // Default content search is dialogue-only so tools/skills/thinking don't pollute hits.
    // Opt in with `-i all`, or target them with `--kind tool_call|skill|thinking|…`.
    if matches!(filter.in_field, SearchFieldArg::Content)
        && filter.kind.is_none()
        && part.kind.is_structure()
    {
        return false;
    }
    regex.is_none_or(|regex| regex.is_match(&part.text))
}

fn part_field_matches(part: &WorkPart, field: SearchFieldArg) -> bool {
    matches!(field, SearchFieldArg::Content | SearchFieldArg::All)
        || matches!(field, SearchFieldArg::Input) && part.io == WorkPartIo::Input
        || matches!(field, SearchFieldArg::Output) && part.io == WorkPartIo::Output
        || matches!(field, SearchFieldArg::Command) && part.kind == WorkPartKind::Command
}

fn meta_matches(record: &WorkRecord, field: SearchFieldArg, regex: Option<&Regex>) -> bool {
    match field {
        SearchFieldArg::Title => regex.is_none_or(|regex| regex.is_match(&record.title)),
        SearchFieldArg::Session => {
            regex.is_none_or(|regex| regex.is_match(record.work_ref.session()))
        }
        SearchFieldArg::All => regex.is_none_or(|regex| {
            regex.is_match(&record.title) || regex.is_match(record.work_ref.session())
        }),
        SearchFieldArg::Content
        | SearchFieldArg::Input
        | SearchFieldArg::Output
        | SearchFieldArg::Command => false,
    }
}

fn match_excluded(matched: &MatchedAnchor<'_>, regex: Option<&Regex>) -> bool {
    let Some(regex) = regex else {
        return false;
    };

    match matched.anchor.at {
        WorkAt::Whole => matched
            .record
            .parts
            .iter()
            .any(|part| regex.is_match(&part.text)),
        WorkAt::Line(_) | WorkAt::Part { .. } => matched
            .record
            .content_for_at(matched.anchor.at)
            .is_some_and(|text| regex.is_match(&text)),
    }
}

fn sort_results(results: &mut [MatchedAnchor<'_>], sort: SearchSortArg) {
    match sort {
        SearchSortArg::Newest => results.sort_by(|a, b| {
            b.record
                .time
                .primary_at()
                .cmp(&a.record.time.primary_at())
                .then_with(|| a.sort_ref.cmp(&b.sort_ref))
        }),
        SearchSortArg::Oldest => results.sort_by(|a, b| {
            a.record
                .time
                .primary_at()
                .cmp(&b.record.time.primary_at())
                .then_with(|| a.sort_ref.cmp(&b.sort_ref))
        }),
        SearchSortArg::Duration => results.sort_by(|a, b| {
            b.record
                .time
                .duration_ms
                .cmp(&a.record.time.duration_ms)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.sort_ref.cmp(&b.sort_ref))
        }),
        SearchSortArg::DurationAsc => results.sort_by(|a, b| {
            a.record
                .time
                .duration_ms
                .cmp(&b.record.time.duration_ms)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.sort_ref.cmp(&b.sort_ref))
        }),
        SearchSortArg::ExitCode => results.sort_by(|a, b| {
            b.record
                .status
                .as_ref()
                .and_then(|status| status.exit_code)
                .cmp(&a.record.status.as_ref().and_then(|status| status.exit_code))
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.sort_ref.cmp(&b.sort_ref))
        }),
        SearchSortArg::ExitCodeAsc => results.sort_by(|a, b| {
            a.record
                .status
                .as_ref()
                .and_then(|status| status.exit_code)
                .cmp(&b.record.status.as_ref().and_then(|status| status.exit_code))
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.sort_ref.cmp(&b.sort_ref))
        }),
    }
}

fn dedup_matches(matches: Vec<MatchedAnchor<'_>>) -> Vec<WorkRef> {
    let mut anchors = Vec::new();
    for matched in matches {
        if !anchors.contains(&matched.anchor) {
            anchors.push(matched.anchor);
        }
    }
    anchors
}

fn sort_anchor_results(anchors: &mut [WorkRef], records: &[WorkRecord], sort: SearchSortArg) {
    anchors.sort_by(|a, b| {
        let left = workset::record_for_anchor(records, a);
        let right = workset::record_for_anchor(records, b);
        match sort {
            SearchSortArg::Newest => right
                .and_then(|record| record.time.primary_at())
                .cmp(&left.and_then(|record| record.time.primary_at()))
                .then_with(|| a.to_string().cmp(&b.to_string())),
            SearchSortArg::Oldest => left
                .and_then(|record| record.time.primary_at())
                .cmp(&right.and_then(|record| record.time.primary_at()))
                .then_with(|| a.to_string().cmp(&b.to_string())),
            SearchSortArg::Duration => right
                .and_then(|record| record.time.duration_ms)
                .cmp(&left.and_then(|record| record.time.duration_ms))
                .then_with(|| a.to_string().cmp(&b.to_string())),
            SearchSortArg::DurationAsc => left
                .and_then(|record| record.time.duration_ms)
                .cmp(&right.and_then(|record| record.time.duration_ms))
                .then_with(|| a.to_string().cmp(&b.to_string())),
            SearchSortArg::ExitCode => {
                right
                    .and_then(|record| record.status.as_ref().and_then(|status| status.exit_code))
                    .cmp(&left.and_then(|record| {
                        record.status.as_ref().and_then(|status| status.exit_code)
                    }))
                    .then_with(|| a.to_string().cmp(&b.to_string()))
            }
            SearchSortArg::ExitCodeAsc => {
                left.and_then(|record| record.status.as_ref().and_then(|status| status.exit_code))
                    .cmp(&right.and_then(|record| {
                        record.status.as_ref().and_then(|status| status.exit_code)
                    }))
                    .then_with(|| a.to_string().cmp(&b.to_string()))
            }
        }
    });
}

fn current_agent_session_paths(
    providers: &[AgentProvider],
    cwd: &Path,
) -> Result<HashSet<PathBuf>> {
    let mut paths = HashSet::new();

    for provider in providers {
        let source = provider.session_provider();
        if let Some(path) = current_agent_session_path(source.as_ref(), *provider, cwd)? {
            paths.insert(comparable_path(&path));
        }
    }

    Ok(paths)
}

fn current_agent_session_path(
    source: &dyn AgentSessionProvider,
    provider: AgentProvider,
    cwd: &Path,
) -> Result<Option<PathBuf>> {
    if let Some(path) = current_agent_transcript_path(provider) {
        return Ok(Some(path));
    }

    if let Some(session_id) = current_agent_session_id(provider) {
        if let Some(path) = source.find_session_by_id(&session_id)? {
            return Ok(Some(path));
        }
    }

    source.find_current_session(cwd)
}

fn current_agent_transcript_path(provider: AgentProvider) -> Option<PathBuf> {
    let env_name = provider.current_transcript_env()?;
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn current_agent_session_id(provider: AgentProvider) -> Option<String> {
    let env_name = provider.current_session_id_env()?;
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn excluded_session_matches(record: &WorkRecord, excluded_sessions: &HashSet<PathBuf>) -> bool {
    if excluded_sessions.is_empty() || record.kind != WorkRecordKind::ChatTurn {
        return false;
    }

    record
        .session
        .path
        .as_deref()
        .map(Path::new)
        .map(comparable_path)
        .is_some_and(|path| excluded_sessions.contains(&path))
}

fn comparable_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duration_ms() {
        assert_eq!(parse_duration_ms("500ms").expect("parse"), 500);
        assert_eq!(parse_duration_ms("2s").expect("parse"), 2_000);
        assert_eq!(parse_duration_ms("3m").expect("parse"), 180_000);
        assert_eq!(parse_duration_ms("1h").expect("parse"), 3_600_000);
    }

    #[test]
    fn rejects_bad_duration() {
        assert!(parse_duration_ms("").is_err());
        assert!(parse_duration_ms("ms").is_err());
        assert!(parse_duration_ms("1d").is_err());
    }

    #[test]
    fn search_defaults_to_latest_five_when_unbounded() {
        let args = SearchArgs {
            source: "terminal".into(),
            match_: None,
            exclude: None,
            in_field: SearchFieldArg::Content,
            kind: None,
            status: None,
            exit_code: None,
            min_duration: None,
            max_duration: None,
            sort: SearchSortArg::Newest,
            cwd: None,
            since: None,
            until: None,
            last: None,
            latest: None,
            limit: None,
            exclude_current: false,
            format: None,
            json: false,
            refs: false,
            save: None,
        };
        let spec = Filter::from_search_args(&args).expect("spec");
        assert_eq!(spec.latest, Some(SEARCH_DEFAULT_LATEST));
        assert_eq!(spec.limit, None);
    }

    #[test]
    fn search_keeps_explicit_limit_without_forcing_latest() {
        let args = SearchArgs {
            source: "terminal".into(),
            match_: None,
            exclude: None,
            in_field: SearchFieldArg::Content,
            kind: None,
            status: None,
            exit_code: None,
            min_duration: None,
            max_duration: None,
            sort: SearchSortArg::Newest,
            cwd: None,
            since: None,
            until: None,
            last: None,
            latest: None,
            limit: Some(12),
            exclude_current: false,
            format: None,
            json: false,
            refs: false,
            save: None,
        };
        let spec = Filter::from_search_args(&args).expect("spec");
        assert_eq!(spec.latest, None);
        assert_eq!(spec.limit, Some(12));
    }

    #[test]
    fn search_keeps_explicit_latest_and_limit() {
        let args = SearchArgs {
            source: "terminal".into(),
            match_: None,
            exclude: None,
            in_field: SearchFieldArg::Content,
            kind: None,
            status: None,
            exit_code: None,
            min_duration: None,
            max_duration: None,
            sort: SearchSortArg::Newest,
            cwd: None,
            since: None,
            until: None,
            last: None,
            latest: Some(3),
            limit: Some(10),
            exclude_current: false,
            format: None,
            json: false,
            refs: false,
            save: None,
        };
        let spec = Filter::from_search_args(&args).expect("spec");
        assert_eq!(spec.latest, Some(3));
        assert_eq!(spec.limit, Some(10));
    }
}
