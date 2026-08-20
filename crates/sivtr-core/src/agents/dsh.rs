//! DeepSeek Harness (`dsh`) session provider.
//!
//! dsh (deepseek-harness) persists each agent session as an append-only
//! `SessionEvent` JSONL log under the harness home (`$DSH_HOME` or `~/.dsh`):
//!
//! ```text
//! <home>/sessions/--<project>--/<session-id>/session.jsonl[.zstd]
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
//! Parsing mirrors dsh's own `deriveMessages()` projection: blocks are built
//! from the surface events only (`user/message`, `assistant/message`,
//! `tool/result`), so packed chunk rows and raw `assistant/chunk` events are
//! skipped without expansion — they are token-level replay data, not dialogue.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, list_sessions_matching, pretty_json_string, push_block, push_tool_block,
    AgentBlockKind, AgentProvider, AgentSession, AgentSessionMeta, AgentSessionProvider,
    SessionInfo,
};

const PROVIDER_NAME: &str = "Dsh";

/// On-disk session log format version this build understands. dsh refuses
/// logs with any other version ("upgrade the harness"), so sivtr does too.
const SESSION_FORMAT_VERSION: u64 = 0;

/// Decompressed byte cap for listing metadata reads (covers the header frame
/// and the first flush batch, where the title normally lands).
const META_READ_CAP: usize = 4 * 1024 * 1024;

/// Compressed byte cap for the same metadata read. The header is always the
/// first line of the first frame, so a torn first frame still yields id/cwd.
const META_COMPRESSED_READ_CAP: u64 = 8 * 1024 * 1024;

/// Maximum lines scanned from a metadata read.
const META_MAX_LINES: usize = 1000;

/// Maximum accepted zstd window. Node's built-in zstd (what dsh uses) stays
/// far below this; the generous cap avoids truncating sessions re-encoded
/// with a high-level zstd CLI. Decoding is local, trusted input.
const MAX_ZSTD_WINDOW: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct DshProvider;

impl AgentSessionProvider for DshProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Dsh
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let root = sessions_root();
        list_sessions_matching(
            PROVIDER_NAME,
            &root,
            cwd,
            |path, is_dir| !is_dir && is_dsh_log(path),
            parse_session_meta,
        )
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

/// Whether the file is a dsh session log artifact (`session.jsonl` /
/// `session.jsonl.zstd`), whichever encoding the deployment uses.
fn is_dsh_log(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("session.jsonl") | Some("session.jsonl.zstd")
    )
}

/// Read a whole session log as text, decompressing `.jsonl.zstd` artifacts.
fn read_log(path: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(path)
        .with_context(|| format!("Failed to read {PROVIDER_NAME} session: {}", path.display()))?;
    if is_zstd(path) {
        decode_zstd(&bytes, None).with_context(|| {
            format!(
                "Failed to decompress {PROVIDER_NAME} session: {}",
                path.display()
            )
        })
    } else {
        Ok(bytes)
    }
}

