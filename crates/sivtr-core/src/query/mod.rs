//! Workspace query orchestration.
//!
//! Aggregates terminal and agent records for a workspace, deduplicates them,
//! and normalizes session display ids. This is the shared read surface used by
//! both the CLI (`show`/`search`/`copy`/`work`/`nav`/`zoom`) and the server
//! transport (`sivtr serve`). Callers decide how to surface
//! [`QueryResult::skipped`] parse failures — the core does no printing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ai::{AgentProvider, AgentSessionProvider};
use crate::record::{WorkRecord, WorkRecordIndex, WorkRef};
use crate::{session, workspace};

/// A session file that could not be parsed, retained so callers can warn.
#[derive(Debug, Clone)]
pub struct SkippedSession {
    pub provider: AgentProvider,
    pub path: PathBuf,
    /// Rendered error message; `anyhow::Error` is not `Clone`, so the reason is
    /// stored as a string for cheap retention and reporting.
    pub error: String,
}

/// The outcome of building a workspace record index.
#[derive(Debug, Default)]
pub struct QueryResult {
    /// Records successfully loaded, ready for `WorkRecordIndex::new`.
    pub records: Vec<WorkRecord>,
    /// Session files that failed to parse, with the reason.
    pub skipped: Vec<SkippedSession>,
}

impl QueryResult {
    pub fn into_index(self) -> WorkRecordIndex {
        WorkRecordIndex::new(self.records)
    }
}

/// Build the record index for a workspace: terminal records plus agent records
/// for the given providers, deduplicated and sorted newest-first.
///
/// `recent_sessions` truncates how many recent agent sessions each provider
/// contributes (terminal logs are always fully loaded for the workspace).
pub fn load_workspace_records(
    providers: &[AgentProvider],
    cwd: &Path,
    recent_sessions: Option<usize>,
) -> Result<QueryResult> {
    let mut result = QueryResult::default();
    result.records.extend(terminal_records(cwd)?);
    result.records.extend(agent_records(
        providers,
        cwd,
        recent_sessions,
        &mut result.skipped,
    )?);
    dedup_records(&mut result.records);
    normalize_session_display_ids(&mut result.records);
    result
        .records
        .sort_by(|a, b| b.time.primary_at().cmp(&a.time.primary_at()));
    Ok(result)
}

fn terminal_records(cwd: &Path) -> Result<Vec<WorkRecord>> {
    let mut records = Vec::new();
    for path in workspace::terminal_log_paths_for_workspace(cwd)? {
        let entries = session::load_entries(&path).context("Failed to read session log")?;
        records.extend(
            entries
                .iter()
                .enumerate()
                .filter_map(|(idx, entry)| WorkRecord::terminal(entry, &path, idx)),
        );
    }
    Ok(records)
}

fn agent_records(
    providers: &[AgentProvider],
    cwd: &Path,
    recent_sessions: Option<usize>,
    skipped: &mut Vec<SkippedSession>,
) -> Result<Vec<WorkRecord>> {
    let mut records = Vec::new();
    for provider in providers {
        let source = provider.session_provider();
        records.extend(agent_records_from_source(
            source.as_ref(),
            cwd,
            recent_sessions,
            skipped,
        )?);
    }
    Ok(records)
}

fn agent_records_from_source(
    source: &dyn AgentSessionProvider,
    cwd: &Path,
    recent_sessions: Option<usize>,
    skipped: &mut Vec<SkippedSession>,
) -> Result<Vec<WorkRecord>> {
    let mut records = Vec::new();
    let mut sessions = source.list_recent_sessions(Some(cwd))?;
    if let Some(limit) = recent_sessions {
        sessions.truncate(limit);
    }

    for info in sessions {
        let session = match source.parse_session_file(&info.path) {
            Ok(session) => session,
            Err(error) => {
                skipped.push(SkippedSession {
                    provider: source.provider(),
                    path: info.path,
                    error: format!("{error:#}"),
                });
                continue;
            }
        };
        records.extend(WorkRecord::chat_turns(source.provider(), &session));
    }

    Ok(records)
}

fn dedup_records(records: &mut Vec<WorkRecord>) {
    let mut positions: HashMap<String, usize> = HashMap::new();
    let mut deduped = Vec::with_capacity(records.len());

    for record in records.drain(..) {
        let key = record_identity_key(&record);
        if let Some(position) = positions.get(&key).copied() {
            if record_is_better(&record, &deduped[position]) {
                deduped[position] = record;
            }
            continue;
        }

        positions.insert(key, deduped.len());
        deduped.push(record);
    }

    *records = deduped;
}

