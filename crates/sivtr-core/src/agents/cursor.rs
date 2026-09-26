use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_value, push_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionMeta,
    AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "Cursor";

/// Cursor agent transcript provider.
///
/// Primary evidence source observed in community/runtime layouts:
/// `~/.cursor/projects/<project-id>/agent-transcripts/**/*.jsonl`
///
/// When no transcripts exist, returns empty rather than inventing content.
#[derive(Debug, Clone, Copy, Default)]
pub struct CursorProvider;

impl AgentSessionProvider for CursorProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Cursor
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        list_recent_jsonl_sessions(
            PROVIDER_NAME,
            &cursor_transcripts_root(),
            cwd,
            parse_cursor_meta,
        )
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

pub fn cursor_home() -> PathBuf {
    if let Ok(path) = std::env::var("CURSOR_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cursor")
}

fn cursor_transcripts_root() -> PathBuf {
    cursor_home().join("projects")
}

fn parse_cursor_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 80, update_meta)
}

fn update_meta(meta: &mut AgentSessionMeta, value: &Value) {
    if meta.id.is_none() {
        meta.id = value
            .get("sessionId")
            .or_else(|| value.get("id"))
            .or_else(|| value.get("conversationId"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if let Some(cwd) = value
        .get("cwd")
        .or_else(|| value.get("workspaceRoot"))
        .or_else(|| value.pointer("/workspace/path"))
        .and_then(Value::as_str)
    {
        meta.add_cwd(cwd);
    }
    if meta.title.is_none() {
        meta.title = value
            .get("title")
            .or_else(|| value.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string);
    }
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    if session.id.is_none() {
        session.id = value
            .get("sessionId")
            .or_else(|| value.get("id"))
            .or_else(|| value.get("conversationId"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if session.cwd.is_none() {
        session.cwd = value
            .get("cwd")
            .or_else(|| value.get("workspaceRoot"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }

    let timestamp = value
        .get("timestamp")
        .or_else(|| value.get("createdAt"))
        .and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        });

    // Common Cursor/Claude-like shapes.
    // Cursor agent transcripts wrap payloads as `{role, message:{content:[...]}}`.
    let payload = value
        .get("message")
        .filter(|message| message.is_object())
        .unwrap_or(value);
    match value
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| value.get("role").and_then(Value::as_str))
    {
        Some("user") | Some("human") => {
            push_text(session, AgentBlockKind::User, timestamp, payload)
        }
        Some("assistant") | Some("ai") => push_assistant(session, timestamp, payload),
        Some("tool") | Some("tool_result") | Some("toolResult") => {
            push_text(session, AgentBlockKind::ToolOutput, timestamp, payload)
        }
        Some("tool_call") | Some("toolCall") => {
            let label = value
                .get("name")
                .or_else(|| value.get("toolName"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let input = value
                .get("input")
                .or_else(|| value.get("args"))
                .or_else(|| value.get("arguments"))
                .unwrap_or(value);
            push_block(
                session,
                AgentBlockKind::ToolCall,
                timestamp,
                label,
                pretty_json_value(input),
            );
        }
        Some("message") => {
            let message = value.get("message").unwrap_or(value);
            match message.get("role").and_then(Value::as_str) {
                Some("user") => push_text(session, AgentBlockKind::User, timestamp, message),
                Some("assistant") => push_assistant(session, timestamp, message),
                Some("tool") => push_text(session, AgentBlockKind::ToolOutput, timestamp, message),
                _ => {}
            }
        }
        _ => {
            // Bubble-like rows sometimes only have text + bubbleType.
            if let Some(kind) = value
                .get("bubbleType")
                .or_else(|| value.get("type"))
                .and_then(Value::as_str)
            {
                match kind {
                    "user" | "human" => push_text(session, AgentBlockKind::User, timestamp, value),
                    "ai" | "assistant" => push_assistant(session, timestamp, value),
                    _ => {}
                }
            }
        }
    }
}

fn push_assistant(session: &mut AgentSession, timestamp: Option<String>, value: &Value) {
    let content = value
        .get("content")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("message"))
        .unwrap_or(value);
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
                    Some("tool_use" | "toolCall" | "tool_call") => {
                        let label = item.get("name").and_then(Value::as_str).map(str::to_string);
                        let input = item
                            .get("input")
                            .or_else(|| item.get("arguments"))
                            .unwrap_or(item);
                        push_block(
                            session,
                            AgentBlockKind::ToolCall,
                            timestamp.clone(),
                            label,
                            pretty_json_value(input),
                        );
                    }
                    Some("tool_result") => push_block(
                        session,
                        AgentBlockKind::ToolOutput,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    ),
                    _ => {
                        let text = extract_content_text(item);
                        if !text.trim().is_empty() {
                            push_block(
                                session,
                                AgentBlockKind::Assistant,
                                timestamp.clone(),
                                None,
                                text,
                            );
                        }
                    }
                }
            }
        }
        other => push_text(session, AgentBlockKind::Assistant, timestamp, other),
    }
}

fn push_text(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    value: &Value,
) {
    let content = value
        .get("content")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("message"))
        .unwrap_or(value);
    let text = extract_content_text(content);
    if !text.trim().is_empty() {
        push_block(session, kind, timestamp, None, text);
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_event, CursorProvider};
    use crate::agents::{AgentBlockKind, AgentProvider, AgentSession, AgentSessionProvider};
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn provider_name_is_cursor() {
        assert_eq!(AgentProvider::Cursor.name(), "Cursor");
        assert_eq!(CursorProvider.provider(), AgentProvider::Cursor);
    }

    #[test]
    fn current_session_and_scoped_listing_require_workspace_membership() {
        let _guard = crate::test_fixtures::EnvGuard::capture(&["CURSOR_HOME", "SIVTR_HOME"]);
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CURSOR_HOME", dir.path());
        std::env::set_var("SIVTR_HOME", dir.path().join("data"));
        let projects = dir.path().join("projects");
        std::fs::create_dir(&projects).unwrap();
        std::fs::write(
            projects.join("other.jsonl"),
            format!(
                "{}\n",
                json!({"sessionId": "other", "cwd": dir.path().join("other")})
            ),
        )
        .unwrap();
        std::fs::write(
            projects.join("unbound.jsonl"),
            "{\"type\":\"user\",\"text\":\"private\"}\n",
        )
        .unwrap();

        assert!(CursorProvider
            .list_recent_sessions(Some(&dir.path().join("shared")))
            .unwrap()
            .is_empty());
        let missing = CursorProvider
            .find_current_session(&dir.path().join("shared"))
            .unwrap();
        let matching = CursorProvider
            .find_current_session(&dir.path().join("other"))
            .unwrap();
        assert_eq!(CursorProvider.list_recent_sessions(None).unwrap().len(), 2);

        assert_eq!(missing, None);
        assert_eq!(matching, Some(projects.join("other.jsonl")));
    }

    #[test]
    fn maps_cursor_like_message_events() {
        let mut session = AgentSession {
            path: PathBuf::from("t.jsonl"),
            id: None,
            cwd: None,
            title: None,
            blocks: Vec::new(),
        };
        apply_event(
            &mut session,
            &json!({"type":"user","text":"hello cursor","cwd":"D:/repo"}),
        );
        apply_event(
            &mut session,
            &json!({
                "type":"assistant",
                "content":[
                    {"type":"text","text":"hi"},
                    {"type":"toolCall","name":"read","input":{"path":"a.rs"}}
                ]
            }),
        );
        assert_eq!(session.cwd.as_deref(), Some("D:/repo"));
        assert_eq!(session.blocks.len(), 3);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
    }

    #[test]
    fn maps_composer_role_message_envelope() {
        let mut session = AgentSession {
            path: PathBuf::from("t.jsonl"),
            id: None,
            cwd: None,
            title: None,
            blocks: Vec::new(),
        };
        apply_event(
            &mut session,
            &json!({
                "role": "user",
                "message": {
                    "content": [{"type": "text", "text": "是否可以考虑把jina模型换成kimi 3"}]
                }
            }),
        );
        apply_event(
            &mut session,
            &json!({
                "role": "assistant",
                "message": {
                    "content": [
                        {"type": "text", "text": "先看 Jina 怎么接。"},
                        {"type": "tool_use", "name": "Read", "input": {"path": "config.py"}}
                    ]
                }
            }),
        );
        assert_eq!(session.blocks.len(), 3);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert!(session.blocks[0].text.contains("kimi 3"));
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert!(session.blocks[1].text.contains("Jina"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("Read"));

        for role in ["tool", "tool_result", "toolResult"] {
            for enveloped in [false, true] {
                let content = json!({"content": [{"type": "text", "text": "tool output"}]});
                let mut event = if enveloped {
                    json!({"message": content})
                } else {
                    content
                };
                event["role"] = json!(role);
                session.blocks.clear();
                apply_event(&mut session, &event);
                assert_eq!(session.blocks.len(), 1);
                assert_eq!(session.blocks[0].kind, AgentBlockKind::ToolOutput);
                assert_eq!(session.blocks[0].text, "tool output");
            }
        }
    }
}
