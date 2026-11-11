//! Declared transcript formats for the providers without a dedicated module.
//!
//! Every provider here names one current on-disk layout in [`Layout`] and one
//! root list in [`profile_for`]. A provider whose format this build cannot
//! declare is `None`: it stays registered for launching agents, but it has no
//! native source, so nothing guesses its way into the archive.

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    extract_content_text, filter_sessions_by_workspace, open_readonly_db, pretty_json_value,
    push_block, push_tool_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionProvider,
    SessionInfo, SessionListing,
};

/// The transcript layout a provider stores today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// One markdown history file holding several `# aider chat started at` runs.
    AiderMarkdown,
    /// `wire.jsonl` events: `turn.prompt` / `content.part` / `tool.call` / `tool.result`.
    KimiWire,
    /// `<id>/base_state.json` plus ordered `events/*.json` event files.
    OpenHandsEvents,
    /// `<id>/history_item.json` plus `ui_messages.json` ask/say items.
    RooCodeHistory,
    /// `*.ndjson` events: `session.input` / `assistant_message.end` / `tool_call.*`.
    PoolsideEvents,
    /// `*.jsonl` events with `kind: Prompt | AssistantMessage | ToolResults`.
    KiroEvents,
    /// `*.json` session documents with `requests[].message` / `requests[].response`.
    VSCodeSession,
    /// Codex-compatible rollout JSONL.
    CodexRollout,
    /// `threads.db` rows holding one JSON (optionally zstd) conversation document.
    ZedThreads,
    /// VS Code `state.vscdb` `ItemTable` rows keyed by chat/session.
    WindsurfState,
}

/// How a provider's sessions are discovered under one root. Directories and
/// multi-run files are the exception; everything else is one file per session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discovery {
    /// Every candidate file is one session.
    File,
    /// Every candidate directory is one session.
    Directory,
    /// One physical file holds several runs, each its own session.
    Runs,
}

#[derive(Debug, Clone, Copy)]
pub struct GenericProfile {
    pub provider: AgentProvider,
    /// Override root variable, read before `default_roots`.
    pub env: Option<&'static str>,
    pub default_roots: &'static [&'static str],
    pub layout: Layout,
    pub discovery: Discovery,
    /// Which file names under a root are session leaves. Unused for
    /// directory discovery.
    pub leaf: Option<Leaf>,
}

/// Which file names under a root are session leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaf {
    /// `.jsonl` only.
    Jsonl,
    /// `.ndjson` only.
    Ndjson,
    /// `.json`, `.jsonl`, or `.ndjson` inside a chat/session storage path.
    ChatStorage,
    /// One named file, wherever it appears under the root.
    Named(&'static str),
    /// `.jsonl` named `wire.jsonl` (Kimi's turn log).
    WireLog,
}

impl Leaf {
    fn matches(self, path: &Path, is_dir: bool) -> bool {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        match self {
            Self::Jsonl => !is_dir && extension.eq_ignore_ascii_case("jsonl"),
            Self::Ndjson => !is_dir && extension.eq_ignore_ascii_case("ndjson"),
            Self::ChatStorage => {
                !is_dir
                    && (extension.eq_ignore_ascii_case("json")
                        || extension.eq_ignore_ascii_case("jsonl"))
                    && path.components().any(|component| {
                        matches!(
                            component
                                .as_os_str()
                                .to_string_lossy()
                                .to_ascii_lowercase()
                                .as_str(),
                            "chatsessions" | "session-state"
                        )
                    })
            }
            Self::Named(wanted) => !is_dir && lower == wanted,
            Self::WireLog => !is_dir && lower == "wire.jsonl",
        }
    }
}

/// Configuration files that are never transcripts, wherever they appear.
const CONFIGURATION_FILE_NAMES: &[&str] = &[
    "config.json",
    "settings.json",
    "package.json",
    "package-lock.json",
    "tsconfig.json",
];

/// One provider's declared current format, or `None` when this build cannot
/// declare it. `None` providers never sync.
pub fn profile_for(provider: AgentProvider) -> Option<&'static GenericProfile> {
    GENERIC_PROFILES
        .iter()
        .find(|profile| profile.provider == provider)
}

pub fn is_supported(provider: AgentProvider) -> bool {
    profile_for(provider).is_some()
}

