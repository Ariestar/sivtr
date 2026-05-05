use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::ai::{
    extract_content_text, list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_string, pretty_json_value, push_block, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionInfo, AgentSessionMeta, AgentSessionProvider,
};

const PROVIDER_NAME: &str = "Codex";

#[derive(Debug, Clone, Copy, Default)]
pub struct CodexProvider;

impl AgentSessionProvider for CodexProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Codex
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        list_recent_jsonl_sessions(&codex_home().join("sessions"), cwd, parse_session_meta)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

pub fn codex_home() -> PathBuf {
    if let Ok(path) = std::env::var("CODEX_HOME") {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 1, |meta, value| {
        let payload = value.get("payload").unwrap_or(&Value::Null);
        meta.id = payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        meta.cwd = payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string);
    })
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let payload = value.get("payload").unwrap_or(&Value::Null);

    match value.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            session.id = payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string);
            session.cwd = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        Some("response_item") => apply_response_item(session, payload, timestamp),
        _ => {}
    }
}

fn apply_response_item(session: &mut AgentSession, payload: &Value, timestamp: Option<String>) {
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => {
            let kind = match payload.get("role").and_then(Value::as_str) {
                Some("user") => AgentBlockKind::User,
                Some("assistant") => {
                    if payload.get("phase").and_then(Value::as_str) == Some("commentary") {
                        return;
                    }
                    AgentBlockKind::Assistant
                }
                _ => return,
            };
            push_block(
                session,
                kind,
                timestamp,
                None,
                extract_content_text(payload.get("content").unwrap_or(&Value::Null)),
            );
        }
        Some("function_call") => push_block(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            payload
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            extract_tool_call_text(payload),
        ),
        Some("function_call_output") => push_block(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            None,
            payload
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        _ => {}
    }
}

fn extract_tool_call_text(payload: &Value) -> String {
    match payload.get("arguments") {
        Some(Value::String(arguments)) => pretty_json_string(arguments),
        Some(arguments) => pretty_json_value(arguments),
        None => pretty_json_value(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::CodexProvider;
    use crate::ai::{
        format_blocks, select_blocks, AgentBlockKind, AgentSelection, AgentSessionProvider,
    };

    #[test]
    fn parses_codex_rollout_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"2026-04-27T00:00:00Z","type":"session_meta","payload":{"id":"abc","cwd":"C:\\repo"}}
{"timestamp":"2026-04-27T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}
{"timestamp":"2026-04-27T00:00:02Z","type":"response_item","payload":{"type":"function_call","name":"shell_command","arguments":"{\"command\":\"echo hi\"}"}}
{"timestamp":"2026-04-27T00:00:03Z","type":"response_item","payload":{"type":"function_call_output","output":"hi"}}
{"timestamp":"2026-04-27T00:00:04Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
    }

    #[test]
    fn selects_last_turn_without_tool_noise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"first"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"old"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"second"}]}}
{"type":"response_item","payload":{"type":"function_call_output","output":"debug"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"new"}]}}
"#,
        )
        .unwrap();
        let session = CodexProvider.parse_session_file(&path).unwrap();

        let blocks = select_blocks(&session, AgentSelection::LastTurn);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "second");
        assert_eq!(blocks[1].text, "new");
        assert_eq!(
            format_blocks(&blocks),
            "## User\nsecond\n\n## Assistant\nnew"
        );
    }

    #[test]
    fn skips_commentary_assistant_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"copy the answer"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary","content":[{"text":"working update"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"real answer"}]}}
"#,
        )
        .unwrap();
        let session = CodexProvider.parse_session_file(&path).unwrap();

        let blocks = select_blocks(&session, AgentSelection::LastAssistant);

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(blocks[0].text, "real answer");
    }
}
