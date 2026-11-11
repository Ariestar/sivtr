use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::agents::{
    filter_sessions_by_workspace, open_readonly_db, pretty_json_string, pretty_json_value,
    push_block, system_time_from_unix_secs, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionProvider, SessionInfo, SessionListing,
};

const STATE_DB_NAME: &str = "state.db";
const SESSION_PATH_PREFIX: &str = "hermes-session-";
const SESSION_PATH_SUFFIX: &str = ".sqlite";

/// Hermes session provider.
///
/// Current Hermes store: `$HERMES_HOME/state.db` tables `sessions` + `messages`.
#[derive(Debug, Clone, Copy, Default)]
pub struct HermesProvider;

impl AgentSessionProvider for HermesProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Hermes
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        Ok(self.list_session_outcome(cwd)?.0)
    }

    fn list_session_outcome(&self, cwd: Option<&Path>) -> Result<SessionListing> {
        crate::agents::profile::list(AgentProvider::Hermes, cwd)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        crate::agents::profile::parse(AgentProvider::Hermes, path)
    }
}

pub(crate) fn list_outcome(_provider: AgentProvider, cwd: Option<&Path>) -> Result<SessionListing> {
    Ok((
        filter_sessions_by_workspace(list_sqlite_sessions()?, cwd),
        Vec::new(),
    ))
}

pub(crate) fn parse_file(_provider: AgentProvider, path: &Path) -> Result<AgentSession> {
    let session_id = session_id_from_sqlite_path(path).with_context(|| {
        format!(
            "Hermes session path `{}` is not a state.db session",
            path.display()
        )
    })?;
    parse_sqlite_session(
        &hermes_state_db_path(),
        &session_id,
        &sqlite_session_path(&session_id),
    )
}

pub fn hermes_home() -> PathBuf {
    if let Ok(path) = std::env::var("HERMES_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
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

pub fn hermes_state_db_path() -> PathBuf {
    hermes_home().join(STATE_DB_NAME)
}

fn list_sqlite_sessions() -> Result<Vec<SessionInfo>> {
    let db_path = hermes_state_db_path();
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let conn = open_readonly_db(&db_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, source, model, cwd, title, started_at, ended_at, archived
             FROM sessions
             ORDER BY COALESCE(ended_at, started_at) DESC, id ASC",
        )
        .with_context(|| format!("Failed to list Hermes sessions in {}", db_path.display()))?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, Option<f64>>(6)?,
            row.get::<_, i64>(7)?,
        ))
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        let (id, source, model, cwd, title, started_at, ended_at, archived) = row?;
        if archived != 0 {
            continue;
        }

        let title = title
            .filter(|value| !value.trim().is_empty())
            .or_else(|| default_title(source.as_deref(), model.as_deref()));

        let modified = system_time_from_unix_secs(ended_at.unwrap_or(started_at));
        sessions.push(SessionInfo {
            path: sqlite_session_path(&id),
            physical_path: Some(db_path.clone()),
            id: Some(id),
            cwd: non_empty_opt(cwd),
            title,
            modified,
        });
    }
    Ok(sessions)
}

fn parse_sqlite_session(db_path: &Path, session_id: &str, path: &Path) -> Result<AgentSession> {
    let conn = open_readonly_db(db_path)?;
    let (source, model, cwd, title, _started_at): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        f64,
    ) = conn
        .query_row(
            "SELECT source, model, cwd, title, started_at FROM sessions WHERE id = ?1 LIMIT 1",
            [session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .with_context(|| {
            format!(
                "Hermes session `{session_id}` not found in {}",
                db_path.display()
            )
        })?;

    let mut session = AgentSession {
        path: path.to_path_buf(),
        id: Some(session_id.to_string()),
        cwd: non_empty_opt(cwd),
        title: title
            .filter(|value| !value.trim().is_empty())
            .or_else(|| default_title(source.as_deref(), model.as_deref())),
        blocks: Vec::new(),
    };

    let mut stmt = conn
        .prepare(
            "SELECT role, content, tool_call_id, tool_calls, tool_name, timestamp, active
             FROM messages
             WHERE session_id = ?1
             ORDER BY timestamp ASC, id ASC",
        )
        .with_context(|| format!("Failed to query Hermes messages in {}", db_path.display()))?;

    let rows = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, i64>(6)?,
        ))
    })?;

    for row in rows {
        let (role, content, _tool_call_id, tool_calls, tool_name, timestamp, active) = row?;
        if active == 0 {
            continue;
        }
        apply_db_message(
            &mut session,
            &role,
            content.as_deref(),
            tool_calls.as_deref(),
            tool_name.as_deref(),
            Some(timestamp_to_rfc3339(timestamp)),
        );
    }

    Ok(session)
}