const GENERIC_PROFILES: &[GenericProfile] = &[
    GenericProfile {
        provider: AgentProvider::Aider,
        env: Some("AIDER_DIR"),
        default_roots: &[],
        layout: Layout::AiderMarkdown,
        discovery: Discovery::Runs,
        leaf: Some(Leaf::Named(".aider.chat.history.md")),
    },
    GenericProfile {
        provider: AgentProvider::Kimi,
        env: Some("KIMI_DIR"),
        default_roots: &[".kimi/sessions", ".kimi-code/sessions"],
        layout: Layout::KimiWire,
        discovery: Discovery::File,
        leaf: Some(Leaf::WireLog),
    },
    GenericProfile {
        provider: AgentProvider::KimiWork,
        env: Some("KIMI_WORK_DIR"),
        default_roots: &[
            "Library/Application Support/kimi-desktop/daimon-share/daimon/runtime/kimi-code/home/sessions",
            ".config/kimi-desktop/daimon-share/daimon/runtime/kimi-code/home/sessions",
            ".local/share/kimi-desktop/daimon-share/daimon/runtime/kimi-code/home/sessions",
            "AppData/Roaming/kimi-desktop/daimon-share/daimon/runtime/kimi-code/home/sessions",
        ],
        layout: Layout::KimiWire,
        discovery: Discovery::File,
        leaf: Some(Leaf::WireLog),
    },
    GenericProfile {
        provider: AgentProvider::Kiro,
        env: Some("KIRO_SESSIONS_DIR"),
        default_roots: &[".kiro/sessions", ".local/share/kiro-cli"],
        layout: Layout::KiroEvents,
        discovery: Discovery::File,
        leaf: Some(Leaf::Jsonl),
    },
    GenericProfile {
        provider: AgentProvider::OpenHands,
        env: Some("OPENHANDS_CONVERSATIONS_DIR"),
        default_roots: &[".openhands/conversations"],
        layout: Layout::OpenHandsEvents,
        discovery: Discovery::Directory,
        leaf: None,
    },
    GenericProfile {
        provider: AgentProvider::Poolside,
        env: Some("POOLSIDE_DIR"),
        default_roots: &[
            "Library/Application Support/poolside",
            ".local/state/poolside",
            "AppData/Roaming/poolside",
        ],
        layout: Layout::PoolsideEvents,
        discovery: Discovery::File,
        leaf: Some(Leaf::Ndjson),
    },
    GenericProfile {
        provider: AgentProvider::RooCode,
        env: Some("ROOCODE_DIR"),
        default_roots: &[
            "Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline",
            ".config/Code/User/globalStorage/rooveterinaryinc.roo-cline",
            "AppData/Roaming/Code/User/globalStorage/rooveterinaryinc.roo-cline",
        ],
        layout: Layout::RooCodeHistory,
        discovery: Discovery::Directory,
        leaf: None,
    },
    GenericProfile {
        provider: AgentProvider::Trae,
        env: Some("TRAE_DIR"),
        default_roots: &[
            "AppData/Roaming/Trae/User",
            "AppData/Roaming/Trae CN/User",
            "Library/Application Support/Trae/User",
            ".config/Trae/User",
        ],
        layout: Layout::VSCodeSession,
        discovery: Discovery::File,
        leaf: Some(Leaf::ChatStorage),
    },
    GenericProfile {
        provider: AgentProvider::TraeX,
        env: Some("TRAEX_SESSIONS_DIR"),
        default_roots: &[".trae/cli/sessions", ".trae/cli/archived_sessions"],
        layout: Layout::CodexRollout,
        discovery: Discovery::File,
        leaf: Some(Leaf::Jsonl),
    },
    GenericProfile {
        provider: AgentProvider::VSCodeCopilot,
        env: Some("VSCODE_COPILOT_DIR"),
        default_roots: &[
            "AppData/Roaming/Code/User",
            "AppData/Roaming/Code - Insiders/User",
            "Library/Application Support/Code/User",
            ".config/Code/User",
        ],
        layout: Layout::VSCodeSession,
        discovery: Discovery::File,
        leaf: Some(Leaf::ChatStorage),
    },
    GenericProfile {
        provider: AgentProvider::Windsurf,
        env: Some("WINDSURF_DIR"),
        default_roots: &[
            "AppData/Roaming/Windsurf/User",
            "Library/Application Support/Windsurf/User",
            ".config/Windsurf/User",
        ],
        layout: Layout::WindsurfState,
        discovery: Discovery::File,
        leaf: Some(Leaf::Named("state.vscdb")),
    },
    GenericProfile {
        provider: AgentProvider::Zed,
        env: Some("ZED_DIR"),
        default_roots: &[
            "Library/Application Support/Zed",
            ".config/zed",
            "AppData/Roaming/Zed",
        ],
        layout: Layout::ZedThreads,
        discovery: Discovery::File,
        leaf: Some(Leaf::Named("threads.db")),
    },
];

