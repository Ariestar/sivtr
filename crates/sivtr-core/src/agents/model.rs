use anyhow::Result;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentProvider {
    Claude,
    Codex,
    Cursor,
    Hermes,
    OpenClaw,
    OpenCode,
    Pi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentBlockKind {
    User,
    Assistant,
    /// Tool invocation (name in `label` when known). MCP tools stay tools.
    ToolCall,
    /// Tool result (name in `label` when known).
    ToolOutput,
    /// Explicit skill invocation / payload (name in `label`).
    Skill,
    /// Model reasoning / thinking channel (not dialogue body).
    Thinking,
}

impl AgentBlockKind {
    pub fn is_dialogue(self) -> bool {
        matches!(self, Self::User | Self::Assistant)
    }

    pub fn is_structure(self) -> bool {
        !self.is_dialogue()
    }

    /// Content-block open tag for structural kinds (`<:tool:bash call:>`, …).
    /// Dialogue returns `None` (plain text, no wrapper).
    pub fn open_marker(self, label: Option<&str>) -> Option<String> {
        let name = normalize_structure_name(label);
        Some(match self {
            Self::User | Self::Assistant => return None,
            Self::ToolCall => format!("<:tool:{name} call:>"),
            Self::ToolOutput => format!("<:tool:{name} result:>"),
            Self::Skill => format!("<:skill:{name}:>"),
            Self::Thinking => "<:thinking:>".to_string(),
        })
    }

    fn close_marker(self, label: Option<&str>) -> Option<String> {
        self.open_marker(label).map(|open| {
            // `<:tag:>` → `<:/tag:>`
            open.replacen("<:", "<:/", 1)
        })
    }

    /// Serialize body for evidence export. Dialogue is plain; structure is marked.
    pub fn format_block(self, label: Option<&str>, text: &str) -> String {
        let text = text.trim();
        match self.open_marker(label) {
            None => text.to_string(),
            Some(open) => {
                let close = self.close_marker(label).unwrap_or_default();
                if text.is_empty() {
                    format!("{open}\n{close}")
                } else {
                    format!("{open}\n{text}\n{close}")
                }
            }
        }
    }
}

fn normalize_structure_name(label: Option<&str>) -> &str {
    label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
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

    pub(crate) fn cwd_candidates(&self) -> impl Iterator<Item = &str> {
        self.cwd_history.iter().map(String::as_str).chain(
            self.cwd
                .as_deref()
                .into_iter()
                .filter(|cwd| !self.cwd_history.iter().any(|existing| existing == cwd)),
        )
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

pub fn normalize_path_for_match(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
}

/// Shared workspace filter for agent session lists.
///
/// Policy (all providers):
/// - `cwd == None` → keep every session
/// - session has no cwd metadata → **keep** (unbound / weixin / cron / missing)
/// - session has cwd → keep only when it matches the target path **or** shares a git remote
pub fn filter_sessions_by_workspace(
    sessions: Vec<AgentSessionInfo>,
    cwd: Option<&Path>,
) -> Vec<AgentSessionInfo> {
    let Some(cwd) = cwd else {
        return sessions;
    };
    let wanted = WorkspaceMatchTarget::new(cwd);
    sessions
        .into_iter()
        .filter(|session| match session.cwd.as_deref() {
            None => true,
            Some(candidate) => wanted.matches(Path::new(candidate)),
        })
        .collect()
}

/// Whether any cwd candidate matches the workspace target.
/// Empty candidate list means "no metadata" → keep (same policy as unbound sessions).
pub(crate) fn workspace_matches_candidates(
    wanted: &WorkspaceMatchTarget,
    mut candidates: impl Iterator<Item = impl AsRef<Path>>,
) -> bool {
    let mut any = false;
    for candidate in candidates.by_ref() {
        any = true;
        if wanted.matches(candidate.as_ref()) {
            return true;
        }
    }
    !any
}

pub(crate) struct WorkspaceMatchTarget {
    normalized_path: String,
    remote_keys: HashSet<String>,
    candidate_remote_keys: RefCell<HashMap<String, HashSet<String>>>,
}

impl WorkspaceMatchTarget {
    pub(crate) fn new(path: &Path) -> Self {
        Self {
            normalized_path: normalize_path_for_match(path),
            remote_keys: git_remote_keys(path),
            candidate_remote_keys: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn matches(&self, candidate: &Path) -> bool {
        let normalized_candidate = normalize_path_for_match(candidate);
        if normalized_candidate == self.normalized_path {
            return true;
        }

        if self.remote_keys.is_empty() {
            return false;
        }

        {
            let cache = self.candidate_remote_keys.borrow();
            if let Some(candidate_keys) = cache.get(&normalized_candidate) {
                return candidate_keys
                    .iter()
                    .any(|candidate_key| self.remote_keys.contains(candidate_key));
            }
        }

        let candidate_keys = git_remote_keys(candidate);
        let matches = candidate_keys
            .iter()
            .any(|candidate_key| self.remote_keys.contains(candidate_key));
        self.candidate_remote_keys
            .borrow_mut()
            .insert(normalized_candidate, candidate_keys);
        matches
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
    let normalized = if let Some((_, rest)) = without_suffix.split_once("://") {
        normalize_remote_authority_path(rest).unwrap_or_else(|| without_suffix.to_string())
    } else if let Some((authority, path)) = split_scp_like_remote(without_suffix) {
        format!(
            "{}/{}",
            authority.rsplit('@').next().unwrap_or(authority),
            path.trim_start_matches('/')
        )
    } else {
        without_suffix.to_string()
    };

    Some(normalized.replace('\\', "/").to_lowercase())
}

fn normalize_remote_authority_path(rest: &str) -> Option<String> {
    let (authority, path) = rest.split_once('/')?;
    let host = authority.rsplit('@').next()?.trim();
    let path = path.trim_start_matches('/').trim();
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{host}/{path}"))
}

fn split_scp_like_remote(remote: &str) -> Option<(&str, &str)> {
    let (authority, path) = remote.split_once(':')?;
    if !authority.contains('@') || path.trim().is_empty() {
        return None;
    }
    Some((authority, path))
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
        // User + tools only (interrupted before assistant): keep the trailing structure.
        let Some(user_idx) = blocks
            .iter()
            .rposition(|block| block.kind == AgentBlockKind::User)
        else {
            return Vec::new();
        };
        return blocks[user_idx..].to_vec();
    };
    let user_idx = blocks[..assistant_idx]
        .iter()
        .rposition(|block| block.kind == AgentBlockKind::User)
        .unwrap_or(assistant_idx);

    // Keep the full turn: user, tools/skills/thinking/mcp, assistant — never strip structure.
    blocks[user_idx..=assistant_idx].to_vec()
}

fn format_block_with_heading(block: &AgentBlock, text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() && block.kind.is_dialogue() {
        return None;
    }
    Some(format_structured_block(
        block.kind,
        block.label.as_deref(),
        text,
    ))
}

/// Serialize a block for human/machine-readable evidence (not Markdown dialogue headings).
///
/// Dialogue stays plain. Structural channels use content-block markers:
/// `<:tool:bash call:>` … `<:tool:bash result:>`, `<:skill:name:>`, `<:thinking:>`.
pub fn format_structured_block(kind: AgentBlockKind, label: Option<&str>, text: &str) -> String {
    kind.format_block(label, text)
}

pub fn is_dialogue_block(kind: AgentBlockKind) -> bool {
    kind.is_dialogue()
}

pub fn is_structure_block(kind: AgentBlockKind) -> bool {
    kind.is_structure()
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
    fn normalizes_generic_git_remote_url_forms() {
        assert_eq!(
            normalize_remote_url("https://gitlab.example.com/team/sivtr.git").as_deref(),
            Some("gitlab.example.com/team/sivtr")
        );
        assert_eq!(
            normalize_remote_url("git@gitlab.example.com:team/sivtr.git").as_deref(),
            Some("gitlab.example.com/team/sivtr")
        );
        assert_eq!(
            normalize_remote_url("ssh://git@gitlab.example.com:2222/team/sivtr.git").as_deref(),
            Some("gitlab.example.com:2222/team/sivtr")
        );
    }

    #[test]
    fn cwd_candidates_do_not_duplicate_the_primary_cwd() {
        let mut tracked = AgentSessionMeta::default();
        tracked.add_cwd("/repo");
        tracked.add_cwd("/repo/subdir");
        assert_eq!(
            tracked.cwd_candidates().collect::<Vec<_>>(),
            vec!["/repo", "/repo/subdir"]
        );

        let fallback = AgentSessionMeta {
            cwd: Some("/repo".to_string()),
            ..AgentSessionMeta::default()
        };
        assert_eq!(fallback.cwd_candidates().collect::<Vec<_>>(), vec!["/repo"]);
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
    fn filter_sessions_keeps_unbound_and_matching_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        write_git_remote(&repo, "origin", "https://github.com/Ariestar/sivtr.git");

        let sessions = vec![
            AgentSessionInfo {
                path: PathBuf::from("unbound"),
                id: Some("u".into()),
                cwd: None,
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            },
            AgentSessionInfo {
                path: PathBuf::from("match"),
                id: Some("m".into()),
                cwd: Some(repo.to_string_lossy().into_owned()),
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            },
            AgentSessionInfo {
                path: PathBuf::from("other"),
                id: Some("o".into()),
                cwd: Some(dir.path().join("other").to_string_lossy().into_owned()),
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            },
        ];

        let filtered = filter_sessions_by_workspace(sessions, Some(&repo));
        let ids: Vec<_> = filtered
            .iter()
            .filter_map(|session| session.id.as_deref())
            .collect();
        assert_eq!(ids, vec!["u", "m"]);
    }
}
