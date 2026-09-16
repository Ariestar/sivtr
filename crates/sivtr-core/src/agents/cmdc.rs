//! Command Code (`cmdc`) session provider.
//!
//! Sessions live per project under the Command Code home (`$COMMANDCODE_HOME`
//! or `~/.commandcode`):
//!
//! ```text
//! <home>/projects/<project-slug>/<session-id>.jsonl
//! ```
//!
//! The first line is the immutable `session` header (`id`, `timestamp`, `cwd`);
//! every later line is one entry. Entries form a tree (`id` / `parentId`) so
//! `/rewind` and `/tree` move a pointer instead of rewriting the file; this
//! provider reads the file linearly, so content parked on an inactive branch is
//! still searchable.
//!
//! A `message` entry carries `message.role` (`user` / `assistant`) and
//! `message.content` parts:
//!
//! - `text` — dialogue; `role` decides whether it belongs to the user or the
//!   assistant
//! - `thinking` — the model reasoning channel
//! - `tool_use` — a tool call (`id`, `name`, `input`)
//! - `tool_result` — a tool result (`tool_use_id`, `content`), delivered on a
//!   `role: "user"` message, which is why the parts decide the block kind and
//!   the role only ever splits `text` between user and assistant
//!
//! Alongside each transcript cmdc writes `<id>.meta.json` (the session title),
//! `<id>.checkpoints.jsonl` (`/rewind` file snapshots), `<id>.prompts.jsonl`
//! (prompt history), `config.json`, and `mcp.json`. Only the transcript is
//! dialogue.

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, list_sessions_matching, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_value, push_block, push_tool_block, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionMeta, AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "Command Code";

/// Maximum lines scanned from a transcript head to fill listing metadata. The
/// `session` header and the opening user prompt are always in the first few
/// entries, so the listing never reads a whole transcript.
const META_MAX_LINES: usize = 200;

/// Command Code session provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct CmdcProvider;

impl AgentSessionProvider for CmdcProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::CommandCode
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        list_sessions_matching(
            PROVIDER_NAME,
            &projects_root(),
            cwd,
            is_session_transcript,
            parse_session_meta,
        )
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_transcript(path)
    }
}

/// Command Code home: `$COMMANDCODE_HOME`, else `~/.commandcode`.
///
/// The product documents no home variable; this override exists so tests and
/// relocated installs can point at another root.
pub fn cmdc_home() -> PathBuf {
    if let Some(path) = std::env::var_os("COMMANDCODE_HOME").filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".commandcode")
}

/// Root holding one directory per project, keyed by a slug of its cwd.
fn projects_root() -> PathBuf {
    cmdc_home().join("projects")
}

/// Whether a direct child is a session transcript. Every other artifact cmdc
/// writes next to a transcript is either a different extension (`.meta.json`,
/// `config.json`, `mcp.json`) or a named sidecar of the same extension.
fn is_session_transcript(path: &Path, is_dir: bool) -> bool {
    if is_dir || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    !name.ends_with(".checkpoints.jsonl") && !name.ends_with(".prompts.jsonl")
}

/// The `session` object that opens every transcript.
fn is_session_header(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("session")
}

/// Error for a file that does not open with a `session` header: empty, or a
/// different JSONL artifact. Shared by both parsers so the contract stays in
/// one place.
fn missing_session_header(path: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "not a {PROVIDER_NAME} session transcript (missing session header): {}",
        path.display()
    )
}

/// First user text of a `message` entry, used as the title fallback. Tool
/// results also arrive on `role: "user"` messages but carry no `text` part, so
/// they can never win here.
fn user_text(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let message = value.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    for part in message.get("content")?.as_array()? {
        if part.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let text = part
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    None
}

/// Session title: the `<id>.meta.json` sidecar's `title` (what `/rename` and
/// the session picker write), else the first user prompt.
fn session_title(path: &Path, first_user: Option<&str>) -> Option<String> {
    let mut meta = AgentSessionMeta {
        title: sidecar_title(path),
        ..AgentSessionMeta::default()
    };
    meta.fallback_title(first_user);
    meta.title
}

/// Title from the session's `<id>.meta.json` sidecar. The sidecar is optional
/// metadata, so an absent one — a session nobody has named yet — simply leaves
/// the title to the first user prompt.
fn sidecar_title(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path.with_extension("meta.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let title = value.get("title").and_then(Value::as_str)?.trim();
    (!title.is_empty()).then(|| title.to_string())
}

/// Parse the bounded head of a transcript for listing metadata: the `session`
/// header's id and cwd, plus the first user prompt when the sidecar holds no
/// title. The header is the first line; a file that does not start with one is
/// not a transcript and is rejected so the shared listing layer skips it.
fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    let mut header_seen = false;
    let mut first_user = None;
    let mut meta = parse_jsonl_meta(path, PROVIDER_NAME, META_MAX_LINES, |meta, value| {
        if !header_seen {
            header_seen = is_session_header(value);
            if header_seen {
                if meta.id.is_none() {
                    meta.id = value.get("id").and_then(Value::as_str).map(str::to_string);
                }
                if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                    meta.add_cwd(cwd);
                }
            }
            return;
        }
        if first_user.is_none() {
            first_user = user_text(value);
        }
    })?;

    if !header_seen {
        return Err(missing_session_header(path));
    }
    meta.title = session_title(path, first_user.as_deref());
    Ok(meta)
}

