use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const WORKSPACES_DIR: &str = "workspaces";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub key: String,
    pub root: String,
    pub created_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone)]
pub struct WorkspacePaths {
    pub key: String,
    pub root: PathBuf,
    pub dir: PathBuf,
    pub terminals_dir: PathBuf,
}

pub fn resolve_current_workspace() -> Result<Option<WorkspacePaths>> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    resolve_workspace_for_dir(&cwd)
}

pub fn resolve_workspace_for_dir(cwd: &Path) -> Result<Option<WorkspacePaths>> {
    let Some(root) = git_root(cwd)? else {
        return Ok(None);
    };
    Ok(Some(paths_for_root(root)?))
}

pub fn ensure_current_workspace() -> Result<Option<WorkspacePaths>> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    ensure_workspace_for_dir(&cwd)
}

pub fn ensure_workspace_for_dir(cwd: &Path) -> Result<Option<WorkspacePaths>> {
    let Some(paths) = resolve_workspace_for_dir(cwd)? else {
        return Ok(None);
    };
    ensure_workspace_metadata(&paths)?;
    fs::create_dir_all(&paths.terminals_dir)?;
    Ok(Some(paths))
}

pub fn data_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sivtr")
}

pub fn terminal_id() -> String {
    std::env::var("SIVTR_TERMINAL_ID")
        .ok()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| format!("session_{}", std::process::id()))
}

pub fn current_terminal_log_path() -> Result<Option<PathBuf>> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    terminal_log_path_for_dir(&cwd)
}

pub fn terminal_log_path_for_command_cwd() -> Result<Option<PathBuf>> {
    let cwd = std::env::var("SIVTR_COMMAND_CWD")
        .ok()
        .filter(|cwd| !cwd.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    terminal_log_path_for_dir(&cwd)
}

pub fn terminal_log_path_for_dir(cwd: &Path) -> Result<Option<PathBuf>> {
    let Some(paths) = ensure_workspace_for_dir(cwd)? else {
        return Ok(None);
    };
    Ok(Some(
        paths.terminals_dir.join(format!("{}.jsonl", terminal_id())),
    ))
}

pub fn current_terminal_state_path() -> Result<Option<PathBuf>> {
    Ok(current_terminal_log_path()?.map(|path| path.with_extension("state")))
}

pub fn current_terminal_capture_path() -> Result<Option<PathBuf>> {
    Ok(current_terminal_log_path()?.map(|path| path.with_extension("capture")))
}

pub fn terminal_log_paths_for_workspace(cwd: &Path) -> Result<Vec<PathBuf>> {
    let Some(paths) = resolve_workspace_for_dir(cwd)? else {
        return Ok(Vec::new());
    };
    if !paths.terminals_dir.exists() {
        return Ok(Vec::new());
    }

    let mut logs = Vec::new();
    for entry in fs::read_dir(&paths.terminals_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            logs.push(path);
        }
    }
    logs.sort_by_key(|path| std::cmp::Reverse(modified_time(path)));
    Ok(logs)
}

/// All known workspaces, parsed from `<data_dir>/workspaces/<key>/workspace.json`,
/// most-recently-seen first. An empty result means sivtr has not recorded any
/// workspace yet (e.g. `sivtr init` was never run in a git repo).
pub fn list_workspaces() -> Result<Vec<WorkspaceMetadata>> {
    let dir = data_dir().join(WORKSPACES_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let meta_path = entry.path().join("workspace.json");
        if !meta_path.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&meta_path) else {
            continue;
        };
        if let Ok(meta) = serde_json::from_str::<WorkspaceMetadata>(&text) {
            out.push(meta);
        }
    }
    out.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at));
    Ok(out)
}

