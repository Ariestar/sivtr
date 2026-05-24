use anyhow::{Context, Result};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkRecord, WorkRecordIndex};
use sivtr_core::{session, workspace};
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
