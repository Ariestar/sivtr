use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use regex::Regex;
use sivtr_core::ai::{AgentProvider, AgentSessionProvider};
use sivtr_core::record::{
    WorkOutcome, WorkPart, WorkPartIo, WorkPartKind, WorkRecord, WorkRecordKind, WorkRef,
    WorkRefTarget,
};

use crate::cli::{
    FilterArgs, SearchArgs, SearchFieldArg, SearchSortArg, SearchStatusArg, WorkPartFilterArg,
    WorkPartsArgs,
};
use crate::commands::memory::show;
use crate::commands::memory::time_filter::{build_time_range, TimeRange};
use crate::commands::memory::workset::{self, WorkSet, WorkSetSource};

struct MatchedAnchor<'a> {
    record: &'a WorkRecord,
    anchor: WorkRef,
    sort_ref: String,
}

#[derive(Clone, Copy)]
enum FilterMode {
    Anchors,
    Parts,
}

pub(crate) struct FilterSpec {
    mode: FilterMode,
    regex: Option<Regex>,
    exclude_regex: Option<Regex>,
    in_field: SearchFieldArg,
    io: WorkPartFilterArg,
    kind: Option<crate::cli::WorkPartKindArg>,
    status: Option<SearchStatusArg>,
    exit_code: Option<i32>,
    min_duration_ms: Option<u64>,
    max_duration_ms: Option<u64>,
    time_range: Option<TimeRange>,
    exclude_current: bool,
    pre_sort: Option<SearchSortArg>,
    latest: Option<usize>,
    post_sort: Option<SearchSortArg>,
    limit: Option<usize>,
}

impl FilterSpec {
    pub(crate) fn from_search_args(args: &SearchArgs) -> Result<Self> {
        let min_duration_ms =
            parse_duration_ms_filter(args.min_duration.as_deref(), "--min-duration")?;
        let max_duration_ms =
            parse_duration_ms_filter(args.max_duration.as_deref(), "--max-duration")?;
        validate_duration_bounds(min_duration_ms, max_duration_ms)?;
        let (time_range, _) = build_time_range(
            args.since.as_deref(),
            args.until.as_deref(),
            args.last.as_deref(),
            Utc::now(),
        )?;
        Ok(Self {
            mode: FilterMode::Anchors,
            regex: compile_regex(args.match_.as_deref())?,
            exclude_regex: compile_regex(args.exclude.as_deref())?,
            in_field: args.in_field,
            io: WorkPartFilterArg::All,
            kind: args.kind,
            status: args.status,
            exit_code: args.exit_code,
            min_duration_ms,
            max_duration_ms,
            time_range,
            exclude_current: args.exclude_current,
            pre_sort: Some(SearchSortArg::Newest),
            latest: args.latest,
            post_sort: Some(args.sort),
            limit: args.limit.or_else(|| args.latest.is_none().then_some(20)),
        })
    }

    pub(crate) fn from_filter_args(args: &FilterArgs) -> Result<Self> {
        let min_duration_ms =
            parse_duration_ms_filter(args.min_duration.as_deref(), "--min-duration")?;
        let max_duration_ms =
            parse_duration_ms_filter(args.max_duration.as_deref(), "--max-duration")?;
        validate_duration_bounds(min_duration_ms, max_duration_ms)?;
        let (time_range, _) = build_time_range(
            args.since.as_deref(),
            args.until.as_deref(),
            args.last.as_deref(),
            Utc::now(),
        )?;
        Ok(Self {
            mode: if args.parts {
                FilterMode::Parts
            } else {
                FilterMode::Anchors
            },
            regex: compile_regex(args.match_.as_deref())?,
            exclude_regex: compile_regex(args.exclude.as_deref())?,
            in_field: args.in_field,
            io: args.io,
            kind: args.kind,
            status: args.status,
            exit_code: args.exit_code,
            min_duration_ms,
            max_duration_ms,
            time_range,
            exclude_current: args.exclude_current,
            pre_sort: args.latest.map(|_| SearchSortArg::Newest),
            latest: args.latest,
            post_sort: args.sort,
            limit: args.limit,
        })
    }

    pub(crate) fn from_work_parts_args(args: &WorkPartsArgs) -> Result<Self> {
        Ok(Self {
            mode: FilterMode::Parts,
            regex: compile_regex(args.match_.as_deref())?,
            exclude_regex: None,
            in_field: SearchFieldArg::Content,
            io: args.io,
            kind: args.kind,
            status: None,
            exit_code: None,
            min_duration_ms: None,
            max_duration_ms: None,
            time_range: None,
            exclude_current: false,
            pre_sort: None,
            latest: None,
            post_sort: None,
            limit: None,
        })
    }
}

