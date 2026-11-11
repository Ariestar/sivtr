use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, filter_sessions_by_workspace, open_readonly_db, pretty_json_value,
    push_block, system_time_from_millis, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionProvider, SessionInfo, SessionListing,
};

const SESSION_PATH_PREFIX: &str = "openclaw-session-";
const SESSION_PATH_SUFFIX: &str = ".json";
const AGENT_DB_NAME: &str = "openclaw-agent.sqlite";

/// OpenClaw session provider.
///
/// Store: `~/.openclaw/agents/<agentId>/agent/openclaw-agent.sqlite`
/// tables `session_entries` + `transcript_events`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenClawProvider;

impl AgentSessionProvider for OpenClawProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::OpenClaw
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        Ok(self.list_session_outcome(cwd)?.0)
    }

    fn list_session_outcome(&self, cwd: Option<&Path>) -> Result<SessionListing> {
        crate::agents::profile::list(AgentProvider::OpenClaw, cwd)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        crate::agents::profile::parse(AgentProvider::OpenClaw, path)
    }
}

pub(crate) fn list_outcome(_provider: AgentProvider, cwd: Option<&Path>) -> Result<SessionListing> {
    let mut sessions = list_sqlite_sessions()?;
    sessions.sort_by_key(|session| session.modified);
    sessions.reverse();
    Ok((filter_sessions_by_workspace(sessions, cwd), Vec::new()))
}

pub(crate) fn parse_file(_provider: AgentProvider, path: &Path) -> Result<AgentSession> {
    let (agent_id, session_id) = session_ids_from_path(path).with_context(|| {
        format!(
            "OpenClaw session path `{}` is not a state database session",
            path.display()
        )
    })?;
    let db_path = agent_db_path(&agent_id)
        .with_context(|| format!("OpenClaw agent `{agent_id}` has no session database"))?;
    parse_sqlite_session(&db_path, &agent_id, &session_id)
}

pub fn openclaw_home() -> PathBuf {
    if let Ok(path) =
        std::env::var("OPENCLAW_STATE_DIR").or_else(|_| std::env::var("OPENCLAW_HOME"))
    {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openclaw")
}

pub fn openclaw_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENCLAW_CONFIG_PATH") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    openclaw_home().join("openclaw.json")
}

fn agents_dir() -> PathBuf {
    openclaw_home().join("agents")
}

fn agent_db_path(agent_id: &str) -> Option<PathBuf> {
    if agent_id.trim().is_empty() {
        return None;
    }
    Some(
        agents_dir()
            .join(agent_id)
            .join("agent")
            .join(AGENT_DB_NAME),
    )
}

fn list_sqlite_sessions() -> Result<Vec<SessionInfo>> {
    let root = agents_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for agent_entry in fs::read_dir(&root)
        .with_context(|| format!("Failed to read OpenClaw agents dir {}", root.display()))?
    {
        let agent_entry = agent_entry?;
        if !agent_entry.file_type()?.is_dir() {
            continue;
        }
        let agent_id = agent_entry.file_name().to_string_lossy().to_string();
        let db_path = agent_entry.path().join("agent").join(AGENT_DB_NAME);
        if !db_path.exists() {
            continue;
        }
        out.extend(list_sessions_from_db(&db_path, &agent_id)?);
    }
    Ok(out)
}

