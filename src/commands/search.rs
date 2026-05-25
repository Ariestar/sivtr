use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use regex::Regex;
use serde::Serialize;
use sivtr_core::ai::{AgentProvider, AgentSessionProvider};
use sivtr_core::record::{RecordTextMode, WorkOutcome, WorkRecord, WorkRecordKind, WorkRef};

use crate::cli::{
    SearchArgs, SearchFieldArg, SearchOutputFormatArg, SearchSortArg, SearchStatusArg,
};
use crate::commands::records::current_work_record_index;
use crate::commands::time_filter::build_time_range;

#[derive(Serialize)]
struct SearchJsonOutput<'a> {
    target: &'a str,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    match_: Option<&'a str>,
    field: &'static str,
    sort: &'static str,
    cwd: String,
    count: usize,
    results: Vec<SearchJsonItem>,
}

#[derive(Serialize)]
struct SearchJsonMatch {
    #[serde(rename = "ref")]
    ref_: String,
    snippet: String,
}

#[derive(Serialize)]
struct SearchJsonItem {
    #[serde(rename = "ref")]
    ref_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_ref: Option<String>,
    timestamp: Option<String>,
    dialogue: String,
    status: WorkOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    matches: Vec<SearchJsonMatch>,
}

struct SearchResultGroup<'a> {
    record: &'a WorkRecord,
    ref_: String,
    matches: Vec<SearchLineMatch>,
}

#[derive(Clone)]
struct SearchLineMatch {
    ref_: String,
    snippet: String,
}

