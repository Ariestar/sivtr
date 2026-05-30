use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};

use anyhow::{bail, Result};
use sivtr_core::record::{WorkRecord, WorkRef};

use crate::cli::{VarCommand, VarSubcommand};
use crate::commands::workset::{self, WorkSet};
use crate::output;

pub fn execute(command: &VarCommand) -> Result<()> {
    match &command.action {
        VarSubcommand::Set(args) => set(&args.name, args.source.as_deref()),
        VarSubcommand::Rm(args) => rm(&args.name),
        VarSubcommand::Cleanup => cleanup(),
        VarSubcommand::Merge(args) => merge(&args.name, &args.sources),
        VarSubcommand::Drop(args) => drop(&args.name, &args.sources),
    }
}

fn set(name: &str, source: Option<&str>) -> Result<()> {
    let source = match source {
        Some(source) => source,
        None if stdin_is_piped() => "@",
        None => {
            bail!(
                "source required when stdin is not piped; use `sivtr var set {name} @last` or pipe WorkSet JSON into stdin"
            );
        }
    };
    let mut set = workset::load_source(source, None)?.into_workset();
    set = dedup(set);
    set.save_as(name)?;
    output::success(format!("saved @{name} ({} items)", set.anchors().len()));
    Ok(())
}

fn rm(name: &str) -> Result<()> {
    workset::delete_saved(name)?;
    output::success(format!("removed @{name}"));
    Ok(())
}

fn cleanup() -> Result<()> {
    let removed = workset::cleanup_saved()?;
    output::success(format!("removed {removed} vars"));
    Ok(())
}

fn merge(name: &str, sources: &[String]) -> Result<()> {
    require_sources(sources, "merge")?;
    let before = workset::load_saved(name)?;
    let before_len = before.anchors().len();
    let mut result = before;
    for source in sources {
        let addition = workset::load_source(source, None)?.into_workset();
        result = merge_sets(result, addition);
    }
    result.name = Some(name.to_string());
    workset::save_named(name, &result)?;
    output::success(format!(
        "updated @{name}: {before_len} -> {} items",
        result.anchors().len()
    ));
    Ok(())
}

fn drop(name: &str, sources: &[String]) -> Result<()> {
    require_sources(sources, "drop")?;
    let before = workset::load_saved(name)?;
    let before_len = before.anchors().len();
    let mut remove = HashSet::new();
    for source in sources {
        let set = workset::load_source(source, None)?.into_workset();
        remove.extend(set.anchors().into_iter().map(|anchor| anchor.to_string()));
    }
    let mut result = remove_anchors(before, &remove);
    result.name = Some(name.to_string());
    workset::save_named(name, &result)?;
    output::success(format!(
        "updated @{name}: {before_len} -> {} items",
        result.anchors().len()
    ));
    Ok(())
}

fn require_sources(sources: &[String], operation: &str) -> Result<()> {
    if sources.is_empty() {
        bail!("var {operation} requires at least one source");
    }
    Ok(())
}

fn stdin_is_piped() -> bool {
    !io::stdin().is_terminal()
}

fn merge_sets(mut base: WorkSet, mut addition: WorkSet) -> WorkSet {
    base.ensure_anchors();
    addition.ensure_anchors();
    let mut records_by_ref = records_by_ref(base.records);
    for record in addition.records {
        records_by_ref
            .entry(record.work_ref.record_ref().to_string())
            .or_insert(record);
    }

    let mut anchors = base.anchors;
    anchors.extend(addition.anchors);
    anchors = unique_anchors(anchors);
    let records = records_for_unique_anchors(records_by_ref, &anchors);
    WorkSet::with_anchors(base.cwd, records, anchors)
}

fn remove_anchors(mut base: WorkSet, remove: &HashSet<String>) -> WorkSet {
    base.ensure_anchors();
    let records_by_ref = records_by_ref(base.records);
    let anchors = unique_anchors(
        base.anchors
            .into_iter()
            .filter(|anchor| !remove.contains(&anchor.to_string()))
            .collect(),
    );
    let records = records_for_unique_anchors(records_by_ref, &anchors);
    WorkSet::with_anchors(base.cwd, records, anchors)
}

fn dedup(mut set: WorkSet) -> WorkSet {
    set.ensure_anchors();
    let records_by_ref = records_by_ref(set.records);
    let anchors = unique_anchors(set.anchors);
    let records = records_for_unique_anchors(records_by_ref, &anchors);
    WorkSet::with_anchors(set.cwd, records, anchors)
}

pub(crate) fn unique_anchors(anchors: Vec<WorkRef>) -> Vec<WorkRef> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for anchor in anchors {
        if seen.insert(anchor.to_string()) {
            unique.push(anchor);
        }
    }
    unique
}

fn records_by_ref(records: Vec<WorkRecord>) -> HashMap<String, WorkRecord> {
    records
        .into_iter()
        .map(|record| (record.work_ref.record_ref().to_string(), record))
        .collect()
}

fn records_for_unique_anchors(
    mut records_by_ref: HashMap<String, WorkRecord>,
    anchors: &[WorkRef],
) -> Vec<WorkRecord> {
    let mut records = Vec::new();
    for anchor in anchors {
        if let Some(record) = records_by_ref.remove(&anchor.record_ref().to_string()) {
            records.push(record);
        }
    }
    records
}
