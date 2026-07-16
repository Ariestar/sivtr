use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agents::{
    extract_content_text, filter_sessions_by_workspace, parse_jsonl_session, pretty_json_string,
    pretty_json_value, push_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionInfo,
    AgentSessionProvider,
};

const PROVIDER_NAME: &str = "Grok";
const CHAT_HISTORY_FILE: &str = "chat_history.jsonl";
const SUMMARY_FILE: &str = "summary.json";

/// Grok (xAI) coding agent sessions.
///
/// Layout (`GROK_HOME`, default `~/.grok`):
/// ```text
/// sessions/<url-encoded-cwd>/<session-id>/
///   summary.json
///   chat_history.jsonl   ← conversation + tools (what we parse)
///   updates.jsonl        ← ACP stream (ignored)
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct GrokProvider;

impl AgentSessionProvider for GrokProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Grok
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        let root = grok_sessions_dir();
        if !root.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(&root)
            .with_context(|| format!("Failed to read Grok sessions dir {}", root.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // Each cwd bucket contains session dirs (and occasionally non-session files).
            for session_entry in fs::read_dir(&path)
                .with_context(|| format!("Failed to read Grok cwd sessions {}", path.display()))?
            {
                let session_entry = session_entry?;
                let session_path = session_entry.path();
                if !session_path.is_dir() {
                    continue;
                }
                if let Some(info) = session_info_from_dir(&session_path)? {
                    sessions.push(info);
                }
            }
        }

        sessions.sort_by_key(|session| session.modified);
        sessions.reverse();
        Ok(filter_sessions_by_workspace(sessions, cwd))
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let session_dir = resolve_session_dir(path)?;
        let history = session_dir.join(CHAT_HISTORY_FILE);
        if !history.exists() {
            anyhow::bail!(
                "Grok session `{}` is missing {CHAT_HISTORY_FILE}",
                session_dir.display()
            );
        }

        let mut session = parse_jsonl_session(&history, PROVIDER_NAME, apply_event)?;
        // Prefer summary.json for stable id/cwd/title; keep path as the session directory.
        session.path = session_dir.clone();
        if let Some(meta) = read_summary(&session_dir.join(SUMMARY_FILE))? {
            if session.id.is_none() {
                session.id = meta.id;
            }
            if session.cwd.is_none() {
                session.cwd = meta.cwd;
            }
            if session.title.is_none() {
                session.title = meta.title;
            }
        }
        if session.id.is_none() {
            session.id = session_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string);
        }
        Ok(session)
    }

    fn find_session_by_id(&self, id: &str) -> Result<Option<PathBuf>> {
        for session in self.list_recent_sessions(None)? {
            if session.id.as_deref() == Some(id)
                || session
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(id))
            {
                return Ok(Some(session.path));
            }
        }
        Ok(None)
    }
}

pub fn grok_home() -> PathBuf {
    if let Ok(path) = std::env::var("GROK_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".grok")
}

pub fn grok_sessions_dir() -> PathBuf {
    grok_home().join("sessions")
}

pub fn grok_config_path() -> PathBuf {
    grok_home().join("config.toml")
}

struct SummaryMeta {
    id: Option<String>,
    cwd: Option<String>,
    title: Option<String>,
    modified: SystemTime,
}

fn session_info_from_dir(session_dir: &Path) -> Result<Option<AgentSessionInfo>> {
    let summary_path = session_dir.join(SUMMARY_FILE);
    let history_path = session_dir.join(CHAT_HISTORY_FILE);
    if !summary_path.exists() && !history_path.exists() {
        return Ok(None);
    }

    let meta = read_summary(&summary_path)?.unwrap_or_else(|| SummaryMeta {
        id: session_dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
        cwd: None,
        title: None,
        modified: fs::metadata(session_dir)
            .and_then(|meta| meta.modified())
            .unwrap_or(UNIX_EPOCH),
    });

    Ok(Some(AgentSessionInfo {
        path: session_dir.to_path_buf(),
        id: meta.id.or_else(|| {
            session_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        }),
        cwd: meta.cwd,
        title: meta.title,
        modified: meta.modified,
    }))
}

fn read_summary(path: &Path) -> Result<Option<SummaryMeta>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("Failed to read Grok summary {}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse Grok summary {}", path.display()))?;

    let info = value.get("info").unwrap_or(&value);
    let id = info
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_string)
        });
    let cwd = info
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(str::to_string);
    let title = value
        .get("generated_title")
        .or_else(|| value.get("session_summary"))
        .or_else(|| value.get("title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);

    let modified = parse_rfc3339(
        value
            .get("last_active_at")
            .or_else(|| value.get("updated_at"))
            .or_else(|| value.get("created_at"))
            .and_then(Value::as_str),
    )
    .or_else(|| fs::metadata(path).and_then(|meta| meta.modified()).ok())
    .unwrap_or(UNIX_EPOCH);

    Ok(Some(SummaryMeta {
        id,
        cwd,
        title,
        modified,
    }))
}

fn resolve_session_dir(path: &Path) -> Result<PathBuf> {
    if path.is_dir() {
        return Ok(path.to_path_buf());
    }
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == CHAT_HISTORY_FILE || name == SUMMARY_FILE)
    {
        return path
            .parent()
            .map(Path::to_path_buf)
            .with_context(|| format!("Grok session path `{}` has no parent dir", path.display()));
    }
    // Bare session id under GROK_HOME/sessions/*/<id>
    if let Some(found) = GrokProvider.find_session_by_id(
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
    )? {
        return Ok(found);
    }
    anyhow::bail!(
        "Grok session path `{}` is not a session directory or {CHAT_HISTORY_FILE}",
        path.display()
    )
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    match value.get("type").and_then(Value::as_str) {
        Some("system") => {}
        Some("user") => apply_user(session, value),
        Some("assistant") => apply_assistant(session, value),
        Some("tool_result") => apply_tool_result(session, value),
        Some("reasoning") => apply_reasoning(session, value),
        _ => {}
    }
}

