//! DeepSeek Harness (`dsh`) session provider.
//!
//! dsh (deepseek-harness) persists each agent session as an append-only
//! `SessionEvent` JSONL log under the harness home (`$DSH_HOME` or `~/.dsh`):
//!
//! ```text
//! <home>/sessions/--<project>--/<session-id>/session[.v4].jsonl[.zstd]
//! ```
//!
//! The first line is the immutable `session` header (`id`, `cwd`,
//! `createdAt`, ...); every later line is one event (`user/message`,
//! `assistant/message`, `tool/call`, `tool/result`, `session/title`, ...) or
//! a packed `assistant/chunk` delta run (`text-chunks` /
//! `reasoning-chunks` / `tool-call-chunks`). Logs are plain JSONL or a
//! concatenation of independent Zstandard frames (`.jsonl.zstd`), depending
//! on the deployment's `compression` setting.
//!
//! Blocks preserve the human transcript (append-origin messages in v4), not
//! compaction's model-only replacement copies. Blocks are built
//! from the surface events only (`user/message`, `assistant/message`,
//! `tool/result`), so packed chunk rows and raw `assistant/chunk` events are
//! skipped without expansion — they are token-level replay data, not dialogue.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::agents::AgentBlockKind;
use crate::agents::{
    list_sessions_matching, AgentProvider, AgentSession, AgentSessionMeta, AgentSessionProvider,
    SessionInfo,
};

#[path = "dsh/io.rs"]
mod io;
#[path = "dsh/messages.rs"]
mod messages;

#[cfg(test)]
use io::decode_zstd;
use io::{is_zstd, read_log, read_log_head};
use messages::{
    apply_event, event_title, is_append_message, is_user_message, user_message_text,
    validate_v4_event,
};

const PROVIDER_NAME: &str = "Dsh";

// Discovery predicates and metadata rules changed; do not reuse v0-only directory entries.
const LISTING_CACHE_KEY: &str = "Dsh-v4";

const META_MAX_LINES: usize = 1000;

#[cfg(test)]
#[path = "dsh/v4_tests.rs"]
mod v4_tests;

#[derive(Debug, Clone, Copy, Default)]
pub struct DshProvider;

impl AgentSessionProvider for DshProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Dsh
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let root = sessions_root();
        let sessions = list_sessions_matching(
            LISTING_CACHE_KEY,
            &root,
            cwd,
            |path, is_dir| !is_dir && is_dsh_log(path),
            parse_session_meta,
        )?;
        let mut current = Vec::new();
        for session in sessions {
            if is_latest_log(&session.path)? {
                current.push(session);
            }
        }
        Ok(current)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let bytes = read_log(path).with_context(|| {
            format!("Failed to read {PROVIDER_NAME} session: {}", path.display())
        })?;
        let text = std::str::from_utf8(&bytes)
            .with_context(|| format!("{PROVIDER_NAME} session is not UTF-8: {}", path.display()))?;
        parse_log_text(path, text)
    }
}

/// Harness home: `$DSH_HOME`, else `~/.dsh`.
pub fn dsh_home() -> PathBuf {
    if let Ok(path) = std::env::var("DSH_HOME") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".dsh")
}

/// Default session-log root of the JSONL persistence backend.
fn sessions_root() -> PathBuf {
    dsh_home().join("sessions")
}

/// Recognize all generations so an unsupported successor cannot expose stale history.
fn is_dsh_log(path: &Path) -> bool {
    log_generation(path).is_some()
}

fn log_generation(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    let name = name.strip_suffix(".zstd").unwrap_or(name);
    if name == "session.jsonl" {
        Some(0)
    } else {
        name.strip_prefix("session.v")?
            .strip_suffix(".jsonl")?
            .parse()
            .ok()
    }
}