fn directory_layout_matches(provider: AgentProvider, path: &Path) -> bool {
    match provider {
        AgentProvider::OpenHands => {
            path.join("base_state.json").is_file() && path.join("events").is_dir()
        }
        AgentProvider::RooCode => path.join("history_item.json").is_file(),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GenericProvider {
    provider: AgentProvider,
}

impl GenericProvider {
    pub const fn new(provider: AgentProvider) -> Self {
        Self { provider }
    }

    fn profile(self) -> Result<&'static GenericProfile> {
        profile_for(self.provider).with_context(|| {
            format!(
                "{} has no declared transcript format in this build",
                self.provider.name()
            )
        })
    }

    fn roots(self) -> Result<Vec<PathBuf>> {
        let profile = self.profile()?;
        if let Some(name) = profile.env {
            if let Some(value) = std::env::var_os(name) {
                if value.is_empty() {
                    anyhow::bail!("{name} must not be empty");
                }
                return Ok(vec![PathBuf::from(value)]);
            }
        }
        if profile.discovery == Discovery::Runs {
            // Aider keeps its history in the project it runs in.
            return Ok(vec![std::env::current_dir()
                .context("failed to determine current directory for Aider")?]);
        }
        let home = dirs::home_dir().context("failed to determine home directory")?;
        Ok(profile
            .default_roots
            .iter()
            .map(|relative| home.join(relative))
            .collect())
    }

    fn candidates(self) -> Result<Vec<PathBuf>> {
        let profile = self.profile()?;
        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        for root in self.roots()? {
            collect_candidates(profile, &root, &mut paths, &mut seen)?;
        }
        paths.sort();
        Ok(paths)
    }
}

impl AgentSessionProvider for GenericProvider {
    fn provider(&self) -> AgentProvider {
        self.provider
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        Ok(self.list_session_outcome(cwd)?.0)
    }

    fn list_session_outcome(&self, cwd: Option<&Path>) -> Result<SessionListing> {
        crate::agents::profile::list(self.provider, cwd)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        crate::agents::profile::parse(self.provider, path)
    }
}

pub(crate) fn list_provider(provider: AgentProvider, cwd: Option<&Path>) -> Result<SessionListing> {
    if !is_supported(provider) {
        return Ok((Vec::new(), Vec::new()));
    }
    let source = GenericProvider::new(provider);
    let profile = source.profile()?;
    let mut sessions = Vec::new();
    let mut failures = Vec::new();
    for path in source.candidates()? {
        // One unreadable file must not drop the provider's whole listing:
        // report it so the sync engine keeps the archived copy.
        if let Err(error) = source.list_candidate(profile, &path, &mut sessions) {
            failures.push((path, format!("{error:#}")));
        }
    }
    sessions.sort_by_key(|session| session.modified);
    sessions.reverse();
    Ok((filter_sessions_by_workspace(sessions, cwd), failures))
}

pub(crate) fn parse_provider(provider: AgentProvider, path: &Path) -> Result<AgentSession> {
    let profile = GenericProvider::new(provider).profile()?;
    match profile.layout {
        Layout::AiderMarkdown => parse_aider(path),
        Layout::KimiWire => parse_kimi(path),
        Layout::OpenHandsEvents => parse_openhands(path),
        Layout::RooCodeHistory => parse_roocode(path),
        Layout::PoolsideEvents => parse_poolside(path),
        Layout::KiroEvents => parse_kiro(path),
        Layout::VSCodeSession => parse_vscode_session(path),
        Layout::CodexRollout => crate::agents::codex::parse_file(AgentProvider::Codex, path),
        Layout::ZedThreads => parse_zed(path),
        Layout::WindsurfState => parse_windsurf(path),
    }
}

impl GenericProvider {
    fn list_candidate(
        self,
        profile: &GenericProfile,
        path: &Path,
        sessions: &mut Vec<SessionInfo>,
    ) -> Result<()> {
        match profile.discovery {
            Discovery::Runs => sessions.extend(list_aider_sessions(path)?),
            Discovery::Directory => {
                let session = parse_provider(self.provider, path)?;
                sessions.push(session_info(path, session, true)?);
            }
            Discovery::File => match profile.layout {
                Layout::ZedThreads => sessions.extend(list_zed_sessions(path)?),
                Layout::WindsurfState => sessions.extend(list_windsurf_sessions(path)?),
                _ => {
                    let session = parse_provider(self.provider, path)?;
                    sessions.push(session_info(path, session, false)?);
                }
            },
        }
        Ok(())
    }
}

fn session_info(path: &Path, session: AgentSession, directory: bool) -> Result<SessionInfo> {
    let modified = fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read mtime {}", path.display()))?;
    Ok(SessionInfo {
        modified,
        path: path.to_path_buf(),
        physical_path: directory.then(|| path.to_path_buf()),
        id: session.id,
        cwd: session.cwd,
        title: session.title,
    })
}

fn collect_candidates(
    profile: &GenericProfile,
    path: &Path,
    output: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()))
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        // Installed packages are never transcripts, and editor storage roots
        // hold whole dependency trees of unrelated extensions.
        if path.file_name().is_some_and(|name| name == "node_modules") {
            return Ok(());
        }
        if directory_layout_matches(profile.provider, path) {
            if seen.insert(path.to_path_buf()) {
                output.push(path.to_path_buf());
            }
            return Ok(());
        }
        for entry in
            fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))?
        {
            collect_candidates(profile, &entry?.path(), output, seen)?;
        }
        return Ok(());
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if CONFIGURATION_FILE_NAMES.contains(&name.as_str()) {
        return Ok(());
    }
    let is_leaf = profile.leaf.is_some_and(|leaf| leaf.matches(path, false));
    if is_leaf && seen.insert(path.to_path_buf()) {
        output.push(path.to_path_buf());
    }
    Ok(())
}

fn parse_aider(path: &Path) -> Result<AgentSession> {
    let (physical, selected) = split_aider_virtual_path(path);
    let text = fs::read_to_string(&physical)
        .with_context(|| format!("failed to read Aider history {}", physical.display()))?;
    let runs = split_aider_runs(&text);
    let selected_runs = match selected {
        Some(index) => vec![runs
            .get(index)
            .with_context(|| format!("Aider run {index} is not present in {}", physical.display()))?
            .clone()],
        None => runs,
    };
    let mut session = empty_session(path);
    session.id = Some(path_id(path));
    session.cwd = physical.parent().map(|parent| parent.display().to_string());
    for run in selected_runs {
        parse_aider_run_body(&mut session, &run.header, &run.body);
    }
    Ok(session)
}

#[derive(Clone)]
struct AiderRun {
    header: String,
    body: String,
}

fn split_aider_runs(text: &str) -> Vec<AiderRun> {
    let mut runs = Vec::new();
    let mut current: Option<AiderRun> = None;
    for raw in text.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(header) = line.strip_prefix("# aider chat started at ") {
            if let Some(run) = current.take() {
                runs.push(run);
            }
            current = Some(AiderRun {
                header: header.trim().to_string(),
                body: String::new(),
            });
        } else if let Some(run) = current.as_mut() {
            run.body.push_str(line);
            run.body.push('\n');
        }
    }
    if let Some(run) = current {
        runs.push(run);
    }
    runs
}

