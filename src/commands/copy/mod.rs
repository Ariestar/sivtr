//! Copy = resolve(address, dialogues, projection) → filter → clipboard.
//!
//! Grammar: `sivtr copy [address] [dialogues]` with projection sugar `in|out|cmd`.
//! Address uses the same source/ref language as search/show. Default address is
//! the current terminal session; default dialogues is `1` (newest).

mod export;
mod load;
mod plan;
mod project;

pub use export::export_picked;
pub use plan::{parse_address_dialogues, CopyFilters, CopyPlan, Projection};

use anyhow::{Context, Result};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::WorkRecord;

use crate::commands::browse;
use crate::commands::memory::filter::Filter;
use crate::commands::memory::workset;
use crate::output;
use crate::tui::workspace::{WorkspaceFocus, WorkspaceSession, WorkspaceSource};

use export::finish_text_pairs;
use load::load_for_plan;
use project::project_record;

/// Single entry: execute a fully built plan.
pub fn execute(plan: CopyPlan) -> Result<()> {
    if plan.pick {
        return execute_pick(&plan);
    }

    let Some(loaded) = load_for_plan(&plan)? else {
        return Ok(());
    };

    let prompt = plan.filters.prompt.as_deref();
    let mut units = Vec::new();
    for record in &loaded.records {
        let unit = project_record(record, loaded.projection, prompt)?;
        if !unit.plain.trim().is_empty() {
            units.push(unit);
        }
    }

    if units.is_empty() {
        output::warning(format!("selected {} content is empty", loaded.label));
        return Ok(());
    }

    let count = units.len();
    finish_text_pairs(
        &units,
        &plan.filters,
        &format!("copied {count} item(s) from {} to clipboard", loaded.label),
    )
}

fn execute_pick(plan: &CopyPlan) -> Result<()> {
    match plan.address.as_deref() {
        None => {
            // Full workspace browser (same product surface as bare `sivtr`).
            let providers = AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect::<Vec<_>>();
            let picked = browse::run(&providers, false, WorkspaceFocus::Sessions)?;
            export_picked(
                &picked,
                plan.filters.print,
                plan.filters.regex.as_deref(),
                plan.filters.lines.as_deref(),
                plan.filters.ansi,
            )
        }
        Some(address) => {
            let expanded = sivtr_core::record::expand_source(address)?;
            let cwd = plan
                .filters
                .cwd
                .clone()
                .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
            let set = workset::query(&expanded, Filter::none(), Some(&cwd))?;
            if set.records.is_empty() {
                output::warning(format!("no records found for `{address}`"));
                return Ok(());
            }
            let mut records = set.records;
            records.sort_by_key(|r| (r.session.id.clone(), r.work_ref.index()));
            if let Some(last) = records.last().map(|r| r.session.id.clone()) {
                records.retain(|r| r.session.id == last);
            }
            let source =
                session_source_from_records(&records).unwrap_or_else(WorkspaceSource::terminal);
            let session = WorkspaceSession {
                source: source.clone(),
                modified: std::time::SystemTime::now(),
                title: expanded.clone(),
                search_title: expanded,
                records,
            };
            let picked =
                browse::run_with_sessions(source, vec![session], WorkspaceFocus::Dialogues)?;
            export_picked(
                &picked,
                plan.filters.print,
                plan.filters.regex.as_deref(),
                plan.filters.lines.as_deref(),
                plan.filters.ansi,
            )
        }
    }
}

fn session_source_from_records(records: &[WorkRecord]) -> Option<WorkspaceSource> {
    let record = records.first()?;
    if let Some(provider) = record.work_ref.provider() {
        Some(WorkspaceSource::agent(provider))
    } else {
        Some(WorkspaceSource::terminal())
    }
}

/// Build plan from CLI pieces (projection sugar + free tokens + flags).
pub fn plan_from_cli(
    projection: Projection,
    free_tokens: &[String],
    pick: bool,
    filters: CopyFilters,
) -> Result<CopyPlan, String> {
    let (address, dialogues) = parse_address_dialogues(free_tokens)?;
    Ok(CopyPlan {
        address,
        dialogues,
        projection,
        pick,
        filters,
    })
}

