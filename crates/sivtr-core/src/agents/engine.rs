//! One discovery/group/parse pipeline for every agent provider.
//!
//! Providers keep their native file parsing; this module owns the lifecycle
//! around it: group physical files by logical session, merge siblings, track
//! per-file stamps, and report whether a scan was complete enough to prune.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::agents::model::{AgentBlock, AgentProvider, AgentSession, SessionInfo};

/// One physical file that contributes to a logical session.
#[derive(Debug, Clone)]
pub struct SessionFile {
    pub path: PathBuf,
    pub physical_path: PathBuf,
    pub stamp: (u64, u32, u64),
}

/// One logical session assembled from one or more physical files.
#[derive(Debug, Clone)]
pub struct EngineSession {
    /// Canonical identity: the provider session id when present, else the
    /// representative source path.
    pub key: String,
    pub path: PathBuf,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub session: AgentSession,
    pub files: Vec<SessionFile>,
}

/// A provider scan. `complete` is false when any listed file failed to stamp
/// or parse, so the caller must not reconcile deletions from this result.
#[derive(Debug, Default)]
pub struct ProviderScan {
    pub sessions: Vec<EngineSession>,
    pub unchanged: usize,
    pub complete: bool,
    pub failures: Vec<(PathBuf, String)>,
}

impl ProviderScan {
    /// Largest stamp across a session's files. A change to any sibling moves
    /// the representative stamp and triggers a re-parse.
    pub fn session_stamp(session: &EngineSession) -> (u64, u32, u64) {
        session
            .files
            .iter()
            .map(|file| file.stamp)
            .max()
            .unwrap_or((0, 0, 0))
    }
}

/// Stable fingerprint of a session's physical files.
pub fn files_fingerprint(files: &[SessionFile]) -> u64 {
    let mut hasher = DefaultHasher::new();
    let mut paths: Vec<_> = files.iter().collect();
    paths.sort_by(|left, right| left.path.cmp(&right.path));
    for file in paths {
        file.path.hash(&mut hasher);
        file.stamp.hash(&mut hasher);
    }
    hasher.finish()
}

/// Discover, group, and parse one provider's sessions.
///
/// `known` maps archived source paths to their last file-set fingerprint. A
/// group whose fingerprint is unchanged is skipped.
pub fn scan_provider(
    provider: AgentProvider,
    cwd: Option<&Path>,
    full: bool,
    known: &HashMap<String, u64>,
) -> Result<ProviderScan> {
    let mut scan = ProviderScan {
        complete: true,
        ..ProviderScan::default()
    };
    let (infos, listing_failures) = crate::agents::profile::list(provider, cwd)?;
    if !listing_failures.is_empty() {
        scan.complete = false;
        scan.failures.extend(listing_failures);
    }
    let groups = group_sessions(provider, infos);

    // A vanished root looks like an empty listing. Keep the archive intact
    // unless the provider previously had nothing either.
    if groups.is_empty() && !known.is_empty() {
        scan.complete = false;
        return Ok(scan);
    }

    for (key, mut infos) in groups {
        let representative = chronological(&mut infos);
        let Some(files) = session_files(&infos, &mut scan.failures) else {
            scan.complete = false;
            continue;
        };
        if !full
            && known.get(representative.path.to_string_lossy().as_ref())
                == Some(&files_fingerprint(&files))
        {
            scan.unchanged += 1;
            continue;
        }

        let (session, sibling_failures) = match parse_group(provider, &infos) {
            Ok(parsed) => parsed,
            Err((path, error)) => {
                scan.failures.push((path, error));
                scan.complete = false;
                continue;
            }
        };
        if !sibling_failures.is_empty() {
            // Healthy siblings still publish; the scan stays incomplete so no
            // deletion is derived from it.
            scan.complete = false;
            scan.failures.extend(sibling_failures);
        }
        scan.sessions
            .push(finish_session(key, representative, session, files));
    }

    Ok(scan)
}

/// Find one session by its full id.
///
/// Used by exact-ref reads so the archive can refresh one session without the
/// global freshness TTL. A short prefix is a selector concern and is left to
/// the caller's selector path; `Ok(None)` keeps the archive untouched.
pub fn scan_session(
    provider: AgentProvider,
    session_id: &str,
    cwd: Option<&Path>,
) -> Result<Option<EngineSession>> {
    let mut infos: Vec<_> = crate::agents::profile::list(provider, cwd)?
        .0
        .into_iter()
        .filter(|info| info.id.as_deref() == Some(session_id))
        .collect();
    if infos.is_empty() {
        return Ok(None);
    }
    // Siblings of a multi-file session share this id, so the filter already
    // collected every file the session needs. The newest file represents
    // the session; the transcript is read oldest first.
    let key = session_key(provider, &infos[0]);
    let representative = chronological(&mut infos);
    let files = session_files(&infos, &mut Vec::new())
        .ok_or_else(|| anyhow::anyhow!("session source disappeared before sync"))?;
    let session = parse_group(provider, &infos)
        .map_err(|(path, error)| anyhow::anyhow!("{}: {error}", path.display()))?
        .0;
    Ok(Some(finish_session(key, representative, session, files)))
}

/// Providers whose logical session can span several native transcript files.
/// Newest file stays the representative. Parsing order is oldest first.
fn chronological(infos: &mut [SessionInfo]) -> SessionInfo {
    let representative = infos
        .iter()
        .max_by_key(|info| info.modified)
        .cloned()
        .expect("session group has at least one file");
    infos.sort_by_key(|info| info.modified);
    representative
}