/// Full parse of one transcript into blocks.
fn parse_transcript(path: &Path) -> Result<AgentSession> {
    let mut header_seen = false;
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut first_user = None;
    let mut session = parse_jsonl_session(path, PROVIDER_NAME, |session, value| {
        if !header_seen {
            header_seen = is_session_header(value);
            if header_seen {
                session.id = value.get("id").and_then(Value::as_str).map(str::to_string);
                session.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
            }
            return;
        }
        apply_entry(session, &mut tool_names, &mut first_user, value);
    })?;

    if !header_seen {
        return Err(missing_session_header(path));
    }
    session.title = session_title(path, first_user.as_deref());
    Ok(session)
}

/// Project one entry onto blocks. Entry types other than `message` — model and
/// effort changes, compaction summaries — carry no dialogue parts and are
/// ignored rather than rejected, so a newer transcript schema degrades instead
/// of failing to load.
fn apply_entry(
    session: &mut AgentSession,
    tool_names: &mut HashMap<String, String>,
    first_user: &mut Option<String>,
    value: &Value,
) {
    if value.get("type").and_then(Value::as_str) != Some("message") {
        return;
    }
    let Some(message) = value.get("message") else {
        return;
    };
    let Some(parts) = message.get("content").and_then(Value::as_array) else {
        return;
    };
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);

    for part in parts {
        let part_str = |key: &str| part.get(key).and_then(Value::as_str).unwrap_or_default();
        match part.get("type").and_then(Value::as_str).unwrap_or_default() {
            "text" => {
                let kind = match role {
                    "user" => AgentBlockKind::User,
                    "assistant" => AgentBlockKind::Assistant,
                    _ => continue,
                };
                if kind == AgentBlockKind::User && first_user.is_none() {
                    *first_user = non_empty(part_str("text"));
                }
                push_block(session, kind, timestamp.clone(), None, part_str("text"));
            }
            "thinking" => push_block(
                session,
                AgentBlockKind::Thinking,
                timestamp.clone(),
                None,
                part_str("thinking"),
            ),
            "tool_use" => {
                let call_id = non_empty(part_str("id"));
                let label = non_empty(part_str("name"));
                if let (Some(call_id), Some(label)) = (call_id.as_deref(), label.as_deref()) {
                    tool_names.insert(call_id.to_string(), label.to_string());
                }
                push_tool_block(
                    session,
                    AgentBlockKind::ToolCall,
                    timestamp.clone(),
                    call_id,
                    label,
                    pretty_json_value(part.get("input").unwrap_or(&Value::Null)),
                    None,
                );
            }
            "tool_result" => {
                let call_id = non_empty(part_str("tool_use_id"));
                // The result carries no tool name, so it is labelled from the
                // `tool_use` that opened the call.
                let label = call_id
                    .as_deref()
                    .and_then(|id| tool_names.get(id).cloned());
                push_tool_block(
                    session,
                    AgentBlockKind::ToolOutput,
                    timestamp.clone(),
                    call_id,
                    label,
                    part.get("content")
                        .map(extract_content_text)
                        .unwrap_or_default(),
                    None,
                );
            }
            _ => {}
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_repo;

    const SESSION_ID: &str = "7f3c1a2e-0d5b-4a91-9c88-2b6f4d1e5a30";

    /// Compact but realistic transcript: one message mixing every content part
    /// kind, a tool result on a `role: "user"` message, and one entry of a type
    /// that carries no dialogue. `{{cwd}}` is substituted per test so Windows
    /// backslashes stay JSON-escaped.
    const FIXTURE: &str = r#"{"type":"session","version":3,"id":"7f3c1a2e-0d5b-4a91-9c88-2b6f4d1e5a30","timestamp":"2026-09-16T02:57:11.112Z","cwd":"{{cwd}}"}
{"type":"message","id":"9c14d12a","parentId":null,"timestamp":"2026-09-16T02:58:10.867Z","message":{"role":"user","content":[{"type":"text","text":"make sivtr support cmdc"}],"meta":{"source":"user","createdAt":1789527486480,"messageId":"4730b9bb-36a9-41cb-8144-ebdc5ad6260d"}}}
{"type":"message","id":"13c036c4","parentId":"9c14d12a","timestamp":"2026-09-16T02:58:10.868Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"I should read the provider registry first.","signature":""},{"type":"text","text":"Let me read the registry."},{"type":"tool_use","id":"call_00_jcJ08iy1tD0Scp2bR5vn5257","name":"read_file","input":{"file_path":"agents/mod.rs"}}],"meta":{"source":"model","messageId":"45815839-d8f6-4939-ac05-2cecc7062ed3"}},"usage":{"inputTokens":26580,"outputTokens":164},"model":"deepseek/deepseek-v4-flash"}
{"type":"message","id":"be8dc328","parentId":"13c036c4","timestamp":"2026-09-16T02:58:21.035Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_00_jcJ08iy1tD0Scp2bR5vn5257","content":[{"type":"text","text":"Read 1/1 file, 620 lines"}]}],"meta":{"source":"tool","messageId":"bbe07bf7-cd5e-4e48-948f-eba73cb04374"}}}
{"type":"message","id":"4c537af5","parentId":"be8dc328","timestamp":"2026-09-16T02:58:22.036Z","message":{"role":"assistant","content":[{"type":"text","text":"The registry lists every provider."}],"meta":{"source":"model","messageId":"61b3f1a2-52d4-4a0d-9c1f-7ad2f4a52b17"}}}
{"type":"model-change","timestamp":"2026-09-16T02:59:00.000Z","model":"deepseek/deepseek-v4-flash"}
"#;

    /// Substitute the fixture's cwd placeholder with a JSON-escaped path.
    fn fixture_with_cwd(cwd: &str) -> String {
        FIXTURE.replace("{{cwd}}", &cwd.replace('\\', "\\\\"))
    }

    /// Write the fixture transcript as `<session-id>.jsonl` into `dir`.
    fn write_transcript(dir: &Path, cwd: &str) -> PathBuf {
        fs::create_dir_all(dir).expect("create transcript dir");
        let path = dir.join(format!("{SESSION_ID}.jsonl"));
        fs::write(&path, fixture_with_cwd(cwd)).expect("write transcript");
        path
    }

    /// Point both `COMMANDCODE_HOME` (provider root) and `SIVTR_HOME` (listing
    /// cache) at throwaway directories for the duration of `body`.
    fn with_home<T>(dir: &Path, body: impl FnOnce() -> T) -> T {
        let previous_home = std::env::var_os("COMMANDCODE_HOME");
        let previous_sivtr = std::env::var_os("SIVTR_HOME");
        std::env::set_var("COMMANDCODE_HOME", dir);
        std::env::set_var("SIVTR_HOME", dir.join("data"));
        let result = body();
        match previous_home {
            Some(value) => std::env::set_var("COMMANDCODE_HOME", value),
            None => std::env::remove_var("COMMANDCODE_HOME"),
        }
        match previous_sivtr {
            Some(value) => std::env::set_var("SIVTR_HOME", value),
            None => std::env::remove_var("SIVTR_HOME"),
        }
        result
    }

    #[test]
    fn parses_every_content_part_kind_without_collapsing_channels() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(dir.path(), "C:\\repo");

        let session = CmdcProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some(SESSION_ID));
        assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
        // The non-message entry adds nothing.
        let kinds: Vec<_> = session.blocks.iter().map(|block| block.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AgentBlockKind::User,
                AgentBlockKind::Thinking,
                AgentBlockKind::Assistant,
                AgentBlockKind::ToolCall,
                AgentBlockKind::ToolOutput,
                AgentBlockKind::Assistant,
            ]
        );
        assert_eq!(session.blocks[0].text, "make sivtr support cmdc");
        assert_eq!(
            session.blocks[0].timestamp.as_deref(),
            Some("2026-09-16T02:58:10.867Z")
        );
        // The reasoning channel reads `thinking`, not `text`.
        assert_eq!(
            session.blocks[1].text,
            "I should read the provider registry first."
        );
        assert_eq!(session.blocks[3].label.as_deref(), Some("read_file"));
        assert_eq!(
            session.blocks[3].call_id.as_deref(),
            Some("call_00_jcJ08iy1tD0Scp2bR5vn5257")
        );
        assert!(session.blocks[3]
            .text
            .contains("\"file_path\": \"agents/mod.rs\""));
        // A tool result is plain text, labelled from the call that opened it.
        assert_eq!(session.blocks[4].text, "Read 1/1 file, 620 lines");
        assert_eq!(session.blocks[4].label.as_deref(), Some("read_file"));
        assert_eq!(
            session.blocks[4].call_id.as_deref(),
            Some("call_00_jcJ08iy1tD0Scp2bR5vn5257")
        );
    }

    #[test]
    fn titles_the_session_from_the_meta_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(dir.path(), "C:\\repo");
        fs::write(
            path.with_extension("meta.json"),
            r#"{"traceIds":["079d6bcc5b4edf5d511d841bbb5d4b62"],"title":"Sivtr Cmdc Support"}"#,
        )
        .unwrap();

        assert_eq!(
            CmdcProvider
                .parse_session_file(&path)
                .unwrap()
                .title
                .as_deref(),
            Some("Sivtr Cmdc Support")
        );

        let meta = parse_session_meta(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("Sivtr Cmdc Support"));
        assert_eq!(meta.id.as_deref(), Some(SESSION_ID));
        assert_eq!(meta.cwd.as_deref(), Some("C:\\repo"));
    }

    #[test]
    fn falls_back_to_the_first_user_prompt_when_no_sidecar_title_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(dir.path(), "C:\\repo");

        assert_eq!(
            CmdcProvider
                .parse_session_file(&path)
                .unwrap()
                .title
                .as_deref(),
            Some("make sivtr support cmdc")
        );
    }

    #[test]
    fn refuses_a_file_without_a_session_header() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = dir.path().join("foreign.jsonl");
        fs::write(
            &foreign,
            "{\"type\":\"message\",\"message\":{\"role\":\"user\"}}\n",
        )
        .unwrap();
        let empty = dir.path().join("empty.jsonl");
        fs::write(&empty, "").unwrap();

        for path in [&foreign, &empty] {
            for error in [
                CmdcProvider.parse_session_file(path).unwrap_err(),
                parse_session_meta(path).unwrap_err(),
            ] {
                assert!(format!("{error:#}").contains("missing session header"));
            }
        }
    }

    #[test]
    fn lists_only_transcripts_and_ignores_sidecars() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("repo");
        make_repo(&cwd);

        let listed = with_home(dir.path(), || {
            let project = dir.path().join("projects").join("repo");
            write_transcript(&project, &cwd.to_string_lossy());
            // Sidecars written next to a transcript are not sessions.
            fs::write(
                project.join(format!("{SESSION_ID}.checkpoints.jsonl")),
                r#"{"id":"ebb24442","turnNumber":1,"prompt":"make sivtr support cmdc","files":[]}"#,
            )
            .unwrap();
            fs::write(project.join("config.json"), r#"{"tasteOnboarding":{}}"#).unwrap();
            fs::write(project.join("mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
            CmdcProvider.list_recent_sessions(None).unwrap()
        });

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id.as_deref(), Some(SESSION_ID));
        assert_eq!(
            listed[0].cwd.as_deref(),
            Some(cwd.to_string_lossy().as_ref())
        );
        assert_eq!(listed[0].title.as_deref(), Some("make sivtr support cmdc"));
    }

    #[test]
    fn filters_sessions_by_workspace() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        make_repo(&repo);
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();

        let listed = with_home(dir.path(), || {
            let projects = dir.path().join("projects");
            write_transcript(&projects.join("repo"), &repo.to_string_lossy());
            write_transcript(&projects.join("elsewhere"), &elsewhere.to_string_lossy());
            CmdcProvider.list_recent_sessions(Some(&repo)).unwrap()
        });

        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].cwd.as_deref(),
            Some(repo.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn provider_is_registered_under_cmdc() {
        assert_eq!(AgentProvider::CommandCode.name(), "Command Code");
        assert_eq!(AgentProvider::CommandCode.command_name(), "cmdc");
        assert_eq!(
            AgentProvider::from_command_name("cmdc"),
            Some(AgentProvider::CommandCode)
        );
        assert_eq!(CmdcProvider.provider(), AgentProvider::CommandCode);
    }

    #[test]
    fn home_honours_the_env_override() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COMMANDCODE_HOME");

        std::env::set_var("COMMANDCODE_HOME", dir.path());
        assert_eq!(cmdc_home(), dir.path());
        assert_eq!(projects_root(), dir.path().join("projects"));

        // An empty override means "unset", not a root of "".
        std::env::set_var("COMMANDCODE_HOME", "");
        assert!(cmdc_home().ends_with(".commandcode"));

        match previous {
            Some(value) => std::env::set_var("COMMANDCODE_HOME", value),
            None => std::env::remove_var("COMMANDCODE_HOME"),
        }
    }

    #[test]
    fn turns_become_cmdc_refs() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_transcript(dir.path(), "C:\\repo");

        let session = CmdcProvider.parse_session_file(&path).unwrap();
        let records = crate::record::WorkRecord::chat_turns(AgentProvider::CommandCode, &session);
        assert!(!records.is_empty());
        assert_eq!(records[0].work_ref.to_string(), "cmdc/7f3c1a2e/1");
    }
}
