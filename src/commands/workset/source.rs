use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkRefTarget, WorkRecord, WorkRef, WorkRefSelector};

use crate::commands::records::current_work_record_index;

use super::WorkSet;

#[derive(Debug, Clone)]
pub enum WorkSetSource {
    Reference(WorkSet),
    Records {
        cwd: PathBuf,
        records: Vec<WorkRecord>,
        anchors: Vec<WorkRef>,
    },
}

impl WorkSetSource {
    pub fn cwd(&self) -> PathBuf {
        match self {
            Self::Reference(set) => PathBuf::from(&set.cwd),
            Self::Records { cwd, .. } => cwd.clone(),
        }
    }

    pub fn into_parts(self) -> (Vec<WorkRecord>, Vec<WorkRef>) {
        match self {
            Self::Reference(mut set) => {
                set.ensure_anchors();
                (set.records, set.anchors)
            }
            Self::Records {
                records, anchors, ..
            } => (records, anchors),
        }
    }

    pub fn into_workset(self) -> WorkSet {
        match self {
            Self::Reference(mut set) => {
                set.ensure_anchors();
                set
            }
            Self::Records {
                cwd,
                records,
                anchors,
            } => WorkSet::with_anchors(cwd.display().to_string(), records, anchors),
        }
    }
}

pub fn load_source(source: &str, cwd: Option<&Path>) -> Result<WorkSetSource> {
    if source == "@" {
        return Ok(WorkSetSource::Reference(read_stdin()?));
    }
    if source.starts_with('@') {
        return Ok(WorkSetSource::Reference(super::load_reference(source)?));
    }

    let cwd = cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    if let Ok(work_ref) = source.parse::<WorkRef>() {
        return resolve_ref_source(source, &cwd, &work_ref);
    }

    resolve_selector_source(source, &cwd)
}

fn read_stdin() -> Result<WorkSet> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("Failed to read WorkSet from stdin")?;
    let mut set: WorkSet =
        serde_json::from_str(&input).context("Failed to parse WorkSet from stdin")?;
    set.ensure_anchors();
    Ok(set)
}

fn resolve_ref_source(source: &str, cwd: &Path, work_ref: &WorkRef) -> Result<WorkSetSource> {
    let record_ref = work_ref.record_ref();
    let providers = record_ref
        .provider()
        .map(|provider| vec![provider])
        .unwrap_or_else(all_agent_providers);
    let index = current_work_record_index(&providers, cwd, None)?;
    let record = index
        .resolve(&record_ref)
        .with_context(|| format!("No record found for ref `{source}`"))?;
    Ok(WorkSetSource::Records {
        cwd: cwd.to_path_buf(),
        records: vec![record.clone()],
        anchors: vec![work_ref.clone()],
    })
}

fn resolve_selector_source(source: &str, cwd: &Path) -> Result<WorkSetSource> {
    let selector: WorkRefSelector = source.parse()?;
    let providers = selector.providers();
    let index = current_work_record_index(&providers, cwd, None)?;
    let mut records = Vec::new();
    let mut anchors = Vec::new();

    for record in index.records() {
        if !selector.matches_work_ref(&record.work_ref) {
            continue;
        }
        let record_ref = record.work_ref.record_ref();
        records.push(record.clone());
        if let Some(part_range) = selector.selected_parts() {
            for &part_index in &part_range.indices {
                let target = WorkRefTarget::Part {
                    io: part_range.io,
                    index: part_index,
                };
                if record.part_for_target(target).is_some() {
                    anchors.push(record_ref.with_target(target));
                }
            }
        } else if let Some(lines) = selector.selected_lines() {
            for line in lines {
                anchors.push(record_ref.with_line(*line));
            }
        } else {
            anchors.push(record_ref);
        }
    }

    if records.is_empty() {
        bail!("No record found for ref selector `{source}`");
    }

    Ok(WorkSetSource::Records {
        cwd: cwd.to_path_buf(),
        records,
        anchors,
    })
}

fn all_agent_providers() -> Vec<AgentProvider> {
    AgentProvider::all()
        .iter()
        .map(|spec| spec.provider)
        .collect()
}
