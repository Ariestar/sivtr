//! Source catalog, session grouping, and non-blocking source loads for browse.
//!
//! Multi-source schedule lives in `workset::query_many`. This module maps catalog
//! ↔ TUI sessions and owns background load jobs so the event loop never blocks.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::commands::memory::filter::Filter;
use crate::commands::memory::workset::{self, QuerySource, QuerySourceResult, REMOTE_QUERY_TIMEOUT};
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse};
use crate::tui::workspace::{
    SourceLoadMarker, WorkspaceSession, WorkspaceSource, WorkspaceSourceKind,
};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::WorkRecord;
use sivtr_core::workspace;

/// Per-source load state. Sessions only appear when Ready (or stale Ready while Loading).
#[derive(Clone, Debug)]
pub enum SourceLoadState {
    Idle,
    Loading {
        /// Previous good snapshot, if any (stale-while-revalidate).
        stale: Option<Vec<WorkspaceSession>>,
        gen: u64,
    },
    Ready(Vec<WorkspaceSession>),
    Failed {
        #[allow(dead_code)]
        message: String,
        /// Keep last good sessions when a refresh fails.
        stale: Option<Vec<WorkspaceSession>>,
    },
}

impl SourceLoadState {
    pub fn marker(&self) -> SourceLoadMarker {
        match self {
            Self::Idle => SourceLoadMarker::Idle,
            Self::Loading { .. } => SourceLoadMarker::Loading,
            Self::Ready(_) => SourceLoadMarker::Ready,
            Self::Failed { .. } => SourceLoadMarker::Failed,
        }
    }

    /// Sessions that may be shown (Ready, or stale while Loading/Failed).
    pub fn visible_sessions(&self) -> &[WorkspaceSession] {
        match self {
            Self::Ready(sessions) => sessions,
            Self::Loading {
                stale: Some(sessions),
                ..
            }
            | Self::Failed {
                stale: Some(sessions),
                ..
            } => sessions,
            _ => &[],
        }
    }

    fn take_stale(&self) -> Option<Vec<WorkspaceSession>> {
        match self {
            Self::Ready(sessions) => Some(sessions.clone()),
            Self::Loading { stale, .. } | Self::Failed { stale, .. } => stale.clone(),
            Self::Idle => None,
        }
    }
}

/// Background job completion for one source load.
#[derive(Debug)]
pub struct SourceLoadEvent {
    pub index: usize,
    pub gen: u64,
    pub result: std::result::Result<Vec<WorkspaceSession>, String>,
}

/// Owns the channel from load workers into the TUI loop.
pub struct SourceLoadPump {
    tx: Sender<SourceLoadEvent>,
    rx: Receiver<SourceLoadEvent>,
    cwd: PathBuf,
    /// Monotonic generation per source index.
    gens: Vec<u64>,
    /// Debounce select-to-refresh for Ready sources.
    last_kick: Vec<Option<Instant>>,
}

impl SourceLoadPump {
    pub fn new(source_count: usize, cwd: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            cwd,
            gens: vec![0; source_count],
            last_kick: vec![None; source_count],
        }
    }

    /// Kick loads for selected sources that need work.
    ///
    /// - Idle / Failed → always load
    /// - Ready → refresh if `refresh_ready` and debounce elapsed
    /// - Loading → skip (already in flight)
    pub fn kick(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        refresh_ready: bool,
    ) {
        const READY_DEBOUNCE: Duration = Duration::from_millis(800);
        let now = Instant::now();
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            let needs = match &states[idx] {
                SourceLoadState::Loading { .. } => false,
                SourceLoadState::Idle | SourceLoadState::Failed { .. } => true,
                SourceLoadState::Ready(_) => {
                    refresh_ready
                        && self.last_kick.get(idx).and_then(|t| *t).is_none_or(|t| {
                            now.duration_since(t) >= READY_DEBOUNCE
                        })
                }
            };
            if !needs {
                continue;
            }
            self.spawn_one(idx, source, states);
        }
    }

    /// Force-refresh selected sources (manual `R`), ignoring debounce.
    pub fn refresh_selected(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
    ) {
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            if matches!(states.get(idx), Some(SourceLoadState::Loading { .. })) {
                continue;
            }
            self.spawn_one(idx, source, states);
        }
    }

    fn spawn_one(
        &mut self,
        idx: usize,
        source: &WorkspaceSource,
        states: &mut [SourceLoadState],
    ) {
        assert!(
            idx < self.gens.len() && idx < states.len(),
            "source index out of range"
        );
        self.gens[idx] = self.gens[idx].saturating_add(1);
        let gen = self.gens[idx];
        self.last_kick[idx] = Some(Instant::now());

        let stale = states[idx].take_stale();
        states[idx] = SourceLoadState::Loading {
            stale,
            gen,
        };

        let selector = source.selector();
        let remote = source.is_remote();
        let cwd = self.cwd.clone();
        let tx = self.tx.clone();
        let source = source.clone();
        thread::spawn(move || {
            let query_source = if remote {
                QuerySource::remote(selector)
            } else {
                QuerySource::local(selector)
            };
            let result = match workset::query_many(
                &[query_source],
                Filter::none(),
                Some(&cwd),
                REMOTE_QUERY_TIMEOUT,
            ) {
                Ok(mut results) => match results.pop() {
                    Some(QuerySourceResult::Ok(set)) => {
                        Ok(sessions_from_records(&source, set.records))
                    }
                    Some(QuerySourceResult::Err(message)) => Err(message),
                    None => Err("empty query result".to_string()),
                },
                Err(error) => Err(format!("{error:#}")),
            };
            let _ = tx.send(SourceLoadEvent {
                index: idx,
                gen,
                result,
            });
        });
    }

    /// Apply all completed jobs. Returns true if UI should redraw.
    pub fn drain(&mut self, states: &mut [SourceLoadState]) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(event) => {
                    if self.apply(event, states) {
                        changed = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        changed
    }

    fn apply(&self, event: SourceLoadEvent, states: &mut [SourceLoadState]) -> bool {
        let Some(state) = states.get_mut(event.index) else {
            return false;
        };
        // Drop stale completions.
        if let SourceLoadState::Loading { gen, .. } = state {
            if *gen != event.gen {
                return false;
            }
        } else {
            // Source was reset; ignore.
            return false;
        }
        let stale = state.take_stale();
        *state = match event.result {
            Ok(sessions) => SourceLoadState::Ready(sessions),
            Err(message) => SourceLoadState::Failed { message, stale },
        };
        true
    }
}

