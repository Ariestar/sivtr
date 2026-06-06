use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentProvider {
    Claude,
    Codex,
    Hermes,
    OpenCode,
    Pi,
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
    AgentProviderSpec {
        provider: AgentProvider::OpenCode,
        name: "OpenCode",
        command_name: "opencode",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: opencode_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Hermes,
        name: "Hermes",
        command_name: "hermes",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: hermes_provider,
    },
    AgentProviderSpec {
        provider: AgentProvider::Pi,
        name: "Pi",
        command_name: "pi",
        current_transcript_env: None,
        current_session_id_env: None,
        factory: pi_provider,
    },
];

fn codex_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::codex::CodexProvider)
}

fn claude_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::claude::ClaudeProvider)
}

fn opencode_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::opencode::OpenCodeProvider::default())
}

fn hermes_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::hermes::HermesProvider)
}

fn pi_provider() -> Box<dyn AgentSessionProvider> {
    Box::new(crate::pi::PiProvider)
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
    pub title: Option<String>,
    pub blocks: Vec<AgentBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionInfo {
    pub path: PathBuf,
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub modified: SystemTime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentSessionMeta {
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub cwd_history: Vec<String>,
    pub title: Option<String>,
}

impl AgentSessionMeta {
    pub fn add_cwd(&mut self, cwd: impl Into<String>) {
        let cwd = cwd.into();
        if cwd.trim().is_empty() {
            return;
        }
        if self.cwd.is_none() {
            self.cwd = Some(cwd.clone());
        }
        if !self.cwd_history.iter().any(|existing| existing == &cwd) {
            self.cwd_history.push(cwd);
        }
    }

    fn cwd_candidates(&self) -> impl Iterator<Item = &str> {
        self.cwd
            .as_deref()
            .into_iter()
            .chain(self.cwd_history.iter().map(String::as_str))
    }
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

impl AgentSelection {
    pub fn label(self) -> &'static str {
        match self {
            Self::LastTurn => "turn",
            Self::LastAssistant => "assistant",
            Self::LastUser => "user",
            Self::LastTool => "tool",
            Self::LastBlocks(_) => "blocks",
            Self::All => "all",
        }
    }
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
    let wanted = cwd.map(WorkspaceMatchTarget::new);
    let mut sessions = Vec::new();

    for path in jsonl_files(root)? {
        let meta = parse_meta(&path)?;
        if let Some(wanted) = wanted.as_ref() {
            if !meta
                .cwd_candidates()
                .any(|candidate| wanted.matches(Path::new(candidate)))
            {
                continue;
            }
        }

        sessions.push(AgentSessionInfo {
            modified: modified_time(&path).unwrap_or(SystemTime::UNIX_EPOCH),
            path,
            id: meta.id,
            cwd: meta.cwd,
            title: meta.title,
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
        title: None,
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

        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) if idx > 0 && is_trailing_partial_json_line(&error) => break,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to parse {provider_name} session line {} as JSON: {}",
                        idx + 1,
                        path.display()
                    )
                });
            }
        };
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
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| object.get("input_text").and_then(Value::as_str))
            .or_else(|| object.get("output_text").and_then(Value::as_str))
            .or_else(|| object.get("content").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
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

pub fn jsonl_files(root: &Path) -> Result<Vec<PathBuf>> {
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

pub fn normalize_path_for_match(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
}

struct WorkspaceMatchTarget {
    normalized_path: String,
    remote_keys: HashSet<String>,
}

impl WorkspaceMatchTarget {
    fn new(path: &Path) -> Self {
        Self {
            normalized_path: normalize_path_for_match(path),
            remote_keys: git_remote_keys(path),
        }
    }

    fn matches(&self, candidate: &Path) -> bool {
        if normalize_path_for_match(candidate) == self.normalized_path {
            return true;
        }

        let candidate_keys = git_remote_keys(candidate);
        !self.remote_keys.is_empty()
            && candidate_keys
                .iter()
                .any(|candidate_key| self.remote_keys.contains(candidate_key))
    }
}

fn git_remote_keys(path: &Path) -> HashSet<String> {
    let Some(root) = git_root(path) else {
        return HashSet::new();
    };
    let Some(config_path) = git_config_path(&root) else {
        return HashSet::new();
    };
    parse_git_remote_keys(&config_path)
}

fn git_root(path: &Path) -> Option<PathBuf> {
    let mut dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };

    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn git_config_path(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git.join("config"));
    }

    let gitdir = fs::read_to_string(&dot_git).ok()?;
    let relative = gitdir.trim().strip_prefix("gitdir:")?.trim();
    let git_dir = resolve_gitdir(root, relative);
    Some(git_dir.join("config"))
}

