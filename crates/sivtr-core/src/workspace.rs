use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const WORKSPACES_DIR: &str = "workspaces";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub key: String,
    pub root: String,
    /// Persisted origin alias (`sivtr origin rename`); `None` = derive from
    /// the root basename. Auto-assigned unique on first sight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
}

fn path_basename(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|segment| !segment.is_empty())
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
    if let Some(path) = std::env::var_os("SIVTR_DATA_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
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
    let (key, display_root) = workspace_identity(&root);
    let dir = data_dir().join(WORKSPACES_DIR).join(&key);
    Ok(WorkspacePaths {
        key,
        root: display_root,
        terminals_dir: dir.join("terminals"),
        dir,
    })
}

/// The identity a checkout resolves to today: its workspace key and display
/// root, both derived from the shared git dir (commondir), so every worktree
/// of one repository resolves to the same key. Migration re-keys stored roots
/// with the same function so a migrated workspace matches a fresh lookup.
fn workspace_identity(root: &Path) -> (String, PathBuf) {
    let common = repo_common_dir(root).unwrap_or_else(|| root.to_path_buf());
    let key = workspace_key(&normalize_repo(&common));
    // Display/query root = the main checkout (commondir's parent when it is the
    // `.git` dir), giving every worktree one stable name; fall back to the
    // checkout itself for unusual layouts (bare dirs, submodules, …).
    let display_root = if common.file_name().and_then(|name| name.to_str()) == Some(".git") {
        common
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf())
    } else {
        root.to_path_buf()
    };
    (key, display_root)
}

/// The shared git directory for a checkout: the `.git` dir itself for a normal
/// repository, or the commondir of a linked worktree. All worktrees of one repo
/// share one common dir, which is what makes repo identity stable across them.
fn repo_common_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(real_path(&dot_git));
    }
    // Linked worktree: `.git` is a file (`gitdir: <path>`), and the shared
    // config/objects/refs live in `<gitdir>/commondir` (main repo's `.git`).
    let gitdir_line = fs::read_to_string(&dot_git).ok()?;
    let gitdir = resolve_gitdir(root, gitdir_line.trim().strip_prefix("gitdir:")?.trim());
    let common = fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .map(|text| resolve_gitdir(&gitdir, text.trim()))
        .unwrap_or_else(|| gitdir.clone());
    Some(real_path(&common))
}

/// The real, filesystem-normalized directory. `fs::canonicalize` resolves the
/// `..` segments and symlinks in git's relative `gitdir:`/`commondir`
/// pointers, and the `\\?\` verbatim prefix it adds on Windows is stripped so
/// keys and displayed roots stay clean. Falls back to the lexical absolute
/// path when the directory no longer exists.
fn real_path(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .map(|canonical| {
            let text = canonical.to_string_lossy();
            let cleaned = match text.strip_prefix(r"\\?\UNC\") {
                Some(rest) => format!(r"\\{rest}"),
                None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
            };
            PathBuf::from(cleaned)
        })
        .unwrap_or_else(|_| absolutize(path))
}

/// Workspace identity of any path: the canonical commondir of the repository it
/// belongs to, lowercased with `/` separators. `None` when the path is not
/// inside any git checkout. Main repo, worktrees, and nested subdirectories of
/// one repository all yield the same identity.
pub(crate) fn repo_identity(path: &Path) -> Option<String> {
    let root = git_root(path).ok().flatten()?;
    repo_common_dir(&root).map(|common| normalize_repo(&common))
}

