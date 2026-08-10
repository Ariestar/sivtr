//! Gemini CLI (Google) agent sessions.
//!
//! Layout (`GEMINI_HOME`, default `~/.gemini`):
//! ```text
//! tmp/<project-id>/chats/
//!   session-<timestamp>-<id8>.jsonl   ← current JSONL format
//!   session-<timestamp>-<id8>.json    ← legacy single-record format
//! ```
//! The project-id is a hash of the working directory; the CLI keeps the
//! id→path mapping in `projects.json`, which is not always present, so
//! sessions without a mapping are listed as unbound (kept for every
//! workspace, same as sessions that carry no cwd metadata).
//!
//! JSONL records: the first line is a header `{sessionId, projectHash,
//! startTime, lastUpdated, summary?}`; message lines are `{id, type:
//! user|gemini|info|error|warning, timestamp, content, thoughts?,
//! toolCalls?, model?}` plus `$set` metadata updates and `$rewindTo`
//! markers. The legacy `.json` form is a single object with the header
//! fields and an inline `messages` array.

use crate::agents::{
    extract_content_text, list_chat_recording_sessions, pretty_json_value, push_block,
    push_parts_blocks, push_tool_block, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionInfo, AgentSessionMeta, AgentSessionProvider,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const PROVIDER_NAME: &str = "Gemini";

/// Upper bound on the header + first messages read for listing metadata.
const META_READ_LIMIT: usize = 64 * 1024;
/// Lines scanned for the first user message when no summary exists.
const META_SCAN_LINES: usize = 30;

#[derive(Debug, Clone, Copy, Default)]
pub struct GeminiProvider;

impl AgentSessionProvider for GeminiProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Gemini
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        list_chat_recording_sessions(PROVIDER_NAME, &gemini_tmp_dir(), cwd, parse_session_meta)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read Gemini session: {}", path.display()))?;

        // Legacy files are a single JSON object (possibly pretty-printed);
        // current files are JSONL with one record per line.
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            if is_legacy_record(&value) {
                return parse_legacy_session(path, &value);
            }
        }

        let mut session = AgentSession {
            path: path.to_path_buf(),
            id: None,
            cwd: None,
            title: None,
            blocks: Vec::new(),
        };
        for line in text.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                apply_event(&mut session, &value);
            }
        }
        Ok(session)
    }
}

pub fn gemini_home() -> PathBuf {
    if let Ok(path) = std::env::var("GEMINI_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".gemini")
}

fn gemini_tmp_dir() -> PathBuf {
    gemini_home().join("tmp")
}

fn is_legacy_record(value: &Value) -> bool {
    value.get("sessionId").is_some() && value.get("messages").is_some()
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    let head = read_head(path)?;
    let mut meta = AgentSessionMeta::default();
    let mut first_user_text: Option<String> = None;

    if let Ok(value) = serde_json::from_str::<Value>(&head) {
        if is_legacy_record(&value) {
            update_meta(&mut meta, &value, &mut first_user_text);
            meta.fallback_title(first_user_text.as_deref());
            return Ok(meta);
        }
    }

    for line in head.lines().take(META_SCAN_LINES) {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            update_meta(&mut meta, &value, &mut first_user_text);
        }
    }
    meta.fallback_title(first_user_text.as_deref());
    Ok(meta)
}