fn list_sessions_from_db(db_path: &Path, agent_id: &str) -> Result<Vec<SessionInfo>> {
    let conn = open_readonly_db(db_path)?;
    // Current OpenClaw schema: session_entries(session_key, session_id, entry_json, updated_at, ...)
    let mut stmt = conn
        .prepare(
            "select session_key, session_id, entry_json, updated_at from session_entries order by updated_at desc, session_key asc",
        )
        .with_context(|| {
            format!(
                "Failed to list OpenClaw sessions in {}",
                db_path.display()
            )
        })?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        let (session_key, session_id, entry_json, updated_at) = row?;
        if session_key.starts_with("__") {
            continue;
        }
        let entry: Value = serde_json::from_str(&entry_json).with_context(|| {
            format!(
                "Failed to parse OpenClaw session entry in {}",
                db_path.display()
            )
        })?;
        if entry
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| {
                status.eq_ignore_ascii_case("archived") || status.eq_ignore_ascii_case("deleted")
            })
        {
            continue;
        }
        let cwd = entry.get("cwd").and_then(Value::as_str).map(str::to_string);
        let title = entry
            .get("displayName")
            .and_then(Value::as_str)
            .map(str::to_string);

        sessions.push(SessionInfo {
            path: openclaw_session_path(agent_id, &session_id),
            physical_path: Some(db_path.to_path_buf()),
            id: Some(session_id),
            cwd,
            title,
            modified: system_time_from_millis(updated_at),
        });
    }
    Ok(sessions)
}

fn parse_sqlite_session(db_path: &Path, agent_id: &str, session_id: &str) -> Result<AgentSession> {
    let conn = open_readonly_db(db_path)?;
    let entry_json: Option<String> = conn
        .query_row(
            "select entry_json from session_entries where session_id = ?1 limit 1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?;

    let entry: Value = match entry_json.as_deref() {
        Some(text) => serde_json::from_str(text).with_context(|| {
            format!(
                "Failed to parse OpenClaw session `{session_id}` in {}",
                db_path.display()
            )
        })?,
        None => Value::Null,
    };

    let mut session = AgentSession {
        path: openclaw_session_path(agent_id, session_id),
        id: Some(session_id.to_string()),
        cwd: entry.get("cwd").and_then(Value::as_str).map(str::to_string),
        title: entry
            .get("displayName")
            .and_then(Value::as_str)
            .map(str::to_string),
        blocks: Vec::new(),
    };

    let mut stmt = conn.prepare(
        "select event_json from transcript_events where session_id = ?1 order by seq asc",
    )?;
    let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
    for row in rows {
        let event_json = row?;
        let event: Value = serde_json::from_str(&event_json).with_context(|| {
            format!("Failed to parse OpenClaw transcript event for `{session_id}`")
        })?;
        apply_transcript_event(&mut session, &event);
    }

    Ok(session)
}

fn apply_transcript_event(session: &mut AgentSession, event: &Value) {
    let timestamp = event
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);

    match event.get("type").and_then(Value::as_str) {
        Some("message") => {
            if let Some(message) = event.get("message") {
                apply_message(session, message, timestamp);
            }
        }
        Some("toolCall") => {
            let label = event
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string);
            let input = event.get("input").unwrap_or(&Value::Null);
            push_block(
                session,
                AgentBlockKind::ToolCall,
                timestamp,
                label,
                pretty_json_value(input),
            );
        }
        Some("toolResult") => {
            let output = event.get("output").unwrap_or(&Value::Null);
            push_block(
                session,
                AgentBlockKind::ToolOutput,
                timestamp,
                None,
                extract_content_text(output),
            );
        }
        _ => {}
    }
}

fn apply_message(session: &mut AgentSession, message: &Value, timestamp: Option<String>) {
    let Some(role) = message.get("role").and_then(Value::as_str) else {
        return;
    };
    let content = message.get("content").unwrap_or(&Value::Null);

    match role {
        "user" => push_content(session, AgentBlockKind::User, timestamp, content),
        "assistant" => push_assistant(session, timestamp, content),
        "tool" => push_content(session, AgentBlockKind::ToolOutput, timestamp, content),
        _ => {}
    }
}

fn push_assistant(session: &mut AgentSession, timestamp: Option<String>, content: &Value) {
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
                    Some("toolCall") => {
                        let label = item.get("name").and_then(Value::as_str).map(str::to_string);
                        let input = item.get("input").unwrap_or(&Value::Null);
                        push_block(
                            session,
                            AgentBlockKind::ToolCall,
                            timestamp.clone(),
                            label,
                            pretty_json_value(input),
                        );
                    }
                    Some("toolResult") => push_block(
                        session,
                        AgentBlockKind::ToolOutput,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    ),
                    _ => {}
                }
            }
        }
        other => push_content(session, AgentBlockKind::Assistant, timestamp, other),
    }
}