/// Source pane catalog: local kinds plus every mounted remote×kind.
pub fn workspace_source_catalog(
    providers: &[AgentProvider],
    cwd: &Path,
) -> Result<Vec<WorkspaceSource>> {
    let mut sources = Vec::new();
    sources.push(WorkspaceSource::terminal());
    for provider in providers {
        sources.push(WorkspaceSource::agent(*provider));
    }

    for alias in list_remote_aliases(cwd)? {
        sources.push(WorkspaceSource::scoped(
            &alias,
            WorkspaceSourceKind::Terminal,
        ));
        for provider in providers {
            sources.push(WorkspaceSource::scoped(
                &alias,
                WorkspaceSourceKind::Agent(*provider),
            ));
        }
    }
    Ok(sources)
}

fn list_remote_aliases(cwd: &Path) -> Result<Vec<String>> {
    let Some(ws) = workspace::resolve_workspace_for_dir(cwd)? else {
        return Ok(Vec::new());
    };
    if crate::commands::remote::serve::ensure_running().is_err() {
        return Ok(Vec::new());
    }
    match ipc::call(LocalRequest::RemoteList {
        workspace_key: ws.key,
    }) {
        Ok(LocalResponse::Mounts(mounts)) => Ok(mounts.into_iter().map(|m| m.alias).collect()),
        Ok(_) | Err(_) => Ok(Vec::new()),
    }
}

pub fn collect_ready_sessions(
    sources: &[WorkspaceSource],
    selected: &[bool],
    states: &[SourceLoadState],
) -> Vec<WorkspaceSession> {
    let mut sessions = Vec::new();
    for (idx, _source) in sources.iter().enumerate() {
        if !selected.get(idx).copied().unwrap_or(false) {
            continue;
        }
        sessions.extend(states[idx].visible_sessions().iter().cloned());
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
    sessions
}

/// Group flat WorkRecords into one WorkspaceSession per session id.
pub fn sessions_from_records(
    source: &WorkspaceSource,
    records: Vec<WorkRecord>,
) -> Vec<WorkspaceSession> {
    let mut groups: BTreeMap<String, Vec<WorkRecord>> = BTreeMap::new();
    for record in records {
        let key = record.work_ref.session().to_string();
        groups.entry(key).or_default().push(record);
    }

    let mut sessions = Vec::with_capacity(groups.len());
    for (session_id, mut records) in groups {
        records.sort_by_key(|record| record.work_ref.path.index());
        let modified = records
            .iter()
            .filter_map(record_modified)
            .max()
            .unwrap_or(UNIX_EPOCH);
        let search_title = session_search_title(&session_id, &records);
        let title = session_title_with_id(search_title.clone(), Some(session_id.as_str()));
        sessions.push(WorkspaceSession {
            source: source.clone(),
            modified,
            title,
            search_title,
            records,
        });
    }
    sessions
}

fn session_search_title(session_id: &str, records: &[WorkRecord]) -> String {
    records
        .iter()
        .find_map(|record| {
            let title = record.title.trim();
            if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            }
        })
        .unwrap_or_else(|| session_id.to_string())
}

fn session_title_with_id(title: String, id: Option<&str>) -> String {
    let id = id.map(|value| value.chars().take(8).collect::<String>());
    match id {
        Some(id) if !id.is_empty() => format!("{title}  [{id}]"),
        _ => title,
    }
}

fn record_modified(record: &WorkRecord) -> Option<SystemTime> {
    let stamp = record.time.primary_at()?;
    let dt = DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))?;
    let secs = dt.timestamp().max(0) as u64;
    let nanos = dt.timestamp_subsec_nanos();
    Some(UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_nanos(nanos.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource, WorkTime,
    };

    #[test]
    fn sessions_from_records_groups_by_session() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let records = vec![
            test_record("s1", 1, "first", "2026-07-17T10:00:00Z"),
            test_record("s1", 2, "second", "2026-07-17T11:00:00Z"),
            test_record("s2", 1, "other", "2026-07-17T12:00:00Z"),
        ];
        let sessions = sessions_from_records(&source, records);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].records.len(), 2);
        assert_eq!(sessions[0].search_title, "first");
        assert_eq!(sessions[1].search_title, "other");
        assert!(!sessions[0].source.is_remote());
    }

    fn test_record(session: &str, index: usize, title: &str, ended_at: &str) -> WorkRecord {
        WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, session, index),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: session.to_string(),
                canonical_id: Some(session.to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::from_components(None, Some(ended_at.to_string()), None),
            status: None,
            title: title.to_string(),
            parts: vec![],
        }
    }
}