fn merge_siblings(provider: AgentProvider) -> bool {
    crate::agents::profile::merges_siblings(provider)
}

/// Archived identity of one listed file: the logical id when the provider
/// merges siblings, else that file's own source path.
fn session_key(provider: AgentProvider, info: &SessionInfo) -> String {
    match info
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) if merge_siblings(provider) => id.to_string(),
        _ => info.path.to_string_lossy().into_owned(),
    }
}

fn group_sessions(
    provider: AgentProvider,
    infos: Vec<SessionInfo>,
) -> BTreeMap<String, Vec<SessionInfo>> {
    let mut groups: BTreeMap<String, Vec<SessionInfo>> = BTreeMap::new();
    for info in infos {
        let key = session_key(provider, &info);
        groups.entry(key).or_default().push(info);
    }
    groups
}

fn session_files(
    infos: &[SessionInfo],
    failures: &mut Vec<(PathBuf, String)>,
) -> Option<Vec<SessionFile>> {
    let mut files = Vec::with_capacity(infos.len());
    for info in infos {
        let physical_path = info
            .physical_path
            .as_deref()
            .unwrap_or(&info.path)
            .to_path_buf();
        let Some(stamp) = crate::cache::file_stamp(&physical_path) else {
            failures.push((
                info.path.clone(),
                format!(
                    "source disappeared before sync: {}",
                    physical_path.display()
                ),
            ));
            return None;
        };
        files.push(SessionFile {
            path: info.path.clone(),
            physical_path,
            stamp,
        });
    }
    Some(files)
}

type ParseFailure = (PathBuf, String);
type ParsedGroup = (AgentSession, Vec<ParseFailure>);

fn parse_group(
    provider: AgentProvider,
    infos: &[SessionInfo],
) -> std::result::Result<ParsedGroup, ParseFailure> {
    let representative = infos.first().expect("session group has at least one file");
    let base = crate::agents::profile::parse(provider, &representative.path)
        .map_err(|error| (representative.path.clone(), format!("{error:#}")))?;
    if !merge_siblings(provider) || infos.len() == 1 {
        return Ok((base, Vec::new()));
    }

    let mut segments = vec![base.blocks];
    let mut failures = Vec::new();
    for info in infos.iter().skip(1) {
        let sibling = match crate::agents::profile::parse(provider, &info.path) {
            Ok(sibling) => sibling,
            Err(error) => {
                // A writer racing the reader must not hide evidence in the
                // other segments.
                failures.push((info.path.clone(), format!("{error:#}")));
                continue;
            }
        };
        if sibling.id != base.id {
            failures.push((
                info.path.clone(),
                "segment identity changed after listing".to_string(),
            ));
            continue;
        }
        segments.push(sibling.blocks);
    }
    let blocks = merge_blocks(segments.into_iter());
    Ok((AgentSession { blocks, ..base }, failures))
}

/// Concatenate segments oldest file first, keeping duplicates only as often
/// as the segment that carries them most, then order all dated blocks.
fn merge_blocks(segments: impl Iterator<Item = Vec<AgentBlock>>) -> Vec<AgentBlock> {
    let mut merged: Vec<AgentBlock> = Vec::new();
    let mut seen: HashMap<BlockIdentity, usize> = HashMap::new();
    for blocks in segments {
        let mut occurrences: HashMap<BlockIdentity, usize> = HashMap::new();
        for block in blocks {
            let identity = block_identity(&block);
            let count = occurrences.entry(identity.clone()).or_insert(0);
            *count += 1;
            let published = seen.entry(identity).or_insert(0);
            if *count > *published {
                *published = *count;
                merged.push(block);
            }
        }
    }
    sort_dated_blocks(&mut merged);
    merged
}

type BlockIdentity = (u8, Option<String>, Option<String>, Option<String>, String);

fn block_identity(block: &AgentBlock) -> BlockIdentity {
    (
        block.kind as u8,
        block.timestamp.clone(),
        block.label.clone(),
        block.call_id.clone(),
        block.text.clone(),
    )
}

fn sort_dated_blocks(blocks: &mut [AgentBlock]) {
    if blocks
        .iter()
        .filter_map(|block| block.timestamp.as_deref())
        .is_sorted()
    {
        return;
    }
    let mut dated: Vec<_> = blocks
        .iter()
        .filter(|block| block.timestamp.is_some())
        .cloned()
        .collect();
    dated.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    for (slot, block) in blocks
        .iter_mut()
        .filter(|block| block.timestamp.is_some())
        .zip(dated)
    {
        *slot = block;
    }
}

fn finish_session(
    key: String,
    info: SessionInfo,
    mut session: AgentSession,
    files: Vec<SessionFile>,
) -> EngineSession {
    if session.id.as_deref().is_none_or(|id| id.trim().is_empty()) {
        session.id = info.id.clone().filter(|id| !id.trim().is_empty());
    }
    if session.cwd.is_none() {
        session.cwd = info.cwd.clone();
    }
    if session.title.is_none() {
        session.title = info.title.clone();
    }
    EngineSession {
        key,
        path: info.path,
        cwd: session.cwd.clone(),
        title: session.title.clone(),
        session,
        files,
    }
}
