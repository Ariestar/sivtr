use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::ai::{
    extract_content_text, list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_string, pretty_json_value, push_block, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionInfo, AgentSessionMeta, AgentSessionProvider,
};
use crate::config::SivtrConfig;

const PROVIDER_NAME: &str = "Codex";

#[derive(Debug, Clone, Copy, Default)]
pub struct CodexProvider;

impl AgentSessionProvider for CodexProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Codex
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
        let wanted = cwd.map(normalize_path_for_match);
        let mut sessions = Vec::new();

        for root in configured_codex_session_dirs() {
            for path in jsonl_files(&root)? {
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
                    modified: std::fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH),
                    path,
                    id: meta.id,
                    cwd: meta.cwd,
                });
            }
        }

        sessions.sort_by_key(|session| session.modified);
        sessions.reverse();
        Ok(sessions)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }

    fn find_session_by_id(&self, id: &str) -> Result<Option<PathBuf>> {
        for path in local_session_files()? {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(id))
            {
                return Ok(Some(path));
            }

            if parse_session_meta(&path)?.id.as_deref() == Some(id) {
                return Ok(Some(path));
            }
        }

        Ok(None)
    }

    fn find_current_session(&self, cwd: &Path) -> Result<Option<PathBuf>> {
        if let Some(path) = find_current_thread_session()? {
            return Ok(Some(path));
        }

        if let Some(session) = list_recent_local_sessions(Some(cwd))?.into_iter().next() {
            return Ok(Some(session.path));
        }

        Ok(list_recent_local_sessions(None)?
            .into_iter()
            .next()
            .map(|session| session.path))
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

pub fn local_codex_sessions_dir() -> PathBuf {
    codex_home().join("sessions")
}

pub fn configured_codex_session_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![local_codex_sessions_dir()];

    if let Ok(extra) = std::env::var("SIVTR_CODEX_SESSION_DIRS") {
        let separator = if cfg!(windows) { ';' } else { ':' };
        dirs.extend(
            extra
                .split(separator)
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(PathBuf::from),
        );
    } else if let Ok(config) = SivtrConfig::load() {
        dirs.extend(config.codex.session_dirs);
    }

    dedup_paths(dirs)
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

fn local_session_files() -> Result<Vec<PathBuf>> {
    jsonl_files(&local_codex_sessions_dir())
}

fn list_recent_local_sessions(cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>> {
    list_recent_jsonl_sessions(&local_codex_sessions_dir(), cwd, parse_session_meta)
}

fn find_current_thread_session() -> Result<Option<PathBuf>> {
    let Ok(thread_id) = std::env::var("CODEX_THREAD_ID") else {
        return Ok(None);
    };

    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Ok(None);
    }

    CodexProvider.find_session_by_id(thread_id)
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut deduped = Vec::new();
    for path in paths {
        if deduped.iter().any(|existing| existing == &path) {
            continue;
        }
        deduped.push(path);
    }
    deduped
}

fn jsonl_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files)?;
    Ok(files)
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))?
    {
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

fn normalize_path_for_match(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{configured_codex_session_dirs, CodexProvider};
    use crate::ai::{
        format_blocks, select_blocks, AgentBlockKind, AgentSelection, AgentSessionProvider,
    };
    use serde_json::json;
    use std::{env, path::PathBuf, time::Duration};

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

    #[test]
    fn find_current_session_prefers_codex_thread_id_over_cwd_match() {
        let temp = tempfile::tempdir().unwrap();
        let codex_home = temp.path().join("codex-home");
        let sessions_dir = codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("07");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        let cwd_match = temp.path().join("cwd-match");
        let thread_match = temp.path().join("thread-match");
        std::fs::create_dir_all(&cwd_match).unwrap();
        std::fs::create_dir_all(&thread_match).unwrap();

        let cwd_session = sessions_dir.join("rollout-cwd.jsonl");
        std::fs::write(
            &cwd_session,
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "type": "session_meta",
                    "payload": { "id": "cwd-session", "cwd": cwd_match }
                }))
                .unwrap()
            ),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(5));

        let thread_session = sessions_dir.join("rollout-thread.jsonl");
        std::fs::write(
            &thread_session,
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "type": "session_meta",
                    "payload": { "id": "thread-session", "cwd": thread_match }
                }))
                .unwrap()
            ),
        )
        .unwrap();

        let previous_codex_home = env::var_os("CODEX_HOME");
        let previous_thread_id = env::var_os("CODEX_THREAD_ID");
        env::set_var("CODEX_HOME", &codex_home);
        env::set_var("CODEX_THREAD_ID", "thread-session");

        let resolved = CodexProvider.find_current_session(&cwd_match).unwrap();

        match previous_codex_home {
            Some(value) => env::set_var("CODEX_HOME", value),
            None => env::remove_var("CODEX_HOME"),
        }
        match previous_thread_id {
            Some(value) => env::set_var("CODEX_THREAD_ID", value),
            None => env::remove_var("CODEX_THREAD_ID"),
        }

        assert_eq!(resolved, Some(thread_session));
    }

    #[test]
    fn configured_codex_session_dirs_reads_env_override_once() {
        let previous = env::var_os("SIVTR_CODEX_SESSION_DIRS");
        let separator = if cfg!(windows) { ";" } else { ":" };
        env::set_var(
            "SIVTR_CODEX_SESSION_DIRS",
            format!("/tmp/a{separator}/tmp/b{separator}/tmp/a"),
        );

        let dirs = configured_codex_session_dirs();

        match previous {
            Some(value) => env::set_var("SIVTR_CODEX_SESSION_DIRS", value),
            None => env::remove_var("SIVTR_CODEX_SESSION_DIRS"),
        }

        assert!(dirs.contains(&PathBuf::from("/tmp/a")));
        assert!(dirs.contains(&PathBuf::from("/tmp/b")));
        assert_eq!(
            dirs.iter()
                .filter(|path| *path == &PathBuf::from("/tmp/a"))
                .count(),
            1
        );
    }

    #[test]
    fn list_recent_sessions_includes_exported_session_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let codex_home = temp.path().join("codex-home");
        let local_sessions = codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("07");
        let exported_root = temp.path().join("shared-export");
        let exported_sessions = exported_root
            .join("sessions")
            .join("2026")
            .join("05")
            .join("06");
        std::fs::create_dir_all(&local_sessions).unwrap();
        std::fs::create_dir_all(&exported_sessions).unwrap();

        let local_path = local_sessions.join("rollout-local.jsonl");
        std::fs::write(
            &local_path,
            serde_json::to_string(&json!({
                "type": "session_meta",
                "payload": { "id": "local-session", "cwd": "/tmp/local" }
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();

        std::thread::sleep(Duration::from_millis(5));

        let exported_path = exported_sessions.join("rollout-exported.jsonl");
        std::fs::write(
            &exported_path,
            serde_json::to_string(&json!({
                "type": "session_meta",
                "payload": { "id": "exported-session", "cwd": "/tmp/exported" }
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();

        let previous_codex_home = env::var_os("CODEX_HOME");
        let previous_dirs = env::var_os("SIVTR_CODEX_SESSION_DIRS");
        env::set_var("CODEX_HOME", &codex_home);
        env::set_var("SIVTR_CODEX_SESSION_DIRS", exported_root.join("sessions"));

        let sessions = CodexProvider.list_recent_sessions(None).unwrap();

        match previous_codex_home {
            Some(value) => env::set_var("CODEX_HOME", value),
            None => env::remove_var("CODEX_HOME"),
        }
        match previous_dirs {
            Some(value) => env::set_var("SIVTR_CODEX_SESSION_DIRS", value),
            None => env::remove_var("SIVTR_CODEX_SESSION_DIRS"),
        }

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id.as_deref(), Some("exported-session"));
        assert_eq!(sessions[1].id.as_deref(), Some("local-session"));
        assert_eq!(sessions[0].path, exported_path);
        assert_eq!(sessions[1].path, local_path);
    }

    #[test]
    fn parses_session_when_last_json_line_is_still_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/tmp/repo"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"hello"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"done"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"partial"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].text, "done");
    }
}
