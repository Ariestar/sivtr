use anyhow::{Context, Result};
use serde::Serialize;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{RecordTextMode, WorkRef};

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

pub fn execute(args: &ShowArgs) -> Result<()> {
    let cwd = args
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let work_ref: WorkRef = args.reference.parse()?;
    let providers = work_ref
        .provider()
        .map(|provider| vec![provider])
        .unwrap_or_else(|| {
            AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect()
        });
    let records = current_work_record_index(&providers, &cwd, None)?;
    let record = records
        .resolve(&work_ref)
        .with_context(|| format!("No record found for ref `{}`", args.reference))?;

    let content = match work_ref.line() {
        Some(line) => record
            .copy_text(RecordTextMode::Combined, false)
            .plain
            .lines()
            .nth(line - 1)
            .with_context(|| format!("No line {line} in ref `{}`", args.reference))?
            .to_string(),
        None => record.copy_text(RecordTextMode::Combined, false).plain,
    };

    if args.json {
        let ref_ = work_ref.to_string();
        let output = ShowJsonItem {
            ref_,
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