pub fn execute(args: &FilterArgs) -> Result<()> {
    let source = workset::load_source(&args.source, args.cwd.as_deref())?;
    let mut set = apply_source(source, FilterSpec::from_filter_args(args)?)?;
    set.save_last()?;
    if let Some(name) = args.save.as_deref() {
        set.save_as(name)?;
    }
    show::print_workset(
        &set,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

pub(crate) fn apply_source(source: WorkSetSource, spec: FilterSpec) -> Result<WorkSet> {
    let cwd = source.cwd();
    let (records, anchors) = source.into_parts();
    apply_parts(cwd, records, anchors, spec)
}

pub(crate) fn apply_parts(
    cwd: PathBuf,
    records: Vec<WorkRecord>,
    anchors: Vec<WorkRef>,
    spec: FilterSpec,
) -> Result<WorkSet> {
    let providers = providers_for_records(&records);
    let excluded_sessions = if spec.exclude_current {
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
        .filter(|(record, _)| record_matches_metadata(record, &spec, &excluded_sessions))
        .flat_map(|(record, anchor)| matching_anchors(record, anchor, &spec))
        .filter(|matched| !match_excluded(matched, spec.exclude_regex.as_ref()))
        .collect::<Vec<_>>();

    if let Some(sort) = spec.pre_sort {
        sort_results(&mut matches, sort);
    }
    let mut anchors = dedup_matches(matches);
    if let Some(latest) = spec.latest {
        anchors.truncate(latest);
    }
    if let Some(sort) = spec.post_sort {
        sort_anchor_results(&mut anchors, &records, sort);
    }
    if let Some(limit) = spec.limit {
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
    spec: &FilterSpec,
    excluded_sessions: &HashSet<PathBuf>,
) -> bool {
    !excluded_session_matches(record, excluded_sessions)
        && status_matches(
            spec.status,
            record
                .status
                .as_ref()
                .map(|status| status.outcome)
                .unwrap_or(WorkOutcome::Unknown),
        )
        && exit_code_matches(
            spec.exit_code,
            record.status.as_ref().and_then(|status| status.exit_code),
        )
        && duration_matches(
            spec.min_duration_ms,
            spec.max_duration_ms,
            record.time.duration_ms,
        )
        && spec
            .time_range
            .as_ref()
            .is_none_or(|range| range.contains_record_time(record.time.primary_at()))
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
    spec: &FilterSpec,
) -> Vec<MatchedAnchor<'a>> {
    match spec.mode {
        FilterMode::Anchors => match anchor.target() {
            WorkRefTarget::Record => record_anchor_matches(record, anchor, spec),
            WorkRefTarget::Line(line) => line_anchor_matches(record, anchor, spec, line),
            WorkRefTarget::Part { .. } => part_anchor_matches(record, anchor, spec),
        },
        FilterMode::Parts => part_anchors_for(record, anchor, spec),
    }
}

fn record_anchor_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    spec: &FilterSpec,
) -> Vec<MatchedAnchor<'a>> {
    if matches!(
        spec.in_field,
        SearchFieldArg::Title | SearchFieldArg::Session
    ) {
        return (spec.kind.is_none() && meta_matches(record, spec.in_field, spec.regex.as_ref()))
            .then(|| matched(record, anchor.clone()))
            .into_iter()
            .collect();
    }

    let matched_meta = spec.kind.is_none()
        && spec.in_field == SearchFieldArg::All
        && meta_matches(record, SearchFieldArg::All, spec.regex.as_ref());
    let matched_part = record
        .parts
        .iter()
        .any(|part| part_matches_filters(part, spec));
    (matched_meta || matched_part)
        .then(|| matched(record, anchor.clone()))
        .into_iter()
        .collect()
}

fn line_anchor_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    spec: &FilterSpec,
    line: usize,
) -> Vec<MatchedAnchor<'a>> {
    let Some(text) = record.content_for_target(WorkRefTarget::Line(line)) else {
        return Vec::new();
    };
    if matches!(
        spec.in_field,
        SearchFieldArg::Title | SearchFieldArg::Session
    ) {
        return (spec.kind.is_none() && meta_matches(record, spec.in_field, spec.regex.as_ref()))
            .then(|| matched(record, anchor.clone()))
            .into_iter()
            .collect();
    }
    spec.regex
        .as_ref()
        .is_none_or(|regex| regex.is_match(&text))
        .then(|| matched(record, anchor.clone()))
        .into_iter()
        .collect()
}

fn part_anchor_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    spec: &FilterSpec,
) -> Vec<MatchedAnchor<'a>> {
    let Some(part) = record.part_for_target(anchor.target()) else {
        return Vec::new();
    };
    part_matches_filters(part, spec)
        .then(|| matched(record, anchor.clone()))
        .into_iter()
        .collect()
}

fn part_anchors_for<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    spec: &FilterSpec,
) -> Vec<MatchedAnchor<'a>> {
    match anchor.target() {
        WorkRefTarget::Part { .. } => part_anchor_matches(record, anchor, spec),
        WorkRefTarget::Record | WorkRefTarget::Line(_) => record
            .parts
            .iter()
            .filter(|part| part_matches_filters(part, spec))
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

fn part_matches_filters(part: &WorkPart, spec: &FilterSpec) -> bool {
    if !spec.io.matches(part.io) {
        return false;
    }
    if spec.kind.is_some_and(|kind| !kind.matches(part.kind)) {
        return false;
    }
    if !part_field_matches(part, spec.in_field) {
        return false;
    }
    spec.regex
        .as_ref()
        .is_none_or(|regex| regex.is_match(&part.text))
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

    match matched.anchor.target() {
        WorkRefTarget::Record => matched
            .record
            .parts
            .iter()
            .any(|part| regex.is_match(&part.text)),
        WorkRefTarget::Line(_) | WorkRefTarget::Part { .. } => matched
            .record
            .content_for_target(matched.anchor.target())
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
}