fn apply_db_message(
    session: &mut AgentSession,
    role: &str,
    content: Option<&str>,
    tool_calls: Option<&str>,
    tool_name: Option<&str>,
    timestamp: Option<String>,
) {
    match role {
        "session_meta" | "system" => {}
        "user" => {
            if let Some(text) = content {
                push_block(session, AgentBlockKind::User, timestamp, None, text);
            }
        }
        "assistant" => {
            if let Some(text) = content.filter(|text| !text.trim().is_empty()) {
                push_block(
                    session,
                    AgentBlockKind::Assistant,
                    timestamp.clone(),
                    None,
                    text,
                );
            }
            if let Some(tool_calls) = tool_calls {
                push_tool_calls_from_json(session, tool_calls, timestamp);
            }
        }
        "tool" => {
            let text = content.unwrap_or_default();
            push_block(
                session,
                AgentBlockKind::ToolOutput,
                timestamp,
                tool_name.map(str::to_string),
                text,
            );
        }
        _ => {}
    }
}

fn push_tool_calls_from_json(
    session: &mut AgentSession,
    tool_calls: &str,
    timestamp: Option<String>,
) {
    let Ok(value) = serde_json::from_str::<Value>(tool_calls) else {
        push_block(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            None,
            pretty_json_string(tool_calls),
        );
        return;
    };

    match value {
        Value::Array(items) => {
            for item in items {
                push_one_tool_call(session, &item, timestamp.clone());
            }
        }
        other => push_one_tool_call(session, &other, timestamp),
    }
}

fn push_one_tool_call(session: &mut AgentSession, tool_call: &Value, timestamp: Option<String>) {
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
    push_block(
        session,
        AgentBlockKind::ToolCall,
        timestamp,
        name,
        arguments,
    );
}

fn default_title(source_or_platform: Option<&str>, model: Option<&str>) -> Option<String> {
    match (source_or_platform, model) {
        (Some(source), Some(model)) if !source.is_empty() && !model.is_empty() => {
            Some(format!("{source} · {model}"))
        }
        (Some(source), _) if !source.is_empty() => Some(source.to_string()),
        (_, Some(model)) if !model.is_empty() => Some(model.to_string()),
        _ => None,
    }
}

fn non_empty_opt(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn timestamp_to_rfc3339(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        return "1970-01-01T00:00:00Z".to_string();
    }
    let secs = value.trunc() as i64;
    let nanos = ((value.fract()) * 1_000_000_000.0).round() as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos.min(999_999_999))
        .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from(UNIX_EPOCH))
        .to_rfc3339()
}

fn sqlite_session_path(session_id: &str) -> PathBuf {
    PathBuf::from(format!(
        "{SESSION_PATH_PREFIX}{session_id}{SESSION_PATH_SUFFIX}"
    ))
}

