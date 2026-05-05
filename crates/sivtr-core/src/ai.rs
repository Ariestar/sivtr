use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentProvider {
    Claude,
    Codex,
}

#[derive(Clone, Copy)]
pub struct AgentProviderSpec {
    pub provider: AgentProvider,
    pub name: &'static str,
    pub command_name: &'static str,
    pub current_transcript_env: Option<&'static str>,
    pub current_session_id_env: Option<&'static str>,
    factory: fn() -> Box<dyn AgentSessionProvider>,
}

const AGENT_PROVIDER_SPECS: &[AgentProviderSpec] = &[
    AgentProviderSpec {
        provider: AgentProvider::Codex,
        name: "Codex",
        command_name: "codex",
        current_transcript_env: None,
        current_session_id_env: Some("CODEX_THREAD_ID"),
        factory: codex_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Claude,
        name: "Claude",
        command_name: "claude",
        current_transcript_env: Some("CLAUDE_TRANSCRIPT_PATH"),
        current_session_id_env: Some("CLAUDE_SESSION_ID"),
        factory: claude_provider,
    },
];

fn codex_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::codex::CodexProvider)
}

fn claude_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::claude::ClaudeProvider)
}

impl AgentProvider {
    pub fn all() -> &'static [AgentProviderSpec] {
        AGENT_PROVIDER_SPECS
    }

    pub fn from_command_name(value: &str) -> Option<Self> {
        Self::all()
            .iter()
            .find(|spec| spec.command_name.eq_ignore_ascii_case(value))
            .map(|spec| spec.provider)
    }

    pub fn spec(self) -> &'static AgentProviderSpec {
        Self::all()
            .iter()
            .find(|spec| spec.provider == self)
            .expect("agent provider registry must contain every AgentProvider variant")
    }

    pub fn name(self) -> &'static str {
        self.spec().name
    }

    pub fn command_name(self) -> &'static str {
        self.spec().command_name
    }

    pub fn current_transcript_env(self) -> Option<&'static str> {
        self.spec().current_transcript_env
    }

    pub fn current_session_id_env(self) -> Option<&'static str> {
        self.spec().current_session_id_env
    }

    pub fn session_provider(self) -> Box<dyn AgentSessionProvider> {
        (self.spec().factory)()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentBlockKind {
    User,
    Assistant,
    ToolCall,
    ToolOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBlock {
    pub kind: AgentBlockKind,
    pub timestamp: Option<String>,
    pub label: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    pub path: PathBuf,
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub blocks: Vec<AgentBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionInfo {
    pub path: PathBuf,
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub modified: SystemTime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentSessionMeta {
    pub id: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSelection {
    LastTurn,
    LastAssistant,
    LastUser,
    LastTool,
    LastBlocks(usize),
    All,
}

pub trait AgentSessionProvider {
    fn provider(&self) -> AgentProvider;

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<AgentSessionInfo>>;

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession>;

    fn find_session_by_id(&self, id: &str) -> Result<Option<PathBuf>> {
        for session in self.list_recent_sessions(None)? {
            if session
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(id))
                || session.id.as_deref() == Some(id)
            {
                return Ok(Some(session.path));
            }
        }

        Ok(None)
    }

    fn find_current_session(&self, cwd: &Path) -> Result<Option<PathBuf>> {
        if let Some(session) = self.list_recent_sessions(Some(cwd))?.into_iter().next() {
            return Ok(Some(session.path));
        }

        Ok(self
            .list_recent_sessions(None)?
            .into_iter()
            .next()
            .map(|session| session.path))
    }
}

pub fn list_recent_jsonl_sessions(
    root: &Path,
    cwd: Option<&Path>,
    parse_meta: impl Fn(&Path) -> Result<AgentSessionMeta>,
) -> Result<Vec<AgentSessionInfo>> {
    let wanted = cwd.map(normalize_path_for_match);
    let mut sessions = Vec::new();

    for path in jsonl_files(root)? {
        let meta = parse_meta(&path)?;
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

pub fn parse_jsonl_session(
    path: &Path,
    provider_name: &str,
    mut apply_event: impl FnMut(&mut AgentSession, &Value),
) -> Result<AgentSession> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read {provider_name} session: {}", path.display()))?;
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
                "Failed to read {provider_name} session line {}: {}",
                idx + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(&line).with_context(|| {
            format!(
                "Failed to parse {provider_name} session line {} as JSON: {}",
                idx + 1,
                path.display()
            )
        })?;
        apply_event(&mut session, &value);
    }

    Ok(session)
}

pub fn parse_jsonl_meta(
    path: &Path,
    provider_name: &str,
    max_lines: usize,
    mut update_meta: impl FnMut(&mut AgentSessionMeta, &Value),
) -> Result<AgentSessionMeta> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to read {provider_name} session: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut meta = AgentSessionMeta::default();

    for (idx, line) in reader.lines().take(max_lines).enumerate() {
        let line = line.with_context(|| {
            format!(
                "Failed to read {provider_name} session metadata line {}: {}",
                idx + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(&line).with_context(|| {
            format!(
                "Failed to parse {provider_name} session metadata as JSON: {}",
                path.display()
            )
        })?;
        update_meta(&mut meta, &value);
        if meta.id.is_some() && meta.cwd.is_some() {
            break;
        }
    }

    Ok(meta)
}

pub fn push_block(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    label: Option<String>,
    text: impl Into<String>,
) {
    let text = text.into().trim().to_string();
    if !text.is_empty() {
        session.blocks.push(AgentBlock {
            kind,
            timestamp,
            label,
            text,
        });
    }
}

pub fn extract_content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("input_text").and_then(Value::as_str))
                    .or_else(|| item.get("output_text").and_then(Value::as_str))
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

pub fn pretty_json_value(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

pub fn pretty_json_string(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| text.to_string())
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

fn normalize_path_for_match(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
}

pub fn select_blocks(session: &AgentSession, selection: AgentSelection) -> Vec<AgentBlock> {
    match selection {
        AgentSelection::LastTurn => select_last_turn(&session.blocks),
        AgentSelection::LastAssistant => {
            select_last_kind(&session.blocks, AgentBlockKind::Assistant)
        }
        AgentSelection::LastUser => select_last_kind(&session.blocks, AgentBlockKind::User),
        AgentSelection::LastTool => select_last_kind(&session.blocks, AgentBlockKind::ToolOutput),
        AgentSelection::LastBlocks(count) => {
            let start = session.blocks.len().saturating_sub(count);
            session.blocks[start..].to_vec()
        }
        AgentSelection::All => session.blocks.clone(),
    }
}

pub fn format_blocks(blocks: &[AgentBlock]) -> String {
    if blocks.len() == 1 {
        return blocks[0].text.trim().to_string();
    }

    blocks
        .iter()
        .filter(|block| !block.text.trim().is_empty())
        .map(format_block_with_heading)
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn select_last_kind(blocks: &[AgentBlock], kind: AgentBlockKind) -> Vec<AgentBlock> {
    blocks
        .iter()
        .rev()
        .find(|block| block.kind == kind)
        .cloned()
        .into_iter()
        .collect()
}

fn select_last_turn(blocks: &[AgentBlock]) -> Vec<AgentBlock> {
    let Some(assistant_idx) = blocks
        .iter()
        .rposition(|block| block.kind == AgentBlockKind::Assistant)
    else {
        return Vec::new();
    };
    let user_idx = blocks[..assistant_idx]
        .iter()
        .rposition(|block| block.kind == AgentBlockKind::User)
        .unwrap_or(assistant_idx);

    blocks[user_idx..=assistant_idx]
        .iter()
        .filter(|block| matches!(block.kind, AgentBlockKind::User | AgentBlockKind::Assistant))
        .cloned()
        .collect()
}

fn format_block_with_heading(block: &AgentBlock) -> String {
    let heading = match block.kind {
        AgentBlockKind::User => "User".to_string(),
        AgentBlockKind::Assistant => "Assistant".to_string(),
        AgentBlockKind::ToolCall => block
            .label
            .as_deref()
            .map(|label| format!("Tool Call: {label}"))
            .unwrap_or_else(|| "Tool Call".to_string()),
        AgentBlockKind::ToolOutput => "Tool Output".to_string(),
    };

    format!("## {heading}\n{}", block.text.trim())
}