fn list_aider_sessions(path: &Path) -> Result<Vec<SessionInfo>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read Aider history {}", path.display()))?;
    let modified = fs::metadata(path)
        .with_context(|| format!("failed to stat Aider history {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read mtime {}", path.display()))?;
    let mut sessions = Vec::new();
    for (index, run) in split_aider_runs(&text).into_iter().enumerate() {
        let virtual_path = aider_virtual_path(path, index);
        let mut parsed = empty_session(&virtual_path);
        parsed.id = Some(path_id(&virtual_path));
        parsed.cwd = path.parent().map(|parent| parent.display().to_string());
        parse_aider_run_body(&mut parsed, &run.header, &run.body);
        parsed.title = title_from_blocks(&parsed);
        sessions.push(SessionInfo {
            modified,
            path: virtual_path,
            physical_path: Some(path.to_path_buf()),
            id: parsed.id,
            cwd: parsed.cwd,
            title: parsed.title,
        });
    }
    Ok(sessions)
}

fn parse_aider_run_body(session: &mut AgentSession, header: &str, body: &str) {
    let mut current: Option<AgentBlockKind> = None;
    let mut content = String::new();
    let timestamp = aider_timestamp(header);
    let flush =
        |session: &mut AgentSession, current: &mut Option<AgentBlockKind>, content: &mut String| {
            if let Some(kind) = *current {
                push_block(session, kind, timestamp.clone(), None, content.as_str());
            }
            *current = None;
            content.clear();
        };

    for line in body.lines() {
        let (kind, line) = if let Some(line) = line.strip_prefix("####") {
            (AgentBlockKind::User, line.trim_start())
        } else if let Some(line) = line.strip_prefix("> ") {
            (AgentBlockKind::ToolOutput, line)
        } else {
            (AgentBlockKind::Assistant, line)
        };
        if current != Some(kind) && !content.trim().is_empty() {
            flush(session, &mut current, &mut content);
        }
        current = Some(kind);
        content.push_str(line);
        content.push('\n');
    }
    flush(session, &mut current, &mut content);
}

fn aider_virtual_path(path: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}#{index}", path.display()))
}

fn split_aider_virtual_path(path: &Path) -> (PathBuf, Option<usize>) {
    let text = path.to_string_lossy();
    let Some((physical, index)) = text.rsplit_once('#') else {
        return (path.to_path_buf(), None);
    };
    let Ok(index) = index.parse::<usize>() else {
        return (path.to_path_buf(), None);
    };
    let physical = PathBuf::from(physical);
    if physical.is_file() {
        (physical, Some(index))
    } else {
        (path.to_path_buf(), None)
    }
}

fn aider_timestamp(value: &str) -> Option<String> {
    chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|time| {
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(time, chrono::Utc)
                .to_rfc3339()
        })
}

/// `wire.jsonl`: one turn event per line, each carrying its own channel.
fn parse_kimi(path: &Path) -> Result<AgentSession> {
    let mut session = empty_session(path);
    session.id = Some(kimi_id(path));
    for value in read_json_values(path)? {
        update_identity(&mut session, &value);
        let timestamp = json_timestamp(&value);
        match value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "turn.prompt" => push_text(
                &mut session,
                AgentBlockKind::User,
                timestamp,
                value.get("content").or_else(|| value.get("prompt")),
            ),
            "content.part" | "ContentPart" => push_text(
                &mut session,
                AgentBlockKind::Assistant,
                timestamp,
                value.get("content").or_else(|| value.get("text")),
            ),
            "tool.call" | "ToolCall" => push_declared_tool(
                &mut session,
                AgentBlockKind::ToolCall,
                timestamp,
                value.get("tool").or_else(|| value.get("call")),
            ),
            "tool.result" | "ToolResult" => push_declared_tool(
                &mut session,
                AgentBlockKind::ToolOutput,
                timestamp,
                value.get("result").or_else(|| value.get("tool")),
            ),
            // Loop events are wrappers around a declared event.
            "context.append_loop_event" => {
                if let Some(event) = value.get("event") {
                    apply_kimi_event(&mut session, event);
                }
            }
            _ => {}
        }
    }
    finish_session(&mut session);
    Ok(session)
}

fn apply_kimi_event(session: &mut AgentSession, event: &Value) {
    let timestamp = json_timestamp(event);
    match event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "turn.prompt" => push_text(
            session,
            AgentBlockKind::User,
            timestamp,
            event.get("content"),
        ),
        "content.part" | "ContentPart" => push_text(
            session,
            AgentBlockKind::Assistant,
            timestamp,
            event.get("content"),
        ),
        "tool.call" | "ToolCall" => push_declared_tool(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            event.get("tool"),
        ),
        "tool.result" | "ToolResult" => push_declared_tool(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            event.get("result"),
        ),
        _ => {}
    }
}

fn kimi_id(path: &Path) -> String {
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    let grand = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    match (grand, parent) {
        (Some(grand), Some(parent)) => sanitize_id(&format!("{grand}-{parent}")),
        (None, Some(parent)) => sanitize_id(parent),
        _ => path_id(path),
    }
}

