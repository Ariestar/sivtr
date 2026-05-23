use anyhow::{Context, Result};
use chrono::Utc;
use regex::Regex;
use serde::Serialize;
use sivtr_core::record::WorkRecord;

use crate::cli::{SearchArgs, SearchScopeArg};
use crate::commands::records::current_work_records;
use crate::commands::time_filter::{build_time_range, TimeRange};

#[derive(Serialize)]
struct SearchJsonOutput<'a> {
    query: &'a str,
    scope: &'static str,
    cwd: String,
    match_count: usize,
    results: Vec<SearchJsonItem>,
}

#[derive(Serialize)]
struct SearchJsonItem {
    #[serde(rename = "ref")]
    ref_: String,
    kind: String,
    timestamp: Option<String>,
    title: SearchJsonTitle,
    content: String,
}

#[derive(Serialize)]
struct SearchJsonTitle {
    session: String,
    dialogue: Option<String>,
}

struct SearchHit<'a> {
    record: &'a WorkRecord,
    line_index: usize,
    content: String,
}

pub fn execute(args: &SearchArgs) -> Result<()> {
    let cwd = args
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let providers = args.provider.providers();
    let now = Utc::now();
    let (time_range, recent_count) = build_time_range(
        args.since.as_deref(),
        args.until.as_deref(),
        args.recent.as_deref(),
        now,
    )?;
    let records = current_work_records(&providers, &cwd, recent_count)?;
    let regex = Regex::new(&format!("(?i){}", args.query))?;
    let results = collect_results(
        &records,
        &regex,
        search_scope(args.scope),
        args.limit,
        time_range.as_ref(),
    );

    if args.json {
        let json = SearchJsonOutput {
            query: &args.query,
            scope: scope_name(args.scope),
            cwd: cwd.display().to_string(),
            match_count: results.len(),
            results: results.into_iter().map(search_json_item).collect(),
        };
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    if results.is_empty() {
        println!("No matches for `{}`", args.query);
        return Ok(());
    }

    for result in results {
        println!("{}", line_ref(result.record, result.line_index));
        println!("  {}", result.record.title);
        println!("  {}", result.content.trim());
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum RecordSearchScope {
    Content,
    Title,
    Session,
}

fn search_scope(scope: SearchScopeArg) -> RecordSearchScope {
    match scope {
        SearchScopeArg::Content => RecordSearchScope::Content,
        SearchScopeArg::Dialogue => RecordSearchScope::Title,
        SearchScopeArg::Session => RecordSearchScope::Session,
    }
}

fn scope_name(scope: SearchScopeArg) -> &'static str {
    match scope {
        SearchScopeArg::Content => "content",
        SearchScopeArg::Dialogue => "dialogue",
        SearchScopeArg::Session => "session",
    }
}

fn collect_results<'a>(
    records: &'a [WorkRecord],
    regex: &Regex,
    scope: RecordSearchScope,
    limit: usize,
    time_range: Option<&TimeRange>,
) -> Vec<SearchHit<'a>> {
    records
        .iter()
        .filter(|record| {
            time_range
                .is_none_or(|range| range.contains_record_time(record.time.occurred_at.as_deref()))
        })
        .filter_map(|record| match scope {
            RecordSearchScope::Content => matching_line(record, regex),
            RecordSearchScope::Title => regex.is_match(&record.title).then(|| SearchHit {
                record,
                line_index: 0,
                content: record.title.clone(),
            }),
            RecordSearchScope::Session => regex.is_match(&record.session.id).then(|| SearchHit {
                record,
                line_index: 0,
                content: record.session.id.clone(),
            }),
        })
        .take(limit)
        .collect()
}

fn matching_line<'a>(record: &'a WorkRecord, regex: &Regex) -> Option<SearchHit<'a>> {
    record
        .text
        .combined
        .lines()
        .enumerate()
        .find(|(_, line)| regex.is_match(line))
        .map(|(line_index, line)| SearchHit {
            record,
            line_index,
            content: line.to_string(),
        })
}

fn search_json_item(hit: SearchHit<'_>) -> SearchJsonItem {
    SearchJsonItem {
        ref_: line_ref(hit.record, hit.line_index),
        kind: record_kind(hit.record).to_string(),
        timestamp: hit.record.time.occurred_at.clone(),
        title: SearchJsonTitle {
            session: hit.record.session.id.clone(),
            dialogue: Some(hit.record.title.clone()),
        },
        content: hit.content,
    }
}

fn record_kind(record: &WorkRecord) -> &'static str {
    match record.source.channel {
        sivtr_core::record::WorkChannel::Terminal => "shell",
        sivtr_core::record::WorkChannel::Chat => "ai",
    }
}

fn line_ref(record: &WorkRecord, line_index: usize) -> String {
    format!("{}/{}", record.session.ref_id, line_index + 1)
}