fn session_id_from_sqlite_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix(SESSION_PATH_PREFIX)?
        .strip_suffix(SESSION_PATH_SUFFIX)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentBlockKind, AgentSessionProvider};
    use rusqlite::Connection;

    fn write_state_db(home: &Path) -> PathBuf {
        let db = home.join(STATE_DB_NAME);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                model TEXT,
                cwd TEXT,
                title TEXT,
                started_at REAL NOT NULL,
                ended_at REAL,
                archived INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT,
                tool_call_id TEXT,
                tool_calls TEXT,
                tool_name TEXT,
                timestamp REAL NOT NULL,
                active INTEGER NOT NULL DEFAULT 1
            );
            "#,
        )
        .unwrap();
        db
    }

    #[test]
    fn lists_sqlite_sessions_with_missing_cwd_only_when_unfiltered() {
        let home = tempfile::tempdir().unwrap();
        let db = write_state_db(home.path());
        let conn = Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source, model, cwd, title, started_at, ended_at, archived)
             VALUES
             ('s_no_cwd', 'weixin', 'm1', NULL, NULL, 100.0, 110.0, 0),
             ('s_match', 'cli', 'm2', '/repo', 'matched', 200.0, 210.0, 0),
             ('s_other', 'cli', 'm3', '/other', 'other', 300.0, 310.0, 0),
             ('s_arch', 'cli', 'm4', NULL, 'gone', 400.0, 410.0, 1)
            ",
            [],
        )
        .unwrap();
        drop(conn);

        let _guard = EnvGuard::set("HERMES_HOME", home.path());
        let sessions = HermesProvider
            .list_recent_sessions(Some(Path::new("/repo")))
            .expect("list");

        let ids: Vec<_> = sessions
            .iter()
            .filter_map(|session| session.id.clone())
            .collect();
        assert_eq!(ids, vec!["s_match"]);
        let all = HermesProvider.list_recent_sessions(None).expect("list all");
        assert_eq!(all.len(), 3);
        assert!(all
            .iter()
            .any(|session| session.id.as_deref() == Some("s_no_cwd")));
    }

    #[test]
    fn parses_sqlite_messages_and_tool_calls() {
        let home = tempfile::tempdir().unwrap();
        let db = write_state_db(home.path());
        let conn = Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source, model, cwd, title, started_at, ended_at, archived)
             VALUES ('sess1', 'weixin', 'anthropic/claude-opus-4.6', NULL, NULL, 100.0, 120.0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_call_id, tool_calls, tool_name, timestamp, active)
             VALUES
             ('sess1', 'session_meta', '', NULL, NULL, NULL, 100.0, 1),
             ('sess1', 'user', 'hello', NULL, NULL, NULL, 101.0, 1),
             ('sess1', 'assistant', '', NULL, '[{\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"terminal\",\"arguments\":\"{\\\"command\\\":\\\"echo hi\\\"}\"}}]', NULL, 102.0, 1),
             ('sess1', 'tool', '{\"output\":\"hi\"}', 'call_1', NULL, 'terminal', 103.0, 1),
             ('sess1', 'assistant', 'done', NULL, NULL, NULL, 104.0, 1)
            ",
            [],
        )
        .unwrap();
        drop(conn);

        let _guard = EnvGuard::set("HERMES_HOME", home.path());
        let path = sqlite_session_path("sess1");
        let session = HermesProvider.parse_session_file(&path).expect("parse");

        assert_eq!(session.id.as_deref(), Some("sess1"));
        assert_eq!(
            session.title.as_deref(),
            Some("weixin · anthropic/claude-opus-4.6")
        );
        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("terminal"));
        assert!(session.blocks[1].text.contains("echo hi"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[2].label.as_deref(), Some("terminal"));
        assert!(session.blocks[2].text.contains("hi"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "done");
    }

    #[test]
    fn system_time_from_unix_secs_converts() {
        let t = system_time_from_unix_secs(1.5);
        assert!(t > UNIX_EPOCH);
        let elapsed = t.duration_since(UNIX_EPOCH).unwrap();
        assert_eq!(elapsed.as_secs(), 1);
        assert!(elapsed.subsec_nanos() >= 400_000_000);
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<Path>) -> Self {
            let _lock = crate::test_env_lock();
            let previous = std::env::var_os(key);
            std::env::set_var(key, value.as_ref());
            Self {
                key,
                previous,
                _lock,
            }
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
