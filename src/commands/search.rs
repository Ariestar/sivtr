use anyhow::{bail, Context, Result};
use chrono::Utc;
use regex::Regex;
use serde::Serialize;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkChannel, WorkOutcome, WorkRecord, WorkRecordMatch, WorkRef};

use crate::cli::{SearchArgs, SearchFieldArg, SearchStatusArg};
use crate::commands::records::current_work_record_index;
use crate::commands::time_filter::build_time_range;

#[derive(Serialize)]
struct SearchJsonOutput<'a> {
    target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    match_: Option<&'a str>,
    field: &'static str,
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
    let regex = args
        .match_
        .as_deref()
        .map(|query| Regex::new(&format!("(?i){query}")))
        .transpose()?;
    let limit = args.limit.or(args.latest).unwrap_or(20);
    let results = records
        .records()
        .iter()
        .filter(|record| {
            target.matches(record)
                && status_matches(args.status, record.status.outcome)
                && time_range.as_ref().is_none_or(|range| {
                    range.contains_record_time(record.time.occurred_at.as_deref())
                })
                && regex
                    .as_ref()
                    .is_none_or(|regex| field_matches(record, args.in_field, regex))
        })
        .map(first_line_match)
        .take(limit)
        .collect::<Vec<_>>();

    if args.json {
        let json = SearchJsonOutput {
            target: &args.target,
            match_: args.match_.as_deref(),
            field: field_name(args.in_field),
            cwd: cwd.display().to_string(),
            match_count: results.len(),
            results: results.into_iter().map(search_json_item).collect(),
        };
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    if results.is_empty() {
        println!("No matches in `{}`", args.target);
        return Ok(());
    }

    for result in results {
        println!("{}", line_ref(result));
        println!("  {}", result.record.title);
        println!("  {}", result.content.trim());
    }

    Ok(())
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
            SearchSource::Terminal if record.source.channel != WorkChannel::Terminal => {
                return false;
            }
            SearchSource::Agent(Some(provider)) => {
                if record.source.channel != WorkChannel::Chat
                    || record.source.provider.as_deref() != Some(provider.command_name())
                {
                    return false;
                }
            }
            SearchSource::Agent(None) if record.source.channel != WorkChannel::Chat => {
                return false;
            }
            _ => {}
        }

        let work_ref = &record.session.work_ref;
        match (self.session.as_deref(), work_ref) {
            (
                Some(expected),
                WorkRef::Terminal { session, .. } | WorkRef::Agent { session, .. },
            ) => {
                if !segment_matches(expected, session) {
                    return false;
                }
            }
            _ => {}
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

fn first_line_match(record: &WorkRecord) -> WorkRecordMatch<'_> {
    let content = record.text.combined.lines().next().unwrap_or(&record.title);
    WorkRecordMatch {
        record,
        line_index: 0,
        content,
    }
}

fn field_matches(record: &WorkRecord, field: SearchFieldArg, regex: &Regex) -> bool {
    match field {
        SearchFieldArg::Content => regex.is_match(&record.text.combined),
        SearchFieldArg::Title => regex.is_match(&record.title),
        SearchFieldArg::Session => regex.is_match(&record.session.id),
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
            regex.is_match(&record.text.combined)
                || regex.is_match(&record.title)
                || regex.is_match(&record.session.id)
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