/// `<conversation>/base_state.json` plus ordered `events/*.json`.
fn parse_openhands(path: &Path) -> Result<AgentSession> {
    let mut session = empty_session(path);
    let base = path.join("base_state.json");
    let value: Value = serde_json::from_str(
        &fs::read_to_string(&base).with_context(|| format!("failed to read {}", base.display()))?,
    )
    .with_context(|| format!("failed to parse {}", base.display()))?;
    update_identity(&mut session, &value);
    session.id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| Some(path_id(path)));

    let events = path.join("events");
    let mut files = fs::read_dir(&events)
        .with_context(|| format!("failed to read {}", events.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort();
    for event_path in files {
        if event_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let value: Value = serde_json::from_str(
            &fs::read_to_string(&event_path)
                .with_context(|| format!("failed to read {}", event_path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", event_path.display()))?;
        apply_openhands_event(&mut session, &value);
    }
    finish_session(&mut session);
    Ok(session)
}

fn apply_openhands_event(session: &mut AgentSession, value: &Value) {
    let timestamp = json_timestamp(value);
    match value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "MessageEvent" => {
            let kind = match value
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("assistant")
            {
                "user" => AgentBlockKind::User,
                "assistant" | "agent" => AgentBlockKind::Assistant,
                _ => return,
            };
            push_text(
                session,
                kind,
                timestamp,
                value.get("message").or_else(|| value.get("content")),
            );
        }
        "ActionEvent" => push_declared_tool(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            value.get("action"),
        ),
        "ObservationEvent" => push_declared_tool(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            value.get("observation"),
        ),
        _ => {}
    }
}

/// `<task>/history_item.json` metadata plus `ui_messages.json` ask/say items.
fn parse_roocode(path: &Path) -> Result<AgentSession> {
    let mut session = empty_session(path);
    let history = path.join("history_item.json");
    let metadata: Value = serde_json::from_str(
        &fs::read_to_string(&history)
            .with_context(|| format!("failed to read {}", history.display()))?,
    )
    .with_context(|| format!("failed to parse {}", history.display()))?;
    session.id = metadata
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        });
    session.cwd = metadata
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::to_string);
    session.title = metadata
        .get("task")
        .and_then(Value::as_str)
        .map(str::to_string);

    let messages = path.join("ui_messages.json");
    if !messages.is_file() {
        finish_session(&mut session);
        return Ok(session);
    }
    let value: Value = serde_json::from_str(
        &fs::read_to_string(&messages)
            .with_context(|| format!("failed to read {}", messages.display()))?,
    )
    .with_context(|| format!("failed to parse {}", messages.display()))?;
    let items = value
        .as_array()
        .context("RooCode ui_messages.json must be an array")?;
    for item in items {
        let kind = match item.get("type").and_then(Value::as_str) {
            Some("ask") => AgentBlockKind::User,
            Some("say") => AgentBlockKind::Assistant,
            _ => continue,
        };
        push_text(
            &mut session,
            kind,
            json_timestamp(item),
            item.get("text")
                .or_else(|| item.get("ask"))
                .or_else(|| item.get("say")),
        );
        if let Some(reasoning) = item.get("reasoning") {
            push_block(
                &mut session,
                AgentBlockKind::Thinking,
                json_timestamp(item),
                None,
                extract_content_text(reasoning),
            );
        }
    }
    finish_session(&mut session);
    Ok(session)
}

/// `*.ndjson` events: `session.input`, `assistant_message.end`, `thought.end`,
/// `tool_call.parsed`, `tool_call.result`.
fn parse_poolside(path: &Path) -> Result<AgentSession> {
    let mut session = empty_session(path);
    for value in read_json_values(path)? {
        update_identity(&mut session, &value);
        let timestamp = json_timestamp(&value);
        match value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "session.input" => push_text(
                &mut session,
                AgentBlockKind::User,
                timestamp,
                value.pointer("/session_input/prompt"),
            ),
            "assistant_message.end" => push_text(
                &mut session,
                AgentBlockKind::Assistant,
                timestamp,
                value.pointer("/assistant_message_end/assistant_message"),
            ),
            "thought.end" => push_block(
                &mut session,
                AgentBlockKind::Thinking,
                timestamp,
                None,
                value
                    .pointer("/thought_end/thought")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            "tool_call.parsed" => push_declared_tool(
                &mut session,
                AgentBlockKind::ToolCall,
                timestamp,
                value.get("tool_call_parsed"),
            ),
            "tool_call.result" => push_declared_tool(
                &mut session,
                AgentBlockKind::ToolOutput,
                timestamp,
                value.get("tool_call_result"),
            ),
            _ => {}
        }
    }
    finish_session(&mut session);
    Ok(session)
}

/// `*.jsonl` events with `kind: Prompt | AssistantMessage | ToolResults`.
fn parse_kiro(path: &Path) -> Result<AgentSession> {
    let mut session = empty_session(path);
    for value in read_json_values(path)? {
        update_identity(&mut session, &value);
        let timestamp = json_timestamp(&value);
        let data = value.get("data").or_else(|| value.get("content"));
        match value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "Prompt" => push_text(&mut session, AgentBlockKind::User, timestamp, data),
            "AssistantMessage" => {
                push_text(&mut session, AgentBlockKind::Assistant, timestamp, data)
            }
            "ToolResults" => push_text(&mut session, AgentBlockKind::ToolOutput, timestamp, data),
            _ => {}
        }
    }
    finish_session(&mut session);
    Ok(session)
}

/// `*.json` session documents with `requests[].message` / `requests[].response`.
fn parse_vscode_session(path: &Path) -> Result<AgentSession> {
    let mut session = empty_session(path);
    for value in read_json_values(path)? {
        update_identity(&mut session, &value);
        let Some(requests) = value.get("requests").and_then(Value::as_array) else {
            continue;
        };
        for request in requests {
            let timestamp = json_timestamp(request);
            if let Some(message) = request.get("message") {
                push_text(
                    &mut session,
                    AgentBlockKind::User,
                    timestamp.clone(),
                    message.get("text").or_else(|| message.get("content")),
                );
            }
            if let Some(response) = request.get("response") {
                push_vscode_response(&mut session, timestamp, response);
            }
        }
    }
    finish_session(&mut session);
    Ok(session)
}

