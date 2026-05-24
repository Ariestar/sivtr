use anyhow::{Context, Result};
use chrono::Utc;
use regex::Regex;
use serde::Serialize;
use sivtr_core::record::{WorkChannel, WorkRecordMatch, WorkRecordSearchScope};

use crate::cli::{SearchArgs, SearchScopeArg};
use crate::commands::records::current_work_record_index;
use crate::commands::time_filter::build_time_range;

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

pub fn execute(args: &SearchArgs) -> Result<()> {
    let cwd = args
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let providers = if args.shell {
        Vec::new()
    } else {
        args.provider.providers()
    };
    let now = Utc::now();
    let (time_range, recent_count) = build_time_range(
        args.since.as_deref(),
        args.until.as_deref(),
        args.recent.as_deref(),
        now,
    )?;
    let records = current_work_record_index(&providers, &cwd, recent_count)?;
    let regex = Regex::new(&format!("(?i){}", args.query))?;
    let results = records.search(&regex, search_scope(args.scope), args.limit, |record| {
        source_matches(args, record.source.channel)
            && time_range
                .as_ref()
                .is_none_or(|range| range.contains_record_time(record.time.occurred_at.as_deref()))
    });

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
        println!("{}", line_ref(result));
        println!("  {}", result.record.title);
        println!("  {}", result.content.trim());
    }

    Ok(())
}

fn search_scope(scope: SearchScopeArg) -> WorkRecordSearchScope {
    match scope {
        SearchScopeArg::Content => WorkRecordSearchScope::Content,
        SearchScopeArg::Dialogue => WorkRecordSearchScope::Title,
        SearchScopeArg::Session => WorkRecordSearchScope::Session,
    }
}

fn source_matches(args: &SearchArgs, channel: WorkChannel) -> bool {
    if args.shell {
        return channel == WorkChannel::Terminal;
    }

    if args.agent {
        return channel == WorkChannel::Chat;
    }

    true
}

fn scope_name(scope: SearchScopeArg) -> &'static str {
    match scope {
        SearchScopeArg::Content => "content",
        SearchScopeArg::Dialogue => "dialogue",
        SearchScopeArg::Session => "session",
    }
}

fn search_json_item(hit: WorkRecordMatch<'_>) -> SearchJsonItem {
    SearchJsonItem {
        ref_: line_ref(hit),
        kind: hit.record.kind_label().to_string(),
        timestamp: hit.record.time.occurred_at.clone(),
        title: SearchJsonTitle {
            session: hit.record.session.id.clone(),
            dialogue: Some(hit.record.title.clone()),
        },
        content: hit.content.to_string(),
    }
}

fn line_ref(hit: WorkRecordMatch<'_>) -> String {
    hit.record
        .session
        .work_ref
        .with_line(hit.line_index + 1)
        .to_string()
}
