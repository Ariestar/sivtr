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

/// Root directory for every path sivtr generates: config, workspaces
/// (terminal logs), sets, cache, history, and the remote daemon state.
///
/// Grok-style single home: `SIVTR_HOME` relocates the whole layout, else
/// `~/.sivtr` on every platform.
pub fn home_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SIVTR_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .map(|home| home.join(".sivtr"))
        .or_else(|| dirs::config_dir().map(|dir| dir.join("sivtr")))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// One legacy location from before the single-home layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPath {
    pub source: PathBuf,
    pub target: PathBuf,
    pub label: &'static str,
}

/// Locations sivtr wrote to before the single-home layout, for `doctor --fix`
/// migration. Entries whose source and target are the same place are omitted.
pub fn legacy_home_paths() -> Vec<LegacyPath> {
    let home = home_dir();
    let mut paths = Vec::new();

    if let Some(config) = dirs::config_dir() {
        let root = config.join("sivtr");
        if !same_location(&root, &home) {
            paths.push(LegacyPath {
                source: root,
                target: home.clone(),
                label: "data root",
            });
        }
    }

    if let Some(sets) = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .or_else(dirs::config_dir)
        .map(|dir| dir.join("sivtr").join("sets"))
    {
        let target = home.join("sets");
        if !same_location(&sets, &target) {
            paths.push(LegacyPath {
                source: sets,
                target,
                label: "workset store",
            });
        }
    }

    if let Some(data) = dirs::data_dir() {
        let db = data.join("sivtr").join("history.db");
        let target = home.join("history.db");
        if !same_location(&db, &target) {
            paths.push(LegacyPath {
                source: db,
                target,
                label: "history database",
            });
        }
    }

    paths
}

fn same_location(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
}

/// Move `source` into `target`, merging directory contents entry by entry so a
/// rename never clobbers an existing destination. Falls back to copy+delete
/// across volumes. Returns paths skipped because the destination already
/// exists.
pub fn move_path(source: &Path, target: &Path) -> Result<Vec<PathBuf>> {
    let mut conflicts = Vec::new();
    move_path_inner(source, target, &mut conflicts)?;
    Ok(conflicts)
}

