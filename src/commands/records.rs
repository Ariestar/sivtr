use std::path::Path;

use anyhow::Result;
use sivtr_core::ai::AgentProvider;
use sivtr_core::query::{load_workspace_records, load_workspace_source, SourceQueryResult};
use sivtr_core::record::WorkRecordIndex;

use crate::output;

/// Build the record index for the current workspace.
///
/// Thin wrapper over [`sivtr_core::query::load_workspace_records`] that keeps
/// the CLI behavior of warning about session files that failed to parse.
pub(crate) fn current_work_record_index(
    providers: &[AgentProvider],
    cwd: &Path,
    recent_sessions: Option<usize>,
) -> Result<WorkRecordIndex> {
    let result = load_workspace_records(providers, cwd, recent_sessions)?;
    warn_skipped(&result.skipped);
    Ok(result.into_index())
}

pub(crate) fn current_work_source(cwd: &Path, source: &str) -> Result<SourceQueryResult> {
    let result = load_workspace_source(cwd, source)?;
    warn_skipped(&result.skipped);
    Ok(result)
}

fn warn_skipped(skipped_sessions: &[sivtr_core::query::SkippedSession]) {
    for skipped in skipped_sessions {
        output::warning(format!(
            "failed to parse {} session {}: {:#}",
            skipped.provider.name(),
            skipped.path.display(),
            skipped.error,
        ));
    }
}