fn resolve_gitdir(root: &Path, gitdir: &str) -> PathBuf {
    let path = PathBuf::from(gitdir);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn parse_git_remote_keys(config_path: &Path) -> HashSet<String> {
    fs::read_to_string(config_path)
        .ok()
        .map(|config| {
            config
                .lines()
                .filter_map(remote_key_from_config_line)
                .collect()
        })
        .unwrap_or_default()
}

fn remote_key_from_config_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let url = trimmed.strip_prefix("url")?.trim_start();
    let url = url.strip_prefix('=')?.trim();
    normalize_remote_url(url)
}

fn normalize_remote_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let without_suffix = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let normalized = if let Some(path) = without_suffix.strip_prefix("git@github.com:") {
        format!("github.com/{path}")
    } else if let Some(path) = without_suffix.strip_prefix("ssh://git@github.com/") {
        format!("github.com/{path}")
    } else if let Some(path) = without_suffix.strip_prefix("https://github.com/") {
        format!("github.com/{path}")
    } else if let Some(path) = without_suffix.strip_prefix("http://github.com/") {
        format!("github.com/{path}")
    } else {
        without_suffix.to_string()
    };

    Some(normalized.replace('\\', "/").to_lowercase())
}

fn is_trailing_partial_json_line(error: &serde_json::Error) -> bool {
    matches!(error.classify(), serde_json::error::Category::Eof)
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
    format_blocks_with_text(blocks, |block| block.text.trim().to_string())
}

pub fn format_blocks_with_text(
    blocks: &[AgentBlock],
    text_for_block: impl Fn(&AgentBlock) -> String,
) -> String {
    if blocks.len() == 1 {
        return text_for_block(&blocks[0]).trim().to_string();
    }

    blocks
        .iter()
        .filter_map(|block| format_block_with_heading(block, &text_for_block(block)))
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

fn format_block_with_heading(block: &AgentBlock, text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

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

    Some(format!("## {heading}\n{text}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_git_remote(repo: &Path, name: &str, url: &str) {
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git").join("config"),
            format!("[remote \"{name}\"]\n\turl = {url}\n"),
        )
        .unwrap();
    }

    #[test]
    fn normalizes_common_github_remote_url_forms() {
        assert_eq!(
            normalize_remote_url("https://github.com/Ariestar/sivtr.git").as_deref(),
            Some("github.com/ariestar/sivtr")
        );
        assert_eq!(
            normalize_remote_url("git@github.com:Ariestar/sivtr.git").as_deref(),
            Some("github.com/ariestar/sivtr")
        );
        assert_eq!(
            normalize_remote_url("ssh://git@github.com/Ariestar/sivtr.git/").as_deref(),
            Some("github.com/ariestar/sivtr")
        );
    }

    #[test]
    fn matches_repositories_with_shared_remote() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("oh-my-ppt-fork");
        let candidate = dir.path().join("oh-my-ppt");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        write_git_remote(
            &target,
            "upstream",
            "https://github.com/arcsin1/oh-my-ppt.git",
        );
        write_git_remote(&candidate, "origin", "git@github.com:arcsin1/oh-my-ppt.git");

        assert!(WorkspaceMatchTarget::new(&target).matches(&candidate));
    }

    #[test]
    fn does_not_match_unrelated_repositories() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("oh-my-ppt-fork");
        let candidate = dir.path().join("sivtr");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        write_git_remote(
            &target,
            "upstream",
            "https://github.com/arcsin1/oh-my-ppt.git",
        );
        write_git_remote(
            &candidate,
            "origin",
            "https://github.com/Ariestar/sivtr.git",
        );

        assert!(!WorkspaceMatchTarget::new(&target).matches(&candidate));
    }

    #[test]
    fn includes_sessions_with_later_matching_cwd_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let target = dir.path().join("oh-my-ppt-fork");
        let candidate = dir.path().join("oh-my-ppt");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        write_git_remote(
            &target,
            "upstream",
            "https://github.com/arcsin1/oh-my-ppt.git",
        );
        write_git_remote(
            &candidate,
            "origin",
            "https://github.com/arcsin1/oh-my-ppt.git",
        );
        let transcript = sessions.join("session.jsonl");
        let first_event = serde_json::json!({
            "sessionId": "abc",
            "cwd": dir.path(),
            "customTitle": "Initial",
        });
        let second_event = serde_json::json!({
            "sessionId": "abc",
            "cwd": candidate,
        });
        fs::write(&transcript, format!("{first_event}\n{second_event}\n")).unwrap();

        let sessions = list_recent_jsonl_sessions(&sessions, Some(&target), |path| {
            parse_jsonl_meta(path, "Claude", 50, |meta, value| {
                if meta.id.is_none() {
                    meta.id = value
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                    meta.add_cwd(cwd);
                }
                if meta.title.is_none() {
                    meta.title = value
                        .get("customTitle")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
            })
        })
        .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_deref(), Some("abc"));
        assert_eq!(
            sessions[0].cwd.as_deref(),
            Some(dir.path().to_str().unwrap())
        );
    }
}
