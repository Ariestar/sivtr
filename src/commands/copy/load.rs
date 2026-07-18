//! Load dialogues for a copy plan from the shared workset / session path.

use anyhow::{Context, Result};
use sivtr_core::capture::scrollback;
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};
use sivtr_core::session;

use crate::commands::select::resolve_selector;
use crate::commands::memory::filter::Filter;
use crate::commands::memory::workset;
use crate::output;

use super::plan::{CopyPlan, DialogueSelect, Projection};

pub(super) struct LoadedCopy {
    pub records: Vec<WorkRecord>,
    /// Projection may be upgraded to Exact when the address pins part/line.
    pub projection: Projection,
    pub label: String,
}

/// Resolve plan.address + dialogues into ordered records (selection already applied).
pub(super) fn load_for_plan(plan: &CopyPlan) -> Result<Option<LoadedCopy>> {
    let cwd = plan
        .filters
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);

    match plan.address.as_deref() {
        None => load_current_terminal(plan),
        Some(address) => load_address(address, plan, &cwd),
    }
}

fn load_current_terminal(plan: &CopyPlan) -> Result<Option<LoadedCopy>> {
    let Some(log_path) = scrollback::session_log_path()? else {
        warn_no_session_log();
        return Ok(None);
    };
    if !log_path.exists() {
        warn_no_session_log();
        return Ok(None);
    }

    let entries = session::load_entries(&log_path).context("Failed to read session log")?;
    if entries.is_empty() {
        output::warning("no commands recorded yet");
        output::hint("run a few commands first, then try `sivtr copy` again");
        return Ok(None);
    }

    let records: Vec<WorkRecord> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| WorkRecord::terminal(entry, &log_path, index))
        .collect();
    if records.is_empty() {
        output::warning("no commands recorded yet");
        output::hint("run a command first, then try `sivtr copy` again");
        return Ok(None);
    }

    let selected = select_relative(&records, &plan.dialogues)?;
    if selected.is_empty() {
        output::warning("nothing selected");
        return Ok(None);
    }

    Ok(Some(LoadedCopy {
        records: selected,
        projection: plan.projection,
        label: "terminal".to_string(),
    }))
}

fn load_address(address: &str, plan: &CopyPlan, cwd: &std::path::Path) -> Result<Option<LoadedCopy>> {
    let expanded = sivtr_core::record::expand_source(address)?;

    // Full WorkRef (session + index [+ at]) → absolute pin.
    if let Ok(work_ref) = expanded.parse::<WorkRef>() {
        return load_pinned_ref(&expanded, &work_ref, plan, cwd);
    }

    // Source-level address: terminal | codex | desk:codex | provider/session …
    let set = workset::query(&expanded, Filter::none(), Some(cwd))?;
    if set.records.is_empty() {
        output::warning(format!("no records found for `{address}`"));
        return Ok(None);
    }

    let mut records = set.records;
    // Oldest → newest so resolve_selector (from end) matches terminal semantics.
    records.sort_by_key(|r| (r.session.id.clone(), r.work_ref.index()));
    // Bare provider sources may span sessions; keep the newest session only.
    records = newest_session_only(records);

    if records.is_empty() {
        output::warning(format!("no records found for `{address}`"));
        return Ok(None);
    }

    let selected = select_relative(&records, &plan.dialogues)?;
    if selected.is_empty() {
        output::warning("nothing selected");
        return Ok(None);
    }

    Ok(Some(LoadedCopy {
        label: expanded,
        records: selected,
        projection: plan.projection,
    }))
}

fn load_pinned_ref(
    expanded: &str,
    work_ref: &WorkRef,
    plan: &CopyPlan,
    cwd: &std::path::Path,
) -> Result<Option<LoadedCopy>> {
    // Absolute address: relative dialogue select is not applied.
    if !matches!(plan.dialogues, DialogueSelect::RecentSingle(1)) {
        anyhow::bail!(
            "address `{expanded}` already pins a record; do not pass a relative dialogue selector"
        );
    }

    let set = workset::query(expanded, Filter::none(), Some(cwd))?;
    let record = workset::record_for_anchor(&set.records, work_ref)
        .with_context(|| format!("No record found for ref `{expanded}`"))?
        .clone();

    let projection = match work_ref.at {
        WorkAt::Whole => plan.projection,
        at => Projection::Exact(at),
    };

    Ok(Some(LoadedCopy {
        records: vec![record],
        projection,
        label: expanded.to_string(),
    }))
}

fn select_relative(records: &[WorkRecord], select: &DialogueSelect) -> Result<Vec<WorkRecord>> {
    let indices = resolve_selector(select, records.len())?;
    Ok(indices
        .into_iter()
        .filter_map(|idx| records.get(idx).cloned())
        .collect())
}

fn newest_session_only(records: Vec<WorkRecord>) -> Vec<WorkRecord> {
    if records.is_empty() {
        return records;
    }
    let last_session = records.last().expect("non-empty").session.id.clone();
    records
        .into_iter()
        .filter(|r| r.session.id == last_session)
        .collect()
}

fn warn_no_session_log() {
    output::warning("no session log found");
    output::hint("run `sivtr init <shell>`, restart the shell, then run some commands");
}
