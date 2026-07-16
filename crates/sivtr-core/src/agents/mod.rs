//! Agent session providers and shared parsing skeletons.
//!
//! - [`model`]: shared session/block types and trait
//! - [`jsonl`]: JSONL discovery/parsing helpers (Claude/Codex/Hermes/Grok/Pi)
//! - [`sqlite`]: readonly SQLite helpers (OpenCode/OpenClaw)
//! - per-provider modules keep only storage paths + schema mapping

pub mod claude;
pub mod codex;
pub mod cursor;
pub mod grok;
pub mod hermes;
pub mod jsonl;
pub mod model;
pub mod openclaw;
pub mod opencode;
pub mod pi;
pub mod sqlite;

pub use jsonl::{jsonl_files, list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session};
pub use model::*;
pub use sqlite::{open_readonly_db, system_time_from_millis, system_time_from_unix_secs};

#[derive(Clone, Copy)]
pub struct AgentProviderSpec {
    pub provider: AgentProvider,
    pub name: &'static str,
    pub command_name: &'static str,
    pub current_transcript_env: Option<&'static str>,
    pub current_session_id_env: Option<&'static str>,
    factory: fn() -> Box<dyn AgentSessionProvider>,
}

const AGENT_PROVIDER_SPECS: &[AgentProviderSpec] = &[
    AgentProviderSpec {
        provider: AgentProvider::Codex,
        name: "Codex",
        command_name: "codex",
        current_transcript_env: None,
        current_session_id_env: Some("CODEX_THREAD_ID"),
        factory: codex_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Claude,
        name: "Claude",
        command_name: "claude",
        current_transcript_env: Some("CLAUDE_TRANSCRIPT_PATH"),
        current_session_id_env: Some("CLAUDE_SESSION_ID"),
        factory: claude_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Cursor,
        name: "Cursor",
        command_name: "cursor",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: cursor_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::OpenCode,
        name: "OpenCode",
        command_name: "opencode",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: opencode_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::OpenClaw,
        name: "OpenClaw",
        command_name: "openclaw",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: openclaw_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Grok,
        name: "Grok",
        command_name: "grok",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: grok_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Hermes,
        name: "Hermes",
        command_name: "hermes",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: hermes_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Pi,
        name: "Pi",
        command_name: "pi",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: pi_provider,
    },
];

fn codex_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::codex::CodexProvider)
}

fn claude_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::claude::ClaudeProvider)
}

fn cursor_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::cursor::CursorProvider)
}

fn opencode_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::opencode::OpenCodeProvider::default())
}

fn openclaw_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::openclaw::OpenClawProvider)
}

fn grok_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::grok::GrokProvider)
}

fn hermes_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::hermes::HermesProvider)
}

fn pi_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::agents::pi::PiProvider)
}

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

    pub fn session_provider(self) -> Box<dyn AgentSessionProvider> {
        (self.spec().factory)()
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