fn normalize_repo(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn absolutize(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_gitdir(base: &Path, gitdir: &str) -> PathBuf {
    let path = PathBuf::from(gitdir);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

/// Human origin label for a workspace: the persisted alias when set,
/// otherwise the root basename (lowercased), falling back to the key. Accepts
/// both `/` and `\` so Windows roots still yield a useful label on Unix.
pub fn workspace_alias(meta: &WorkspaceMetadata) -> String {
    if let Some(alias) = meta
        .alias
        .as_deref()
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
    {
        return alias.to_string();
    }
    path_basename(&meta.root)
        .unwrap_or(meta.key.as_str())
        .to_ascii_lowercase()
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
        alias: assign_unique_alias(paths),
        created_at: now.clone(),
        last_seen_at: now,
    };
    fs::write(path, serde_json::to_string_pretty(&metadata)?)?;
    Ok(())
}

/// First-seen alias for a new workspace: the root basename, or `basename-2`,
/// `basename-3`, … when that name is already taken by another workspace.
fn assign_unique_alias(paths: &WorkspacePaths) -> Option<String> {
    let base = paths
        .root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)?;
    let taken: HashSet<String> = list_workspaces()
        .ok()
        .into_iter()
        .flatten()
        .map(|meta| workspace_alias(&meta))
        .collect();
    if !taken.contains(&base) {
        return Some(base);
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.contains(candidate))
}

/// Set a workspace's origin alias. Callers own name validation (uniqueness,
/// non-empty); this just persists the alias on the workspace owning `root`.
pub fn rename_workspace(root: &str, new_alias: &str) -> Result<WorkspaceMetadata> {
    let mut updated = list_workspaces()?
        .into_iter()
        .find(|meta| meta.root == root)
        .with_context(|| format!("no workspace with root `{root}`"))?;
    updated.alias = Some(new_alias.to_string());
    let meta_path = data_dir()
        .join(WORKSPACES_DIR)
        .join(&updated.key)
        .join("workspace.json");
    write_workspace_metadata(&meta_path, &updated)?;
    Ok(updated)
}

fn write_workspace_metadata(path: &Path, meta: &WorkspaceMetadata) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(meta)?)?;
    Ok(())
}

/// Outcome of [`inspect_workspace_keys`] / [`migrate_workspace_keys`].
#[derive(Debug, Default)]
pub struct WorkspaceMigration {
    /// `(old_key, new_key)` dirs re-keyed (or needing one).
    pub migrated: Vec<(String, String)>,
    /// `(old_key, new_key)` legacy dirs merged into the current key's dir.
    pub merged: Vec<(String, String)>,
    /// Dirs that could not be migrated, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
    /// Workspaces already on the current (commondir) key scheme.
    pub current: usize,
}

/// Dry-run: report which workspace dirs need re-keying without renaming.
pub fn inspect_workspace_keys() -> Result<WorkspaceMigration> {
    scan_workspace_keys(false)
}

/// Re-key workspace dirs to the commondir scheme (run via `sivtr doctor --fix`).
///
/// Keys were derived from the checkout root; since the commondir change every
/// checkout of one repository resolves to the shared git dir instead, so
/// stored roots must be re-keyed or their captured sessions become
/// unreachable. Worktrees and the main checkout of one repo converge on one
/// key, merging terminal logs. Idempotent: a second run is a no-op.
pub fn migrate_workspace_keys() -> Result<WorkspaceMigration> {
    scan_workspace_keys(true)
}

fn scan_workspace_keys(apply: bool) -> Result<WorkspaceMigration> {
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
        let (new_key, display_root) = workspace_identity(Path::new(&meta.root));
        if new_key == old_key {
            // Heal metadata left stale by a failed post-rename write: the dir
            // already carries the current key, so only the fields need repair.
            if apply && (meta.key != new_key || meta.root != display_root.to_string_lossy()) {
                meta.key = new_key;
                meta.root = display_root.to_string_lossy().to_string();
                if let Err(e) = write_workspace_metadata(&meta_path, &meta) {
                    report
                        .skipped
                        .push((dir, format!("failed to rewrite workspace.json: {e}")));
                    continue;
                }
            }
            report.current += 1;
            continue;
        }
        let target = base.join(&new_key);
        if target.exists() {
            if apply {
                match merge_workspace_dir(&dir, &target) {
                    Ok(()) => report.merged.push((old_key, new_key)),
                    Err(e) => report
                        .skipped
                        .push((dir, format!("failed to merge into {new_key}: {e}"))),
                }
            } else {
                report.merged.push((old_key, new_key));
            }
            continue;
        }
        if !apply {
            report.migrated.push((old_key, new_key));
            continue;
        }
        match fs::rename(&dir, &target) {
            Ok(()) => {
                meta.key = new_key.clone();
                meta.root = display_root.to_string_lossy().to_string();
                match write_workspace_metadata(&target.join("workspace.json"), &meta) {
                    Ok(()) => report.migrated.push((old_key, new_key)),
                    Err(e) => report.skipped.push((
                        target,
                        format!("renamed to {new_key} but workspace.json update failed: {e}"),
                    )),
                }
            }
            Err(e) => report.skipped.push((dir, format!("rename failed: {e}"))),
        }
    }
    Ok(report)
}

