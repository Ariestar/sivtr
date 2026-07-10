use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sivtr_core::record::{WorkRecord, WorkRef, WorkRefBody};

use crate::commands::records::current_work_source;

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
    if let Some((origin, body)) = source.split_once("://") {
        if body.is_empty() {
            anyhow::bail!("remote source `{source}` is missing a selector");
        }
        if origin.eq_ignore_ascii_case("local") {
            return load_local_source(body, &cwd);
        }
        if origin.eq_ignore_ascii_case("sivtr") {
            let mut parts = body.splitn(4, '/');
            let peer_id = parts.next().filter(|value| !value.is_empty());
            let share_id = parts.next().filter(|value| !value.is_empty());
            let alias = parts.next().filter(|value| !value.is_empty());
            let selector = parts.next().filter(|value| !value.is_empty());
            let (Some(peer_id), Some(share_id), Some(alias), Some(selector)) =
                (peer_id, share_id, alias, selector)
            else {
                anyhow::bail!("invalid canonical remote source `{source}`");
            };
            let response =
                crate::remote::RemoteClient::load_canonical(peer_id, share_id, alias, selector)?;
            return Ok(WorkSetSource::Records {
                cwd,
                records: response.records,
                anchors: response.anchors,
            });
        }
        let alias = origin.to_ascii_lowercase();
        let workspace = sivtr_core::workspace::resolve_workspace_for_dir(&cwd)?
            .with_context(|| format!("Remote `{alias}` requires a git workspace context"))?;
        let response =
            crate::remote::RemoteClient::new(&workspace.key, &alias).load_source(body)?;
        return Ok(WorkSetSource::Records {
            cwd,
            records: response.records,
            anchors: response.anchors,
        });
    }
    load_local_source(source, &cwd)
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

fn load_local_source(source: &str, cwd: &Path) -> Result<WorkSetSource> {
    let result = current_work_source(cwd, source)?;
    Ok(WorkSetSource::Records {
        cwd: cwd.to_path_buf(),
        records: result.records,
        anchors: result.anchors,
    })
}

pub fn load_context_records(
    source_records: &[WorkRecord],
    source_anchors: &[WorkRef],
    cwd: &Path,
) -> Result<Vec<WorkRecord>> {
    let mut sources = Vec::new();
    let mut seen_sources = HashSet::new();
    for anchor in source_anchors {
        let record = super::record_for_anchor(source_records, anchor)
            .with_context(|| format!("No record found for ref `{anchor}`"))?;
        let body = match record.work_ref.body() {
            WorkRefBody::Terminal { session, .. } => format!("terminal/{session}"),
            WorkRefBody::Agent {
                provider, session, ..
            } => format!("{}/{session}", provider.command_name()),
        };
        let source = match (anchor.remote_name(), anchor.remote_ids()) {
            (Some(alias), Some((peer_id, share_id))) => {
                format!("sivtr://{peer_id}/{share_id}/{alias}/{body}")
            }
            (Some(alias), None) => format!("{alias}://{body}"),
            (None, _) => body,
        };
        if seen_sources.insert(source.clone()) {
            sources.push(source);
        }
    }

    let mut records = Vec::new();
    let mut seen_records = HashSet::new();
    for source in sources {
        let loaded = load_source(&source, Some(cwd))?;
        let (loaded_records, _) = loaded.into_parts();
        for record in loaded_records {
            let key = record.work_ref.record_ref().to_string();
            if seen_records.insert(key) {
                records.push(record);
            }
        }
    }
    Ok(records)
}
