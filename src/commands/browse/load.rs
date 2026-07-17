//! Source catalog and session grouping for the workspace browser.
//!
//! Multi-source schedule (parallel remotes + timeout) lives in
//! `workset::query_many`. This module only maps catalog ↔ TUI sessions.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// Per-source load state shown in the Source pane.
#[derive(Clone, Debug)]
pub enum SourceLoadState {
    Idle,
    Ready(Vec<WorkspaceSession>),
    Failed(String),
}

impl SourceLoadState {
    pub fn marker(&self) -> SourceLoadMarker {
        match self {
            Self::Idle => SourceLoadMarker::Idle,
            Self::Ready(_) => SourceLoadMarker::Ready,
            Self::Failed(_) => SourceLoadMarker::Failed,
        }
    }

    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Failed(message) => Some(message.as_str()),
            _ => None,
        }
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
    // Daemon may be down — treat as "no remotes", not a hard failure for the TUI.
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

/// Load every selected source that is still Idle/Failed via `workset::query_many`.
pub fn ensure_sources_loaded(
    sources: &[WorkspaceSource],
    selected: &[bool],
    states: &mut [SourceLoadState],
    cwd: &Path,
) -> Result<()> {
    let need: Vec<usize> = sources
        .iter()
        .enumerate()
        .filter(|(idx, _)| selected.get(*idx).copied().unwrap_or(false))
        .filter(|(idx, _)| !matches!(states.get(*idx), Some(SourceLoadState::Ready(_))))
        .map(|(idx, _)| idx)
        .collect();
    if need.is_empty() {
        return Ok(());
    }

    let batch: Vec<QuerySource> = need
        .iter()
        .map(|&idx| {
            let source = &sources[idx];
            if source.is_remote() {
                QuerySource::remote(source.selector())
            } else {
                QuerySource::local(source.selector())
            }
        })
        .collect();

    let results = workset::query_many(&batch, Filter::none(), Some(cwd), REMOTE_QUERY_TIMEOUT)?;
    for (slot, result) in need.into_iter().zip(results) {
        states[slot] = match result {
            QuerySourceResult::Ok(set) => {
                SourceLoadState::Ready(sessions_from_records(&sources[slot], set.records))
            }
            QuerySourceResult::Err(message) => {
                if sources[slot].is_remote() {
                    SourceLoadState::Failed(message)
                } else {
                    // Local hard-fail: surface as Failed so the pane stays up, but
                    // empty local is already Ok(empty) from query_many.
                    SourceLoadState::Failed(message)
                }
            }
        };
    }
    // Local hard errors already returned Err from query_many; remotes are Failed.
    Ok(())
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
        if let SourceLoadState::Ready(loaded) = &states[idx] {
            sessions.extend(loaded.iter().cloned());
        }
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