fn read_head(path: &Path) -> Result<String> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read Gemini session: {}", path.display()))?;
    let mut buf = Vec::new();
    file.take(META_READ_LIMIT as u64)
        .read_to_end(&mut buf)
        .with_context(|| format!("Failed to read Gemini session: {}", path.display()))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn update_meta(meta: &mut AgentSessionMeta, value: &Value, first_user_text: &mut Option<String>) {
    if value.get("id").is_some() {
        // Message records: track the first user text as a title fallback.
        if first_user_text.is_none() && value.get("type").and_then(Value::as_str) == Some("user") {
            let text = dialogue_text(value.get("content").unwrap_or(&Value::Null));
            if !text.trim().is_empty() {
                *first_user_text = Some(text);
            }
        }
        return;
    }

    if meta.id.is_none() {
        meta.id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if meta.title.is_none() {
        meta.title = value
            .get("summary")
            .and_then(Value::as_str)
            .filter(|summary| !summary.trim().is_empty())
            .map(str::to_string);
    }
}

/// Visible text from a Gemini content value, excluding thought parts.
fn dialogue_text(content: &Value) -> String {
    match content {
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

fn parse_legacy_session(path: &Path, value: &Value) -> Result<AgentSession> {
    let mut session = AgentSession {
        path: path.to_path_buf(),
        id: value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string),
        cwd: None,
        title: value
            .get("summary")
            .and_then(Value::as_str)
            .filter(|summary| !summary.trim().is_empty())
            .map(str::to_string),
        blocks: Vec::new(),
    };

    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        for message in messages {
            apply_event(&mut session, message);
        }
    }
    Ok(session)
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    if value.get("id").is_none() {
        // Header line or metadata update; messages always carry an id.
        if session.id.is_none() {
            session.id = value
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if session.title.is_none() {
            let summary = value
                .get("$set")
                .and_then(|set| set.get("summary"))
                .or_else(|| value.get("summary"));
            session.title = summary
                .and_then(Value::as_str)
                .filter(|summary| !summary.trim().is_empty())
                .map(str::to_string);
        }
        return;
    }

    let Some(message_type) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let content = value.get("content").unwrap_or(&Value::Null);

    match message_type {
        "user" | "info" | "error" | "warning" => {
            push_parts_blocks(session, AgentBlockKind::User, timestamp, content);
        }
        "gemini" => {
            if let Some(thoughts) = value.get("thoughts").and_then(Value::as_array) {
                for thought in thoughts {
                    let text = thought
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    push_block(
                        session,
                        AgentBlockKind::Thinking,
                        timestamp.clone(),
                        None,
                        text,
                    );
                }
            }
            push_parts_blocks(
                session,
                AgentBlockKind::Assistant,
                timestamp.clone(),
                content,
            );
            if let Some(calls) = value.get("toolCalls").and_then(Value::as_array) {
                for call in calls {
                    apply_tool_call(session, timestamp.clone(), call);
                }
            }
        }
        _ => {}
    }
}

fn apply_tool_call(session: &mut AgentSession, timestamp: Option<String>, call: &Value) {
    let id = call.get("id").and_then(Value::as_str).map(str::to_string);
    let name = call.get("name").and_then(Value::as_str).map(str::to_string);
    let args = call.get("args").unwrap_or(&Value::Null);
    push_tool_block(
        session,
        AgentBlockKind::ToolCall,
        timestamp.clone(),
        id.clone(),
        name,
        pretty_json_value(args),
    );

    if let Some(result) = call.get("result") {
        push_tool_block(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            id,
            None,
            extract_content_text(result),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentSessionProvider;
    use std::time::{Duration, UNIX_EPOCH};

    fn touch(path: &Path, epoch_secs: u64) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(epoch_secs))
            .unwrap();
    }

    #[test]
    fn parses_current_jsonl_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2026-01-25T15-10-130c64ed.jsonl");
        std::fs::write(
            &path,
            r#"{"sessionId":"abc-123","projectHash":"h1","startTime":"2026-01-25T15:10:52Z","lastUpdated":"2026-01-25T15:11:09Z"}
{"id":"m1","timestamp":"2026-01-25T15:10:52Z","type":"user","content":"hello"}
{"id":"m2","timestamp":"2026-01-25T15:11:09Z","type":"gemini","content":[{"text":"hi"}],"thoughts":[{"subject":"s","description":"thinking here","timestamp":"2026-01-25T15:11:00Z"}],"toolCalls":[{"id":"t1","name":"bash","args":{"command":"ls"},"result":{"text":"file.txt"}}]}
{"$set":{"summary":"summarized"}}
"#,
        )
        .unwrap();

        let session = GeminiProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc-123"));
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "thinking here");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].text, "hi");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[3].label.as_deref(), Some("bash"));
        assert_eq!(session.blocks[3].call_id.as_deref(), Some("t1"));
        assert!(session.blocks[3].text.contains("ls"));
        assert_eq!(session.blocks[4].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].text, "file.txt");
    }

    #[test]
    fn parses_legacy_single_json_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2026-01-25T15-10-130c64ed.json");
        std::fs::write(
            &path,
            r#"{
  "sessionId": "legacy-1",
  "projectHash": "h2",
  "startTime": "2026-01-25T15:10:52Z",
  "lastUpdated": "2026-01-25T15:11:09Z",
  "messages": [
    {"id": "m1", "timestamp": "2026-01-25T15:10:52Z", "type": "user", "content": "hello legacy"},
    {"id": "m2", "timestamp": "2026-01-25T15:11:09Z", "type": "gemini", "content": "hi"}
  ]
}
"#,
        )
        .unwrap();

        let session = GeminiProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("legacy-1"));
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello legacy");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[1].text, "hi");
    }

    #[test]
    fn lists_chat_recording_sessions() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GEMINI_HOME", dir.path());

        let chats = dir.path().join("tmp").join("hash1").join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        let newer = chats.join("session-2026-01-25T15-10-130c64ed.jsonl");
        let older = chats.join("session-2026-01-20T10-00-abc12345.jsonl");
        let nested = chats.join("parent").join("sub.jsonl");
        std::fs::create_dir_all(chats.join("parent")).unwrap();
        std::fs::write(&newer, r#"{"sessionId":"n1","projectHash":"h","startTime":"2026-01-25T15:10:52Z","lastUpdated":"2026-01-25T15:11:09Z"}"#).unwrap();
        std::fs::write(&older, r#"{"sessionId":"o1","projectHash":"h","startTime":"2026-01-20T10:00:00Z","lastUpdated":"2026-01-20T10:01:00Z"}"#).unwrap();
        std::fs::write(&nested, r#"{"sessionId":"sub","projectHash":"h","startTime":"2026-01-25T15:12:00Z","lastUpdated":"2026-01-25T15:12:01Z"}"#).unwrap();
        touch(&newer, 1_800_000_000);
        touch(&older, 1_700_000_000);
        touch(&nested, 1_800_000_001);

        let sessions = GeminiProvider.list_recent_sessions(None).unwrap();

        // Subagent files nested under chats/<parent>/ are skipped.
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id.as_deref(), Some("n1"));
        assert_eq!(sessions[1].id.as_deref(), Some("o1"));
    }
}