fn is_latest_log(path: &Path) -> Result<bool> {
    let Some(parent) = path.parent() else {
        return Ok(true);
    };
    let rank = (log_generation(path), is_zstd(path));
    // Predecessors stay immutable on disk. Even an unreadable newer generation
    // suppresses the old one: falling back would silently present stale history.
    for entry in fs::read_dir(parent).context("Failed to inspect Dsh session generations")? {
        let entry = entry?;
        let sibling = entry.path();
        if entry.file_type()?.is_file()
            && is_dsh_log(&sibling)
            && (log_generation(&sibling), is_zstd(&sibling)) > rank
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Parse the bounded head of a log into listing metadata. The first line must
/// be a readable `session` header (dsh refuses anything else, so a log whose
/// head does not start with one is skipped, not listed as unbound); a torn
/// tail inside the window is tolerated.
fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    let text = read_log_head(path)?;
    let mut meta = AgentSessionMeta::default();
    let mut first_user: Option<String> = None;
    let mut header_seen = false;
    let mut version = 0;
    for (idx, line) in text.lines().take(META_MAX_LINES).enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if !header_seen {
            let value: Value = serde_json::from_str(line).with_context(|| {
                format!(
                    "Failed to parse {PROVIDER_NAME} session metadata: {}",
                    path.display()
                )
            })?;
            if value.get("type").and_then(Value::as_str) != Some("session") {
                return Err(missing_session_header(path));
            }
            version = apply_header_to_meta(&mut meta, path, &value).with_context(|| {
                format!(
                    "Failed to parse {PROVIDER_NAME} session metadata: {}",
                    path.display()
                )
            })?;
            header_seen = true;
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if version == 4 {
            validate_v4_event(&value).with_context(|| {
                format!(
                    "Invalid Dsh v4 session metadata at line {}: {}",
                    idx + 1,
                    path.display()
                )
            })?;
        }
        match value.get("type").and_then(Value::as_str) {
            Some("session/title") => {
                // dsh titles are latest-wins snapshots.
                if let Some(title) = event_title(&value) {
                    meta.title = Some(title);
                }
            }
            Some("user/message")
                if is_append_message(&value, version)
                    && is_user_message(&value, version)
                    && first_user.is_none() =>
            {
                first_user = user_message_text(&value);
            }
            _ => {}
        }
    }
    if !header_seen {
        return Err(missing_session_header(path));
    }
    meta.fallback_title(first_user.as_deref());
    Ok(meta)
}

fn apply_header_to_meta(meta: &mut AgentSessionMeta, path: &Path, value: &Value) -> Result<u64> {
    let version = check_format_version(path, value)?;
    if meta.id.is_none() {
        meta.id = value.get("id").and_then(Value::as_str).map(str::to_string);
    }
    if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
        meta.add_cwd(cwd);
    }
    Ok(version)
}

fn check_format_version(path: &Path, value: &Value) -> Result<u64> {
    let version = value.get("version").and_then(Value::as_u64);
    if !matches!(version, Some(0 | 4)) {
        bail!(
            "unsupported {PROVIDER_NAME} session log version {version:?} (this build reads versions 0 and 4)"
        );
    }
    let version = version.unwrap();
    if log_generation(path).is_some_and(|generation| generation != version) {
        bail!("Dsh session filename does not match header version {version}");
    }
    Ok(version)
}

/// Full parse of one session log into blocks, following dsh's derived-history
/// projection: `user/message` (non-plugin), `assistant/message` parts, and
/// `tool/result` contents. `tool/call` events only supply the callId → tool
/// name map used to label tool results.
fn parse_log_text(path: &Path, text: &str) -> Result<AgentSession> {
    let mut session = AgentSession {
        path: path.to_path_buf(),
        id: None,
        cwd: None,
        title: None,
        blocks: Vec::new(),
    };
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut first_user: Option<String> = None;
    let mut header_seen = false;
    let mut version = 0;

    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            // Torn tail line without a newline: dsh keeps a torn tail only
            // for the in-progress batch, so treat it as the end of the log.
            Err(error) if header_seen && error.classify() == serde_json::error::Category::Eof => {
                break;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to parse {PROVIDER_NAME} session line {} as JSON: {}",
                        idx + 1,
                        path.display()
                    )
                });
            }
        };
        if !header_seen {
            if value.get("type").and_then(Value::as_str) != Some("session") {
                return Err(missing_session_header(path));
            }
            version = check_format_version(path, &value)?;
            session.id = value.get("id").and_then(Value::as_str).map(str::to_string);
            session.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
            header_seen = true;
            continue;
        }
        if version == 4 {
            validate_v4_event(&value).with_context(|| {
                format!(
                    "Invalid Dsh v4 session at line {}: {}",
                    idx + 1,
                    path.display()
                )
            })?;
        }
        apply_event(
            &mut session,
            &mut tool_names,
            &mut first_user,
            &value,
            version,
        );
    }

    if !header_seen {
        return Err(missing_session_header(path));
    }
    if session.title.is_none() {
        session.title = first_user
            .as_deref()
            .map(|text| text.lines().next().unwrap_or(text).trim().to_string())
            .filter(|title| !title.is_empty());
    }
    Ok(session)
}

/// Error for a log with no readable `session` header: empty, whitespace-only,
/// or a first line of a different type. Shared by the metadata and full
/// parsers so the contract stays in one place.
fn missing_session_header(path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "not a {PROVIDER_NAME} session log (missing session header): {}",
        path.display()
    )
}

#[cfg(test)]
#[path = "dsh/tests.rs"]
mod tests;
