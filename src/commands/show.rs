use anyhow::{Context, Result};
use serde::Serialize;
use sivtr_core::record::{WorkRef, WorkRefSelector, WorkRefTarget};

use crate::cli::ShowArgs;
use crate::commands::records::current_work_record_index;

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

struct ShowItem {
    ref_: WorkRef,
    content: String,
    kind: String,
    timestamp: Option<String>,
    session: String,
    dialogue: Option<String>,
}

pub fn execute(args: &ShowArgs) -> Result<()> {
    let cwd = args
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);

    if let Ok(work_ref) = args.reference.parse::<WorkRef>() {
        return show_by_ref(args, &cwd, &work_ref);
    }

    let selector: WorkRefSelector = args.reference.parse()?;
    let providers = selector.providers();
    let records = current_work_record_index(&providers, &cwd, None)?;
    let mut items = Vec::new();
    for record in records
        .records()
        .iter()
        .filter(|record| selector.matches_work_ref(&record.work_ref))
    {
        let line_refs = match selector.selected_lines() {
            Some(lines) => lines
                .iter()
                .map(|line| record.work_ref.with_line(*line))
                .collect::<Vec<_>>(),
            None => vec![record.work_ref.clone()],
        };

        for work_ref in line_refs {
            let content = record
                .content_for_target(work_ref.target())
                .map(str::to_string)
                .with_context(|| format!("No content found for ref `{work_ref}`"))?;
            let display_ref = match work_ref.target() {
                WorkRefTarget::Record => record.work_ref.with_target(work_ref.target()),
                _ => work_ref,
            };
            items.push(ShowItem {
                ref_: display_ref,
                content,
                kind: record.kind_label().to_string(),
                timestamp: record.time.primary_at().map(str::to_string),
                session: record.work_ref.session().to_string(),
                dialogue: Some(record.title.clone()),
            });
        }
    }

    if items.is_empty() {
        anyhow::bail!("No record found for ref selector `{}`", args.reference);
    }

    if args.json {
        let output = items.into_iter().map(show_json_item).collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let multi = items.len() > 1;
    for item in items {
        if multi {
            println!("--- {} ---", item.ref_);
        }
        print!("{}", item.content);
        if !item.content.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

fn show_by_ref(args: &ShowArgs, cwd: &std::path::Path, work_ref: &WorkRef) -> Result<()> {
    let providers = work_ref
        .provider()
        .map(|provider| vec![provider])
        .unwrap_or_else(|| {
            sivtr_core::ai::AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect()
        });
    let records = current_work_record_index(&providers, cwd, None)?;
    let record = records
        .resolve(work_ref)
        .with_context(|| format!("No record found for ref `{}`", args.reference))?;
    let content = record
        .content_for_target(work_ref.target())
        .map(str::to_string)
        .with_context(|| format!("No content found for ref `{}`", args.reference))?;

    if args.json {
        let output = ShowJsonItem {
            ref_: work_ref.to_string(),
            kind: record.kind_label().to_string(),
            timestamp: record.time.primary_at().map(str::to_string),
            title: ShowJsonTitle {
                session: record.work_ref.session().to_string(),
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

fn show_json_item(item: ShowItem) -> ShowJsonItem {
    ShowJsonItem {
        ref_: item.ref_.to_string(),
        kind: item.kind,
        timestamp: item.timestamp,
        title: ShowJsonTitle {
            session: item.session,
            dialogue: item.dialogue,
        },
        content: item.content,
    }
}