struct SearchMatch<'a> {
    record: &'a WorkRecord,
    ref_: String,
    snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchTarget {
    source: SearchSource,
    session: Option<String>,
    record_index: Option<usize>,
    line_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SearchSource {
    Terminal,
    Agent(Option<AgentProvider>),
}

pub fn execute(args: &SearchArgs) -> Result<()> {
    let cwd = args
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let target = parse_target(&args.target)?;
    let providers = target.providers();
    let now = Utc::now();
    let (time_range, _) = build_time_range(
        args.since.as_deref(),
        args.until.as_deref(),
        args.last.as_deref(),
        now,
    )?;
    let records = current_work_record_index(&providers, &cwd, None)?;
    let excluded_sessions = if args.exclude_current {
        current_agent_session_paths(&providers, &cwd)?
    } else {
        HashSet::new()
    };
    let regex = args
        .match_
        .as_deref()
        .map(|query| Regex::new(&format!("(?i){query}")))
        .transpose()?;
    let exclude_regex = args
        .exclude
        .as_deref()
        .map(|query| Regex::new(&format!("(?i){query}")))
        .transpose()?;
    let min_duration_ms = parse_duration_ms_filter(args.min_duration.as_deref(), "--min-duration")?;
    let max_duration_ms = parse_duration_ms_filter(args.max_duration.as_deref(), "--max-duration")?;
    if let (Some(min), Some(max)) = (min_duration_ms, max_duration_ms) {
        if min > max {
            bail!("--min-duration must be less than or equal to --max-duration");
        }
    }
    let mut matches = records
        .records()
        .iter()
        .filter(|record| {
            target.matches(record)
                && !excluded_session_matches(record, &excluded_sessions)
                && status_matches(args.status, record.status.outcome)
                && exit_code_matches(args.exit_code, record.status.exit_code)
                && duration_matches(min_duration_ms, max_duration_ms, record.time.duration_ms)
                && time_range
                    .as_ref()
                    .is_none_or(|range| range.contains_record_time(record.time.primary_at()))
        })
        .flat_map(|record| matching_refs(record, &target, args.in_field, regex.as_ref()))
        .filter(|matched| !match_excluded(matched, args.in_field, exclude_regex.as_ref()))
        .collect::<Vec<_>>();
    sort_results(&mut matches, SearchSortArg::Newest);
    let mut results = group_results(matches);
    if let Some(latest) = args.latest {
        results.truncate(latest);
    }
    sort_group_results(&mut results, args.sort);
    if let Some(limit) = args.limit.or_else(|| args.latest.is_none().then_some(20)) {
        results.truncate(limit);
    }

    match search_format(args) {
        SearchOutputFormatArg::Json => {
            let json = SearchJsonOutput {
                target: &args.target,
                match_: args.match_.as_deref(),
                field: field_name(args.in_field),
                sort: sort_name(args.sort),
                cwd: cwd.display().to_string(),
                count: results.len(),
                results: results.into_iter().map(search_json_item).collect(),
            };
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        SearchOutputFormatArg::Compact => print_compact_results(&results),
        SearchOutputFormatArg::Timeline => print_timeline_results(&results),
        SearchOutputFormatArg::Md => print_markdown_results(&results),
        SearchOutputFormatArg::Refs => print_ref_results(&results),
    }

    Ok(())
}

fn search_format(args: &SearchArgs) -> SearchOutputFormatArg {
    if args.refs {
        SearchOutputFormatArg::Refs
    } else if args.json {
        SearchOutputFormatArg::Json
    } else {
        args.format
    }
}

impl SearchTarget {
    fn providers(&self) -> Vec<AgentProvider> {
        match self.source {
            SearchSource::Terminal => Vec::new(),
            SearchSource::Agent(Some(provider)) => vec![provider],
            SearchSource::Agent(None) => AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect(),
        }
    }

    fn matches(&self, record: &WorkRecord) -> bool {
        match self.source {
            SearchSource::Terminal if record.kind != WorkRecordKind::TerminalCommand => {
                return false;
            }
            SearchSource::Agent(Some(provider)) => {
                if record.kind != WorkRecordKind::ChatTurn
                    || record.work_ref.provider() != Some(provider)
                {
                    return false;
                }
            }
            SearchSource::Agent(None) if record.kind != WorkRecordKind::ChatTurn => {
                return false;
            }
            _ => {}
        }

        let work_ref = &record.work_ref;
        if let (
            Some(expected),
            WorkRef::Terminal { session, .. } | WorkRef::Agent { session, .. },
        ) = (self.session.as_deref(), work_ref)
        {
            if !segment_matches(expected, session) {
                return false;
            }
        }

        match (self.record_index, work_ref) {
            (
                Some(expected),
                WorkRef::Terminal { record_index, .. }
                | WorkRef::Agent {
                    turn_index: record_index,
                    ..
                },
            ) if expected != *record_index => return false,
            _ => {}
        }

        true
    }
}

fn target_line_matches(record: &WorkRecord, line_index: Option<usize>) -> bool {
    match line_index {
        Some(line) => combined_text(record).lines().nth(line - 1).is_some(),
        None => true,
    }
}

fn parse_target(target: &str) -> Result<SearchTarget> {
    let parts = target
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        bail!("search target is empty");
    }

    let source = if parts[0].eq_ignore_ascii_case("terminal") {
        SearchSource::Terminal
    } else if parts[0].eq_ignore_ascii_case("agent") {
        SearchSource::Agent(None)
    } else if let Some(provider) = AgentProvider::from_command_name(parts[0]) {
        SearchSource::Agent(Some(provider))
    } else {
        bail!("unknown search target `{target}`; expected terminal, agent, or provider name");
    };

    let session = parts
        .get(1)
        .filter(|part| **part != "*")
        .map(|part| (*part).to_string());
    let record_index = parts
        .get(2)
        .filter(|part| **part != "*")
        .map(|part| parse_one_based(part, "record", target))
        .transpose()?;
    let line_index = parts
        .get(3)
        .filter(|part| **part != "*")
        .map(|part| parse_one_based(part, "line", target))
        .transpose()?;

    if parts.len() > 4 {
        bail!("invalid search target `{target}`; expected up to four path segments");
    }

    Ok(SearchTarget {
        source,
        session,
        record_index,
        line_index,
    })
}

fn parse_one_based(value: &str, label: &str, target: &str) -> Result<usize> {
    let parsed = value.parse::<usize>().with_context(|| {
        format!("invalid search target `{target}`; {label} index must be a positive integer or *")
    })?;
    if parsed == 0 {
        bail!("invalid search target `{target}`; {label} index must be 1-based");
    }
    Ok(parsed)
}

fn segment_matches(expected: &str, actual: &str) -> bool {
    actual == expected || actual.starts_with(expected)
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

fn field_matches(record: &WorkRecord, field: SearchFieldArg, regex: &Regex) -> bool {
    match field {
        SearchFieldArg::Content => regex.is_match(&combined_text(record)),
        SearchFieldArg::Title => regex.is_match(&record.title),
        SearchFieldArg::Session => regex.is_match(record.work_ref.session()),
        SearchFieldArg::Input => record
            .text
            .input
            .as_deref()
            .is_some_and(|text| regex.is_match(text)),
        SearchFieldArg::Output => record
            .text
            .output
            .as_deref()
            .is_some_and(|text| regex.is_match(text)),
        SearchFieldArg::Command => record
            .text
            .input
            .as_deref()
            .is_some_and(|text| regex.is_match(text)),
        SearchFieldArg::All => {
            regex.is_match(&combined_text(record))
                || regex.is_match(&record.title)
                || regex.is_match(record.work_ref.session())
        }
    }
}

fn match_excluded(matched: &SearchMatch<'_>, field: SearchFieldArg, regex: Option<&Regex>) -> bool {
    let Some(regex) = regex else {
        return false;
    };

    if regex.is_match(&matched.snippet) {
        return true;
    }

    if line_search_field(field) {
        return matched
            .ref_
            .rsplit_once('/')
            .and_then(|(_, line)| line.parse::<usize>().ok())
            .and_then(|line| {
                combined_text(matched.record)
                    .lines()
                    .nth(line - 1)
                    .map(str::to_string)
            })
            .is_some_and(|line| regex.is_match(&line));
    }

    field_matches(matched.record, field, regex)
}

fn field_name(field: SearchFieldArg) -> &'static str {
    match field {
        SearchFieldArg::Content => "content",
        SearchFieldArg::Title => "title",
        SearchFieldArg::Session => "session",
        SearchFieldArg::Input => "input",
        SearchFieldArg::Output => "output",
        SearchFieldArg::Command => "command",
        SearchFieldArg::All => "all",
    }
}

fn sort_name(sort: SearchSortArg) -> &'static str {
    match sort {
        SearchSortArg::Newest => "newest",
        SearchSortArg::Oldest => "oldest",
        SearchSortArg::Duration => "duration",
        SearchSortArg::DurationAsc => "duration-asc",
        SearchSortArg::ExitCode => "exit-code",
        SearchSortArg::ExitCodeAsc => "exit-code-asc",
    }
}

