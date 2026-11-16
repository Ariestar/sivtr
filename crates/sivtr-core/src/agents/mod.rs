//! Agent session providers and shared parsing skeletons.
//!
//! - [`model`]: shared session/block types and trait
//! - [`jsonl`]: JSONL discovery/parsing helpers (Claude/Codex/Hermes/Grok/Pi)
//! - [`sqlite`]: readonly SQLite helpers (OpenCode/OpenClaw)
//! - per-provider modules keep only storage paths + schema mapping

pub mod claude;
pub mod cmdc;
pub mod codex;
pub mod cursor;
pub mod dsh;
pub mod engine;
pub mod gemini;
pub mod generic;
pub mod goose;
pub mod grok;
pub mod hermes;
pub mod jsonl;
pub mod model;
pub mod openclaw;
pub mod opencode;
pub mod pi;
pub mod profile;
pub mod qoder;
pub mod qwen;
pub mod sqlite;
pub mod zcode;

pub use jsonl::{
    jsonl_files, list_chat_recording_sessions, list_chat_recording_sessions_outcome,
    list_recent_jsonl_sessions, list_recent_jsonl_sessions_outcome, list_sessions_matching,
    list_sessions_matching_outcome, parse_jsonl_meta, parse_jsonl_session,
};
pub use model::*;
pub use sqlite::{open_readonly_db, system_time_from_millis, system_time_from_unix_secs};

#[derive(Clone, Copy)]
pub struct AgentProviderSpec {
    pub provider: AgentProvider,
    pub name: &'static str,
    pub command_name: &'static str,
    pub current_transcript_env: Option<&'static str>,
    pub current_session_id_env: Option<&'static str>,
}

const AGENT_PROVIDER_SPECS: &[AgentProviderSpec] = &[
    AgentProviderSpec {
        provider: AgentProvider::Codex,
        name: "Codex",
        command_name: "codex",
        current_transcript_env: None,
        current_session_id_env: Some("CODEX_THREAD_ID"),
    },
    AgentProviderSpec {
        provider: AgentProvider::Claude,
        name: "Claude",
        command_name: "claude",
        current_transcript_env: Some("CLAUDE_TRANSCRIPT_PATH"),
        current_session_id_env: Some("CLAUDE_SESSION_ID"),
    },
    AgentProviderSpec {
        provider: AgentProvider::Cursor,
        name: "Cursor",
        command_name: "cursor",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Dsh,
        name: "Dsh",
        command_name: "dsh",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::OpenCode,
        name: "OpenCode",
        command_name: "opencode",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::OpenClaw,
        name: "OpenClaw",
        command_name: "openclaw",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Grok,
        name: "Grok",
        command_name: "grok",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Hermes,
        name: "Hermes",
        command_name: "hermes",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Pi,
        name: "Pi",
        command_name: "pi",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Qoder,
        name: "Qoder",
        command_name: "qoder",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::QoderCn,
        name: "Qoder-CN",
        command_name: "qoder-cn",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Gemini,
        name: "Gemini",
        command_name: "gemini",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Goose,
        name: "Goose",
        command_name: "goose",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Qwen,
        name: "Qwen",
        command_name: "qwen",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Zcode,
        name: "ZCode",
        command_name: "zcode",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::ClaudeAi,
        name: "Claude.ai",
        command_name: "claude-ai",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::ChatGpt,
        name: "ChatGPT",
        command_name: "chatgpt",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::GeminiApps,
        name: "Gemini Apps",
        command_name: "gemini-apps",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Amp,
        name: "Amp",
        command_name: "amp",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Aider,
        name: "Aider",
        command_name: "aider",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Antigravity,
        name: "Antigravity",
        command_name: "antigravity",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::AntigravityCli,
        name: "Antigravity CLI",
        command_name: "antigravity-cli",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::CommandCode,
        name: "Command Code",
        command_name: "cmdc",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Copilot,
        name: "Copilot",
        command_name: "copilot",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::DeepSeekTui,
        name: "DeepSeek TUI",
        command_name: "deepseek-tui",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Forge,
        name: "Forge",
        command_name: "forge",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Gptme,
        name: "gptme",
        command_name: "gptme",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Iflow,
        name: "iFlow",
        command_name: "iflow",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Kilo,
        name: "Kilo",
        command_name: "kilo",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Kimi,
        name: "Kimi",
        command_name: "kimi",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::KimiWork,
        name: "Kimi Work",
        command_name: "kimi-work",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Kiro,
        name: "Kiro",
        command_name: "kiro",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::OpenHands,
        name: "OpenHands",
        command_name: "openhands",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Poolside,
        name: "Poolside",
        command_name: "poolside",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::PositAssistant,
        name: "Posit Assistant",
        command_name: "posit-assistant",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::QwenPaw,
        name: "QwenPaw",
        command_name: "qwenpaw",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Reasonix,
        name: "Reasonix",
        command_name: "reasonix",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::RooCode,
        name: "RooCode",
        command_name: "roocode",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Shelley,
        name: "Shelley",
        command_name: "shelley",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Trae,
        name: "Trae",
        command_name: "trae",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::TraeX,
        name: "TraeX",
        command_name: "traex",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Vibe,
        name: "Mistral Vibe",
        command_name: "vibe",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::VSCodeCopilot,
        name: "VSCode Copilot",
        command_name: "vscode-copilot",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Windsurf,
        name: "Windsurf",
        command_name: "windsurf",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Zed,
        name: "Zed",
        command_name: "zed",
        current_transcript_env: None,
        current_session_id_env: None,
    },
    AgentProviderSpec {
        provider: AgentProvider::Zencoder,
        name: "Zencoder",
        command_name: "zencoder",
        current_transcript_env: None,
        current_session_id_env: None,
    },
];