/// Read the head of a session log for listing metadata, decompressing only
/// the leading compressed bytes of a `.jsonl.zstd` artifact.
fn read_log_head(path: &Path) -> Result<String> {
    let compressed = is_zstd(path);
    let mut file = fs::File::open(path)
        .with_context(|| format!("Failed to read {PROVIDER_NAME} session: {}", path.display()))?;
    let cap = if compressed {
        META_COMPRESSED_READ_CAP
    } else {
        META_READ_CAP as u64
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(cap)
        .read_to_end(&mut bytes)
        .with_context(|| format!("Failed to read {PROVIDER_NAME} session: {}", path.display()))?;
    let bytes = if compressed {
        decode_zstd(&bytes, Some(META_READ_CAP)).with_context(|| {
            format!(
                "Failed to decompress {PROVIDER_NAME} session: {}",
                path.display()
            )
        })?
    } else {
        bytes
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn is_zstd(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("zstd")
}

/// Decompress a concatenation of independent Zstandard frames (dsh's layout:
/// one checksummed frame for the header line, then one frame per flush
/// batch). A torn or unreadable trailing frame is dropped, mirroring the
/// JSONL backend's crash-recovery truncation; `cap` bounds decompressed
/// output for head reads.
fn decode_zstd(bytes: &[u8], cap: Option<usize>) -> Result<Vec<u8>> {
    use ruzstd::decoding::StreamingDecoder;

    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() && !cap.is_some_and(|cap| out.len() >= cap) {
        let mut cursor = std::io::Cursor::new(rest);
        let mut decoder =
            match StreamingDecoder::new_with_max_window_size(&mut cursor, MAX_ZSTD_WINDOW) {
                // Not a readable frame header: torn tail or trailing bytes. Keep
                // the complete frames decoded so far.
                Err(_) => break,
                Ok(decoder) => decoder,
            };
        let mut frame = Vec::new();
        let limit = cap.map(|cap| cap.saturating_sub(out.len()));
        let result = match limit {
            Some(limit) => decoder.by_ref().take(limit as u64).read_to_end(&mut frame),
            None => decoder.read_to_end(&mut frame),
        };
        let consumed = decoder.get_ref().position() as usize;
        match result {
            Ok(_) => {
                out.extend_from_slice(&frame);
                if consumed == 0 {
                    break;
                }
                rest = &rest[consumed..];
            }
            // Torn frame body: keep only the frames completed before it.
            Err(_) => break,
        }
    }
    Ok(out)
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
    for line in text.lines().take(META_MAX_LINES) {
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
            apply_header_to_meta(&mut meta, &value).with_context(|| {
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
        match value.get("type").and_then(Value::as_str) {
            Some("session/title") => {
                // dsh titles are latest-wins snapshots.
                if let Some(title) = event_title(&value) {
                    meta.title = Some(title);
                }
            }
            Some("user/message") if is_user_message(&value) && first_user.is_none() => {
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

fn apply_header_to_meta(meta: &mut AgentSessionMeta, value: &Value) -> Result<()> {
    check_format_version(value)?;
    if meta.id.is_none() {
        meta.id = value.get("id").and_then(Value::as_str).map(str::to_string);
    }
    if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
        meta.add_cwd(cwd);
    }
    Ok(())
}

fn check_format_version(value: &Value) -> Result<()> {
    let version = value.get("version").and_then(Value::as_u64);
    if version != Some(SESSION_FORMAT_VERSION) {
        bail!(
            "unsupported {PROVIDER_NAME} session log version {version:?} (this build reads version {SESSION_FORMAT_VERSION})"
        );
    }
    Ok(())
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
            check_format_version(&value)?;
            session.id = value.get("id").and_then(Value::as_str).map(str::to_string);
            session.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
            header_seen = true;
            continue;
        }
        apply_event(&mut session, &mut tool_names, &mut first_user, &value);
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

fn apply_event(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    first_user: &mut Option<String>,
    value: &Value,
) {
    let timestamp = value
        .get("time")
        .and_then(Value::as_i64)
        .map(|ms| ms.to_string());
    match value.get("type").and_then(Value::as_str) {
        Some("session/title") => {
            // dsh titles are latest-wins snapshots; keep the last one seen.
            if let Some(title) = event_title(value) {
                session.title = Some(title);
            }
        }
        Some("user/message") => {
            if !is_user_message(value) {
                return;
            }
            let Some(text) = user_message_text(value) else {
                return;
            };
            if first_user.is_none() {
                *first_user = Some(text.clone());
            }
            push_block(session, AgentBlockKind::User, timestamp, None, text);
        }
        Some("assistant/message") => {
            let Some(message) = value.pointer("/data/message") else {
                return;
            };
            apply_assistant_message(session, tool_names, timestamp, message);
        }
        Some("tool/call") => {
            if let (Some(call_id), Some(name)) = (
                value.pointer("/data/callId").and_then(Value::as_str),
                value.pointer("/data/name").and_then(Value::as_str),
            ) {
                tool_names.insert(call_id.to_string(), name.to_string());
            }
        }
        Some("tool/result") => {
            let Some(message) = value.pointer("/data/message") else {
                return;
            };
            apply_tool_result_message(session, tool_names, timestamp, message);
        }
        _ => {}
    }
}

fn event_title(value: &Value) -> Option<String> {
    value
        .pointer("/data/title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
}

/// dsh projects several synthetic user-role messages onto the surface:
/// runtime-context snapshots (`plugin`), workspace-instruction and
/// skill-catalog injections (`agent-instructions`, `skill-catalog`), cron
/// notices, and goal continuations. Only direct human prompts
/// (`source.kind == "user"`, or a missing source) belong in the dialogue —
/// the rest is runtime context that would pollute search.
fn is_user_message(value: &Value) -> bool {
    matches!(
        value.pointer("/data/source/kind").and_then(Value::as_str),
        None | Some("user")
    )
}

fn user_message_text(value: &Value) -> Option<String> {
    let text = value
        .pointer("/data/content")
        .map(extract_content_text)
        .unwrap_or_default();
    (!text.trim().is_empty()).then_some(text)
}

fn apply_assistant_message(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    timestamp: Option<String>,
    message: &Value,
) {
    let Some(content) = message.get("content") else {
        return;
    };
    let Value::Array(parts) = content else {
        return;
    };
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => push_block(
                session,
                AgentBlockKind::Assistant,
                timestamp.clone(),
                None,
                part.get("text").and_then(Value::as_str).unwrap_or_default(),
            ),
            Some("reasoning") => push_block(
                session,
                AgentBlockKind::Thinking,
                timestamp.clone(),
                None,
                part.get("text").and_then(Value::as_str).unwrap_or_default(),
            ),
            Some("tool-call") => {
                let call_id = part.get("id").and_then(Value::as_str).map(str::to_string);
                let name = part.get("name").and_then(Value::as_str).map(str::to_string);
                let arguments = part
                    .get("arguments")
                    .and_then(Value::as_str)
                    .map(pretty_json_string)
                    .unwrap_or_default();
                if let (Some(call_id), Some(name)) = (call_id.as_deref(), name.as_deref()) {
                    tool_names.insert(call_id.to_string(), name.to_string());
                }
                push_tool_block(
                    session,
                    AgentBlockKind::ToolCall,
                    timestamp.clone(),
                    call_id,
                    name,
                    arguments,
                    None,
                );
            }
            Some("tool-result") => {
                apply_tool_result_part(session, tool_names, timestamp.clone(), part)
            }
            _ => {}
        }
    }
}

fn apply_tool_result_message(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    timestamp: Option<String>,
    message: &Value,
) {
    let Some(content) = message.get("content") else {
        return;
    };
    let Value::Array(parts) = content else {
        return;
    };
    for part in parts {
        if part.get("type").and_then(Value::as_str) == Some("tool-result") {
            apply_tool_result_part(session, tool_names, timestamp.clone(), part);
        }
    }
}

fn apply_tool_result_part(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    timestamp: Option<String>,
    part: &Value,
) {
    let call_id = part
        .get("toolCallId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let label = call_id
        .as_deref()
        .and_then(|id| tool_names.get(id).cloned());
    let text = part
        .get("content")
        .map(extract_content_text)
        .unwrap_or_default();
    push_tool_block(
        session,
        AgentBlockKind::ToolOutput,
        timestamp,
        call_id,
        label,
        text,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{select_blocks, AgentSelection};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env_lock()
    }

    /// Compact but realistic dsh log: packed chunk rows are present but must
    /// not produce blocks; injected user/messages (workspace instructions,
    /// system-prompt snapshots) must be skipped. `{{cwd}}` is substituted per
    /// test so Windows backslashes stay JSON-escaped.
    const FIXTURE: &str = r#"{"type":"session","version":0,"id":"ff1c1e99-3bd4-4ef8-a954-80d607d628ba","createdAt":1783352165190,"cwd":"{{cwd}}","delegationDepth":0}
{"type":"user/message","seq":4,"time":1785498765364,"data":{"content":[{"type":"text","text":"Use the bash tool to run exactly: echo HELLO. Report the tool result you got back verbatim, then stop."}],"source":{"kind":"user"},"role":"user","id":"a207bd9d-9312-46ed-baaf-7a07a6f08ae8"},"surfaceOp":"append"}
{"type":"user/message","seq":5,"time":1785730418683,"data":{"content":[{"type":"text","text":"Instructions from: AGENTS.md\n\nThis file is the single source of truth."}],"source":{"kind":"agent-instructions"},"role":"user","id":"1c954f81-4e70-4e28-bf11-5f8424f09391"},"surfaceOp":"append"}
{"type":"session/title","seq":6,"time":1785730418683,"data":{"title":"Use the bash tool to","messageSeqs":[4],"source":{"kind":"fallback"}}}
{"type":"assistant/chunk","seq":9,"time":1783352166048,"data":{"turn":1,"step":1,"chunk":{"type":"block-start","index":0,"blockType":"reasoning"}}}
{"type":"reasoning-chunks","seq0":10,"time0":1783352166075,"data":{"turn":1,"step":1,"index":0,"dt":[0,0,1,0,0,28],"texts":["The"," user"," wants"," me"," to"," run"]}}
{"type":"assistant/chunk","seq":27,"time":1783352166250,"data":{"turn":1,"step":1,"chunk":{"type":"block-start","index":1,"blockType":"tool-call"}}}
{"type":"tool-call-chunks","seq0":28,"time0":1783352166250,"data":{"turn":1,"step":1,"index":1,"dt":[0,28,1],"id":"call_00_JliP571Bh0QQ8QExbSPk0080","name":"bash","args":["{","\"","command"]}}
{"type":"assistant/message","seq":57,"time":1785730418696,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[{"type":"reasoning","text":"The user wants me to run a simple bash command and report the result verbatim."},{"type":"tool-call","id":"call_00_JliP571Bh0QQ8QExbSPk0080","name":"bash","arguments":"{\"command\": \"echo HELLO\", \"description\": \"Run echo HELLO\"}"}],"source":{"kind":"model","provider":"deepseek-official","model":"deepseek-v4-flash"},"id":"658eb4a4-7462-43d8-91eb-13d09363db20"}},"sourceEventSeqs":[9,10,27,28],"surfaceOp":"append"}
{"type":"tool/call","seq":58,"time":1785730418696,"data":{"turn":1,"step":1,"callId":"call_00_JliP571Bh0QQ8QExbSPk0080","name":"bash","arguments":"{\"command\": \"echo HELLO\", \"description\": \"Run echo HELLO\"}"}}
{"type":"tool/result","seq":61,"time":1785730418702,"data":{"turn":1,"step":1,"message":{"source":{"kind":"tool","callId":"call_00_JliP571Bh0QQ8QExbSPk0080"},"content":[{"type":"tool-result","toolCallId":"call_00_JliP571Bh0QQ8QExbSPk0080","content":[{"type":"text","text":"Error: bash is disabled by policy in this session"}],"isError":true}],"role":"user","id":"85f289f4-cb3c-468e-bbad-e66fefe2346f"}},"sourceEventSeqs":[58],"surfaceOp":"append"}
{"type":"text-chunks","seq0":87,"time0":1783352167672,"data":{"turn":1,"step":2,"index":1,"dt":[29,0,1],"texts":["The"," tool"," returned"]}}
{"type":"assistant/message","seq":121,"time":1785730418716,"data":{"turn":1,"step":2,"message":{"role":"assistant","content":[{"type":"text","text":"The tool returned:\n\n> Error: bash is disabled by policy in this session\n\nI cannot run the command because the bash tool is disabled by policy."}],"source":{"kind":"model","provider":"deepseek-official","model":"deepseek-v4-flash"},"id":"0bea7b77-242e-4399-bd10-90324a37fff0"}},"sourceEventSeqs":[87,88],"surfaceOp":"append"}
{"type":"turn/end","seq":123,"time":1785730418717,"data":{"turn":1,"reason":{"kind":"completed"}}}
"#;

    /// Substitute the fixture's cwd placeholder with a JSON-escaped path.
    fn fixture_with_cwd(cwd: &str) -> String {
        FIXTURE.replace("{{cwd}}", &cwd.replace('\\', "\\\\"))
    }

    #[test]
    fn parses_dsh_session_blocks_from_surface_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, fixture_with_cwd("C:\\repo")).unwrap();

        let session = DshProvider.parse_session_file(&path).unwrap();

        assert_eq!(
            session.id.as_deref(),
            Some("ff1c1e99-3bd4-4ef8-a954-80d607d628ba")
        );
        assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
        assert_eq!(session.title.as_deref(), Some("Use the bash tool to"));
        // user, thinking, tool-call, tool-output, assistant. The injected
        // message, chunk rows, and tool/call must not add blocks.
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "Use the bash tool to run exactly: echo HELLO. Report the tool result you got back verbatim, then stop.");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("bash"));
        assert!(session.blocks[2]
            .text
            .contains("\"command\": \"echo HELLO\""));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[3].label.as_deref(), Some("bash"));
        assert_eq!(
            session.blocks[3].text,
            "Error: bash is disabled by policy in this session"
        );
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
        assert!(session.blocks[4].text.contains("I cannot run the command"));
    }

    #[test]
    fn only_user_sourced_messages_become_user_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","version":0,"id":"s1","cwd":"C:\\repo","delegationDepth":0}
{"type":"user/message","seq":1,"time":1000,"data":{"content":[{"type":"text","text":"direct prompt"}],"source":{"kind":"user"},"role":"user","id":"m1"},"surfaceOp":"append"}
{"type":"user/message","seq":2,"time":2000,"data":{"content":[{"type":"text","text":"workspace instructions"}],"source":{"kind":"agent-instructions"},"role":"user","id":"m2"},"surfaceOp":"append"}
{"type":"user/message","seq":3,"time":3000,"data":{"content":[{"type":"text","text":"skill catalog"}],"source":{"kind":"skill-catalog"},"role":"user","id":"m3"},"surfaceOp":"append"}
{"type":"user/message","seq":4,"time":4000,"data":{"content":[{"type":"text","text":"runtime snapshot"}],"source":{"kind":"plugin","plugin":"@deepseek-ai/dsh-system-prompt","form":"snapshot"},"role":"user","id":"m4"},"surfaceOp":"append"}
{"type":"user/message","seq":5,"time":5000,"data":{"content":[{"type":"text","text":"goal continuation"}],"source":{"kind":"goal"},"role":"user","id":"m5"},"surfaceOp":"append"}
{"type":"assistant/message","seq":6,"time":6000,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[{"type":"text","text":"done"}],"source":{"kind":"model","provider":"deepseek-official"},"id":"a1"}},"surfaceOp":"append"}
"#,
        )
        .unwrap();

        let session = DshProvider.parse_session_file(&path).unwrap();

        let user_blocks: Vec<_> = session
            .blocks
            .iter()
            .filter(|block| block.kind == AgentBlockKind::User)
            .collect();
        assert_eq!(user_blocks.len(), 1);
        assert_eq!(user_blocks[0].text, "direct prompt");
        assert_eq!(session.title.as_deref(), Some("direct prompt"));
    }

    #[test]
    fn fallback_title_uses_first_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","version":0,"id":"s1","cwd":"C:\\repo","delegationDepth":0}
{"type":"user/message","seq":1,"time":1000,"data":{"content":[{"type":"text","text":"Fix the flaky test"}],"source":{"kind":"user"},"role":"user","id":"m1"},"surfaceOp":"append"}
{"type":"assistant/message","seq":2,"time":2000,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[{"type":"text","text":"On it."}],"source":{"kind":"model","provider":"deepseek-official"},"id":"a1"}},"surfaceOp":"append"}
"#,
        )
        .unwrap();

        let session = DshProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.title.as_deref(), Some("Fix the flaky test"));
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text, "Fix the flaky test");
        assert_eq!(session.blocks[1].text, "On it.");
    }

    #[test]
    fn refuses_foreign_session_format_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","version":1,"id":"future","cwd":"C:\\repo","delegationDepth":0}
{"type":"user/message","seq":1,"time":1000,"data":{"content":[{"type":"text","text":"hi"}],"source":{"kind":"user"},"role":"user","id":"m1"},"surfaceOp":"append"}
"#,
        )
        .unwrap();

        let error = DshProvider.parse_session_file(&path).unwrap_err();
        assert!(format!("{error:#}").contains("unsupported Dsh session log version"));
    }

    #[test]
    fn refuses_log_without_session_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user/message","seq":1,"time":1000,"data":{"content":[{"type":"text","text":"hi"}],"source":{"kind":"user"},"role":"user","id":"m1"},"surfaceOp":"append"}
