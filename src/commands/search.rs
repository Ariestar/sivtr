use anyhow::{Context, Result};
use sivtr_core::record::{semantic_search, WorkRef};

use crate::cli::{SearchArgs, SearchFieldArg, SearchSortArg};
use crate::commands::{filter, show, workset};
use crate::commands::workset::WorkSetSource;

pub fn execute(args: &SearchArgs) -> Result<()> {
    let source = workset::load_source(&args.source, args.cwd.as_deref())?;
    if args.semantic {
<<<<<<< HEAD
        return execute_semantic_search(args, source);
=======
        return execute_semantic_search(
            args,
            &records,
            time_range.as_ref(),
            min_duration_ms,
            max_duration_ms,
        );
>>>>>>> f65cdcc (fix: resolve all six architecture issues)
    }

    let mut workset = filter::apply_source(source, filter::FilterSpec::from_search_args(args)?)?;
    workset.save_last()?;
    if let Some(name) = args.save.as_deref() {
        workset.save_as(name)?;
    }
    show::print_workset(
        &workset,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

<<<<<<< HEAD
fn execute_semantic_search(args: &SearchArgs, source: WorkSetSource) -> Result<()> {
=======
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
    args: &SearchArgs,
    regex: Option<&Regex>,
) -> Vec<SearchMatch<'a>> {
    match anchor.target() {
        WorkRefTarget::Record => record_matches(record, anchor, args, regex),
        WorkRefTarget::Line(line) => line_matches(record, anchor, args, line, regex),
        WorkRefTarget::Part { .. } => part_anchor_matches(record, anchor, args, regex),
    }
}

fn record_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    args: &SearchArgs,
    regex: Option<&Regex>,
) -> Vec<SearchMatch<'a>> {
    if matches!(
        args.in_field,
        SearchFieldArg::Title | SearchFieldArg::Session
    ) {
        return (args.kind.is_none() && meta_matches(record, args.in_field, regex))
            .then(|| SearchMatch {
                record,
                anchor: anchor.clone(),
                sort_ref: anchor.to_string(),
            })
            .into_iter()
            .collect();
    }

    let matched_meta = args.kind.is_none()
        && args.in_field == SearchFieldArg::All
        && meta_matches(record, SearchFieldArg::All, regex);
    let matched_part = record.parts.iter().any(|part| {
        part_matches_filters(part, args) && regex.is_none_or(|regex| regex.is_match(&part.text))
    });
    (matched_meta || matched_part)
        .then(|| SearchMatch {
            record,
            anchor: anchor.clone(),
            sort_ref: anchor.to_string(),
        })
        .into_iter()
        .collect()
}

fn line_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    args: &SearchArgs,
    line: usize,
    regex: Option<&Regex>,
) -> Vec<SearchMatch<'a>> {
    let Some(text) = record.content_for_target(WorkRefTarget::Line(line)) else {
        return Vec::new();
    };
    if matches!(
        args.in_field,
        SearchFieldArg::Title | SearchFieldArg::Session
    ) {
        return (args.kind.is_none() && meta_matches(record, args.in_field, regex))
            .then(|| SearchMatch {
                record,
                anchor: anchor.clone(),
                sort_ref: anchor.to_string(),
            })
            .into_iter()
            .collect();
    }
    regex
        .is_none_or(|regex| regex.is_match(&text))
        .then(|| SearchMatch {
            record,
            anchor: anchor.clone(),
            sort_ref: anchor.to_string(),
        })
        .into_iter()
        .collect()
}

fn part_anchor_matches<'a>(
    record: &'a WorkRecord,
    anchor: &WorkRef,
    args: &SearchArgs,
    regex: Option<&Regex>,
) -> Vec<SearchMatch<'a>> {
    let Some(part) = record.part_for_target(anchor.target()) else {
        return Vec::new();
    };
    if !part_matches_filters(part, args) {
        return Vec::new();
    }
    regex
        .is_none_or(|regex| regex.is_match(&part.text))
        .then(|| SearchMatch {
            record,
            anchor: anchor.clone(),
            sort_ref: anchor.to_string(),
        })
        .into_iter()
        .collect()
}

fn part_matches_filters(part: &WorkPart, args: &SearchArgs) -> bool {
    if args.kind.is_some_and(|kind| !kind.matches(part.kind)) {
        return false;
    }

    matches!(args.in_field, SearchFieldArg::Content | SearchFieldArg::All)
        || matches!(args.in_field, SearchFieldArg::Input) && part.io == WorkPartIo::Input
        || matches!(args.in_field, SearchFieldArg::Output) && part.io == WorkPartIo::Output
        || matches!(args.in_field, SearchFieldArg::Command) && part.kind == WorkPartKind::Command
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

fn match_excluded(matched: &SearchMatch<'_>, regex: Option<&Regex>) -> bool {
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

fn sort_results(results: &mut [SearchMatch<'_>], sort: SearchSortArg) {
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

fn dedup_matches(matches: Vec<SearchMatch<'_>>) -> Vec<WorkRef> {
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

fn execute_semantic_search(
    args: &SearchArgs,
    records: &[WorkRecord],
    time_range: Option<&crate::commands::time_filter::TimeRange>,
    min_duration_ms: Option<u64>,
    max_duration_ms: Option<u64>,
) -> Result<()> {
>>>>>>> f65cdcc (fix: resolve all six architecture issues)
    let query = args
        .match_
        .as_deref()
        .context("--match is required with --semantic")?;

    let mut filter_args = args.clone();
    filter_args.match_ = None;
    filter_args.exclude = None;
    filter_args.kind = None;
    filter_args.in_field = SearchFieldArg::All;
    filter_args.latest = None;
    filter_args.limit = None;
    filter_args.sort = SearchSortArg::Newest;

    let filtered = filter::apply_source(
        source,
        filter::FilterSpec::from_search_args(&filter_args)?,
    )?;
    let limit = args.limit.or(args.latest).unwrap_or(20);
<<<<<<< HEAD
    let results = semantic_search(&filtered.records, query, limit, |_| true);
=======
    let results = semantic_search(records, query, limit, |record| {
        status_matches(
            args.status,
            record
                .status
                .as_ref()
                .map(|s| s.outcome)
                .unwrap_or(WorkOutcome::Unknown),
        ) && exit_code_matches(
            args.exit_code,
            record.status.as_ref().and_then(|s| s.exit_code),
        ) && duration_matches(min_duration_ms, max_duration_ms, record.time.duration_ms)
            && time_range
                .as_ref()
                .is_none_or(|range| range.contains_record_time(record.time.primary_at()))
    });
    if args.json {
        let json_results: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "ref": r.record_ref.to_string(),
                    "score": r.score,
                    "matched_terms": r.matched_terms,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_results)?);
        return Ok(());
    }
>>>>>>> f65cdcc (fix: resolve all six architecture issues)
    if results.is_empty() {
        println!("No semantic matches for `{query}`");
        return Ok(());
    }

    let anchors: Vec<WorkRef> = results.iter().map(|result| result.record_ref.clone()).collect();
    let records = workset::records_for_anchors(&filtered.records, &anchors);
    let mut semantic = workset::WorkSet::with_anchors(filtered.cwd, records, anchors);
    semantic.save_last()?;
    if let Some(name) = args.save.as_deref() {
        semantic.save_as(name)?;
    }

    for result in &results {
        eprintln!(
            "{}  score:{}  [{}]",
            result.record_ref,
            result.score,
            result.matched_terms.join(", ")
        );
    }

    show::print_workset(
        &semantic,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}