/// Merge a legacy workspace dir into the current-key dir: copy terminal logs
/// the target lacks (and the source alias when the target has none), then
/// remove the legacy dir.
fn merge_workspace_dir(legacy: &Path, current: &Path) -> Result<()> {
    let legacy_terminals = legacy.join("terminals");
    let current_terminals = current.join("terminals");
    if legacy_terminals.is_dir() {
        fs::create_dir_all(&current_terminals)?;
        for entry in fs::read_dir(&legacy_terminals)? {
            let entry = entry?;
            let src = entry.path();
            if !src.is_file() {
                continue;
            }
            let dest = current_terminals.join(entry.file_name());
            if !dest.exists() {
                fs::copy(&src, &dest).with_context(|| {
                    format!("failed to copy {} -> {}", src.display(), dest.display())
                })?;
            }
        }
    }
    let current_meta_path = current.join("workspace.json");
    if let (Some(current_meta), Some(legacy_meta)) = (
        load_workspace_metadata(&current_meta_path),
        load_workspace_metadata(&legacy.join("workspace.json")),
    ) {
        if current_meta.alias.is_none() && legacy_meta.alias.is_some() {
            let mut updated = current_meta;
            updated.alias = legacy_meta.alias;
            write_workspace_metadata(&current_meta_path, &updated)?;
        }
    }
    fs::remove_dir_all(legacy)
        .with_context(|| format!("failed to remove legacy workspace {}", legacy.display()))?;
    Ok(())
}

