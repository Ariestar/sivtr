//! Qwen Code (Alibaba) agent sessions.
//!
//! Layout (`QWEN_HOME`, default `~/.qwen`):
//! ```text
//! tmp/<project-hash>/chats/<session-id>.jsonl
//! ```
//! One JSONL file per session. The first line is a header
//! `{id, workspaceRootPath, name?, ...}`; following lines are chat records:
//! `{uuid, sessionId, type: user|assistant|tool_result|system, subtype?,
//! cwd, timestamp, message?: {role, parts}, toolCallResult?, model?}`.
//! Every record carries the session id and working directory, so the header
//! is only a convenience — the parser reads them from whichever record
//! appears first.

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::agents::{
    list_chat_recording_sessions, parse_jsonl_session, pretty_json_value, push_parts_blocks,
    push_tool_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionMeta,
    AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "Qwen";

/// Lines scanned for listing metadata (header + first messages).
const META_SCAN_LINES: usize = 40;

#[derive(Debug, Clone, Copy, Default)]
pub struct QwenProvider;

impl AgentSessionProvider for QwenProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Qwen
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        list_chat_recording_sessions(PROVIDER_NAME, &qwen_tmp_dir(), cwd, parse_session_meta)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

pub fn qwen_home() -> PathBuf {
    if let Ok(path) = std::env::var("QWEN_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".qwen")
}

fn qwen_tmp_dir() -> PathBuf {
    qwen_home().join("tmp")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read Qwen session: {}", path.display()))?;
    let reader = std::io::BufReader::new(file);

    let mut meta = AgentSessionMeta::default();
    let mut first_user_text: Option<String> = None;

    for line in reader.lines().take(META_SCAN_LINES) {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            update_meta(&mut meta, &value, &mut first_user_text);
        }
    }
    meta.fallback_title(first_user_text.as_deref());
    Ok(meta)
}

fn update_meta(meta: &mut AgentSessionMeta, value: &Value, first_user_text: &mut Option<String>) {
    let record_type = value.get("type").and_then(Value::as_str);
    let (id, cwd, title) = extract_identity(value);

    if meta.id.is_none() {
        meta.id = id;
    }
    if meta.cwd.is_none() {
        if let Some(cwd) = cwd {
            meta.add_cwd(cwd);
        }
    }
    if meta.title.is_none() {
        meta.title = title;
    }
    if first_user_text.is_none() && record_type == Some("user") {
        let text = extract_record_text(value);
        if !text.trim().is_empty() {
            *first_user_text = Some(text);
        }
    }
}

/// Expand a leading `~` in portable workspace paths to the home directory.
fn expand_home(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    let home = dirs::home_dir().unwrap_or_default();
    format!("{}{}", home.display(), rest)
}

/// Session id, cwd and user-set name from a header or chat record.
fn extract_identity(value: &Value) -> (Option<String>, Option<String>, Option<String>) {
    let id = value
        .get("sessionId")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let cwd = value
        .get("cwd")
        .or_else(|| value.get("workspaceRootPath"))
        .and_then(Value::as_str)
        .map(expand_home)
        .filter(|cwd| !cwd.trim().is_empty());
    let title = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string);
    (id, cwd, title)
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    let (id, cwd, title) = extract_identity(value);
    if session.id.is_none() {
        session.id = id;
    }
    if session.cwd.is_none() {
        session.cwd = cwd;
    }
    if session.title.is_none() {
        session.title = title;
    }

    let Some(record_type) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let message = value.get("message").unwrap_or(&Value::Null);
    let parts = message.get("parts").unwrap_or(&Value::Null);

    match record_type {
        "user" => push_parts_blocks(session, AgentBlockKind::User, timestamp, parts),
        "assistant" => {
            push_parts_blocks(session, AgentBlockKind::Assistant, timestamp.clone(), parts);
            if let Some(result) = value.get("toolCallResult") {
                push_tool_block(
                    session,
                    AgentBlockKind::ToolOutput,
                    timestamp,
                    result
                        .get("callId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    result
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    pretty_json_value(result),
                    None,
                );
            }
        }
        "tool_result" => {
            let label = value
                .get("toolCallResult")
                .and_then(|result| result.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string);
            push_tool_block(
                session,
                AgentBlockKind::ToolOutput,
                timestamp,
                None,
                label,
                extract_record_text(value),
                None,
            );
        }
        "system" => {}
        _ => {}
    }
}

/// Visible text from a Qwen record's `message` content (text parts only).
fn extract_record_text(value: &Value) -> String {
    value
        .get("message")
        .and_then(|message| message.get("parts"))
        .map(extract_parts_text)
        .unwrap_or_default()
}

fn extract_parts_text(parts: &Value) -> String {
    match parts {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                if item.get("thought").and_then(Value::as_bool) == Some(true) {
                    return None;
                }
                item.get("text").and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentSessionProvider;

    #[test]
    fn parses_qwen_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("9f8e7d6c-5b4a-4321-9abc-def012345678.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"9f8e7d6c-5b4a-4321-9abc-def012345678","workspaceRootPath":"D:\\repo","createdAt":1779265000000}
{"uuid":"r1","sessionId":"9f8e7d6c-5b4a-4321-9abc-def012345678","type":"user","cwd":"D:\\repo","timestamp":"2026-05-20T00:00:01Z","message":{"role":"user","parts":[{"text":"hello"}]}}
{"uuid":"r2","sessionId":"9f8e7d6c-5b4a-4321-9abc-def012345678","type":"assistant","cwd":"D:\\repo","timestamp":"2026-05-20T00:00:02Z","message":{"role":"model","parts":[{"thought":true,"text":"hidden"},{"text":"thinking out loud"}]}}
{"uuid":"r3","sessionId":"9f8e7d6c-5b4a-4321-9abc-def012345678","type":"assistant","cwd":"D:\\repo","timestamp":"2026-05-20T00:00:03Z","message":{"role":"model","parts":[{"functionCall":{"name":"bash","args":{"command":"echo hi"}}}]}}
{"uuid":"r4","sessionId":"9f8e7d6c-5b4a-4321-9abc-def012345678","type":"tool_result","cwd":"D:\\repo","timestamp":"2026-05-20T00:00:04Z","message":{"role":"tool","parts":[{"text":"hi"}]},"toolCallResult":{"name":"bash","status":"success"}}
{"uuid":"r5","sessionId":"9f8e7d6c-5b4a-4321-9abc-def012345678","type":"system","subtype":"chat_compression","cwd":"D:\\repo","timestamp":"2026-05-20T00:00:05Z"}
{"uuid":"r6","sessionId":"9f8e7d6c-5b4a-4321-9abc-def012345678","type":"assistant","cwd":"D:\\repo","timestamp":"2026-05-20T00:00:06Z","message":{"role":"model","parts":[{"text":"done"}]}}
"#,
        )
        .unwrap();

        let session = QwenProvider.parse_session_file(&path).unwrap();

        assert_eq!(
            session.id.as_deref(),
            Some("9f8e7d6c-5b4a-4321-9abc-def012345678")
        );
        assert_eq!(session.cwd.as_deref(), Some("D:\\repo"));
        assert_eq!(session.blocks.len(), 6);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "hidden");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].text, "thinking out loud");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[3].label.as_deref(), Some("bash"));
        assert!(session.blocks[3].text.contains("echo hi"));
        assert_eq!(session.blocks[4].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].text, "hi");
        assert_eq!(session.blocks[4].label.as_deref(), Some("bash"));
        assert_eq!(session.blocks[5].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[5].text, "done");
    }
}
