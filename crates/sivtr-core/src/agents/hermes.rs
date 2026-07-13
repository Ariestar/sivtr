use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agents::{
    list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session, pretty_json_string,
    push_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionInfo, AgentSessionMeta,
    AgentSessionProvider,
};

const PROVIDER_NAME: &str = "Hermes";

#[derive(Debug, Clone, Copy, Default)]
pub struct HermesProvider;

impl AgentSessionProvider for HermesProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Hermes
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        list_recent_jsonl_sessions(&hermes_sessions_dir(), cwd, parse_session_meta)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let mut session = parse_jsonl_session(path, PROVIDER_NAME, apply_event)?;
        if session.id.is_none() {
            session.id = session_id_from_path(path);
        }
        Ok(session)
    }
}

pub fn hermes_home() -> PathBuf {
    if let Ok(path) = std::env::var("HERMES_HOME") {
        return PathBuf::from(path);
    }

    if cfg!(windows) {
        if let Some(local_data) = dirs::data_local_dir() {
            return local_data.join("hermes");
        }
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hermes")
}

pub fn hermes_sessions_dir() -> PathBuf {
    hermes_home().join("sessions")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    let id = session_id_from_path(path);
    let mut meta = parse_jsonl_meta(path, PROVIDER_NAME, 1, |_, _| {})?;
    meta.id = id;
    Ok(meta)
}

fn session_id_from_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    match value.get("role").and_then(Value::as_str) {
        Some("session_meta") => {}
        Some("user") => push_user_message(session, value),
        Some("assistant") => push_assistant_message(session, value),
        Some("tool") => push_tool_output(session, value),
        _ => {}
    }
}

fn push_user_message(session: &mut AgentSession, value: &Value) {
    let timestamp = extract_timestamp(value);
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    push_block(session, AgentBlockKind::User, timestamp, None, content);
}

fn push_assistant_message(session: &mut AgentSession, value: &Value) {
    let timestamp = extract_timestamp(value);

    let content = value
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !content.is_empty() {
        push_block(
            session,
            AgentBlockKind::Assistant,
            timestamp.clone(),
            None,
            content,
        );
    }

    if let Some(Value::Array(tool_calls)) = value.get("tool_calls") {
        for tool_call in tool_calls {
            let function = tool_call.get("function").unwrap_or(&Value::Null);
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string);
            let arguments = function
                .get("arguments")
                .and_then(Value::as_str)
                .map(pretty_json_string)
                .unwrap_or_default();
            push_block(
                session,
                AgentBlockKind::ToolCall,
                timestamp.clone(),
                name,
                arguments,
            );
        }
    }
}

fn push_tool_output(session: &mut AgentSession, value: &Value) {
    let timestamp = extract_timestamp(value);
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    push_block(
        session,
        AgentBlockKind::ToolOutput,
        timestamp,
        None,
        content,
    );
}

fn extract_timestamp(value: &Value) -> Option<String> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::HermesProvider;
    use crate::agents::{AgentBlockKind, AgentSessionProvider};

    #[test]
    fn parses_hermes_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("20260426_134409_4f3e2502.jsonl");
        std::fs::write(
            &path,
            r#"{"role":"session_meta","model":"gpt-5.4-mini","platform":"cli","timestamp":"2026-04-26T13:47:42Z"}
{"role":"user","content":"hello","timestamp":"2026-04-26T13:47:43Z"}
{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"terminal","arguments":"{\"command\":\"echo hi\"}"}}],"timestamp":"2026-04-26T13:47:44Z"}
{"role":"tool","content":"{\"output\":\"hi\",\"exit_code\":0}","tool_call_id":"call_1","timestamp":"2026-04-26T13:47:45Z"}
{"role":"assistant","content":"done","timestamp":"2026-04-26T13:47:46Z"}
"#,
        )
        .unwrap();

        let session = HermesProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("20260426_134409_4f3e2502"));
        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("terminal"));
        assert!(session.blocks[1].text.contains("echo hi"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert!(session.blocks[2].text.contains("hi"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "done");
    }

    #[test]
    fn skips_reasoning_and_empty_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"role":"session_meta","model":"gpt-5.4-mini","timestamp":"2026-04-26T00:00:00Z"}
{"role":"user","content":"test","timestamp":"2026-04-26T00:00:01Z"}
{"role":"assistant","content":"answer","reasoning":"hidden thinking","codex_reasoning_items":[{"type":"reasoning","encrypted_content":"xxx"}],"timestamp":"2026-04-26T00:00:02Z"}
"#,
        )
        .unwrap();

        let session = HermesProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "test");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[1].text, "answer");
    }

    #[test]
    fn handles_multiple_tool_calls_in_one_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"role":"session_meta","timestamp":"2026-04-26T00:00:00Z"}
{"role":"user","content":"do both","timestamp":"2026-04-26T00:00:01Z"}
{"role":"assistant","content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.rs\"}"}},{"id":"c2","type":"function","function":{"name":"terminal","arguments":"{\"command\":\"ls\"}"}}],"timestamp":"2026-04-26T00:00:02Z"}
{"role":"tool","content":"file content","tool_call_id":"c1","timestamp":"2026-04-26T00:00:03Z"}
{"role":"tool","content":"dir listing","tool_call_id":"c2","timestamp":"2026-04-26T00:00:04Z"}
{"role":"assistant","content":"all done","timestamp":"2026-04-26T00:00:05Z"}
"#,
        )
        .unwrap();

        let session = HermesProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 6);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("read_file"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("terminal"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[5].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[5].text, "all done");
    }
}