fn push_vscode_response(session: &mut AgentSession, timestamp: Option<String>, response: &Value) {
    let Some(items) = response.as_array() else {
        push_text(
            session,
            AgentBlockKind::Assistant,
            timestamp,
            Some(response),
        );
        return;
    };
    for item in items {
        if let Some(text) = item.get("value").and_then(Value::as_str) {
            push_block(
                session,
                AgentBlockKind::Assistant,
                timestamp.clone(),
                None,
                text,
            );
        } else if item.get("toolSpecificData").is_some() {
            push_declared_tool(
                session,
                AgentBlockKind::ToolCall,
                timestamp.clone(),
                Some(item),
            );
        } else {
            push_text(
                session,
                AgentBlockKind::Assistant,
                timestamp.clone(),
                Some(item),
            );
        }
    }
}

/// `threads.db`: one row per thread holding a JSON (optionally zstd) document.
fn list_zed_sessions(path: &Path) -> Result<Vec<SessionInfo>> {
    let conn = open_readonly_db(path)?;
    let modified = file_modified(path)?;
    let mut stmt =
        conn.prepare("SELECT CAST(id AS TEXT), CAST(summary AS TEXT) FROM threads ORDER BY rowid")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut sessions = Vec::new();
    for (id, title) in rows {
        let Some(id) = id.map(|id| addressable_session_id(&id)) else {
            continue;
        };
        sessions.push(SessionInfo {
            modified,
            path: sqlite_virtual_path(path, &id),
            physical_path: Some(path.to_path_buf()),
            id: Some(id),
            cwd: None,
            title: title.filter(|title| !title.trim().is_empty()),
        });
    }
    Ok(sessions)
}

fn parse_zed(path: &Path) -> Result<AgentSession> {
    let (physical, only_session) = split_sqlite_virtual_path(path);
    let conn = open_readonly_db(&physical)?;
    let mut session = empty_session(path);
    session.id = only_session.or_else(|| Some(path_id(&physical)));
    let mut stmt = conn.prepare(
        "SELECT CAST(data_type AS TEXT), CAST(data AS BLOB) FROM threads
         WHERE (?1 IS NULL OR CAST(id AS TEXT) = ?1) ORDER BY rowid",
    )?;
    let rows = stmt.query_map(rusqlite::params![session.id], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<Vec<u8>>>(1)?,
        ))
    })?;
    for row in rows {
        let (data_type, Some(bytes)) = row? else {
            continue;
        };
        let bytes = decode_sqlite_payload(&bytes, data_type.as_deref().unwrap_or(""))?;
        let value: Value =
            serde_json::from_slice(&bytes).context("Zed thread data is not valid JSON")?;
        apply_thread_document(&mut session, &value);
    }
    finish_session(&mut session);
    Ok(session)
}

/// Consecutive `{"User": …}` / `{"Agent": …}` entries in a thread document.
fn apply_thread_document(session: &mut AgentSession, value: &Value) {
    let Some(messages) = value.get("messages").and_then(Value::as_array) else {
        return;
    };
    for message in messages {
        if let Some(user) = message.get("User") {
            push_text(
                session,
                AgentBlockKind::User,
                None,
                Some(user.get("content").unwrap_or(user)),
            );
        } else if let Some(agent) = message.get("Agent") {
            push_text(
                session,
                AgentBlockKind::Assistant,
                None,
                Some(agent.get("content").unwrap_or(agent)),
            );
        }
    }
}

/// VS Code `state.vscdb`: `ItemTable` rows keyed by chat/session.
fn list_windsurf_sessions(path: &Path) -> Result<Vec<SessionInfo>> {
    let conn = open_readonly_db(path)?;
    let modified = file_modified(path)?;
    let mut stmt = conn.prepare(
        "SELECT key, CAST(value AS TEXT) FROM ItemTable
         WHERE value IS NOT NULL ORDER BY key",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut sessions = Vec::new();
    let mut seen = HashSet::new();
    for (key, value) in rows {
        if !is_session_key(&key) {
            continue;
        }
        let Ok(document) = serde_json::from_str::<Value>(&value) else {
            continue;
        };
        for id in document_session_ids(&document) {
            let id = addressable_session_id(&id);
            if !seen.insert(id.clone()) {
                continue;
            }
            sessions.push(SessionInfo {
                modified,
                path: sqlite_virtual_path(path, &id),
                physical_path: Some(path.to_path_buf()),
                id: Some(id),
                cwd: None,
                title: None,
            });
        }
    }
    Ok(sessions)
}

fn parse_windsurf(path: &Path) -> Result<AgentSession> {
    let (physical, only_session) = split_sqlite_virtual_path(path);
    let conn = open_readonly_db(&physical)?;
    let mut session = empty_session(path);
    session.id = only_session.or_else(|| Some(path_id(&physical)));
    let mut stmt = conn.prepare(
        "SELECT key, CAST(value AS TEXT) FROM ItemTable
         WHERE value IS NOT NULL ORDER BY key",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (key, value) in rows {
        if !is_session_key(&key) {
            continue;
        }
        let document: Value = serde_json::from_str(&value)
            .with_context(|| format!("ItemTable value `{key}` is not JSON"))?;
        if let Some(only_session) = session.id.as_deref() {
            let ids = document_session_ids(&document);
            if !ids.is_empty() && !ids.iter().any(|id| id == only_session) {
                continue;
            }
        }
        apply_thread_document(&mut session, &document);
    }
    finish_session(&mut session);
    Ok(session)
}

fn is_session_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("chat") || lower.contains("session")
}

fn document_session_ids(value: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    collect_keyed_ids(value, &mut ids);
    ids
}

fn collect_keyed_ids(value: &Value, ids: &mut Vec<String>) {
    if let Some(id) = value.get("sessionId").and_then(Value::as_str) {
        ids.push(id.to_string());
    }
    match value {
        Value::Object(map) => map.values().for_each(|value| collect_keyed_ids(value, ids)),
        Value::Array(items) => items.iter().for_each(|value| collect_keyed_ids(value, ids)),
        _ => {}
    }
}