fn move_path_inner(source: &Path, target: &Path, conflicts: &mut Vec<PathBuf>) -> Result<()> {
    if source.is_dir() {
        for entry in
            fs::read_dir(source).with_context(|| format!("Failed to read {}", source.display()))?
        {
            let entry = entry?;
            move_path_inner(&entry.path(), &target.join(entry.file_name()), conflicts)?;
        }
        let _ = fs::remove_dir(source);
        return Ok(());
    }
    if target.exists() {
        conflicts.push(target.to_path_buf());
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    if fs::rename(source, target).is_err() {
        fs::copy(source, target).with_context(|| {
            format!(
                "Failed to copy {} to {}",
                source.display(),
                target.display()
            )
        })?;
        fs::remove_file(source)?;
    }
    Ok(())
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

/// All known workspaces, parsed from `<home>/workspaces/<key>/workspace.json`,
/// most-recently-seen first. An empty result means sivtr has not recorded any
/// workspace yet (e.g. `sivtr init` was never run in a git repo).
pub fn list_workspaces() -> Result<Vec<WorkspaceMetadata>> {
    let dir = home_dir().join(WORKSPACES_DIR);
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

/// Outcome of [`inspect_workspace_keys`] / [`migrate_workspace_keys`].
#[derive(Debug, Default)]
pub struct WorkspaceMigration {
    /// `(old_key, new_key)` for each workspace that needs (or received) a rename.
    pub migrated: Vec<(String, String)>,
    /// Workspaces already on the current key scheme (no rename needed).
    pub current: usize,
    /// Legacy dirs whose target key already exists (duplicate of current scheme).
    /// On migrate, these are merged into the current dir then removed.
    pub duplicates: Vec<(String, String)>,
    /// `(old_key, kept_key)` for duplicates removed during migrate.
    pub removed_duplicates: Vec<(String, String)>,
    /// Dirs that could not be migrated, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

impl WorkspaceMigration {
    pub fn changed(&self) -> bool {
        !self.migrated.is_empty() || !self.removed_duplicates.is_empty()
    }

    pub fn needs_attention(&self) -> bool {
        !self.migrated.is_empty() || !self.duplicates.is_empty() || !self.skipped.is_empty()
    }
}

/// Dry-run: report which workspace dirs need re-keying without renaming them.
pub fn inspect_workspace_keys() -> Result<WorkspaceMigration> {
    scan_workspace_keys(false)
}

/// Re-key workspace dirs whose stored root predates the absolute-based key
/// scheme.
///
/// Legacy `workspace.json` roots were stored via `std::fs::canonicalize`, which
/// prepends a `\\?\` verbatim prefix on Windows. The current scheme derives the
/// key from `std::path::absolute` (no prefix), so legacy dirs no longer match
/// the key a fresh access computes — their captured sessions become unreachable.
/// This strips the legacy prefix, recomputes the key, and renames the dir +
/// rewrites `workspace.json` when they differ.
///
/// If the target key already exists, unique terminal logs are copied into the
/// current dir and the legacy dir is removed. Idempotent: a second run is a
/// no-op.
pub fn migrate_workspace_keys() -> Result<WorkspaceMigration> {
    scan_workspace_keys(true)
}

fn scan_workspace_keys(apply: bool) -> Result<WorkspaceMigration> {
    let base = home_dir().join(WORKSPACES_DIR);
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
            // Key matches; only tidy the root field when applying migration.
            if apply && meta.root != new_root_text {
                meta.root = new_root_text;
                let _ = write_workspace_metadata(&meta_path, &meta);
            }
            report.current += 1;
            continue;
        }

        let target = base.join(&new_key);
        if target.exists() {
            // Current-scheme dir already owns this root. Keep it; drop the legacy
            // duplicate after copying any terminal logs the current dir lacks.
            if apply {
                match merge_then_remove_duplicate(&dir, &target) {
                    Ok(()) => report
                        .removed_duplicates
                        .push((old_key.clone(), new_key.clone())),
                    Err(e) => report
                        .skipped
                        .push((dir, format!("failed to remove duplicate of {new_key}: {e}"))),
                }
            } else {
                report.duplicates.push((old_key, new_key));
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
                meta.root = new_root_text;
                let _ = write_workspace_metadata(&target.join("workspace.json"), &meta);
                report.migrated.push((old_key, new_key));
            }
            Err(e) => report.skipped.push((dir, format!("rename failed: {e}"))),
        }
    }
    Ok(report)
}

/// Copy any terminal logs from `legacy` that `current` lacks, then delete `legacy`.
fn merge_then_remove_duplicate(legacy: &Path, current: &Path) -> Result<()> {
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
    fs::remove_dir_all(legacy)
        .with_context(|| format!("failed to remove legacy workspace {}", legacy.display()))?;
    Ok(())
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
    let dir = home_dir().join(WORKSPACES_DIR).join(&key);
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
    use super::{
        git_root, home_dir, inspect_workspace_keys, legacy_home_paths, migrate_workspace_keys,
        move_path, terminal_session_id_from_path, workspace_key, WorkspaceMetadata,
    };
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

    #[test]
    fn inspect_workspace_keys_is_dry_run_and_migrate_renames() {
        let data = unique_test_dir("workspace-keys");
        let _guard = EnvGuard::set("SIVTR_HOME", &data);

        let root = unique_test_dir("legacy-root");
        let legacy_root = format!(r"\\?\{}", root.display());
        let old_key = workspace_key(&legacy_root);
        let new_key = workspace_key(&root.to_string_lossy());
        assert_ne!(old_key, new_key);

        let old_dir = data.join("workspaces").join(&old_key);
        std::fs::create_dir_all(&old_dir).expect("workspace dir");
        let meta = WorkspaceMetadata {
            key: old_key.clone(),
            root: legacy_root,
            created_at: "t0".into(),
            last_seen_at: "t0".into(),
        };
        std::fs::write(
            old_dir.join("workspace.json"),
            serde_json::to_string_pretty(&meta).expect("serialize"),
        )
        .expect("write meta");

        let inspect = inspect_workspace_keys().expect("inspect");
        assert_eq!(inspect.migrated, vec![(old_key.clone(), new_key.clone())]);
        assert!(old_dir.exists(), "inspect must not rename");

        let migrate = migrate_workspace_keys().expect("migrate");
        assert_eq!(migrate.migrated, vec![(old_key, new_key.clone())]);
        assert!(!old_dir.exists());
        assert!(data.join("workspaces").join(&new_key).exists());

        let second = migrate_workspace_keys().expect("idempotent");
        assert!(second.migrated.is_empty());
        assert_eq!(second.current, 1);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn migrate_removes_legacy_duplicate_when_current_key_exists() {
        let data = unique_test_dir("workspace-dup");
        let _guard = EnvGuard::set("SIVTR_HOME", &data);

        let root = unique_test_dir("dup-root");
        let legacy_root = format!(r"\\?\{}", root.display());
        let old_key = workspace_key(&legacy_root);
        let new_key = workspace_key(&root.to_string_lossy());
        assert_ne!(old_key, new_key);

        let old_dir = data.join("workspaces").join(&old_key);
        let new_dir = data.join("workspaces").join(&new_key);
        std::fs::create_dir_all(old_dir.join("terminals")).expect("legacy terminals");
        std::fs::create_dir_all(new_dir.join("terminals")).expect("current terminals");

        std::fs::write(
            old_dir.join("workspace.json"),
            serde_json::to_string_pretty(&WorkspaceMetadata {
                key: old_key.clone(),
                root: legacy_root,
                created_at: "t0".into(),
                last_seen_at: "t0".into(),
            })
            .expect("serialize legacy"),
        )
        .expect("write legacy meta");
        std::fs::write(
            new_dir.join("workspace.json"),
            serde_json::to_string_pretty(&WorkspaceMetadata {
                key: new_key.clone(),
                root: root.to_string_lossy().to_string(),
                created_at: "t0".into(),
                last_seen_at: "t0".into(),
            })
            .expect("serialize current"),
        )
        .expect("write current meta");

        // Unique log only in legacy, shared name already in current.
        std::fs::write(
            old_dir.join("terminals").join("only-old.jsonl"),
            b"old-only",
        )
        .expect("legacy unique log");
        std::fs::write(
            old_dir.join("terminals").join("shared.jsonl"),
            b"legacy-shared",
        )
        .expect("legacy shared log");
        std::fs::write(
            new_dir.join("terminals").join("shared.jsonl"),
            b"current-shared",
        )
        .expect("current shared log");

        let inspect = inspect_workspace_keys().expect("inspect");
        assert_eq!(inspect.duplicates, vec![(old_key.clone(), new_key.clone())]);
        assert!(old_dir.exists(), "inspect must not delete");

        let migrate = migrate_workspace_keys().expect("migrate");
        assert_eq!(migrate.removed_duplicates, vec![(old_key, new_key.clone())]);
        assert!(!old_dir.exists(), "legacy duplicate removed");
        assert_eq!(
            std::fs::read_to_string(new_dir.join("terminals").join("only-old.jsonl"))
                .expect("unique log copied"),
            "old-only"
        );
        assert_eq!(
            std::fs::read_to_string(new_dir.join("terminals").join("shared.jsonl"))
                .expect("shared log kept"),
            "current-shared"
        );

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data);
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            // Env mutation is process-global; serialize every env-touching test.
            let _lock = crate::test_env_lock();
            let previous = std::env::var_os(key);
            // SAFETY: test-only temporary env mutation, restored in Drop, guarded by the lock.
            unsafe { std::env::set_var(key, value) };
            Self {
                key,
                previous,
                _lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe { std::env::set_var(self.key, value) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    #[test]
    fn home_dir_prefers_sivtr_home() {
        let _lock = crate::test_env_lock();
        let home = unique_test_dir("home-env");
        let old_home = std::env::var_os("SIVTR_HOME");
        unsafe { std::env::set_var("SIVTR_HOME", &home) };
        assert_eq!(home_dir(), home);
        match old_home {
            Some(value) => unsafe { std::env::set_var("SIVTR_HOME", value) },
            None => unsafe { std::env::remove_var("SIVTR_HOME") },
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn legacy_home_paths_never_target_their_source() {
        let _lock = crate::test_env_lock();
        let home = unique_test_dir("legacy-home");
        let old = std::env::var_os("SIVTR_HOME");
        unsafe { std::env::set_var("SIVTR_HOME", &home) };
        for path in legacy_home_paths() {
            assert_ne!(path.source, path.target, "{}", path.label);
        }
        match old {
            Some(value) => unsafe { std::env::set_var("SIVTR_HOME", value) },
            None => unsafe { std::env::remove_var("SIVTR_HOME") },
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn move_path_merges_directories_and_reports_conflicts() {
        let source = unique_test_dir("move-src");
        let target = unique_test_dir("move-dst");
        std::fs::create_dir_all(source.join("workspaces")).expect("workspaces");
        std::fs::write(source.join("workspaces/a.jsonl"), "a").expect("a");
        std::fs::write(source.join("config.toml"), "c").expect("config");
        std::fs::create_dir_all(target.join("workspaces")).expect("target workspaces");
        std::fs::write(target.join("workspaces/a.jsonl"), "existing").expect("existing");

        let conflicts = move_path(&source, &target).expect("move");

        assert_eq!(conflicts, vec![target.join("workspaces").join("a.jsonl")]);
        assert_eq!(
            std::fs::read_to_string(target.join("workspaces/a.jsonl")).expect("read"),
            "existing"
        );
        assert!(
            target.join("config.toml").exists(),
            "moved entries land in target"
        );
        assert!(
            source.join("workspaces/a.jsonl").exists(),
            "conflicting entry stays in source"
        );
        assert!(
            !source.join("config.toml").exists(),
            "moved entries leave source"
        );

        let _ = std::fs::remove_dir_all(source);
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn move_path_skips_existing_files() {
        let source = unique_test_dir("move-file-src");
        let target = unique_test_dir("move-file-dst");
        std::fs::write(source.join("history.db"), "new").expect("new");
        std::fs::write(target.join("history.db"), "old").expect("old");

        let conflicts =
            move_path(&source.join("history.db"), &target.join("history.db")).expect("move");

        assert_eq!(conflicts, vec![target.join("history.db")]);
        assert_eq!(
            std::fs::read_to_string(target.join("history.db")).expect("read"),
            "old"
        );
        assert!(
            source.join("history.db").exists(),
            "conflicting source stays"
        );

        let _ = std::fs::remove_dir_all(source);
        let _ = std::fs::remove_dir_all(target);
    }
}
