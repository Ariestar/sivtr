use anyhow::{Context, Result};
use sivtr_core::ai::AgentProvider;
use sivtr_core::capture::scrollback;
use sivtr_core::record::WorkRecord;
use sivtr_core::session;
use std::path::Path;

pub(crate) fn current_work_records(
    providers: &[AgentProvider],
    cwd: &Path,
    recent_sessions: Option<usize>,
) -> Result<Vec<WorkRecord>> {
    let mut records = Vec::new();
    records.extend(terminal_records()?);
    records.extend(agent_records(providers, cwd, recent_sessions)?);
    records.sort_by(|a, b| b.time.occurred_at.cmp(&a.time.occurred_at));
    Ok(records)
}

fn terminal_records() -> Result<Vec<WorkRecord>> {
    let path = scrollback::session_log_path();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let entries = session::load_entries(&path).context("Failed to read session log")?;
    Ok(entries
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| WorkRecord::terminal(entry, &path, idx))
        .collect())
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
