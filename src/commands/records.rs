use anyhow::{Context, Result};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkRecord, WorkRecordIndex, WorkRef};
use sivtr_core::{session, workspace};
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn current_work_record_index(
    providers: &[AgentProvider],
    cwd: &Path,
    recent_sessions: Option<usize>,
) -> Result<WorkRecordIndex> {
    current_work_records(providers, cwd, recent_sessions).map(WorkRecordIndex::new)
}

fn current_work_records(
    providers: &[AgentProvider],
    cwd: &Path,
    recent_sessions: Option<usize>,
) -> Result<Vec<WorkRecord>> {
    let mut records = Vec::new();
    records.extend(terminal_records(cwd)?);
    records.extend(agent_records(providers, cwd, recent_sessions)?);
    dedup_records(&mut records);
    normalize_session_display_ids(&mut records);
    records.sort_by(|a, b| b.time.primary_at().cmp(&a.time.primary_at()));
    Ok(records)
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
) -> Result<Vec<WorkRecord>> {
    let mut records = Vec::new();

    for provider in providers {
        let source = provider.session_provider();
        let mut sessions = source.list_recent_sessions(Some(cwd))?;
        if let Some(limit) = recent_sessions {
            sessions.truncate(limit);
        }

        for info in sessions {
            let session = source.parse_session_file(&info.path)?;
            records.extend(WorkRecord::chat_turns(source.provider(), &session));
        }
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
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartIo, WorkPartKind, WorkRecordKind, WorkSessionRef,
        WorkSource, WorkTime,
    };

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
        records[1].parts.push(sivtr_core::record::WorkPart {
            io: sivtr_core::record::WorkPartIo::Output,
            kind: sivtr_core::record::WorkPartKind::AssistantMessage,
            index: 1,
            occurred_at: None,
            label: Some("assistant".to_string()),
            text: "assistant with more detail".to_string(),
            ansi: None,
            tags: Vec::new(),
        });

        dedup_records(&mut records);

        assert_eq!(records.len(), 1);
        assert!(records[0]
            .parts
            .iter()
            .any(|part| part.text == "assistant with more detail"));
        assert_eq!(records[0].session.id, "session-01234567");
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
                tags: Vec::new(),
                    ansi: None,
                },
                WorkPart {
                    io: WorkPartIo::Output,
                    kind: WorkPartKind::AssistantMessage,
                    index: 1,
                    occurred_at: None,
                    label: None,
                tags: Vec::new(),
                    text: "assistant".to_string(),
                    ansi: None,
                },
            ],
        }
    }
}
