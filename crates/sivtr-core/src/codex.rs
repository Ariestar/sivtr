use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::ai::{
    AgentBlock, AgentBlockKind, AgentProvider, AgentSession, AgentSessionInfo, AgentSessionProvider,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct CodexProvider;

impl AgentSessionProvider for CodexProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Codex
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        list_recent_sessions(cwd)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_session_file(path)
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

fn list_recent_sessions(cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
    let wanted = cwd.map(normalize_path_for_match);
    let mut sessions = Vec::new();

    for path in session_files()? {
        let meta = parse_session_meta(&path)?;
        if let Some(wanted) = wanted.as_deref() {
            let matches_cwd = meta
                .cwd
                .as_deref()
                .map(|cwd| normalize_path_for_match(Path::new(cwd)) == wanted)
                .unwrap_or(false);
            if !matches_cwd {
                continue;
            }
        }

        sessions.push(AgentSessionInfo {
            modified: modified_time(&path).unwrap_or(SystemTime::UNIX_EPOCH),
            path,
            id: meta.id,
            cwd: meta.cwd,
        });
    }

    sessions.sort_by_key(|session| session.modified);
    sessions.reverse();
    Ok(sessions)
}

fn parse_session_file(path: &Path) -> Result<AgentSession> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read Codex session: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut session = AgentSession {
        path: path.to_path_buf(),
        id: None,
        cwd: None,
        blocks: Vec::new(),
    };

    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "Failed to read Codex session line {}: {}",
                idx + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(&line).with_context(|| {
            format!(
                "Failed to parse Codex session line {} as JSON: {}",
                idx + 1,
                path.display()
            )
        })?;
        apply_event(&mut session, &value);
    }

    Ok(session)
}

fn session_files() -> Result<Vec<PathBuf>> {
    let sessions_dir = codex_home().join("sessions");
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_jsonl_files(&sessions_dir, &mut files)?;
    Ok(files)
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn modified_time(path: &Path) -> Result<SystemTime> {
    Ok(fs::metadata(path)?.modified()?)
}

#[derive(Default)]
struct CodexMeta {
    id: Option<String>,
    cwd: Option<String>,
}

fn parse_session_meta(path: &Path) -> Result<CodexMeta> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read Codex session: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        return Ok(CodexMeta::default());
    }

    let value: Value = serde_json::from_str(&line).with_context(|| {
        format!(
            "Failed to parse Codex session metadata as JSON: {}",
            path.display()
        )
    })?;
    let payload = value.get("payload").unwrap_or(&Value::Null);
    Ok(CodexMeta {
        id: payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string),
        cwd: payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn normalize_path_for_match(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
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
            let text = extract_content_text(payload.get("content").unwrap_or(&Value::Null));
            push_block(session, kind, timestamp, None, text);
        }
        Some("function_call") => {
            let label = payload
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string);
            let text = extract_tool_call_text(payload);
            push_block(session, AgentBlockKind::ToolCall, timestamp, label, text);
        }
        Some("function_call_output") => {
            let text = payload
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            push_block(session, AgentBlockKind::ToolOutput, timestamp, None, text);
        }
        _ => {}
    }
}

fn push_block(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    label: Option<String>,
    text: String,
) {
    let text = text.trim().to_string();
    if !text.is_empty() {
        session.blocks.push(AgentBlock {
            kind,
            timestamp,
            label,
            text,
        });
    }
}

fn extract_content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("input_text").and_then(Value::as_str))
                    .or_else(|| item.get("output_text").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn extract_tool_call_text(payload: &Value) -> String {
    if let Some(arguments) = payload.get("arguments") {
        if let Some(arguments) = arguments.as_str() {
            return pretty_json_string(arguments);
        }
        return serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string());
    }

    serde_json::to_string_pretty(payload).unwrap_or_default()
}

fn pretty_json_string(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_session_file;
    use crate::ai::{format_blocks, select_blocks, AgentBlockKind, AgentSelection};

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

        let session = parse_session_file(&path).unwrap();

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
        let session = parse_session_file(&path).unwrap();

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
        let session = parse_session_file(&path).unwrap();

        let blocks = select_blocks(&session, AgentSelection::LastAssistant);

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(blocks[0].text, "real answer");
    }
}
