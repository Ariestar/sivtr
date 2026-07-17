use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sivtr_core::query::load_workspace_source;
use sivtr_core::record::{expand_source, WorkPath, WorkRecord, WorkRef};
use sivtr_core::workspace;

use crate::commands::memory::filter::{self, Filter};
use crate::commands::memory::records::warn_skipped;

use super::WorkSet;

/// Unified query: local and remote share one shape.
///
/// Remote is only transport: same `Filter` is sent, peer runs the same local path
/// on the share root, result comes back.
pub fn query(source: &str, filter: Filter, cwd: Option<&Path>) -> Result<WorkSet> {
    if source == "@" {
        return apply_loaded(read_stdin()?, filter);
    }
    if source.starts_with('@') {
        return apply_loaded(super::load_reference(source)?, filter);
    }

    let cwd = cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);

    let source = expand_source(source)?;

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
            return run_local(path, &cwd, filter);
        }
        let scope = scope.to_ascii_lowercase();

        if let Some(ws) = workspace::resolve_workspace_for_dir(&cwd)? {
            if let Some(set) = try_remote(&ws.key, &scope, path, filter.clone(), &cwd)? {
                return Ok(set);
            }
        }

        if !scope.contains('/') {
            if let Some(root) =
                crate::commands::remote::workspace::resolve_local_workspace_by_name(&scope)?
            {
                return run_local(path, &root, filter);
            }
        }

        anyhow::bail!(
            "unknown scope `{scope}`; use `sivtr remote list` for remotes or `sivtr ws list` for local workspaces"
        );
    }

    run_local(&source, &cwd, filter)
}

/// Peer-side: same local query on share root, optional redact.
pub fn run_on_share(
    root: &Path,
    source: &str,
    filter: Filter,
    redact: bool,
) -> Result<(Vec<WorkRecord>, Vec<WorkRef>)> {
    let mut set = run_local(source, root, filter.for_remote_peer())?;
    if redact {
        set.records = set
            .records
            .iter()
            .map(crate::remote::redact::redact_record)
            .collect();
    }
    Ok((set.records, set.anchors))
}

fn run_local(source: &str, root: &Path, filter: Filter) -> Result<WorkSet> {
    let result = load_workspace_source(root, source)?;
    warn_skipped(&result.skipped);
    apply_loaded(
        WorkSet::with_anchors(
            root.display().to_string(),
            result.records,
            result.anchors,
        ),
        filter,
    )
}

fn apply_loaded(set: WorkSet, filter: Filter) -> Result<WorkSet> {
    filter::apply(
        PathBuf::from(&set.cwd),
        set.records,
        set.anchors,
        filter,
    )
}

fn try_remote(
    workspace_key: &str,
    remote_name: &str,
    path: &str,
    filter: Filter,
    cwd: &Path,
) -> Result<Option<WorkSet>> {
    use crate::remote::ipc;
    use crate::remote::protocol::{LocalRequest, LocalResponse};

    crate::commands::remote::serve::ensure_running()?;
    let mounts = match ipc::call(LocalRequest::RemoteList {
        workspace_key: workspace_key.to_string(),
    })? {
        LocalResponse::Mounts(mounts) => mounts,
        _ => return Ok(None),
    };
    if !mounts
        .iter()
        .any(|mount| mount.alias.eq_ignore_ascii_case(remote_name))
    {
        return Ok(None);
    }

    match ipc::call(LocalRequest::RemoteQuery {
        workspace_key: workspace_key.to_string(),
        alias: remote_name.to_ascii_lowercase(),
        source: path.to_string(),
        filter,
    })? {
        LocalResponse::Query(response) => Ok(Some(WorkSet::with_anchors(
            cwd.display().to_string(),
            response.records,
            response.anchors,
        ))),
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
        let set = query(&source, Filter::none(), Some(cwd))?;
        for record in set.records {
            let key = record.work_ref.whole().to_string();
            if seen_records.insert(key) {
                records.push(record);
            }
        }
    }
    Ok(records)
}
