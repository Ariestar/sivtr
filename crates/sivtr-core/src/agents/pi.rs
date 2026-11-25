use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, list_recent_jsonl_sessions_outcome, parse_jsonl_meta,
    parse_jsonl_session, pretty_json_value, push_block, AgentBlockKind, AgentProvider,
    AgentSession, AgentSessionMeta, SessionListing,
};

const PROVIDER_NAME: &str = "Pi";

#[derive(Debug, Clone, Copy, Default)]
pub struct PiProvider;

pub(crate) fn list_outcome(_provider: AgentProvider, cwd: Option<&Path>) -> Result<SessionListing> {
    list_recent_jsonl_sessions_outcome(PROVIDER_NAME, &pi_sessions_dir(), cwd, parse_session_meta)
}

pub(crate) fn parse_file(_provider: AgentProvider, path: &Path) -> Result<AgentSession> {
    parse_jsonl_session(path, PROVIDER_NAME, apply_event)
}

/// Pi's agent folder: `$PI_CODING_AGENT_DIR`, else `~/.pi/agent`.
pub fn pi_home() -> PathBuf {
    if let Some(path) = non_empty_env("PI_CODING_AGENT_DIR") {
        return path;
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
        .join("agent")
}

pub fn pi_sessions_dir() -> PathBuf {
    if let Some(path) = non_empty_env("PI_CODING_AGENT_SESSION_DIR") {
        return path;
    }
    if let Some(path) = session_dir_setting() {
        return path;
    }
    pi_home().join("sessions")
}

fn non_empty_env(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

fn session_dir_setting() -> Option<PathBuf> {
    let text = std::fs::read_to_string(pi_home().join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let dir = value
        .get("sessionDir")
        .and_then(Value::as_str)
        .filter(|dir| !dir.trim().is_empty())?;
    Some(expand_home(dir))
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest);
    }
    PathBuf::from(path)
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
                    Some("thinking") => {
                        let thinking = item
                            .get("thinking")
                            .map(extract_content_text)
                            .filter(|text| !text.trim().is_empty())
                            .unwrap_or_else(|| extract_content_text(item));
                        push_block(
                            session,
                            AgentBlockKind::Thinking,
                            timestamp.clone(),
                            None,
                            thinking,
                        );
                    }
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
    use super::parse_file;
    use crate::agents::{AgentBlockKind, AgentProvider};

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

        let session = parse_file(AgentProvider::Pi, &path).unwrap();

        assert_eq!(session.id.as_deref(), Some("pi-session"));
        assert_eq!(session.cwd.as_deref(), Some("D:\\sivtr"));
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "hidden");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("bash"));
        assert!(session.blocks[2].text.contains("echo hi"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[3].text, "hi");
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[4].text, "done");
    }
}