fn matching_refs<'a>(
    record: &'a WorkRecord,
    target: &SearchTarget,
    field: SearchFieldArg,
    regex: Option<&Regex>,
) -> Vec<SearchMatch<'a>> {
    if !target_line_matches(record, target.line_index) {
        return Vec::new();
    }

    let Some(regex) = regex else {
        let ref_ = target
            .line_index
            .map(|line| record.work_ref.with_line(line).to_string())
            .unwrap_or_else(|| record.work_ref.to_string());
        return vec![SearchMatch {
            record,
            ref_: ref_.clone(),
            snippet: match target.line_index {
                Some(line) => combined_text(record)
                    .lines()
                    .nth(line - 1)
                    .map(snippet)
                    .unwrap_or_default(),
                None => record_snippet(record, field),
            },
        }];
    };

    if let Some(line) = target.line_index {
        let combined = combined_text(record);
        let text = combined.lines().nth(line - 1).unwrap_or_default();
        if line_matches_field(record, field, line, text, regex) {
            return vec![SearchMatch {
                record,
                ref_: record.work_ref.with_line(line).to_string(),
                snippet: snippet(text),
            }];
        }
        return Vec::new();
    }

    if line_search_field(field) {
        let matches = combined_text(record)
            .lines()
            .enumerate()
            .filter(|(_, line)| regex.is_match(line))
            .map(|(idx, line)| SearchMatch {
                record,
                ref_: record.work_ref.with_line(idx + 1).to_string(),
                snippet: snippet(line),
            })
            .collect::<Vec<_>>();
        if !matches.is_empty() {
            return matches;
        }
    }

    if field_matches(record, field, regex) {
        return vec![SearchMatch {
            record,
            ref_: record.work_ref.to_string(),
            snippet: record_snippet(record, field),
        }];
    }

    Vec::new()
}