fn apply_user(session: &mut AgentSession, value: &Value) {
    // Injected scaffolding (system reminders, project instructions) is not a user turn.
    if value.get("synthetic_reason").is_some() {
        return;
    }

    let text = extract_user_text(value.get("content").unwrap_or(&Value::Null));
    if text.trim().is_empty() || is_scaffolding_user_text(&text) {
        return;
    }
    push_block(session, AgentBlockKind::User, None, None, text);
}

fn apply_assistant(session: &mut AgentSession, value: &Value) {
    let content = value.get("content").unwrap_or(&Value::Null);
    let text = extract_content_text(content);
    if !text.trim().is_empty() {
        push_block(session, AgentBlockKind::Assistant, None, None, text);
    }

    if let Some(tool_calls) = value.get("tool_calls") {
        match tool_calls {
            Value::Array(items) => {
                for tool_call in items {
                    push_one_tool_call(session, tool_call);
                }
            }
            Value::String(raw) => {
                if let Ok(Value::Array(items)) = serde_json::from_str(raw) {
                    for tool_call in items {
                        push_one_tool_call(session, &tool_call);
                    }
                } else {
                    push_block(
                        session,
                        AgentBlockKind::ToolCall,
                        None,
                        None,
                        pretty_json_string(raw),
                    );
                }
            }
            other => push_one_tool_call(session, other),
        }
    }
}

fn apply_tool_result(session: &mut AgentSession, value: &Value) {
    let content = value.get("content").unwrap_or(&Value::Null);
    let text = match content {
        Value::String(text) => text.clone(),
        other => extract_content_text(other),
    };
    push_block(session, AgentBlockKind::ToolOutput, None, None, text);
}

fn apply_reasoning(session: &mut AgentSession, value: &Value) {
    let summary = value.get("summary").unwrap_or(&Value::Null);
    let text = match summary {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::String(text) => text.clone(),
        other => extract_content_text(other),
    };
    if text.trim().is_empty() {
        return;
    }
    push_block(session, AgentBlockKind::Thinking, None, None, text);
}

fn push_one_tool_call(session: &mut AgentSession, tool_call: &Value) {
    let function = tool_call.get("function").unwrap_or(tool_call);
    let name = function
        .get("name")
        .or_else(|| tool_call.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let arguments = function
        .get("arguments")
        .or_else(|| tool_call.get("arguments"))
        .or_else(|| tool_call.get("input"))
        .map(|value| match value {
            Value::String(text) => pretty_json_string(text),
            other => pretty_json_value(other),
        })
        .unwrap_or_else(|| pretty_json_value(tool_call));
    push_block(session, AgentBlockKind::ToolCall, None, name, arguments);
}

fn extract_user_text(content: &Value) -> String {
    let raw = extract_content_text(content);
    extract_user_query(&raw).unwrap_or(raw)
}

fn extract_user_query(text: &str) -> Option<String> {
    let start_tag = "<user_query>";
    let end_tag = "</user_query>";
    let start = text.find(start_tag)? + start_tag.len();
    let end = text[start..].find(end_tag)? + start;
    let query = text[start..end].trim();
    if query.is_empty() {
        None
    } else {
        Some(query.to_string())
    }
}

fn is_scaffolding_user_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("<user_info>")
        || trimmed.starts_with("<system-reminder>")
        || trimmed.starts_with("<image_files>")
        || trimmed.starts_with("<environment_context>")
}