fn decode_sqlite_payload(bytes: &[u8], data_type: &str) -> Result<Vec<u8>> {
    if data_type.eq_ignore_ascii_case("zstd") {
        use ruzstd::decoding::StreamingDecoder;
        let mut decoder = StreamingDecoder::new(std::io::Cursor::new(bytes))
            .context("failed to open zstd thread payload")?;
        let mut output = Vec::new();
        decoder
            .read_to_end(&mut output)
            .context("failed to decode zstd thread payload")?;
        return Ok(output);
    }
    Ok(bytes.to_vec())
}

fn read_json_values(path: &Path) -> Result<Vec<Value>> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("json") {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read JSON transcript {}", path.display()))?;
        return Ok(vec![serde_json::from_str(&text).with_context(|| {
            format!("failed to parse JSON transcript {}", path.display())
        })?]);
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed to read transcript {}", path.display()))?;
    let mut values = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line =
            line.with_context(|| format!("failed to read {} line {}", path.display(), index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(value) => values.push(value),
            // A writer can leave the last line unfinished.
            Err(error) if index > 0 && error.classify() == serde_json::error::Category::Eof => {
                break
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to parse {} line {}", path.display(), index + 1)
                })
            }
        }
    }
    Ok(values)
}

fn empty_session(path: &Path) -> AgentSession {
    AgentSession {
        path: path.to_path_buf(),
        id: None,
        cwd: None,
        title: None,
        blocks: Vec::new(),
    }
}

fn finish_session(session: &mut AgentSession) {
    if session.id.is_none() {
        session.id = session
            .path
            .file_stem()
            .and_then(|name| name.to_str())
            .map(str::to_string);
    }
    if session.title.is_none() {
        session.title = title_from_blocks(session);
    }
}

fn title_from_blocks(session: &AgentSession) -> Option<String> {
    let first_user = session
        .blocks
        .iter()
        .find(|block| block.kind == AgentBlockKind::User)
        .map(|block| block.text.as_str());
    let mut meta = crate::agents::AgentSessionMeta::default();
    meta.fallback_title(first_user);
    meta.title
}

/// Session identity, cwd, and title from the fields these formats declare.
fn update_identity(session: &mut AgentSession, value: &Value) {
    if session.id.is_none() {
        session.id = value
            .get("sessionId")
            .or_else(|| value.get("session_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string);
    }
    if session.cwd.is_none() {
        session.cwd = value
            .get("cwd")
            .or_else(|| value.get("workspace"))
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.trim().is_empty())
            .map(str::to_string);
    }
    if session.title.is_none() {
        session.title = value
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string);
    }
}

fn push_text(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    content: Option<&Value>,
) {
    let Some(content) = content else {
        return;
    };
    push_block(
        session,
        kind,
        timestamp,
        None,
        extract_content_text(content),
    );
}