fn record_identity_key(record: &WorkRecord) -> String {
    match (&record.session.canonical_id, &record.work_ref) {
        (Some(canonical_id), WorkRef::Terminal { record_index, .. }) => {
            format!("terminal:{canonical_id}:{record_index}")
        }
        (
            Some(canonical_id),
            WorkRef::Agent {
                provider,
                turn_index,
                ..
            },
        ) => format!("{}:{canonical_id}:{turn_index}", provider.command_name()),
        (None, _) => record.work_ref.to_string(),
    }
}

fn record_is_better(candidate: &WorkRecord, existing: &WorkRecord) -> bool {
    candidate
        .parts
        .len()
        .cmp(&existing.parts.len())
        .then_with(|| {
            candidate
                .combined_text()
                .len()
                .cmp(&existing.combined_text().len())
        })
        .then_with(|| candidate.time.primary_at().cmp(&existing.time.primary_at()))
        .is_gt()
}

fn normalize_session_display_ids(records: &mut [WorkRecord]) {
    let mut source_sessions: HashMap<String, Vec<String>> = HashMap::new();

    for record in records.iter() {
        let Some(canonical_id) = record.session.canonical_id.as_deref() else {
            continue;
        };
        let source_key = session_source_key(&record.work_ref);
        let sessions = source_sessions.entry(source_key).or_default();
        if !sessions.iter().any(|existing| existing == canonical_id) {
            sessions.push(canonical_id.to_string());
        }
    }

    for record in records.iter_mut() {
        let Some(canonical_id) = record.session.canonical_id.as_deref() else {
            continue;
        };
        let source_key = session_source_key(&record.work_ref);
        let Some(all_sessions) = source_sessions.get(&source_key) else {
            continue;
        };
        let display_id = compact_unique_session_id(canonical_id, all_sessions);
        if record.session.id != display_id {
            rewrite_record_session_display_id(record, &display_id);
        }
    }
}

fn session_source_key(reference: &WorkRef) -> String {
    match reference {
        WorkRef::Terminal { .. } => "terminal".to_string(),
        WorkRef::Agent { provider, .. } => format!("agent:{}", provider.command_name()),
    }
}

fn compact_unique_session_id(canonical_id: &str, all_sessions: &[String]) -> String {
    let canonical_len = canonical_id.chars().count();
    if canonical_len <= 8 {
        return canonical_id.to_string();
    }

    for prefix_len in 8..=canonical_len {
        let candidate = prefix_chars(canonical_id, prefix_len);
        let unique = all_sessions
            .iter()
            .all(|other| other == canonical_id || prefix_chars(other, prefix_len) != candidate);
        if unique {
            return candidate;
        }
    }

    canonical_id.to_string()
}

fn prefix_chars(value: &str, len: usize) -> String {
    value.chars().take(len).collect()
}

