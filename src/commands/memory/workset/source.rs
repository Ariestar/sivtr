use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use sivtr_core::origin::Reach;
use sivtr_core::query::{load_workspace_source, NO_RECORD_FOR_SELECTOR};
use sivtr_core::record::{expand_source, WorkPath, WorkRecord, WorkRef};

use crate::commands::memory::filter::{self, Filter};
use crate::commands::memory::records::warn_skipped;
use crate::commands::remote::serve;
use crate::output;

use super::WorkSet;

/// Default deadline for one remote source inside [`query_many`].
pub const REMOTE_QUERY_TIMEOUT: Duration = Duration::from_secs(3);

/// Socket-read headroom for one group fan-out inside [`query`]: the daemon
/// dials every member in parallel under its own per-share budget, so this
/// must stay above that budget plus local query time.
const GROUP_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// How one source is scheduled inside [`query_many`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryTransport {
    /// Disk-local (or named local workspace). Failures abort the batch caller if desired.
    Local,
    /// Mounted remote alias. Failures are isolated when using [`query_many`].
    Remote,
}

/// One source to load via the unified query path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuerySource {
    /// Selector accepted by [`query`] (`codex`, `desk:terminal`, …).
    pub selector: String,
    pub transport: QueryTransport,
}

impl QuerySource {
    pub fn local(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            transport: QueryTransport::Local,
        }
    }

    pub fn remote(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            transport: QueryTransport::Remote,
        }
    }

    pub fn is_remote(&self) -> bool {
        self.transport == QueryTransport::Remote
    }
}

/// Per-source outcome from [`query_many`]. Failures never drop other sources.
#[derive(Debug)]
pub enum QuerySourceResult {
    Ok(WorkSet),
    Err(String),
}

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

        // A scoped query may address a remote mount; the daemon must be up
        // before the passive registry lookup can see its mounts.
        serve::ensure_running()?;
        // One lookup: the registry is the single alias table (local
        // workspaces, remote mounts, cloud), each entry carrying its reach
        // payload; resolution applies kind precedence on name collisions.
        // Groups (`team`, `team/alice`) are a roster fan-out over many
        // devices, not a single origin, so they are tried only on a miss.
        let registry = crate::origins::collect(&cwd)?;
        return match registry.resolve(&scope)? {
            Some(entry) => match &entry.reach {
                Reach::Local { root } => run_local(path, Path::new(root), filter),
                Reach::Remote { workspace_key, alias } => try_remote_timed(
                    workspace_key,
                    alias,
                    path,
                    filter,
                    &cwd,
                    Duration::from_secs(30),
                )
                .with_context(|| format!("remote mount `{alias}` unavailable")),
                Reach::Cloud => {
                    anyhow::bail!("cloud source `{}` is not available yet", entry.origin.name)
                }
            },
            None => match try_group(&scope, path, filter, &cwd)? {
                Some(set) => Ok(set),
                None => anyhow::bail!(
                    "unknown scope `{scope}`; use `sivtr ws list` for local workspaces, `sivtr remote list` for remotes, or `sivtr group list` for groups"
                ),
            },
        };
    }

    run_local(&source, &cwd, filter)
}