fn push_declared_tool(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    tool: Option<&Value>,
) {
    let Some(tool) = tool else {
        return;
    };
    let label = tool
        .get("name")
        .or_else(|| tool.get("tool"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let call_id = tool
        .get("id")
        .or_else(|| tool.get("call_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    // Payloads are structured arguments or results, so a field with no text
    // renders as JSON rather than vanishing.
    let body = tool
        .get("input")
        .or_else(|| tool.get("args"))
        .or_else(|| tool.get("output"))
        .or_else(|| tool.get("content"))
        .map(text_or_json)
        .unwrap_or_else(|| text_or_json(tool));
    push_tool_block(session, kind, timestamp, call_id, label, body, None);
}

fn text_or_json(value: &Value) -> String {
    let text = extract_content_text(value);
    if text.trim().is_empty() {
        pretty_json_value(value)
    } else {
        text
    }
}

fn json_timestamp(value: &Value) -> Option<String> {
    let field = value
        .get("timestamp")
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("createdAt"))
        .or_else(|| value.get("time"))?;
    field
        .as_str()
        .map(str::to_string)
        .or_else(|| field.as_i64().map(|number| number.to_string()))
}

fn file_modified(path: &Path) -> Result<SystemTime> {
    fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read mtime {}", path.display()))
}

fn addressable_session_id(raw: &str) -> String {
    if raw
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        raw.to_string()
    } else {
        sanitize_id(raw)
    }
}

fn sqlite_virtual_path(physical: &Path, session_id: &str) -> PathBuf {
    PathBuf::from(format!("{}#{session_id}", physical.display()))
}

fn split_sqlite_virtual_path(path: &Path) -> (PathBuf, Option<String>) {
    let text = path.to_string_lossy();
    let Some((physical, session_id)) = text.rsplit_once('#') else {
        return (path.to_path_buf(), None);
    };
    let physical = PathBuf::from(physical);
    if physical.is_file() {
        (physical, Some(session_id.to_string()))
    } else {
        (path.to_path_buf(), None)
    }
}

fn path_id(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sanitize_id(value: &str) -> String {
    let id: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if id.is_empty() {
        "session".to_string()
    } else {
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aider_runs_without_collapsing_channels() {
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            path.path(),
            "# aider chat started at 2026-08-31 12:00:00\n#### fix bug\n\nassistant answer\n> cargo test\n",
        )
        .unwrap();
        let session = parse_aider(path.path()).unwrap();
        assert_eq!(session.blocks.len(), 3);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
    }

    #[test]
    fn aider_container_exposes_each_run_with_one_physical_stamp() {
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            path.path(),
            "# aider chat started at 2026-08-31 12:00:00\n#### first\nanswer\n# aider chat started at 2026-08-31 13:00:00\n#### second\nanswer\n",
        )
        .unwrap();
        let sessions = list_aider_sessions(path.path()).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|session| {
            session.physical_path.as_deref() == Some(path.path())
                && session.path.to_string_lossy().contains('#')
        }));
    }

    #[test]
    fn kimi_wire_log_keeps_prompt_part_and_tool_channels() {
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            path.path(),
            concat!(
                "{\"type\":\"turn.prompt\",\"content\":\"question\"}\n",
                "{\"type\":\"content.part\",\"content\":\"answer\"}\n",
                "{\"type\":\"tool.call\",\"tool\":{\"name\":\"shell\",\"args\":{\"command\":\"ls\"}}}\n",
                "{\"type\":\"tool.result\",\"result\":{\"content\":\"a.rs\"}}\n",
            ),
        )
        .unwrap();
        let session = parse_kimi(path.path()).unwrap();
        let kinds: Vec<_> = session.blocks.iter().map(|block| block.kind).collect();
        assert_eq!(
            kinds,
            [
                AgentBlockKind::User,
                AgentBlockKind::Assistant,
                AgentBlockKind::ToolCall,
                AgentBlockKind::ToolOutput
            ]
        );
        assert_eq!(session.blocks[2].label.as_deref(), Some("shell"));
    }

    #[test]
    fn declared_formats_cover_the_kept_catalog() {
        assert!(profile_for(AgentProvider::Zed).is_some());
        assert_eq!(
            profile_for(AgentProvider::TraeX).map(|profile| profile.layout),
            Some(Layout::CodexRollout)
        );
        // Undeclared formats stay registered but never sync.
        assert!(!is_supported(AgentProvider::Amp));
        assert_eq!(
            GenericProvider::new(AgentProvider::Amp)
                .list_recent_sessions(None)
                .unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn zed_threads_expose_each_row_as_a_logical_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                summary TEXT,
                data_type TEXT,
                data BLOB
            )",
        )
        .unwrap();
        for (id, text) in [("thread-a", "first"), ("thread-b", "second")] {
            let document = serde_json::json!({
                "messages": [
                    {"User": {"content": {"text": format!("prompt {text}")}}},
                    {"Agent": {"content": {"text": format!("answer {text}")}}}
                ]
            });
            conn.execute(
                "INSERT INTO threads (id, summary, data_type, data) VALUES (?1, ?2, 'json', ?3)",
                rusqlite::params![id, text, document.to_string()],
            )
            .unwrap();
        }
        drop(conn);

        let sessions = list_zed_sessions(&path).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|session| {
            session.physical_path.as_deref() == Some(path.as_path())
                && session.path.to_string_lossy().contains('#')
        }));
        for session in sessions {
            let parsed = parse_zed(&session.path).unwrap();
            assert_eq!(parsed.id, session.id);
            assert_eq!(parsed.blocks.len(), 2);
        }
    }

    #[test]
    fn discovery_skips_node_modules_and_plain_configuration_files() {
        let _guard = crate::test_fixtures::EnvGuard::capture(&[
            "VSCODE_COPILOT_DIR",
            "OPENHANDS_CONVERSATIONS_DIR",
        ]);
        let dir = tempfile::tempdir().expect("test tempdir");
        let write = |relative: &str, text: &str| {
            let path = dir.path().join(relative);
            std::fs::create_dir_all(path.parent().expect("fixture path has a parent"))
                .expect("create fixture directory");
            std::fs::write(&path, text).expect("write fixture file");
            path
        };
        let session =
            r#"{"requests":[{"message":{"text":"question"},"response":[{"value":"answer"}]}]}"#;
        // JSON with comments, as tool configuration files commonly are.
        let jsonc = "// settings\n{\n  \"strict\": true,\n}\n";

        let chat = write("code/workspaceStorage/ws1/chatSessions/chat.json", session);
        write(
            "code/globalStorage/some.extension/node_modules/call-bind/tsconfig.json",
            jsonc,
        );
        write(
            "code/globalStorage/some.extension/node_modules/psl/data/rules.json",
            "[]",
        );
        write("code/globalStorage/some.extension/package.json", "{}");
        write("code/globalStorage/some.extension/settings.json", "{}");
        std::env::set_var("VSCODE_COPILOT_DIR", dir.path().join("code"));
        assert_eq!(
            GenericProvider::new(AgentProvider::VSCodeCopilot)
                .candidates()
                .expect("list VS Code candidates"),
            vec![chat]
        );

        // A conversation directory is claimed whole, never walked into.
        let conversation = dir.path().join("conversations/conv-1");
        std::fs::create_dir_all(conversation.join("events")).unwrap();
        std::fs::write(conversation.join("base_state.json"), "{\"id\":\"conv-1\"}").unwrap();
        std::fs::write(
            conversation.join("events/event-00001.json"),
            r#"{"kind":"MessageEvent","source":"user","message":"question"}"#,
        )
        .unwrap();
        std::env::set_var(
            "OPENHANDS_CONVERSATIONS_DIR",
            dir.path().join("conversations"),
        );
        assert_eq!(
            GenericProvider::new(AgentProvider::OpenHands)
                .candidates()
                .expect("list OpenHands conversations"),
            vec![conversation.clone()]
        );
        let parsed = GenericProvider::new(AgentProvider::OpenHands)
            .parse_session_file(&conversation)
            .expect("parse OpenHands conversation");
        assert_eq!(parsed.id.as_deref(), Some("conv-1"));
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].kind, AgentBlockKind::User);
    }
}