/// Outcome of [`migrate_workspace_keys`].
#[derive(Debug, Default)]
pub struct WorkspaceMigration {
    /// `(old_key, new_key)` for each renamed workspace dir.
    pub migrated: Vec<(String, String)>,
    /// Workspaces already on the current key scheme (no rename needed).
    pub current: usize,
    /// Dirs that could not be migrated, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

impl WorkspaceMigration {
    pub fn changed(&self) -> bool {
        !self.migrated.is_empty()
    }
}

/// Re-key workspace dirs whose stored root predates the absolute-based key
/// scheme.
///
/// Legacy `workspace.json` roots were stored via `std::fs::canonicalize`, which
/// prepends a `\\?\` verbatim prefix on Windows. The current scheme derives the
/// key from `std::path::absolute` (no prefix), so legacy dirs no longer match
/// the key a fresh access computes — their captured sessions become unreachable.
/// This strips the legacy prefix, recomputes the key, and renames the dir +
/// rewrites `workspace.json` when they differ. Idempotent: a second run is a
/// no-op.
pub fn migrate_workspace_keys() -> Result<WorkspaceMigration> {
    let base = data_dir().join(WORKSPACES_DIR);
    let mut report = WorkspaceMigration::default();
    if !base.exists() {
        return Ok(report);
    }

    for entry in fs::read_dir(&base)? {
        let Ok(entry) = entry else {
            continue;
        };
        let dir = entry.path();
        let Some(old_key) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let meta_path = dir.join("workspace.json");
        let Some(mut meta) = load_workspace_metadata(&meta_path) else {
            continue;
        };

        // Legacy canonicalize roots carry a `\\?\` (or `\\?\UNC\`) prefix that
        // the current absolute-based scheme does not produce. Strip it to
        // recompute the key the way a fresh access would.
        let cleaned = strip_legacy_verbatim(&meta.root);
        let new_root =
            std::path::absolute(Path::new(&cleaned)).unwrap_or_else(|_| PathBuf::from(&cleaned));
        let new_root_text = new_root.to_string_lossy().to_string();
        let new_key = workspace_key(&new_root_text);

        if new_key == old_key {
            // Key matches; just tidy the root field if it still carries the
            // legacy prefix.
            if meta.root != new_root_text {
                meta.root = new_root_text;
                let _ = write_workspace_metadata(&meta_path, &meta);
            }
            report.current += 1;
            continue;
        }

        let target = base.join(&new_key);
        if target.exists() {
            report
                .skipped
                .push((dir, format!("target key {new_key} already exists")));
            continue;
        }
        match fs::rename(&dir, &target) {
            Ok(()) => {
                meta.key = new_key.clone();
                meta.root = new_root_text;
                let _ = write_workspace_metadata(&target.join("workspace.json"), &meta);
                report.migrated.push((old_key, new_key));
            }
            Err(e) => report.skipped.push((dir, format!("rename failed: {e}"))),
        }
    }
    Ok(report)
}

fn load_workspace_metadata(path: &Path) -> Option<WorkspaceMetadata> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_workspace_metadata(path: &Path, meta: &WorkspaceMetadata) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(meta)?)?;
    Ok(())
}

/// Strip a legacy `\\?\` / `\\?\UNC\` verbatim prefix from a stored root, as
/// written by the old `canonicalize`-based scheme. Only used when reading
/// legacy `workspace.json` data during migration.
fn strip_legacy_verbatim(root: &str) -> String {
    if let Some(rest) = root.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        root.strip_prefix(r"\\?\")
            .map(str::to_string)
            .unwrap_or_else(|| root.to_string())
    }
}

pub fn terminal_session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("terminal")
        .to_string()
}

fn paths_for_root(root: PathBuf) -> Result<WorkspacePaths> {
    // Absolutify without canonicalizing: `std::fs::canonicalize` adds a `\\?\`
    // verbatim prefix on Windows (root cause of ugly displayed paths and keys),
    // and resolves symlinks we don't need. `absolute` makes the path absolute
    // (so a relative `--cwd` still keys stably) without either side effect.
    let root = std::path::absolute(&root).unwrap_or(root);
    let key = workspace_key(&root.to_string_lossy());
    let dir = data_dir().join(WORKSPACES_DIR).join(&key);
    Ok(WorkspacePaths {
        key,
        root,
        terminals_dir: dir.join("terminals"),
        dir,
    })
}

fn ensure_workspace_metadata(paths: &WorkspacePaths) -> Result<()> {
    fs::create_dir_all(&paths.dir)?;
    let path = paths.dir.join("workspace.json");
    if path.exists() {
        return Ok(());
    }

    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let metadata = WorkspaceMetadata {
        key: paths.key.clone(),
        root: paths.root.to_string_lossy().to_string(),
        created_at: now.clone(),
        last_seen_at: now,
    };
    fs::write(path, serde_json::to_string_pretty(&metadata)?)?;
    Ok(())
}

fn git_root(cwd: &Path) -> Result<Option<PathBuf>> {
    let mut dir = if cwd.is_dir() {
        cwd.to_path_buf()
    } else {
        cwd.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cwd.to_path_buf())
    };

    loop {
        if dir.join(".git").exists() {
            return Ok(Some(dir));
        }

        if !dir.pop() {
            return Ok(None);
        }
    }
}

fn workspace_key(root: &str) -> String {
    let normalized = root.replace('\\', "/").to_lowercase();
    let hash = fnv1a64(normalized.as_bytes());
    format!("{hash:016x}")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn modified_time(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::{git_root, terminal_session_id_from_path, workspace_key};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sivtr-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("test dir should be created");
        dir
    }

    #[test]
    fn workspace_key_normalizes_case_and_separators() {
        assert_eq!(workspace_key("D:\\sivtr"), workspace_key("d:/sivtr"));
    }

    #[test]
    fn finds_git_root_by_walking_parents() {
        let root = unique_test_dir("workspace-root");
        std::fs::create_dir(root.join(".git")).expect(".git dir should be created");
        let nested = root.join("crates").join("core");
        std::fs::create_dir_all(&nested).expect("nested dir should be created");

        assert_eq!(
            git_root(&nested).expect("git root should resolve"),
            Some(root.clone())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn treats_git_file_as_workspace_root() {
        let root = unique_test_dir("workspace-git-file");
        std::fs::write(root.join(".git"), "gitdir: ../repo.git")
            .expect(".git file should be written");

        assert_eq!(
            git_root(&root).expect("git root should resolve"),
            Some(root.clone())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_session_id_uses_file_stem() {
        assert_eq!(
            terminal_session_id_from_path(Path::new("session_123.jsonl")),
            "session_123"
        );
    }
}
