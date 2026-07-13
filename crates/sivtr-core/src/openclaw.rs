use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::ai::{AgentProvider, AgentSession, AgentSessionInfo, AgentSessionProvider};

/// OpenClaw agent host support.
///
/// Session parsing is intentionally a stub for now: OpenClaw's runtime store is
/// SQLite (`agents/<id>/agent/openclaw-agent.sqlite`) plus optional legacy
/// `sessions/` archives. MCP install/detect works independently so hosts can
/// still use sivtr memory immediately.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenClawProvider;

impl AgentSessionProvider for OpenClawProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::OpenClaw
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

pub fn openclaw_home() -> PathBuf {
    if let Ok(path) = std::env::var("OPENCLAW_STATE_DIR").or_else(|_| std::env::var("OPENCLAW_HOME"))
    {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openclaw")
}

pub fn openclaw_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENCLAW_CONFIG_PATH") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    openclaw_home().join("openclaw.json")
}

#[cfg(test)]
mod tests {
    use super::{openclaw_config_path, openclaw_home, OpenClawProvider};
    use crate::ai::{AgentProvider, AgentSessionProvider};

    #[test]
    fn stub_provider_returns_no_sessions() {
        let provider = OpenClawProvider;
        assert_eq!(provider.provider(), AgentProvider::OpenClaw);
        assert!(provider
            .list_recent_sessions(None)
            .expect("openclaw sessions")
            .is_empty());
    }

    #[test]
    fn provider_name_is_openclaw() {
        assert_eq!(AgentProvider::OpenClaw.name(), "OpenClaw");
        assert_eq!(AgentProvider::OpenClaw.command_name(), "openclaw");
    }

    #[test]
    fn config_path_lives_under_home() {
        let home = openclaw_home();
        assert_eq!(openclaw_config_path(), home.join("openclaw.json"));
    }
}
