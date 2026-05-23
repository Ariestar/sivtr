use anyhow::{bail, Context, Result};
use serde::Serialize;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkChannel, WorkRecord};

use crate::cli::ShowArgs;
use crate::commands::records::current_work_records;

#[derive(Debug)]
struct ParsedRef {
    record_ref: String,
    line: Option<usize>,
}

#[derive(Serialize)]
struct ShowJsonItem {
    #[serde(rename = "ref")]
    ref_: String,
    kind: String,
    timestamp: Option<String>,
    title: ShowJsonTitle,
    content: String,
}

#[derive(Serialize)]
struct ShowJsonTitle {
    session: String,
    dialogue: Option<String>,
}

pub fn execute(args: &ShowArgs) -> Result<()> {
    let cwd = args
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let parsed = parse_ref(&args.reference)?;
    let providers = provider_for_ref(&parsed.record_ref)
        .map(|provider| vec![provider])
        .unwrap_or_else(|| {
            AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect()
        });
    let records = current_work_records(&providers, &cwd, None)?;
    let record = records
        .iter()
        .find(|record| record.session.ref_id == parsed.record_ref)
        .with_context(|| format!("No record found for ref `{}`", args.reference))?;

    let content = match parsed.line {
        Some(line) => {
            if line == 0 {
                bail!("Line index in ref must be 1-based");
            }
            record
                .text
                .combined
                .lines()
                .nth(line - 1)
                .with_context(|| format!("No line {line} in ref `{}`", args.reference))?
                .to_string()
        }
        None => record.text.combined.clone(),
    };

    if args.json {
        let ref_ = match parsed.line {
            Some(line) => format!("{}/{}", record.session.ref_id, line),
            None => record.session.ref_id.clone(),
        };
        let output = ShowJsonItem {
            ref_,
            kind: record_kind(record).to_string(),
            timestamp: record.time.occurred_at.clone(),
            title: ShowJsonTitle {
                session: record.session.id.clone(),
                dialogue: Some(record.title.clone()),
            },
            content,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    print!("{content}");
    if !content.ends_with('\n') {
        println!();
    }
    Ok(())
}

fn parse_ref(reference: &str) -> Result<ParsedRef> {
    let parts = reference
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if !(2..=4).contains(&parts.len()) {
        bail!("Invalid ref `{reference}`; expected source/session/index[/line]");
    }

    let line = parts
        .last()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|_| parts.len() == 4);
    let record_part_count = if line.is_some() {
        parts.len() - 1
    } else {
        parts.len()
    };
    let record_ref = parts[..record_part_count].join("/");

    Ok(ParsedRef { record_ref, line })
}

fn provider_for_ref(reference: &str) -> Option<AgentProvider> {
    reference
        .split('/')
        .next()
        .and_then(AgentProvider::from_command_name)
}

fn record_kind(record: &WorkRecord) -> &'static str {
    match record.source.channel {
        WorkChannel::Terminal => "shell",
        WorkChannel::Chat => "ai",
    }
}
