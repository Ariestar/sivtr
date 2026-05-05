use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::ai::{
    extract_content_text, list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_value, push_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionInfo,
    AgentSessionMeta, AgentSessionProvider,
};

const PROVIDER_NAME: &str = "Claude";

#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeProvider;

impl AgentSessionProvider for ClaudeProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Claude
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        list_recent_jsonl_sessions(&claude_home().join("projects"), cwd, parse_session_meta)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

pub fn claude_home() -> PathBuf {
    if let Ok(path) = std::env::var("CLAUDE_HOME") {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 50, update_meta)
}

fn update_meta(meta: &mut AgentSessionMeta, value: &Value) {
    if meta.id.is_none() {
        meta.id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if meta.cwd.is_none() {
        meta.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
    }
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    update_session_meta(session, value);

    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);

    match value.get("type").and_then(Value::as_str) {
        Some("user") => apply_message(session, value, AgentBlockKind::User, timestamp),
        Some("assistant") => apply_message(session, value, AgentBlockKind::Assistant, timestamp),
        _ => {}
    }
}

fn update_session_meta(session: &mut AgentSession, value: &Value) {
    if session.id.is_none() {
        session.id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if session.cwd.is_none() {
        session.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
    }
}

fn apply_message(
    session: &mut AgentSession,
    value: &Value,
    fallback_kind: AgentBlockKind,
    timestamp: Option<String>,
) {
    let message = value.get("message").unwrap_or(&Value::Null);
    let kind = match message.get("role").and_then(Value::as_str) {
        Some("user") => AgentBlockKind::User,
        Some("assistant") => AgentBlockKind::Assistant,
        _ => fallback_kind,
    };

    push_content_blocks(
        session,
        kind,
        timestamp,
        message.get("content").unwrap_or(&Value::Null),
    );
}

fn push_content_blocks(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    content: &Value,
) {
    match content {
        Value::Array(items) => {
            let mut text_parts = Vec::new();
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => text_parts.push(extract_content_text(item)),
                    Some("tool_use") => push_tool_use(session, timestamp.clone(), item),
                    Some("tool_result") => push_block(
                        session,
                        AgentBlockKind::ToolOutput,
                        timestamp.clone(),
                        None,
                        extract_content_text(item.get("content").unwrap_or(&Value::Null)),
                    ),
                    _ => {}
                }
            }
            push_block(session, kind, timestamp, None, text_parts.join("\n\n"));
        }
        _ => push_block(
            session,
            kind,
            timestamp,
            None,
            extract_content_text(content),
        ),
    }
}

fn push_tool_use(session: &mut AgentSession, timestamp: Option<String>, item: &Value) {
    push_block(
        session,
        AgentBlockKind::ToolCall,
        timestamp,
        item.get("name").and_then(Value::as_str).map(str::to_string),
        item.get("input")
            .map(pretty_json_value)
            .unwrap_or_else(|| pretty_json_value(item)),
    );
}

#[cfg(test)]
mod tests {
    use super::ClaudeProvider;
    use crate::ai::{
        format_blocks, select_blocks, AgentBlockKind, AgentSelection, AgentSessionProvider,
    };

    #[test]
    fn parses_claude_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:00Z","message":{"role":"user","content":"hello"}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"I will check."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"echo hi"}}]}}
{"type":"user","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"hi"}]}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
    }

    #[test]
    fn selects_last_claude_turn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"first"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"old"}]}}
{"type":"user","message":{"role":"user","content":"second"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"new"}]}}
"#,
        )
        .unwrap();
        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        let blocks = select_blocks(&session, AgentSelection::LastTurn);

        assert_eq!(blocks.len(), 2);
        assert_eq!(
            format_blocks(&blocks),
            "## User\nsecond\n\n## Assistant\nnew"
        );
    }
}