impl AgentProvider {
    pub fn all() -> &'static [AgentProviderSpec] {
        AGENT_PROVIDER_SPECS
    }

    pub fn from_command_name(value: &str) -> Option<Self> {
        Self::all()
            .iter()
            .find(|spec| spec.command_name.eq_ignore_ascii_case(value))
            .map(|spec| spec.provider)
    }

    pub fn spec(self) -> &'static AgentProviderSpec {
        Self::all()
            .iter()
            .find(|spec| spec.provider == self)
            .expect("agent provider registry must contain every AgentProvider variant")
    }

    pub fn name(self) -> &'static str {
        self.spec().name
    }

    pub fn command_name(self) -> &'static str {
        self.spec().command_name
    }

    pub fn current_transcript_env(self) -> Option<&'static str> {
        self.spec().current_transcript_env
    }

    pub fn current_session_id_env(self) -> Option<&'static str> {
        self.spec().current_session_id_env
    }

    /// Whether this provider has a native local source to sync. Web exports
    /// are imported explicitly and therefore belong to the archive namespace
    /// without participating in native-source reconciliation. A generic
    /// provider without a declared transcript format only launches agents.
    pub fn has_native_source(self) -> bool {
        if matches!(self, Self::ClaudeAi | Self::ChatGpt | Self::GeminiApps) {
            return false;
        }
        profile::is_registered(self)
    }

    /// Registered provider CLI names (`codex`, `claude`, …), registry order.
    pub fn command_names() -> impl Iterator<Item = &'static str> {
        Self::all().iter().map(|spec| spec.command_name)
    }

    /// Comma-separated registered provider CLI names for help and errors.
    pub fn command_names_csv() -> String {
        Self::command_names().collect::<Vec<_>>().join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::AgentProvider;
    use std::collections::HashSet;

    #[test]
    fn registry_has_unique_expanded_provider_names() {
        let names: HashSet<_> = AgentProvider::command_names().collect();
        assert!(names.len() >= 40);
        assert_eq!(names.len(), AgentProvider::all().len());
        for spec in AgentProvider::all() {
            assert_eq!(
                AgentProvider::from_command_name(spec.command_name),
                Some(spec.provider)
            );
        }
    }
}