fn push_content(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    content: &Value,
) {
    let text = extract_content_text(content);
    if !text.trim().is_empty() {
        push_block(session, kind, timestamp, None, text);
    }
}

fn openclaw_session_path(agent_id: &str, session_id: &str) -> PathBuf {
    PathBuf::from(format!(
        "{SESSION_PATH_PREFIX}{agent_id}--{session_id}{SESSION_PATH_SUFFIX}"
    ))
}

fn session_ids_from_path(path: &Path) -> Option<(String, String)> {
    let name = path.file_name()?.to_str()?;
    let rest = name
        .strip_prefix(SESSION_PATH_PREFIX)?
        .strip_suffix(SESSION_PATH_SUFFIX)?;
    let (agent_id, session_id) = rest.split_once("--")?;
    if agent_id.is_empty() || session_id.is_empty() {
        return None;
    }
    Some((agent_id.to_string(), session_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{
        openclaw_config_path, openclaw_home, openclaw_session_path, parse_sqlite_session,
        session_ids_from_path, OpenClawProvider, AGENT_DB_NAME,
    };
    use crate::agents::{AgentBlockKind, AgentProvider, AgentSessionProvider};
    use rusqlite::Connection;
    use std::path::PathBuf;

    #[test]
    fn provider_name_is_openclaw() {
        assert_eq!(AgentProvider::OpenClaw.name(), "OpenClaw");
        assert_eq!(AgentProvider::OpenClaw.command_name(), "openclaw");
    }

    #[test]
    fn config_path_lives_under_home() {
        let home = openclaw_home();
        assert_eq!(openclaw_config_path(), home.join("openclaw.json"));
    }

    #[test]
    fn session_path_round_trips_ids() {
        let path = openclaw_session_path("main", "sess-1");
        assert_eq!(
            session_ids_from_path(&path),
            Some(("main".into(), "sess-1".into()))
        );
    }

    #[test]
    fn parses_sqlite_transcript_events() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(AGENT_DB_NAME);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"
            create table session_entries (
                session_key text primary key,
                session_id text not null,
                entry_json text not null,
                updated_at integer not null
            );
            create table transcript_events (
                session_id text not null,
                seq integer not null,
                event_json text not null,
                created_at integer not null
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "insert into session_entries (session_key, session_id, entry_json, updated_at) values (?1,?2,?3,?4)",
            rusqlite::params![
                "agent:main:main",
                "sess-1",
                r#"{"sessionId":"sess-1","cwd":"D:/Coding/sivtr","displayName":"Main"}"#,
                1000_i64
            ],
        )
        .unwrap();
        conn.execute(
            "insert into transcript_events (session_id, seq, event_json, created_at) values (?1,?2,?3,?4)",
            rusqlite::params![
                "sess-1",
                1_i64,
                r#"{"type":"message","id":"m1","parentId":null,"timestamp":"t1","message":{"role":"user","content":"hello openclaw"}}"#,
                1001_i64
            ],
        )
        .unwrap();
        conn.execute(
            "insert into transcript_events (session_id, seq, event_json, created_at) values (?1,?2,?3,?4)",
            rusqlite::params![
                "sess-1",
                2_i64,
                r#"{"type":"message","id":"m2","parentId":"m1","timestamp":"t2","message":{"role":"assistant","content":[{"type":"text","text":"hi"},{"type":"toolCall","name":"bash","input":{"cmd":"ls"}}]}}"#,
                1002_i64
            ],
        )
        .unwrap();

        let session = parse_sqlite_session(&db, "main", "sess-1").expect("parse");
        assert_eq!(session.id.as_deref(), Some("sess-1"));
        assert_eq!(session.cwd.as_deref(), Some("D:/Coding/sivtr"));
        assert_eq!(session.blocks.len(), 3);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert!(session.blocks[0].text.contains("hello openclaw"));
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("bash"));
        assert_eq!(OpenClawProvider.provider(), AgentProvider::OpenClaw);
        let _ = PathBuf::from("noop");
    }
}