fn line_search_field(field: SearchFieldArg) -> bool {
    matches!(
        field,
        SearchFieldArg::Content | SearchFieldArg::Output | SearchFieldArg::All
    )
}

fn line_matches_field(
    record: &WorkRecord,
    field: SearchFieldArg,
    line_index: usize,
    line: &str,
    regex: &Regex,
) -> bool {
    match field {
        SearchFieldArg::Content | SearchFieldArg::Output | SearchFieldArg::All => {
            regex.is_match(line)
        }
        _ => {
            field_matches(record, field, regex)
                && combined_text(record).lines().nth(line_index - 1).is_some()
        }
    }
}

fn sort_results(results: &mut [SearchMatch<'_>], sort: SearchSortArg) {
    match sort {
        SearchSortArg::Newest => results.sort_by(|a, b| {
            b.record
                .time
                .primary_at()
                .cmp(&a.record.time.primary_at())
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::Oldest => results.sort_by(|a, b| {
            a.record
                .time
                .primary_at()
                .cmp(&b.record.time.primary_at())
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::Duration => results.sort_by(|a, b| {
            b.record
                .time
                .duration_ms
                .cmp(&a.record.time.duration_ms)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::DurationAsc => results.sort_by(|a, b| {
            a.record
                .time
                .duration_ms
                .cmp(&b.record.time.duration_ms)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::ExitCode => results.sort_by(|a, b| {
            b.record
                .status
                .exit_code
                .cmp(&a.record.status.exit_code)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::ExitCodeAsc => results.sort_by(|a, b| {
            a.record
                .status
                .exit_code
                .cmp(&b.record.status.exit_code)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
    }
}

fn combined_text(record: &WorkRecord) -> String {
    record.copy_text(RecordTextMode::Combined, false).plain
}

fn group_results(matches: Vec<SearchMatch<'_>>) -> Vec<SearchResultGroup<'_>> {
    let mut results: Vec<SearchResultGroup<'_>> = Vec::new();

    for matched in matches {
        let record_ref = matched.record.work_ref.record_ref().to_string();
        if let Some(group) = results.iter_mut().find(|group| group.ref_ == record_ref) {
            group.matches.push(SearchLineMatch {
                ref_: matched.ref_,
                snippet: matched.snippet,
            });
        } else {
            results.push(SearchResultGroup {
                record: matched.record,
                ref_: record_ref.clone(),
                matches: vec![SearchLineMatch {
                    ref_: matched.ref_,
                    snippet: matched.snippet,
                }],
            });
        }
    }

    results
}

fn sort_group_results(results: &mut [SearchResultGroup<'_>], sort: SearchSortArg) {
    match sort {
        SearchSortArg::Newest => results.sort_by(|a, b| {
            b.record
                .time
                .primary_at()
                .cmp(&a.record.time.primary_at())
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::Oldest => results.sort_by(|a, b| {
            a.record
                .time
                .primary_at()
                .cmp(&b.record.time.primary_at())
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::Duration => results.sort_by(|a, b| {
            b.record
                .time
                .duration_ms
                .cmp(&a.record.time.duration_ms)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::DurationAsc => results.sort_by(|a, b| {
            a.record
                .time
                .duration_ms
                .cmp(&b.record.time.duration_ms)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::ExitCode => results.sort_by(|a, b| {
            b.record
                .status
                .exit_code
                .cmp(&a.record.status.exit_code)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
        SearchSortArg::ExitCodeAsc => results.sort_by(|a, b| {
            a.record
                .status
                .exit_code
                .cmp(&b.record.status.exit_code)
                .then_with(|| b.record.time.primary_at().cmp(&a.record.time.primary_at()))
                .then_with(|| a.ref_.cmp(&b.ref_))
        }),
    }
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
        .session_path
        .as_deref()
        .map(Path::new)
        .map(comparable_path)
        .is_some_and(|path| excluded_sessions.contains(&path))
}

fn comparable_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn record_snippet(record: &WorkRecord, field: SearchFieldArg) -> String {
    let text = match field {
        SearchFieldArg::Title => record.title.as_str(),
        SearchFieldArg::Session => record.work_ref.session(),
        SearchFieldArg::Input | SearchFieldArg::Command => {
            record.text.input.as_deref().unwrap_or("")
        }
        SearchFieldArg::Output => record.text.output.as_deref().unwrap_or(""),
        SearchFieldArg::Content | SearchFieldArg::All => return first_content_snippet(record),
    };

    snippet(text)
}

fn first_content_snippet(record: &WorkRecord) -> String {
    combined_text(record)
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(snippet)
        .unwrap_or_default()
}

fn snippet(text: &str) -> String {
    const LIMIT: usize = 160;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shortened = collapsed.chars().take(LIMIT).collect::<String>();
    if collapsed.chars().count() > LIMIT {
        shortened.push('…');
    }
    shortened
}

fn search_json_item(result: SearchResultGroup<'_>) -> SearchJsonItem {
    SearchJsonItem {
        ref_: result.ref_,
        parent_ref: None,
        timestamp: result.record.time.primary_at().map(str::to_string),
        dialogue: result.record.title.clone(),
        status: result.record.status.outcome,
        exit_code: result.record.status.exit_code,
        duration_ms: result.record.time.duration_ms,
        matches: result
            .matches
            .into_iter()
            .map(|matched| SearchJsonMatch {
                ref_: matched.ref_,
                snippet: matched.snippet,
            })
            .collect(),
    }
}

fn print_compact_results(results: &[SearchResultGroup<'_>]) {
    for result in results {
        println!(
            "{}  {:<8}  {}",
            short_time(result.record),
            source_label(result.record),
            result.record.title
        );
    }
}

fn print_timeline_results(results: &[SearchResultGroup<'_>]) {
    let mut previous_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
    for result in results {
        let timestamp = result
            .record
            .time
            .primary_at()
            .and_then(sivtr_core::time::parse_timestamp);
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
            short_time(result.record),
            source_label(result.record),
            result.ref_,
            result.record.title
        );
    }
}

fn print_markdown_results(results: &[SearchResultGroup<'_>]) {
    for result in results {
        println!(
            "- **{}** `{}` {}",
            short_time(result.record),
            result.ref_,
            escape_markdown_title(&result.record.title)
        );
    }
}

fn print_ref_results(results: &[SearchResultGroup<'_>]) {
    for result in results {
        println!("{}", result.ref_);
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
        WorkRecordKind::TerminalCommand => "terminal",
        WorkRecordKind::ChatTurn => record
            .work_ref
            .provider()
            .map(|provider| provider.command_name())
            .unwrap_or("agent"),
    }
}

fn escape_markdown_title(title: &str) -> String {
    title.replace('[', "\\[").replace(']', "\\]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{WorkPayload, WorkRecordKind, WorkStatus, WorkText, WorkTime};

    #[test]
    fn parses_search_targets() {
        assert_eq!(
            parse_target("terminal/session_1/3/2").unwrap(),
            SearchTarget {
                source: SearchSource::Terminal,
                session: Some("session_1".to_string()),
                record_index: Some(3),
                line_index: Some(2),
            }
        );
        assert_eq!(
            parse_target("pi/*/*").unwrap(),
            SearchTarget {
                source: SearchSource::Agent(Some(AgentProvider::Pi)),
                session: None,
                record_index: None,
                line_index: None,
            }
        );
        assert_eq!(
            parse_target("agent").unwrap().source,
            SearchSource::Agent(None)
        );
    }

    #[test]
    fn rejects_invalid_targets() {
        assert!(parse_target("unknown").is_err());
        assert!(parse_target("pi/session/0").is_err());
        assert!(parse_target("pi/session/one").is_err());
    }

    #[test]
    fn matching_refs_returns_line_refs_for_content_matches() {
        let record = test_terminal_record("terminal/session_1/3", "alpha\nneedle\nneedle again");
        let target = parse_target("terminal/session_1/3").unwrap();
        let regex = Regex::new("needle").unwrap();

        let matches = matching_refs(&record, &target, SearchFieldArg::Content, Some(&regex));

        assert_eq!(
            matches
                .iter()
                .map(|item| item.ref_.as_str())
                .collect::<Vec<_>>(),
            vec!["terminal/session_1/3/2", "terminal/session_1/3/3"]
        );
    }

    #[test]
    fn json_matches_do_not_include_line_field() {
        let record = test_terminal_record("terminal/session_1/3", "alpha\nneedle");
        let target = parse_target("terminal/session_1/3").unwrap();
        let regex = Regex::new("needle").unwrap();
        let group = group_results(matching_refs(
            &record,
            &target,
            SearchFieldArg::Content,
            Some(&regex),
        ))
        .remove(0);

        let value = serde_json::to_value(search_json_item(group)).unwrap();

        assert!(value["matches"][0].get("line").is_none());
        assert_eq!(value["matches"][0]["ref"], "terminal/session_1/3/2");
    }

    #[test]
    fn print_ref_results_uses_record_refs() {
        let record = test_terminal_record("terminal/session_1/3", "alpha\nneedle");
        let target = parse_target("terminal/session_1/3").unwrap();
        let regex = Regex::new("needle").unwrap();
        let group = group_results(matching_refs(
            &record,
            &target,
            SearchFieldArg::Content,
            Some(&regex),
        ))
        .remove(0);

        assert_eq!(group.ref_, "terminal/session_1/3");
        assert_eq!(group.matches[0].ref_, "terminal/session_1/3/2");
    }

    #[test]
    fn target_line_segment_filters_to_specific_line() {
        let record = test_terminal_record("terminal/session_1/3", "alpha\nneedle");
        let target = parse_target("terminal/session_1/3/2").unwrap();
        let regex = Regex::new("needle").unwrap();

        let matches = matching_refs(&record, &target, SearchFieldArg::Content, Some(&regex));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].ref_, "terminal/session_1/3/2");
    }

    #[test]
    fn match_excluded_filters_matching_snippets() {
        let record =
            test_terminal_record("terminal/session_1/3", "alpha\nneedle example\nneedle real");
        let target = parse_target("terminal/session_1/3").unwrap();
        let regex = Regex::new("needle").unwrap();
        let exclude = Regex::new("example").unwrap();
        let matches = matching_refs(&record, &target, SearchFieldArg::Content, Some(&regex))
            .into_iter()
            .filter(|matched| !match_excluded(matched, SearchFieldArg::Content, Some(&exclude)))
            .collect::<Vec<_>>();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].ref_, "terminal/session_1/3/3");
        assert_eq!(matches[0].snippet, "needle real");
    }

    #[test]
    fn parses_duration_filters_as_milliseconds() {
        assert_eq!(parse_duration_ms("500").unwrap(), 500);
        assert_eq!(parse_duration_ms("2s").unwrap(), 2_000);
        assert_eq!(parse_duration_ms("3m").unwrap(), 180_000);
    }

    #[test]
    fn filters_exit_code_and_duration() {
        assert!(exit_code_matches(Some(101), Some(101)));
        assert!(!exit_code_matches(Some(101), Some(0)));
        assert!(duration_matches(Some(100), Some(200), Some(150)));
        assert!(!duration_matches(Some(100), Some(200), Some(250)));
        assert!(!duration_matches(Some(100), None, None));
    }

    fn test_terminal_record(_ref_id: &str, combined: &str) -> WorkRecord {
        WorkRecord {
            schema_version: 1,
            work_ref: WorkRef::terminal_record("session_1", 3),
            kind: WorkRecordKind::TerminalCommand,
            session_path: None,
            cwd: None,
            time: WorkTime::from_components(
                Some("2026-05-24T00:00:00Z".to_string()),
                None,
                Some(150),
            ),
            status: WorkStatus {
                outcome: WorkOutcome::Failure,
                exit_code: Some(101),
            },
            title: "cargo test".to_string(),
            text: WorkText {
                input: Some("cargo test".to_string()),
                output: Some(combined.to_string()),
            },
            payload: WorkPayload::TerminalCommand {
                prompt: String::new(),
                command: String::new(),
                output: combined.to_string(),
                prompt_ansi: None,
                output_ansi: None,
            },
        }
    }
}