/// Load many sources. Locals run first (fail hard); remotes run in parallel with
/// a per-source timeout. Order of `results` matches `sources`.
///
/// This is the multi-source schedule layer shared by browse (and later search
/// `--all`). Each remote still uses [`query`]; speedup comes from overlapping
/// RTT and failing stuck peers without blocking the batch.
pub fn query_many(
    sources: &[QuerySource],
    filter: Filter,
    cwd: Option<&Path>,
    remote_timeout: Duration,
) -> Result<Vec<QuerySourceResult>> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }

    let cwd = cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);

    let mut results: Vec<Option<QuerySourceResult>> = sources.iter().map(|_| None).collect();

    let mut remote_idxs = Vec::new();
    for (idx, source) in sources.iter().enumerate() {
        if source.is_remote() {
            remote_idxs.push(idx);
            continue;
        }
        match query(&source.selector, filter.clone(), Some(&cwd)) {
            Ok(set) => results[idx] = Some(QuerySourceResult::Ok(set)),
            Err(error) => {
                let message = error.to_string();
                // Empty selector is normal for browse; keep parity with single-source callers.
                if message.starts_with(NO_RECORD_FOR_SELECTOR) {
                    results[idx] = Some(QuerySourceResult::Ok(WorkSet::with_anchors(
                        cwd.display().to_string(),
                        Vec::new(),
                        Vec::new(),
                    )));
                } else {
                    return Err(error).context(format!("Failed to load `{}`", source.selector));
                }
            }
        }
    }

    if remote_idxs.is_empty() {
        return Ok(results
            .into_iter()
            .map(|slot| slot.expect("local result filled"))
            .collect());
    }

    let (tx, rx) = mpsc::channel();
    let workers = remote_idxs.len();
    for &idx in &remote_idxs {
        let selector = sources[idx].selector.clone();
        let filter = filter.clone();
        let cwd = cwd.clone();
        let tx = tx.clone();
        thread::spawn(move || {
            let result = match query_remote_bounded(&selector, filter, &cwd, remote_timeout) {
                Ok(set) => Ok(set),
                Err(error) => {
                    let message = error.to_string();
                    if message.starts_with(NO_RECORD_FOR_SELECTOR) {
                        Ok(WorkSet::with_anchors(
                            cwd.display().to_string(),
                            Vec::new(),
                            Vec::new(),
                        ))
                    } else if is_timeout_error(&message) {
                        Err("timeout".to_string())
                    } else {
                        Err(format!("{error:#}"))
                    }
                }
            };
            let _ = tx.send((idx, result));
        });
    }
    drop(tx);

    let mut remaining = workers;
    while remaining > 0 {
        match rx.recv_timeout(remote_timeout + Duration::from_secs(1)) {
            Ok((idx, Ok(set))) => {
                results[idx] = Some(QuerySourceResult::Ok(set));
                remaining -= 1;
            }
            Ok((idx, Err(message))) => {
                results[idx] = Some(QuerySourceResult::Err(message));
                remaining -= 1;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                for &idx in &remote_idxs {
                    if results[idx].is_none() {
                        results[idx] = Some(QuerySourceResult::Err("timeout".to_string()));
                    }
                }
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                for &idx in &remote_idxs {
                    if results[idx].is_none() {
                        results[idx] =
                            Some(QuerySourceResult::Err("load worker exited".to_string()));
                    }
                }
                break;
            }
        }
    }

    Ok(results
        .into_iter()
        .map(|slot| slot.expect("every source has a result"))
        .collect())
}

fn query_remote_bounded(
    selector: &str,
    filter: Filter,
    cwd: &Path,
    read_timeout: Duration,
) -> Result<WorkSet> {
    // Only confirmed mounts need the timed IPC path, so the daemon socket
    // itself respects the interactive deadline. Everything else — groups,
    // named local workspaces, plain selectors — goes through the unified
    // [`query`], which resolves it in one registry lookup.
    let Some((scope, path)) = selector.split_once(':') else {
        return query(selector, filter, Some(cwd));
    };
    if path.is_empty() || path.starts_with('/') || scope.eq_ignore_ascii_case("local") {
        return query(selector, filter, Some(cwd));
    }
    // Remote mounts need the daemon; start it before the passive lookup.
    serve::ensure_running()?;
    let registry = crate::origins::collect(cwd)?;
    let Some(entry) = registry.resolve(scope)? else {
        return query(selector, filter, Some(cwd));
    };
    match &entry.reach {
        Reach::Remote {
            workspace_key,
            alias,
        } => try_remote_timed(workspace_key, alias, path, filter, cwd, read_timeout)
            .with_context(|| format!("remote mount `{alias}` unavailable")),
        _ => query(selector, filter, Some(cwd)),
    }
}

fn is_timeout_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("os error 10060")
        || lower.contains("i/o operation")
}

/// Peer-side query on a share root, optional redact. An empty workspace has
/// no sessions and reports an empty result instead of erroring, so one
/// member's empty contribution cannot abort a group fan-out.
pub fn run_on_share(
    root: &Path,
    source: &str,
    filter: Filter,
    redact: bool,
) -> Result<(Vec<WorkRecord>, Vec<WorkRef>)> {
    match run_local(source, root, filter.for_remote_peer()) {
        Ok(mut set) => {
            if redact {
                set.records = set
                    .records
                    .iter()
                    .map(crate::remote::redact::redact_record)
                    .collect();
            }
            Ok((set.records, set.anchors))
        }
        Err(error) if error.to_string().starts_with(NO_RECORD_FOR_SELECTOR) => {
            Ok((Vec::new(), Vec::new()))
        }
        Err(error) => Err(error),
    }
}

fn run_local(source: &str, root: &Path, filter: Filter) -> Result<WorkSet> {
    let result = load_workspace_source(root, source)?;
    warn_skipped(&result.skipped);
    apply_loaded(
        WorkSet::with_anchors(root.display().to_string(), result.records, result.anchors),
        filter,
    )
}

