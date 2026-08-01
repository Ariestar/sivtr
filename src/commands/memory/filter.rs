use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use sivtr_core::record::{WorkOutcome, WorkRecord, WorkRef};
use sivtr_core::search::{Field, FilterMode, PartKind, Searcher, Sort};

use crate::cli::{FilterArgs, SearchArgs, WorkPartsArgs};
use crate::commands::memory::show;
use crate::commands::memory::time_filter::build_time_range;
use crate::commands::memory::workset::{self, WorkSet};

pub use sivtr_core::search::Filter;

/// Default search bound when neither `--latest` nor `--limit` is set.
const SEARCH_DEFAULT_LATEST: usize = 5;

/// Build the search filter: recency default, relevance rank from `--match`.
///
/// A `--match` query signals retrieval intent, so relevance becomes the
/// default sort (BM25 ranks the matched set); without it the search stays a
/// recency-bounded browse. An explicit `--sort` always wins.
pub fn from_search_args(args: &SearchArgs) -> Result<Filter> {
    let sort = args.sort.unwrap_or(if args.match_.is_some() {
        Sort::Relevance
    } else {
        Sort::Newest
    });
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
        Some(sort),
        args.limit,
    )?;
    // Search always bounds: default latest=5 when neither latest nor limit set.
    // Relevance ranks the whole result set, so it skips the recency window.
    if filter.latest.is_none() && filter.limit.is_none() && sort != Sort::Relevance {
        filter.latest = Some(SEARCH_DEFAULT_LATEST);
    }
    if sort == Sort::Relevance {
        filter.rank = args.match_.clone();
    }
    Ok(filter)
}

/// Build the filter command filter (parts mode optional).
pub fn from_filter_args(args: &FilterArgs) -> Result<Filter> {
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
    Ok(filter)
}

/// Build the work-parts filter: part-kind bound plus per-part pattern.
pub fn from_work_parts_args(args: &WorkPartsArgs) -> Result<Filter> {
    Ok(Filter {
        mode: FilterMode::Parts,
        pattern: args.match_.clone(),
        kind: args.kind,
        ..Filter::default()
    })
}

#[allow(clippy::too_many_arguments)]
fn common_bounds(
    match_: Option<&str>,
    exclude: Option<&str>,
    in_field: Field,
    kind: Option<PartKind>,
    status: Option<WorkOutcome>,
    exit_code: Option<i32>,
    min_duration: Option<&str>,
    max_duration: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
    last: Option<&str>,
    exclude_current: bool,
    latest: Option<usize>,
    sort: Option<Sort>,
    limit: Option<usize>,
) -> Result<Filter> {
    let min_duration_ms = parse_duration_ms_filter(min_duration, "--min-duration")?;
    let max_duration_ms = parse_duration_ms_filter(max_duration, "--max-duration")?;
    validate_duration_bounds(min_duration_ms, max_duration_ms)?;
    let (time_range, _) = build_time_range(since, until, last, Utc::now())?;
    Ok(Filter {
        mode: FilterMode::Anchors,
        pattern: match_.map(str::to_string),
        exclude_regex: exclude.map(str::to_string),
        in_field,
        kind,
        status,
        exit_code,
        min_duration_ms,
        max_duration_ms,
        since: time_range
            .as_ref()
            .and_then(|range| range.since)
            .map(|time| time.to_rfc3339()),
        until: time_range
            .as_ref()
            .and_then(|range| range.until)
            .map(|time| time.to_rfc3339()),
        exclude_current,
        sort: sort.unwrap_or_default(),
        latest,
        limit,
        ..Filter::default()
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
    let mut set = workset::query(&args.source, from_filter_args(args)?, args.cwd.as_deref())?;
    set.save_last()?;
    if let Some(name) = args.save.as_deref() {
        set.save_as(name)?;
    }
    Ok(set)
}

/// Apply the filter pipeline over loaded records: metadata bounds → per-line
/// pattern → exclude → dedup → `latest` window → final sort → `limit`.
pub(crate) fn apply(
    cwd: PathBuf,
    records: Vec<WorkRecord>,
    anchors: Vec<WorkRef>,
    filter: Filter,
) -> Result<WorkSet> {
    let searcher = Searcher::new(&records);
    let hits = searcher.search(&filter, &anchors, &cwd)?;
    let anchors: Vec<WorkRef> = hits.into_iter().map(|hit| hit.anchor).collect();
    let selected_records = workset::records_for_anchors(&records, &anchors);
    Ok(WorkSet::with_anchors(
        cwd.display().to_string(),
        selected_records,
        anchors,
    ))
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
            in_field: Field::Content,
            kind: None,
            status: None,
            exit_code: None,
            min_duration: None,
            max_duration: None,
            sort: None,
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
        let spec = from_search_args(&args).expect("spec");
        assert_eq!(spec.latest, Some(SEARCH_DEFAULT_LATEST));
        assert_eq!(spec.limit, None);
    }

    #[test]
    fn search_keeps_explicit_limit_without_forcing_latest() {
        let args = SearchArgs {
            source: "terminal".into(),
            match_: None,
            exclude: None,
            in_field: Field::Content,
            kind: None,
            status: None,
            exit_code: None,
            min_duration: None,
            max_duration: None,
            sort: None,
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
        let spec = from_search_args(&args).expect("spec");
        assert_eq!(spec.latest, None);
        assert_eq!(spec.limit, Some(12));
    }

    #[test]
    fn search_keeps_explicit_latest_and_limit() {
        let args = SearchArgs {
            source: "terminal".into(),
            match_: None,
            exclude: None,
            in_field: Field::Content,
            kind: None,
            status: None,
            exit_code: None,
            min_duration: None,
            max_duration: None,
            sort: None,
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
        let spec = from_search_args(&args).expect("spec");
        assert_eq!(spec.latest, Some(3));
        assert_eq!(spec.limit, Some(10));
    }
}