fn load_workspace_metadata(path: &Path) -> Option<WorkspaceMetadata> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
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
    use super::{
        git_root, inspect_workspace_keys, migrate_workspace_keys, paths_for_root, real_path,
        rename_workspace, repo_identity, terminal_session_id_from_path, workspace_alias,
        workspace_identity, workspace_key, WorkspaceMetadata,
    };
    use crate::test_fixtures::{make_repo, make_worktree};
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
    fn real_path_normalizes_relative_commondir() {
        // A worktree's `commondir` is relative to its gitdir and walks back
        // to the main `.git`; `real_path` must collapse it to the same real
        // directory on every platform (Windows `\\?\` prefix and short names
        // included).
        let dir = unique_test_dir("real-path");
        let repo = dir.join("repo");
        let gitdir = repo.join(".git").join("worktrees").join("stack");
        std::fs::create_dir_all(&gitdir).expect("gitdir should be created");
        assert_eq!(
            real_path(&gitdir.join("../..")),
            real_path(&repo.join(".git"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn worktree_and_main_share_repo_identity() {
        let dir = unique_test_dir("repo-identity");
        let main = dir.join("sivtr");
        let worktree = dir.join("sivtr-tui-stack");
        make_repo(&main);
        make_worktree(&main, &worktree, "sivtr-tui-stack");

        let main_identity = repo_identity(&main).expect("main repo identity");
        let worktree_identity = repo_identity(&worktree).expect("worktree identity");
        let subdir_identity =
            repo_identity(&main.join("crates").join("core")).expect("subdir identity");
        assert_eq!(main_identity, worktree_identity);
        assert_eq!(main_identity, subdir_identity);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn different_repos_have_different_identity() {
        let dir = unique_test_dir("repo-identity-diff");
        let first = dir.join("sivtr");
        let second = dir.join("md-dragger");
        make_repo(&first);
        make_repo(&second);

        let first_identity = repo_identity(&first).expect("first identity");
        let second_identity = repo_identity(&second).expect("second identity");
        assert_ne!(first_identity, second_identity);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn worktree_and_main_share_workspace_key() {
        let dir = unique_test_dir("workspace-key");
        let main = dir.join("sivtr");
        let worktree = dir.join("sivtr-tui-stack");
        make_repo(&main);
        make_worktree(&main, &worktree, "sivtr-tui-stack");

        let main_paths = paths_for_root(main.clone()).expect("main paths");
        let worktree_paths = paths_for_root(worktree.clone()).expect("worktree paths");
        assert_eq!(main_paths.key, worktree_paths.key);
        // Display root collapses to the main checkout for both (compared
        // through the real path: `canonicalize` resolves short names on
        // Windows).
        let main_root = real_path(&main);
        assert_eq!(main_paths.root, main_root);
        assert_eq!(worktree_paths.root, main_root);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn non_repo_path_has_no_identity() {
        let dir = unique_test_dir("repo-identity-none");
        assert_eq!(repo_identity(&dir), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migrate_rekeys_root_based_keys_and_merges_worktree_logs() {
        let _lock = crate::test_env_lock();
        let data = unique_test_dir("workspace-migrate");
        let old_data = std::env::var_os("SIVTR_DATA_DIR");
        std::env::set_var("SIVTR_DATA_DIR", &data);

        let main = unique_test_dir("repo-main");
        let worktree = unique_test_dir("repo-worktree");
        make_repo(&main);
        make_worktree(&main, &worktree, "wt");

        let new_key = workspace_identity(&main).0;
        let main_old_key = workspace_key(&main.to_string_lossy());
        let wt_old_key = workspace_key(&worktree.to_string_lossy());
        assert_ne!(new_key, main_old_key);
        assert_ne!(new_key, wt_old_key);

        let workspaces = data.join("workspaces");
        let main_dir = workspaces.join(&main_old_key);
        let wt_dir = workspaces.join(&wt_old_key);
        std::fs::create_dir_all(main_dir.join("terminals")).expect("main terminals");
        std::fs::create_dir_all(wt_dir.join("terminals")).expect("wt terminals");
        std::fs::write(main_dir.join("terminals/s1.jsonl"), "x").expect("s1");
        std::fs::write(wt_dir.join("terminals/s2.jsonl"), "y").expect("s2");
        for (dir, key, root) in [
            (&main_dir, &main_old_key, &main),
            (&wt_dir, &wt_old_key, &worktree),
        ] {
            let meta = WorkspaceMetadata {
                key: key.clone(),
                root: root.to_string_lossy().to_string(),
                alias: None,
                created_at: "t0".into(),
                last_seen_at: "t0".into(),
            };
            std::fs::write(
                dir.join("workspace.json"),
                serde_json::to_string_pretty(&meta).expect("serialize"),
            )
            .expect("write meta");
        }

        let inspect = inspect_workspace_keys().expect("inspect");
        assert_eq!(inspect.migrated.len(), 2);
        assert!(main_dir.exists(), "inspect must not rename");

        let migrated = migrate_workspace_keys().expect("migrate");
        assert_eq!(migrated.migrated.len() + migrated.merged.len(), 2);

        // Both checkouts converge on the commondir key with logs merged.
        let merged = workspaces.join(&new_key);
        assert!(merged.join("terminals/s1.jsonl").exists());
        assert!(merged.join("terminals/s2.jsonl").exists());
        assert!(!main_dir.exists());
        assert!(!wt_dir.exists());

        // Idempotent: a second run is a no-op.
        let second = migrate_workspace_keys().expect("idempotent");
        assert!(second.migrated.is_empty() && second.merged.is_empty());
        assert_eq!(second.current, 1);

        match old_data {
            Some(value) => std::env::set_var("SIVTR_DATA_DIR", value),
            None => std::env::remove_var("SIVTR_DATA_DIR"),
        }
        let _ = std::fs::remove_dir_all(main);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn migrate_heals_stale_metadata_after_failed_rename_write() {
        let _lock = crate::test_env_lock();
        let data = unique_test_dir("workspace-heal");
        let old_data = std::env::var_os("SIVTR_DATA_DIR");
        std::env::set_var("SIVTR_DATA_DIR", &data);

        let main = unique_test_dir("repo-main");
        make_repo(&main);

        // The dir already carries the commondir key, but a previous
        // post-rename metadata write failed: workspace.json still holds the
        // stale key. A --fix run must repair the fields in place.
        let new_key = workspace_identity(&main).0;
        let dir = data.join("workspaces").join(&new_key);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let meta = WorkspaceMetadata {
            key: "stale-key".into(),
            root: real_path(&main).to_string_lossy().to_string(),
            alias: None,
            created_at: "t0".into(),
            last_seen_at: "t0".into(),
        };
        let meta_path = dir.join("workspace.json");
        std::fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).expect("serialize"),
        )
        .expect("write meta");

        let report = migrate_workspace_keys().expect("migrate");
        assert!(report.migrated.is_empty() && report.merged.is_empty());
        assert!(report.skipped.is_empty());
        assert_eq!(report.current, 1);

        let healed: WorkspaceMetadata =
            serde_json::from_str(&std::fs::read_to_string(&meta_path).expect("read meta"))
                .expect("parse meta");
        assert_eq!(healed.key, new_key);
        assert_eq!(healed.root, real_path(&main).to_string_lossy().to_string());

        match old_data {
            Some(value) => std::env::set_var("SIVTR_DATA_DIR", value),
            None => std::env::remove_var("SIVTR_DATA_DIR"),
        }
        let _ = std::fs::remove_dir_all(main);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn terminal_session_id_uses_file_stem() {
        assert_eq!(
            terminal_session_id_from_path(Path::new("session_123.jsonl")),
            "session_123"
        );
    }

    #[test]
    fn display_name_uses_basename_without_alias() {
        let unix = WorkspaceMetadata {
            key: "abc".into(),
            root: "/home/user/Coding/sivtr".into(),
            alias: None,
            created_at: "t".into(),
            last_seen_at: "t".into(),
        };
        assert_eq!(workspace_alias(&unix), "sivtr");

        let windows = WorkspaceMetadata {
            key: "abc".into(),
            root: r"D:\Coding\sivtr".into(),
            alias: None,
            created_at: "t".into(),
            last_seen_at: "t".into(),
        };
        assert_eq!(workspace_alias(&windows), "sivtr");
    }

    #[test]
    fn display_name_prefers_persisted_alias() {
        let meta = WorkspaceMetadata {
            key: "abc".into(),
            root: "D:\\Coding\\sivtr-tui-stack".into(),
            alias: Some("sivtr".into()),
            created_at: "t".into(),
            last_seen_at: "t".into(),
        };
        assert_eq!(workspace_alias(&meta), "sivtr");
    }

    #[test]
    fn rename_workspace_persists_alias() {
        let _guard = crate::test_env_lock();
        let data = unique_test_dir("rename-data");
        let previous = std::env::var_os("SIVTR_DATA_DIR");
        // SAFETY: test-only env mutation, guarded by the shared test lock.
        unsafe { std::env::set_var("SIVTR_DATA_DIR", &data) };

        let repo = unique_test_dir("rename-ws").join("sivtr");
        make_repo(&repo);
        let paths = paths_for_root(repo).expect("paths");
        std::fs::create_dir_all(&paths.dir).expect("workspace dir");
        let now = "t";
        let meta = WorkspaceMetadata {
            key: paths.key.clone(),
            root: paths.root.to_string_lossy().to_string(),
            alias: Some("sivtr".into()),
            created_at: now.into(),
            last_seen_at: now.into(),
        };
        let meta_path = paths.dir.join("workspace.json");
        std::fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).expect("json"),
        )
        .expect("write meta");

        let root = paths.root.to_string_lossy().to_string();
        let renamed = rename_workspace(&root, "core").expect("rename");
        assert_eq!(renamed.alias.as_deref(), Some("core"));
        let persisted: WorkspaceMetadata =
            serde_json::from_str(&std::fs::read_to_string(&meta_path).expect("read meta"))
                .expect("parse meta");
        assert_eq!(persisted.alias.as_deref(), Some("core"));

        match previous {
            Some(value) => unsafe { std::env::set_var("SIVTR_DATA_DIR", value) },
            None => unsafe { std::env::remove_var("SIVTR_DATA_DIR") },
        }
        let _ = std::fs::remove_dir_all(data);
        let _ = std::fs::remove_dir_all(&paths.dir);
    }
}
