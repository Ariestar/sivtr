use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sivtr_core::query::load_workspace_source;
use sivtr_core::record::{reject_legacy_scheme_syntax, WorkRecord, WorkRef, WorkRefBody};

use crate::commands::memory::records::warn_skipped;

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

    // `origin:body` — origin is mount alias / local workspace name / device/workspace.
    // Bare body is local current workspace.
    if let Some((origin, body)) = source.split_once(':') {
        if body.is_empty() {
            anyhow::bail!("source `{source}` is missing a selector after `:`");
        }
        reject_legacy_scheme_syntax(source)?;
        if origin.eq_ignore_ascii_case("local") {
            return load_local_source(body, &cwd);
        }
        let origin = origin.to_ascii_lowercase();

        // 1. Workspace-local mount alias (remote share).
        if let Some(workspace) = sivtr_core::workspace::resolve_workspace_for_dir(&cwd)? {
            if let Some(response) = try_remote_mount(&workspace.key, &origin, body)? {
                return Ok(WorkSetSource::Records {
                    cwd,
                    records: response.records,
                    anchors: response.anchors,
                });
            }
        }

        // 2. Local workspace by directory basename (e.g. `docs:terminal`).
        if !origin.contains('/') {
            if let Some(root) =
                crate::commands::remote::workspace::resolve_local_workspace_by_name(&origin)?
            {
                return load_local_source(body, &root);
            }
        }

        anyhow::bail!(
            "unknown origin `{origin}`; use `sivtr remote list` for mounts or `sivtr wb list` for local workspaces"
        );
    }
    load_local_source(source, &cwd)
}

fn try_remote_mount(
    workspace_key: &str,
    origin: &str,
    body: &str,
) -> Result<Option<crate::remote::protocol::SourceResponse>> {
    use crate::remote::ipc;
    use crate::remote::protocol::{LocalRequest, LocalResponse};

    // Auto-start daemon when reading remote origins, then check mounts.
    let _ = crate::commands::remote::serve::ensure_running();
    if !ipc::running() {
        return Ok(None);
    }
    // Only treat origin as a mount if it is registered; other errors (network,
    // auth) must surface instead of being mistaken for a local workspace name.
    let mounts = match ipc::call(LocalRequest::RemoteList {
        workspace_key: workspace_key.to_string(),
    }) {
        Ok(LocalResponse::Mounts(mounts)) => mounts,
        Ok(_) | Err(_) => return Ok(None),
    };
    if !mounts
        .iter()
        .any(|mount| mount.alias.eq_ignore_ascii_case(origin))
    {
        return Ok(None);
    }

    match ipc::call(LocalRequest::RemoteSource {
        workspace_key: workspace_key.to_string(),
        alias: origin.to_ascii_lowercase(),
        source: body.to_string(),
    })? {
        LocalResponse::Source(response) => Ok(Some(response)),
        response => anyhow::bail!("Unexpected daemon response: {response:?}"),
    }
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
    let result = load_workspace_source(cwd, source)?;
    warn_skipped(&result.skipped);
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
        let source = match anchor.remote_name() {
            Some(origin) => format!("{origin}:{body}"),
            None => body,
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
