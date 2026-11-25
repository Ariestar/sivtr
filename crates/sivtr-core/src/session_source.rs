//! Terminal session logs.
//!
//! Agent transcripts are listed and parsed by [`crate::agents::profile`] and
//! stored by the archive sync engine. [`SessionSource`] is the terminal ingest
//! contract; tests also implement it for a mocked source.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::record::WorkRecord;
use crate::{session, workspace};

/// Metadata for one discovered session file: enough to filter, sort, and
/// address records without parsing the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Logical source path passed to the provider parser. Container providers
    /// use a virtual path so one physical database can expose many sessions.
    pub path: PathBuf,
    /// Physical file or directory used for freshness stamping. `None` means
    /// `path` itself is the physical source.
    pub physical_path: Option<PathBuf>,
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub modified: SystemTime,
}

/// A conversation source.
pub trait SessionSource {
    /// Cache namespace and selector prefix (`"terminal"`, `"codex"`, …).
    fn namespace(&self) -> &'static str;

    /// Discover session files, workspace-filtered when `cwd` is given.
    fn list_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>>;

    /// Parse one session file into records.
    fn parse_file(&self, path: &Path) -> Result<Vec<WorkRecord>>;
}

/// Terminal session logs. One namespace: users switch shells inside one
/// terminal session, so per-shell identity carries no signal.
pub struct TerminalSource;

impl SessionSource for TerminalSource {
    fn namespace(&self) -> &'static str {
        "terminal"
    }

    fn list_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let Some(cwd) = cwd else {
            return Ok(Vec::new());
        };
        let mut infos = Vec::new();
        let paths = workspace::terminal_log_paths_for_workspace(cwd).with_context(|| {
            format!(
                "Failed to list terminal sessions for workspace {}",
                cwd.display()
            )
        })?;
        for path in paths {
            let modified = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .with_context(|| format!("Failed to stamp terminal log {}", path.display()))?;
            infos.push(SessionInfo {
                path,
                physical_path: None,
                id: None,
                cwd: None,
                title: None,
                modified,
            });
        }
        Ok(infos)
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<WorkRecord>> {
        let entries = session::load_entries(path).context("Failed to read session log")?;
        Ok(entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| WorkRecord::terminal(entry, path, idx))
            .collect())
    }
}