fn parse_rfc3339(value: Option<&str>) -> Option<SystemTime> {
    let value = value?;
    let dt = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    let secs = dt.timestamp();
    let nanos = dt.timestamp_subsec_nanos();
    if secs < 0 {
        return Some(UNIX_EPOCH);
    }
    Some(UNIX_EPOCH + Duration::new(secs as u64, nanos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentBlockKind, AgentSessionProvider};

    fn write_session(
        home: &Path,
        cwd_bucket: &str,
        session_id: &str,
        history: &str,
        summary: &str,
    ) {
        let dir = home.join("sessions").join(cwd_bucket).join(session_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(CHAT_HISTORY_FILE), history).unwrap();
        fs::write(dir.join(SUMMARY_FILE), summary).unwrap();
    }

    #[test]
    fn parses_grok_messages_tools_and_reasoning() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "019f6119-df57-7fe1-8e38-e2e41d5a506e";
        write_session(
            home.path(),
            "D%3A%5CCoding%5Crepo",
            session_id,
            r#"{"type":"system","content":"You are Grok"}
{"type":"user","content":[{"type":"text","text":"<user_info>\nWorkspace Path: D:\\repo\n</user_info>"}]}
{"type":"user","content":[{"type":"text","text":"<system-reminder>\nskills\n</system-reminder>"}],"synthetic_reason":"system_reminder"}
{"type":"user","content":[{"type":"text","text":"<user_query>\nfix the ghosting\n</user_query>"}],"prompt_index":0}
{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Inspect gallery CSS"}],"status":"completed"}
{"type":"assistant","content":"Looking at the gallery","tool_calls":[{"id":"call-1","name":"read_file","arguments":"{\"target_file\":\"a.tsx\"}"}],"model_id":"grok-4.5"}
{"type":"tool_result","tool_call_id":"call-1","content":"file body"}
{"type":"assistant","content":"done","model_id":"grok-4.5"}
"#,
            r#"{
  "info": {"id": "019f6119-df57-7fe1-8e38-e2e41d5a506e", "cwd": "D:\\Coding\\repo"},
  "generated_title": "fix gallery ghosting",
  "last_active_at": "2026-07-14T15:19:44.953389100Z"
}"#,
        );

        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let path = home
            .path()
            .join("sessions")
            .join("D%3A%5CCoding%5Crepo")
            .join(session_id);
        let session = GrokProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some(session_id));
        assert_eq!(session.cwd.as_deref(), Some("D:\\Coding\\repo"));
        assert_eq!(session.title.as_deref(), Some("fix gallery ghosting"));
        assert_eq!(session.blocks.len(), 6);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "fix the ghosting");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "Inspect gallery CSS");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].text, "Looking at the gallery");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[3].label.as_deref(), Some("read_file"));
        assert!(session.blocks[3].text.contains("a.tsx"));
        assert_eq!(session.blocks[4].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].text, "file body");
        assert_eq!(session.blocks[5].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[5].text, "done");
    }

    #[test]
    fn lists_sessions_and_filters_by_cwd() {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        write_session(
            home.path(),
            "repo-bucket",
            "sess-match",
            "{\"type\":\"user\",\"content\":\"hi\"}\n",
            &format!(
                r#"{{"info":{{"id":"sess-match","cwd":{}}},"generated_title":"match","last_active_at":"2026-07-14T15:00:00Z"}}"#,
                serde_json::to_string(&repo).unwrap()
            ),
        );
        write_session(
            home.path(),
            "other-bucket",
            "sess-other",
            "{\"type\":\"user\",\"content\":\"yo\"}\n",
            r#"{"info":{"id":"sess-other","cwd":"/other"},"generated_title":"other","last_active_at":"2026-07-14T16:00:00Z"}"#,
        );

        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let listed = GrokProvider
            .list_recent_sessions(Some(&repo))
            .expect("list");
        let ids: Vec<_> = listed
            .iter()
            .filter_map(|session| session.id.clone())
            .collect();
        assert!(ids.contains(&"sess-match".to_string()));
        assert!(!ids.contains(&"sess-other".to_string()));
        assert_eq!(listed[0].title.as_deref(), Some("match"));
    }

    #[test]
    fn parse_accepts_chat_history_path() {
        let home = tempfile::tempdir().unwrap();
        write_session(
            home.path(),
            "bucket",
            "sess1",
            r#"{"type":"user","content":[{"type":"text","text":"hello"}]}"#,
            r#"{"info":{"id":"sess1","cwd":"/tmp"},"session_summary":"title"}"#,
        );
        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let history = home
            .path()
            .join("sessions")
            .join("bucket")
            .join("sess1")
            .join(CHAT_HISTORY_FILE);
        let session = GrokProvider.parse_session_file(&history).unwrap();
        assert_eq!(session.id.as_deref(), Some("sess1"));
        assert_eq!(session.blocks[0].text, "hello");
    }

    #[test]
    fn empty_assistant_with_tools_still_records_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHAT_HISTORY_FILE);
        fs::write(
            &path,
            r#"{"type":"assistant","content":"","tool_calls":[{"id":"c1","name":"bash","arguments":"{\"command\":\"ls\"}"}]}
{"type":"tool_result","tool_call_id":"c1","content":"a.rs"}
"#,
        )
        .unwrap();
        // minimal summary sibling so resolve works when given dir
        fs::write(
            dir.path().join(SUMMARY_FILE),
            r#"{"info":{"id":"x","cwd":"/tmp"}}"#,
        )
        .unwrap();

        let session = GrokProvider.parse_session_file(dir.path()).unwrap();
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[0].label.as_deref(), Some("bash"));
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolOutput);
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<Path>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value.as_ref());
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
