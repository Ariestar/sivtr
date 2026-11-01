use std::path::Path;

use anyhow::Result;
use sivtr_core::agents::AgentProvider;
use sivtr_core::query::{load_workspace_records, LoadMode};
use sivtr_core::record::WorkRecordIndex;
use sivtr_core::session_source::workspace_sources;

/// Build the record index for the current workspace.
///
/// Thin wrapper over [`sivtr_core::query::load_workspace_records`] that keeps
/// the CLI behavior of warning about session files that failed to parse.
/// Loads metadata only (no part text); full-record callers use
/// [`crate::commands::memory::workset::query`] instead.
pub(crate) fn current_work_record_index(
    providers: &[AgentProvider],
    cwd: &Path,
    recent_sessions: Option<usize>,
) -> Result<WorkRecordIndex> {
    let sources = workspace_sources(providers);
    let result = load_workspace_records(&sources, cwd, recent_sessions, LoadMode::Light)?;
    warn_skipped(&result.skipped);
    Ok(result.into_index())
}

/// Report session files that failed to parse through the shared diagnostics
/// sink, not straight to stderr: the browse TUI loads through this path too,
/// and a direct write lands on top of its alternate screen.
pub(crate) fn warn_skipped(skipped_sessions: &[sivtr_core::query::SkippedSession]) {
    for skipped in skipped_sessions {
        sivtr_core::diagnostics::warn(format!(
            "failed to parse {} session {}: {:#}",
            skipped.namespace,
            skipped.path.display(),
            skipped.error,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::warn_skipped;
    use sivtr_core::query::SkippedSession;
    use std::path::PathBuf;

    #[test]
    fn skipped_sessions_go_to_the_diagnostics_sink() {
        warn_skipped(&[SkippedSession {
            namespace: "cursor".to_string(),
            path: PathBuf::from("/tmp/skipped-session-marker.jsonl"),
            error: "no stable session id".to_string(),
        }]);
        assert!(sivtr_core::diagnostics::log()
            .iter()
            .any(|entry| entry.contains("skipped-session-marker.jsonl")));
    }
}
