use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::ai::{
    extract_content_text, list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_value, push_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionInfo,
    AgentSessionMeta, AgentSessionProvider,
};

const PROVIDER_NAME: &str = "Pi";

#[derive(Debug, Clone, Copy, Default)]
pub struct PiProvider;

impl AgentSessionProvider for PiProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Pi
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        list_recent_jsonl_sessions(&pi_sessions_dir(), cwd, parse_session_meta)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

pub fn pi_home() -> PathBuf {
    if let Ok(path) = std::env::var("PI_HOME") {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
}

pub fn pi_sessions_dir() -> PathBuf {
    pi_home().join("agent").join("sessions")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 1, update_meta)
}

fn update_meta(meta: &mut AgentSessionMeta, value: &Value) {
    if value.get("type").and_then(Value::as_str) != Some("session") {
        return;
    }

    meta.id = value.get("id").and_then(Value::as_str).map(str::to_string);
    if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
        meta.add_cwd(cwd);
    }
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    match value.get("type").and_then(Value::as_str) {
        Some("session") => update_session_meta(session, value),
        Some("message") => apply_message(session, value),
        Some("model_change" | "thinking_level_change" | "compaction") => {}
        _ => {}
    }
}

fn update_session_meta(session: &mut AgentSession, value: &Value) {
    if session.id.is_none() {
        session.id = value.get("id").and_then(Value::as_str).map(str::to_string);
    }
    if session.cwd.is_none() {
        session.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
    }
}

fn apply_message(session: &mut AgentSession, value: &Value) {
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let message = value.get("message").unwrap_or(&Value::Null);

    match message.get("role").and_then(Value::as_str) {
        Some("user") => push_text_items(
            session,
            AgentBlockKind::User,
            timestamp,
            message.get("content").unwrap_or(&Value::Null),
        ),
        Some("assistant") => push_assistant_items(
            session,
            timestamp,
            message.get("content").unwrap_or(&Value::Null),
        ),
        Some("toolResult") => push_text_items(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            message.get("content").unwrap_or(&Value::Null),
        ),
        _ => {}
    }
}

fn push_assistant_items(session: &mut AgentSession, timestamp: Option<String>, content: &Value) {
    match content {
        Value::Array(items) => {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => push_block(
                        session,
                        AgentBlockKind::Assistant,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    ),
                    Some("toolCall") => push_block(
                        session,
                        AgentBlockKind::ToolCall,
                        timestamp.clone(),
                        item.get("name").and_then(Value::as_str).map(str::to_string),
                        item.get("arguments")
                            .map(pretty_json_value)
                            .unwrap_or_default(),
                    ),
                    Some("thinking") => {}
                    _ => {}
                }
            }
        }
        Value::String(text) => push_block(
            session,
            AgentBlockKind::Assistant,
            timestamp,
            None,
            text.clone(),
        ),
        _ => {}
    }
}

fn push_text_items(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    content: &Value,
) {
    push_block(
        session,
        kind,
        timestamp,
        None,
        extract_content_text(content),
    );
}

#[cfg(test)]
mod tests {
    use super::PiProvider;
    use crate::ai::{AgentBlockKind, AgentSessionProvider};

    #[test]
    fn parses_pi_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","version":3,"id":"pi-session","timestamp":"2026-05-21T00:00:00Z","cwd":"D:\\sivtr"}
{"type":"message","timestamp":"2026-05-21T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}
{"type":"message","timestamp":"2026-05-21T00:00:02Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"toolCall","name":"bash","arguments":{"command":"echo hi"}}]}}
{"type":"message","timestamp":"2026-05-21T00:00:03Z","message":{"role":"toolResult","content":[{"type":"text","text":"hi"}]}}
{"type":"message","timestamp":"2026-05-21T00:00:04Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
{"type":"compaction","summary":"summary should not become a block"}
"#,
        )
        .unwrap();

        let session = PiProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("pi-session"));
        assert_eq!(session.cwd.as_deref(), Some("D:\\sivtr"));
        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("bash"));
        assert!(session.blocks[1].text.contains("echo hi"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[2].text, "hi");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "done");
    }
}
