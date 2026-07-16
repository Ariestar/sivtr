use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sivtr_core::query::load_workspace_source;
use sivtr_core::record::{expand_source, WorkPath, WorkRecord, WorkRef};
use sivtr_core::workspace;

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

    let source = expand_source(source)?;

    // `scope:path` — scope is a remote name (`remote add`) or local workspace name.
    // Bare path is local current workspace.
    if let Some((scope, path)) = source.split_once(':') {
        if path.is_empty() {
            anyhow::bail!("source `{source}` is missing a selector after `:`");
        }
        if path.starts_with('/') {
            anyhow::bail!(
                "Invalid source `{source}`; use `scope:path` (for example `desk:terminal`), not `://`"
            );
        }
        if scope.eq_ignore_ascii_case("local") {
            return load_local_source(path, &cwd);
        }
        let scope = scope.to_ascii_lowercase();

        // 1. Workspace-local remote name (`sivtr remote add desk …`).
        if let Some(ws) = workspace::resolve_workspace_for_dir(&cwd)? {
            if let Some(response) = try_remote_mount(&ws.key, &scope, path)? {
                return Ok(WorkSetSource::Records {
                    cwd,
                    records: response.records,
                    anchors: response.anchors,
                });
            }
        }

        // 2. Local workspace by directory basename (e.g. `docs:terminal`).
        if !scope.contains('/') {
            if let Some(root) =
                crate::commands::remote::workspace::resolve_local_workspace_by_name(&scope)?
            {
                return load_local_source(path, &root);
            }
        }

        anyhow::bail!(
            "unknown scope `{scope}`; use `sivtr remote list` for remotes or `sivtr ws list` for local workspaces"
        );
    }
    load_local_source(&source, &cwd)
}

fn try_remote_mount(
    workspace_key: &str,
    scope: &str,
    path: &str,
) -> Result<Option<crate::remote::protocol::SourceResponse>> {
    use crate::remote::ipc;
    use crate::remote::protocol::{LocalRequest, LocalResponse};

    // Auto-start daemon when reading remote scopes, then check mounts.
    crate::commands::remote::serve::ensure_running()?;
    // Only treat scope as a mount if it is registered; other errors (network,
    // auth) must surface instead of being mistaken for a local workspace name.
    let mounts = match ipc::call(LocalRequest::RemoteList {
        workspace_key: workspace_key.to_string(),
    }) {
        Ok(LocalResponse::Mounts(mounts)) => mounts,
        Ok(_) => return Ok(None),
        Err(error) => return Err(error),
    };
    if !mounts
        .iter()
        .any(|mount| mount.alias.eq_ignore_ascii_case(scope))
    {
        return Ok(None);
    }

    match ipc::call(LocalRequest::RemoteSource {
        workspace_key: workspace_key.to_string(),
        alias: scope.to_ascii_lowercase(),
        source: path.to_string(),
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
        let path = match &record.work_ref.path {
            WorkPath::Terminal { session, .. } => format!("terminal/{session}"),
            WorkPath::Agent {
                provider, session, ..
            } => format!("{}/{session}", provider.command_name()),
        };
        let source = match anchor.scope_name() {
            Some(scope) => format!("{scope}:{path}"),
            None => path,
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
            let key = record.work_ref.whole().to_string();
            if seen_records.insert(key) {
                records.push(record);
            }
        }
    }
    Ok(records)
}