"#,
        )
        .unwrap();

        let error = DshProvider.parse_session_file(&path).unwrap_err();
        assert!(format!("{error:#}").contains("missing session header"));
    }

    #[test]
    fn refuses_empty_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "").unwrap();

        let error = DshProvider.parse_session_file(&path).unwrap_err();
        assert!(format!("{error:#}").contains("missing session header"));
    }

    #[test]
    fn refuses_empty_zstd_log() {
        // A .zstd artifact with no complete frame decodes to nothing, which
        // must be rejected like an empty plain log.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl.zstd");
        std::fs::write(&path, []).unwrap();

        let error = DshProvider.parse_session_file(&path).unwrap_err();
        assert!(format!("{error:#}").contains("missing session header"));
    }

    #[test]
    fn lists_dsh_sessions_under_dsh_home() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("DSH_HOME");
        std::env::set_var("DSH_HOME", dir.path());

        let sessions = dir.path().join("sessions").join("--repo--").join("s1");
        std::fs::create_dir_all(&sessions).unwrap();
        let repo_cwd = dir.path().join("repo");
        std::fs::write(
            sessions.join("session.jsonl"),
            fixture_with_cwd(&repo_cwd.to_string_lossy()),
        )
        .unwrap();
        // Malformed sibling must be skipped during listing.
        let bad = dir.path().join("sessions").join("--repo--").join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("session.jsonl"), "{not json}\n").unwrap();
        // Empty sibling must be skipped during listing, not listed as unbound.
        let empty = dir.path().join("sessions").join("--repo--").join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("session.jsonl"), "").unwrap();

        let listed = DshProvider.list_recent_sessions(None).unwrap();

        match previous {
            Some(value) => std::env::set_var("DSH_HOME", value),
            None => std::env::remove_var("DSH_HOME"),
        }

        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].id.as_deref(),
            Some("ff1c1e99-3bd4-4ef8-a954-80d607d628ba")
        );
        assert_eq!(
            listed[0].cwd.as_deref(),
            Some(repo_cwd.to_string_lossy().as_ref())
        );
        assert_eq!(listed[0].title.as_deref(), Some("Use the bash tool to"));
    }

    #[test]
    fn filters_dsh_sessions_by_workspace() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("DSH_HOME");
        std::env::set_var("DSH_HOME", dir.path());

        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let in_repo = dir.path().join("sessions").join("--repo--").join("s1");
        std::fs::create_dir_all(&in_repo).unwrap();
        std::fs::write(
            in_repo.join("session.jsonl"),
            fixture_with_cwd(&repo.to_string_lossy()),
        )
        .unwrap();
        let other = dir.path().join("sessions").join("--elsewhere--").join("s2");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("session.jsonl"),
            fixture_with_cwd(&dir.path().join("elsewhere").to_string_lossy()),
        )
        .unwrap();

        let listed = DshProvider.list_recent_sessions(Some(&repo)).unwrap();

        match previous {
            Some(value) => std::env::set_var("DSH_HOME", value),
            None => std::env::remove_var("DSH_HOME"),
        }

        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].id.as_deref(),
            Some("ff1c1e99-3bd4-4ef8-a954-80d607d628ba")
        );
    }

    #[test]
    fn decodes_zstd_concat_frames_and_torn_tail() {
        let bytes = zstd_fixture();

        let decoded = decode_zstd(&bytes, None).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), FIXTURE);

        // A torn trailing frame must be dropped, keeping the complete frames.
        let mut torn = bytes.clone();
        torn.extend_from_slice(&[0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x01]);
        let decoded = decode_zstd(&torn, None).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), FIXTURE);
    }

    #[test]
    fn parses_zstd_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl.zstd");
        std::fs::write(&path, zstd_fixture()).unwrap();

        let session = DshProvider.parse_session_file(&path).unwrap();

        assert_eq!(
            session.id.as_deref(),
            Some("ff1c1e99-3bd4-4ef8-a954-80d607d628ba")
        );
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(select_blocks(&session, AgentSelection::LastTurn).len(), 5);
    }

    /// dsh writes `session.jsonl.zstd` as one checksummed zstd frame per flush
    /// batch (header frame first). The fixture is committed as plaintext;
    /// compress the header line and the events as two separate frames, as the
    /// backend would.
    fn zstd_fixture() -> Vec<u8> {
        let fixture_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dsh/session.jsonl.zstd");
        fs::read(&fixture_path)
            .unwrap_or_else(|_| panic!("missing zstd fixture: {}", fixture_path.display()))
    }
}
