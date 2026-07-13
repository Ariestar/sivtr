use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::agents::{AgentProvider, AgentSession, AgentSessionInfo, AgentSessionProvider};

#[derive(Debug, Clone, Copy, Default)]
pub struct CursorProvider;

impl AgentSessionProvider for CursorProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Cursor
    }

    fn list_recent_sessions(&self, _cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        Ok(Vec::new())
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        Ok(AgentSession {
            path: path.to_path_buf(),
            id: None,
            cwd: None,
            title: None,
            blocks: Vec::new(),
        })
    }
}

pub fn cursor_home() -> PathBuf {
    if let Ok(path) = std::env::var("CURSOR_HOME") {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cursor")
}

#[cfg(test)]
mod tests {
    use super::CursorProvider;
    use crate::agents::{AgentProvider, AgentSessionProvider};

    #[test]
    fn stub_provider_returns_no_sessions() {
        let provider = CursorProvider;
        assert_eq!(provider.provider(), AgentProvider::Cursor);
        assert!(provider
            .list_recent_sessions(None)
            .expect("cursor sessions")
            .is_empty());
    }

    #[test]
    fn provider_name_is_cursor() {
        assert_eq!(AgentProvider::Cursor.name(), "Cursor");
    }
}
