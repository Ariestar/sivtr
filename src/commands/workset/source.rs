use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkRecord, WorkRef, WorkRefSelector};

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

    // A concrete WorkRef (local or remote). Selectors like `terminal/*` are not
    // valid WorkRefs and fall through to the selector path below.
    if let Ok(work_ref) = source.parse::<WorkRef>() {
        // A remote origin routes to that device's `sivtr serve` instead of the
        // local workspace index.
        if let Some(alias) = work_ref.remote_name() {
            return resolve_remote_ref_source(alias, &work_ref);
        }
        let cwd = cwd
            .map(Path::to_path_buf)
            .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
        return resolve_ref_source(source, &cwd, &work_ref);
    }

    let cwd = cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    resolve_selector_source(source, &cwd)
}

/// Resolve a remote WorkRef by calling that device's serve endpoint. The body
/// (without the `alias://` prefix) is sent to `/resolve`; the returned record
/// keeps its origin on the anchor so downstream refs round-trip as remote.
fn resolve_remote_ref_source(alias: &str, work_ref: &WorkRef) -> Result<WorkSetSource> {
    let remote = crate::remote::lookup(alias)?;
    let client = crate::remote::RemoteClient::new(alias, remote);
    // Serve expects a local-shape ref; render the body alone (Local is the
    // bare shorthand, so wrapping in Local yields the body string).
    let body_ref = WorkRef::Local(work_ref.body().clone()).to_string();
    let record = client.resolve(&body_ref)?;
    Ok(WorkSetSource::Records {
        cwd: PathBuf::from("."),
        records: vec![record],
        anchors: vec![work_ref.clone()],
    })
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
        if let Some(lines) = selector.selected_lines() {
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