fn rewrite_record_session_display_id(record: &mut WorkRecord, display_id: &str) {
    record.session.id = display_id.to_string();
    record.work_ref = match &record.work_ref {
        WorkRef::Terminal { record_index, .. } => {
            WorkRef::terminal_record(display_id, *record_index)
        }
        WorkRef::Agent {
            provider,
            turn_index,
            ..
        } => WorkRef::agent_record(*provider, display_id, *turn_index),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{
        AgentBlock, AgentBlockKind, AgentSession, AgentSessionInfo, AgentSessionProvider,
    };
    use crate::record::{
        WorkChannel, WorkPart, WorkPartIo, WorkPartKind, WorkRecordKind, WorkSessionRef,
        WorkSource, WorkTime,
    };
    use anyhow::Result;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    #[test]
    fn keeps_short_session_ids_when_already_unique() {
        let mut records = vec![test_record(
            WorkRef::agent_record(AgentProvider::Codex, "abcdef12", 1),
            "abcdef12",
            Some("abcdef1234567890"),
        )];

        normalize_session_display_ids(&mut records);

        assert_eq!(records[0].session.id, "abcdef12");
        assert_eq!(records[0].work_ref.to_string(), "codex/abcdef12/1");
    }

    #[test]
    fn extends_display_ids_to_break_canonical_prefix_collisions() {
        let mut records = vec![
            test_record(
                WorkRef::agent_record(AgentProvider::Codex, "abcdef12", 1),
                "abcdef12",
                Some("abcdef1234567890"),
            ),
            test_record(
                WorkRef::agent_record(AgentProvider::Codex, "abcdef12", 2),
                "abcdef12",
                Some("abcdef1299999999"),
            ),
        ];

        normalize_session_display_ids(&mut records);

        assert_eq!(records[0].session.id, "abcdef123");
        assert_eq!(records[0].work_ref.to_string(), "codex/abcdef123/1");
        assert_eq!(records[1].session.id, "abcdef129");
        assert_eq!(records[1].work_ref.to_string(), "codex/abcdef129/2");
    }

    #[test]
    fn keeps_provider_namespaces_independent_for_compaction() {
        let mut records = vec![
            test_record(
                WorkRef::agent_record(AgentProvider::Codex, "abcdef12", 1),
                "abcdef12",
                Some("abcdef1234567890"),
            ),
            test_record(
                WorkRef::agent_record(AgentProvider::Claude, "abcdef12", 1),
                "abcdef12",
                Some("abcdef1299999999"),
            ),
        ];

        normalize_session_display_ids(&mut records);

        assert_eq!(records[0].session.id, "abcdef12");
        assert_eq!(records[1].session.id, "abcdef12");
    }

    #[test]
    fn deduplicates_canonical_records_and_keeps_more_complete_copy() {
        let mut records = vec![
            test_record(
                WorkRef::agent_record(AgentProvider::Codex, "abcdef12", 1),
                "abcdef12",
                Some("session-0123456789abcdef"),
            ),
            test_record(
                WorkRef::agent_record(AgentProvider::Codex, "session-01234567", 1),
                "session-01234567",
                Some("session-0123456789abcdef"),
            ),
        ];
        records[1].parts.push(WorkPart {
            io: WorkPartIo::Output,
            kind: WorkPartKind::AssistantMessage,
            index: 1,
            occurred_at: None,
            label: Some("assistant".to_string()),
            text: "assistant with more detail".to_string(),
            ansi: None,
        });

        dedup_records(&mut records);

        assert_eq!(records.len(), 1);
        assert!(records[0]
            .parts
            .iter()
            .any(|part| part.text == "assistant with more detail"));
        assert_eq!(records[0].session.id, "session-01234567");
    }

    #[test]
    fn agent_records_skips_malformed_session_files_and_reports_them() {
        let cwd = PathBuf::from("/repo");
        let source = BrokenAgentSource {
            infos: vec![
                AgentSessionInfo {
                    path: PathBuf::from("broken.jsonl"),
                    id: Some("broken".to_string()),
                    cwd: Some("/repo".to_string()),
                    title: Some("broken".to_string()),
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                },
                AgentSessionInfo {
                    path: PathBuf::from("good.jsonl"),
                    id: Some("good".to_string()),
                    cwd: Some("/repo".to_string()),
                    title: Some("good".to_string()),
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                },
            ],
        };
        let mut skipped = Vec::new();
        let records = agent_records_from_source(&source, &cwd, Some(10), &mut skipped).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session.id, "good");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].path, PathBuf::from("broken.jsonl"));
        assert_eq!(skipped[0].provider, AgentProvider::Claude);
    }

    struct BrokenAgentSource {
        infos: Vec<AgentSessionInfo>,
    }

    impl AgentSessionProvider for BrokenAgentSource {
        fn provider(&self) -> AgentProvider {
            AgentProvider::Claude
        }

        fn list_recent_sessions(&self, _cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
            Ok(self.infos.clone())
        }

        fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
            if path == Path::new("broken.jsonl") {
                anyhow::bail!("synthetic parse error")
            }

            Ok(AgentSession {
                path: path.to_path_buf(),
                id: Some("good".to_string()),
                cwd: Some("/repo".to_string()),
                title: Some("good".to_string()),
                blocks: vec![
                    AgentBlock {
                        kind: AgentBlockKind::User,
                        timestamp: None,
                        label: None,
                        text: "question".to_string(),
                    },
                    AgentBlock {
                        kind: AgentBlockKind::Assistant,
                        timestamp: None,
                        label: None,
                        text: "assistant".to_string(),
                    },
                ],
            })
        }
    }

    fn test_record(work_ref: WorkRef, display_id: &str, canonical_id: Option<&str>) -> WorkRecord {
        WorkRecord {
            schema_version: 1,
            work_ref: work_ref.clone(),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: display_id.to_string(),
                canonical_id: canonical_id.map(str::to_string),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "title".to_string(),
            parts: vec![
                WorkPart {
                    io: WorkPartIo::Input,
                    kind: WorkPartKind::UserMessage,
                    index: 1,
                    occurred_at: None,
                    label: None,
                    text: "user".to_string(),
                    ansi: None,
                },
                WorkPart {
                    io: WorkPartIo::Output,
                    kind: WorkPartKind::AssistantMessage,
                    index: 1,
                    occurred_at: None,
                    label: None,
                    text: "assistant".to_string(),
                    ansi: None,
                },
            ],
        }
    }
}