fn apply_loaded(set: WorkSet, filter: Filter) -> Result<WorkSet> {
    filter::apply(PathBuf::from(&set.cwd), set.records, set.anchors, filter)
}

fn try_remote_timed(
    workspace_key: &str,
    alias: &str,
    path: &str,
    filter: Filter,
    cwd: &Path,
    read_timeout: Duration,
) -> Result<WorkSet> {
    use crate::remote::ipc;
    use crate::remote::protocol::{LocalRequest, LocalResponse};

    // The registry already confirmed the mount; ensure the daemon is up so a
    // stale socket yields a clear error instead of a confusing one.
    crate::commands::remote::serve::ensure_running()?;
    match ipc::call_with_read_timeout(
        LocalRequest::RemoteQuery {
            workspace_key: workspace_key.to_string(),
            alias: alias.to_ascii_lowercase(),
            source: path.to_string(),
            filter,
        },
        read_timeout,
    )? {
        LocalResponse::Query(response) => Ok(WorkSet::with_anchors(
            cwd.display().to_string(),
            response.records,
            response.anchors,
        )),
        response => anyhow::bail!("Unexpected daemon response: {response:?}"),
    }
}

/// Group fan-out: `team:...` (all members), `team/alice:...` (one member), or
/// `team/alice/proj-b:...` (one member, one contributed share). Returns
/// `Ok(None)` when `scope` is not a group on this device so the caller can
/// continue the scope cascade.
fn try_group(scope: &str, path: &str, filter: Filter, cwd: &Path) -> Result<Option<WorkSet>> {
    use crate::remote::ipc;
    use crate::remote::protocol::{LocalRequest, LocalResponse};

    let Some((group, member, share)) = split_group_scope(scope) else {
        return Ok(None);
    };
    crate::commands::remote::serve::ensure_running()
        .context("failed to start the sivtr daemon for a group query")?;
    // The daemon answers `None` for an unknown group; the fan-out happens
    // inside it (parallel per-member dials), so the socket read gets enough
    // headroom beyond the daemon's per-peer budget.
    match ipc::call_with_read_timeout(
        LocalRequest::GroupQuery {
            group,
            member,
            share,
            source: path.to_string(),
            filter,
        },
        GROUP_QUERY_TIMEOUT,
    )
    .context("group query failed")?
    {
        LocalResponse::GroupQuery(None) => Ok(None),
        LocalResponse::GroupQuery(Some(response)) => {
            if !response.skipped.is_empty() {
                output::info(format!(
                    "group members offline: {}",
                    response.skipped.join(", ")
                ));
            }
            Ok(Some(WorkSet::with_anchors(
                cwd.display().to_string(),
                response.query.records,
                response.query.anchors,
            )))
        }
        response => anyhow::bail!("Unexpected daemon response: {response:?}"),
    }
}

/// Split a group scope: `team` (all members), `team/alice` (one member), or
/// `team/alice/proj-b` (one member, one contributed share). Returns `None` for
/// anything that is not a valid group scope so the caller can fall through to
/// the local-workspace cascade.
fn split_group_scope(scope: &str) -> Option<(String, Option<String>, Option<String>)> {
    let parts: Vec<&str> = scope.split('/').collect();
    match parts.as_slice() {
        [group] if is_identifier(group) => Some((group.to_string(), None, None)),
        [group, member] if is_identifier(group) && is_identifier(member) => {
            Some((group.to_string(), Some(member.to_string()), None))
        }
        [group, member, share]
            if is_identifier(group) && is_identifier(member) && is_identifier(share) =>
        {
            Some((
                group.to_string(),
                Some(member.to_string()),
                Some(share.to_string()),
            ))
        }
        _ => None,
    }
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
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

#[cfg(test)]
mod tests {
    use super::split_group_scope;

    #[test]
    fn group_scope_splits_team_and_member_forms() {
        assert_eq!(
            split_group_scope("team"),
            Some(("team".to_string(), None, None))
        );
        assert_eq!(
            split_group_scope("team/alice"),
            Some(("team".to_string(), Some("alice".to_string()), None))
        );
        assert_eq!(
            split_group_scope("team/alice/proj-b"),
            Some((
                "team".to_string(),
                Some("alice".to_string()),
                Some("proj-b".to_string())
            ))
        );
        // Not group-shaped: falls through to the local-workspace cascade.
        assert_eq!(split_group_scope("team/a/b/c"), None);
        assert_eq!(split_group_scope("team@alice"), None);
        assert_eq!(split_group_scope(""), None);
        assert_eq!(split_group_scope("team/"), None);
        assert_eq!(split_group_scope("a b"), None);
    }
}
