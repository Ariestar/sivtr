use crate::cli::{CodexAction, CodexCommand, CodexExportArgs};
use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use sha2::{Digest, Sha256};
use sivtr_core::codex::local_codex_sessions_dir;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, TryRecvError, TrySendError};
use std::thread;
use std::time::Duration;
use std::time::SystemTime;

const MAX_INCREMENTAL_APPEND_BYTES: u64 = 8 * 1024 * 1024;
const CONTENT_FINGERPRINT_BUFFER_BYTES: usize = 64 * 1024;
const APPEND_PREFIX_GUARD_BYTES: u64 = 64 * 1024;
const WATCH_DEBOUNCE_LIMIT: Duration = Duration::from_millis(250);

#[derive(Debug)]
enum WatchSignal {
    Changed,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportWait {
    Changed,
    ReconcileTimeout,
    WatcherDisconnected,
}

struct ExportWakeup {
    receiver: Receiver<WatchSignal>,
    _watcher: RecommendedWatcher,
}

pub fn execute(command: CodexCommand) -> Result<()> {
    match command.action {
        CodexAction::Export(args) => export(args),
    }
}

fn export(args: CodexExportArgs) -> Result<()> {
    let source_root = local_codex_sessions_dir();
    if !source_root.exists() {
        anyhow::bail!("No local Codex sessions found at {}", source_root.display());
    }

    let target_root = args.dest.join("sessions");
    let watch_interval = args
        .watch
        .then(|| resolve_watch_interval(&args))
        .transpose()?;
    let mut export_wakeup = watch_interval.and_then(|_| match start_export_wakeup(&source_root) {
        Ok(wakeup) => Some(wakeup),
        Err(error) => {
            eprintln!(
                "sivtr: warning: native Codex session watching is unavailable; polling instead: {error}"
            );
            None
        }
    });
    let mut checkpoints = HashMap::new();

    loop {
        let copied = export_once(&source_root, &target_root, args.limit, &mut checkpoints)?;
        if copied > 0 || watch_interval.is_none() {
            eprintln!(
                "sivtr: updated {} Codex session file(s) in {}",
                copied,
                target_root.display()
            );
        }

        let Some(watch_interval) = watch_interval else {
            return Ok(());
        };
        let wait = export_wakeup
            .as_ref()
            .map(|wakeup| wait_for_export_cycle(&wakeup.receiver, watch_interval));
        if wait == Some(ExportWait::WatcherDisconnected) {
            eprintln!("sivtr: warning: Codex session watcher disconnected; polling instead");
            export_wakeup = None;
            thread::sleep(watch_interval);
        } else if wait.is_none() {
            thread::sleep(watch_interval);
        }
    }
}

fn resolve_watch_interval(args: &CodexExportArgs) -> Result<Duration> {
    if let Some(interval_ms) = args.interval_ms {
        if interval_ms == 0 {
            anyhow::bail!("`--interval-ms` must be greater than 0 when `--watch` is enabled");
        }
        return Ok(Duration::from_millis(interval_ms));
    }

    if args.interval == 0 {
        anyhow::bail!("`--interval` must be greater than 0 when `--watch` is enabled");
    }

    Ok(Duration::from_secs(args.interval))
}

fn wait_for_export_cycle(receiver: &Receiver<WatchSignal>, interval: Duration) -> ExportWait {
    match receiver.recv_timeout(interval) {
        Ok(signal) => {
            report_watch_signal(signal);
            thread::sleep(interval.min(WATCH_DEBOUNCE_LIMIT));
            loop {
                match receiver.try_recv() {
                    Ok(signal) => report_watch_signal(signal),
                    Err(TryRecvError::Empty) => return ExportWait::Changed,
                    Err(TryRecvError::Disconnected) => return ExportWait::WatcherDisconnected,
                }
            }
        }
        Err(RecvTimeoutError::Timeout) => ExportWait::ReconcileTimeout,
        Err(RecvTimeoutError::Disconnected) => ExportWait::WatcherDisconnected,
    }
}

fn report_watch_signal(signal: WatchSignal) {
    if let WatchSignal::Failed(error) = signal {
        eprintln!("sivtr: warning: Codex session watcher failed: {error}");
    }
}

fn start_export_wakeup(source_root: &Path) -> Result<ExportWakeup> {
    let (sender, receiver) = sync_channel(1);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let signal = match event {
            Ok(_) => WatchSignal::Changed,
            Err(error) => WatchSignal::Failed(error.to_string()),
        };
        // Capacity one deliberately coalesces bursts; reconciliation covers dropped wakeups.
        match sender.try_send(signal) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => (),
        }
    })
    .context("Failed to initialize native filesystem watcher")?;
    watcher
        .watch(source_root, RecursiveMode::Recursive)
        .with_context(|| format!("Failed to watch {}", source_root.display()))?;

    Ok(ExportWakeup {
        receiver,
        _watcher: watcher,
    })
}

fn export_once(
    source_root: &Path,
    target_root: &Path,
    limit: usize,
    checkpoints: &mut HashMap<PathBuf, LocalFileObservation>,
) -> Result<usize> {
    let files = collect_session_files(source_root, limit)?;
    if files.is_empty() {
        anyhow::bail!("No local Codex sessions found at {}", source_root.display());
    }

    fs::create_dir_all(target_root)
        .with_context(|| format!("Failed to create {}", target_root.display()))?;
    set_shared_read_permissions(target_root)?;

    let mut kept = HashSet::new();
    let mut updated = 0;
    let mut seen_dirs = HashSet::new();
    // Copy oldest -> newest so exported mtimes preserve true recency.
    // Copying newest first would make older files appear newer in the shared tree.
    for source in files.iter().rev() {
        let relative = source
            .strip_prefix(source_root)
            .with_context(|| format!("Failed to relativize {}", source.display()))?;
        kept.insert(relative.to_path_buf());

        let target = target_root.join(relative);
        if let Some(parent) = target.parent() {
            if seen_dirs.insert(parent.to_path_buf()) {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create {}", parent.display()))?;
                set_shared_read_permissions_recursive(target_root, parent)?;
            }
        }
        let published =
            synchronize_session_file(source, &target, checkpoints.get(relative).copied())?;
        checkpoints.insert(relative.to_path_buf(), published.source);
        if published.change != SessionFileChange::Unchanged {
            updated += 1;
        }
    }

    remove_stale_exported_files(target_root, &kept);
    checkpoints.retain(|relative, _| kept.contains(relative));
    Ok(updated)
}

fn collect_session_files(root: &Path, limit: usize) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files)?;
    files.sort_by_key(|path| modified_time(path).unwrap_or(SystemTime::UNIX_EPOCH));
    files.reverse();
    if limit > 0 {
        files.truncate(limit);
    }
    Ok(files)
}

fn collect_jsonl_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root).with_context(|| format!("Failed to read {}", root.display()))? {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalFileObservation {
    len: u64,
    modified: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PortableContentFingerprint {
    len: u64,
    sha256: [u8; 32],
}

fn local_file_observation(path: &Path) -> Result<LocalFileObservation> {
    let metadata = fs::metadata(path)?;
    Ok(LocalFileObservation {
        len: metadata.len(),
        modified: metadata.modified()?,
    })
}

fn exported_file_observation(path: &Path) -> Result<Option<LocalFileObservation>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        return Ok(None);
    }
    Ok(Some(LocalFileObservation {
        len: metadata.len(),
        modified: metadata.modified()?,
    }))
}

fn portable_content_fingerprint(path: &Path, byte_len: u64) -> Result<PortableContentFingerprint> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("Failed to open {} for content verification", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; CONTENT_FINGERPRINT_BUFFER_BYTES];
    let mut remaining = byte_len;

    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..chunk_len]).with_context(|| {
            format!(
                "File changed while verifying {} at {} bytes",
                path.display(),
                byte_len
            )
        })?;
        hasher.update(&buffer[..chunk_len]);
        remaining -= chunk_len as u64;
    }

    Ok(PortableContentFingerprint {
        len: byte_len,
        sha256: hasher.finalize().into(),
    })
}

fn target_is_source_prefix(source: &Path, target: &Path, target_len: u64) -> Result<bool> {
    Ok(portable_content_fingerprint(source, target_len)?
        == portable_content_fingerprint(target, target_len)?)
}

fn source_matches_target_tail(source: &Path, target: &Path, target_len: u64) -> Result<bool> {
    let guard_len = target_len.min(APPEND_PREFIX_GUARD_BYTES);
    let offset = target_len - guard_len;
    let guard_len = usize::try_from(guard_len).context("Codex prefix guard is too large")?;
    let mut source_guard = vec![0; guard_len];
    let mut target_guard = vec![0; guard_len];

    let mut source_file = fs::File::open(source)
        .with_context(|| format!("Failed to open Codex session {}", source.display()))?;
    source_file
        .seek(SeekFrom::Start(offset))
        .with_context(|| format!("Failed to seek Codex session {}", source.display()))?;
    source_file
        .read_exact(&mut source_guard)
        .with_context(|| format!("Codex session changed while verifying {}", source.display()))?;

    let mut target_file = fs::File::open(target)
        .with_context(|| format!("Failed to open Codex export {}", target.display()))?;
    target_file
        .seek(SeekFrom::Start(offset))
        .with_context(|| format!("Failed to seek Codex export {}", target.display()))?;
    target_file
        .read_exact(&mut target_guard)
        .with_context(|| format!("Codex export changed while verifying {}", target.display()))?;

    Ok(source_guard == target_guard)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionFileChange {
    Unchanged,
    Appended,
    Replaced,
}

struct PublishedSessionFile {
    change: SessionFileChange,
    source: LocalFileObservation,
}

fn synchronize_session_file(
    source: &Path,
    target: &Path,
    last_published: Option<LocalFileObservation>,
) -> Result<PublishedSessionFile> {
    let source_observation = local_file_observation(source)?;
    let target_observation = exported_file_observation(target)?;

    if target_observation == Some(source_observation) && last_published == Some(source_observation)
    {
        set_shared_read_permissions(target)?;
        return Ok(PublishedSessionFile {
            change: SessionFileChange::Unchanged,
            source: source_observation,
        });
    }

    if target_observation.is_some_and(|target| target.len == source_observation.len)
        && target_is_source_prefix(source, target, source_observation.len)?
    {
        set_shared_read_permissions(target)?;
        if target_observation != Some(source_observation) {
            preserve_source_modified_time(source, target);
        }
        return Ok(PublishedSessionFile {
            change: SessionFileChange::Unchanged,
            source: source_observation,
        });
    }

    let append_fits_memory = target_observation.is_some_and(|target| {
        source_observation.len > target.len
            && source_observation.len - target.len <= MAX_INCREMENTAL_APPEND_BYTES
    });
    let hot_prefix_candidate = last_published == target_observation
        && last_published.is_some_and(|published| source_observation.len > published.len)
        && append_fits_memory;
    let trusted_hot_prefix = if hot_prefix_candidate {
        let target_len = target_observation
            .map(|observation| observation.len)
            .context("Missing target observation for prefix guard")?;
        source_matches_target_tail(source, target, target_len)?
    } else {
        false
    };
    let verified_cold_prefix = if !trusted_hot_prefix && append_fits_memory {
        let target_len = target_observation
            .map(|observation| observation.len)
            .context("Missing target observation for prefix verification")?;
        target_is_source_prefix(source, target, target_len)?
    } else {
        false
    };

    if trusted_hot_prefix || verified_cold_prefix {
        let offset = target_observation
            .map(|observation| observation.len)
            .context("Missing target observation for incremental append")?;
        append_session_file_snapshot(source, target, offset, source_observation)?;
        return Ok(PublishedSessionFile {
            change: SessionFileChange::Appended,
            source: source_observation,
        });
    }

    copy_session_file_atomically(source, target)?;
    Ok(PublishedSessionFile {
        change: SessionFileChange::Replaced,
        source: source_observation,
    })
}

fn append_session_file_snapshot(
    source: &Path,
    target: &Path,
    offset: u64,
    source_observation: LocalFileObservation,
) -> Result<()> {
    let append_len = source_observation.len - offset;
    let append_len = usize::try_from(append_len).context("Codex append is too large for memory")?;
    let mut appended = vec![0; append_len];
    let mut source_file = fs::File::open(source)
        .with_context(|| format!("Failed to open Codex session {}", source.display()))?;
    source_file
        .seek(SeekFrom::Start(offset))
        .with_context(|| format!("Failed to seek Codex session {}", source.display()))?;
    source_file
        .read_exact(&mut appended)
        .with_context(|| format!("Codex session changed while reading {}", source.display()))?;

    let mut target_file = fs::OpenOptions::new()
        .append(true)
        .open(target)
        .with_context(|| format!("Failed to append Codex session {}", target.display()))?;
    target_file
        .write_all(&appended)
        .with_context(|| format!("Failed to append Codex session {}", target.display()))?;
    drop(target_file);
    set_shared_read_permissions(target)?;
    preserve_source_modified_time(source, target);
    Ok(())
}

fn copy_session_file_atomically(source: &Path, target: &Path) -> Result<()> {
    let temp = target.with_extension("jsonl.tmp");
    remove_existing_atomic_temp_file(&temp)?;

    fs::copy(source, &temp).with_context(|| {
        format!(
            "Failed to copy Codex session from {} to {}",
            source.display(),
            temp.display()
        )
    })?;
    set_shared_read_permissions(&temp)?;
    fs::rename(&temp, target).with_context(|| format!("Failed to publish {}", target.display()))?;
    preserve_source_modified_time(source, target);
    Ok(())
}

fn remove_existing_atomic_temp_file(temp: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(temp) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_dir() {
        anyhow::bail!("Atomic export temp path is a directory: {}", temp.display());
    }
    fs::remove_file(temp).with_context(|| format!("Failed to remove {}", temp.display()))?;
    Ok(())
}

fn preserve_source_modified_time(source: &Path, target: &Path) {
    let modified = match fs::metadata(source).and_then(|metadata| metadata.modified()) {
        Ok(modified) => modified,
        Err(_) => return,
    };

    let file = match fs::OpenOptions::new().write(true).open(target) {
        Ok(file) => file,
        Err(_) => return,
    };

    if let Err(error) = file.set_times(fs::FileTimes::new().set_modified(modified)) {
        eprintln!(
            "sivtr: warning: failed to preserve mtime from {} to {}: {}",
            source.display(),
            target.display(),
            error
        );
    }
}

fn remove_stale_exported_files(root: &Path, kept: &HashSet<PathBuf>) {
    if !root.exists() {
        return;
    }

    remove_stale_exported_files_inner(root, root, kept);
}

fn remove_stale_exported_files_inner(root: &Path, dir: &Path, kept: &HashSet<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!(
                "sivtr: warning: failed to read {} during stale export cleanup: {}",
                dir.display(),
                err
            );
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!(
                    "sivtr: warning: failed to inspect an entry under {}: {}",
                    dir.display(),
                    err
                );
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            remove_stale_exported_files_inner(root, &path, kept);

            match fs::read_dir(&path) {
                Ok(mut children) => {
                    if children.next().is_none() {
                        if let Err(err) = fs::remove_dir(&path) {
                            eprintln!(
                                "sivtr: warning: failed to remove empty export directory {}: {}",
                                path.display(),
                                err
                            );
                        }
                    }
                }
                Err(err) => {
                    eprintln!(
                        "sivtr: warning: failed to re-read {} for cleanup: {}",
                        path.display(),
                        err
                    );
                }
            }
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        let relative = match path.strip_prefix(root) {
            Ok(relative) => relative,
            Err(err) => {
                eprintln!(
                    "sivtr: warning: failed to relativize {}: {}",
                    path.display(),
                    err
                );
                continue;
            }
        };
        if !kept.contains(relative) {
            if let Err(err) = fs::remove_file(&path) {
                eprintln!(
                    "sivtr: warning: failed to remove stale export {}: {}",
                    path.display(),
                    err
                );
            }
        }
    }
}

#[cfg(unix)]
fn set_shared_read_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if path.is_dir() { 0o755 } else { 0o644 };
    if fs::metadata(path)?.permissions().mode() & 0o7777 == mode {
        return Ok(());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to chmod {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_shared_read_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn set_shared_read_permissions_recursive(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("Failed to relativize {}", path.display()))?;

    let mut current = root.to_path_buf();
    set_shared_read_permissions(&current)?;

    for component in relative.components() {
        current.push(component.as_os_str());
        set_shared_read_permissions(&current)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::set_shared_read_permissions_recursive;
    use super::{
        collect_session_files, copy_session_file_atomically, export_once, local_file_observation,
        resolve_watch_interval, start_export_wakeup, synchronize_session_file,
        wait_for_export_cycle, ExportWait, SessionFileChange, WatchSignal,
    };
    use crate::cli::CodexExportArgs;
    use std::collections::HashMap;
    use std::io::Write;
    #[cfg(unix)]
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    #[test]
    fn collect_session_files_sorts_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a.jsonl");
        let second = dir.path().join("b.jsonl");
        std::fs::write(&first, "{}\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(&second, "{}\n").unwrap();

        let files = collect_session_files(dir.path(), 0).unwrap();

        assert_eq!(files, vec![second, first]);
    }

    #[cfg(unix)]
    #[test]
    fn shared_permissions_are_applied_to_nested_export_directories() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sessions");
        let nested = root.join(Path::new("2026/05/07"));
        std::fs::create_dir_all(&nested).unwrap();

        set_shared_read_permissions_recursive(&root, &nested).unwrap();

        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::metadata(root.join("2026"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::metadata(root.join("2026/05"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn export_once_removes_stale_files_outside_limit() {
        let dir = tempfile::tempdir().unwrap();
        let source_root = dir.path().join("source");
        let target_root = dir.path().join("target").join("sessions");
        let nested = source_root.join("2026/05/08");
        std::fs::create_dir_all(&nested).unwrap();

        let newest = nested.join("newest.jsonl");
        let older = nested.join("older.jsonl");
        std::fs::write(
            &older,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"older\"}}\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            &newest,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"newest\"}}\n",
        )
        .unwrap();

        let mut checkpoints = HashMap::new();
        export_once(&source_root, &target_root, 1, &mut checkpoints).unwrap();

        assert!(target_root.join("2026/05/08/newest.jsonl").exists());
        assert!(!target_root.join("2026/05/08/older.jsonl").exists());
        assert_eq!(checkpoints.len(), 1);
        assert!(checkpoints.contains_key(&PathBuf::from("2026/05/08/newest.jsonl")));
    }

    #[test]
    fn export_once_skips_unchanged_files_and_republishes_changes() {
        let dir = tempfile::tempdir().unwrap();
        let source_root = dir.path().join("source");
        let target_root = dir.path().join("target").join("sessions");
        std::fs::create_dir_all(&source_root).unwrap();
        let source = source_root.join("session.jsonl");
        let mut checkpoints = HashMap::new();

        std::fs::write(&source, "initial\n").unwrap();
        assert_eq!(
            export_once(&source_root, &target_root, 0, &mut checkpoints).unwrap(),
            1
        );
        let target = target_root.join("session.jsonl");
        let stable_metadata = std::fs::metadata(&target).unwrap();

        assert_eq!(
            export_once(&source_root, &target_root, 0, &mut checkpoints).unwrap(),
            0
        );
        let unchanged_metadata = std::fs::metadata(&target).unwrap();
        assert_eq!(
            unchanged_metadata.modified().unwrap(),
            stable_metadata.modified().unwrap()
        );

        std::fs::write(&source, "changed-content\n").unwrap();
        assert_eq!(
            export_once(&source_root, &target_root, 0, &mut checkpoints).unwrap(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(target).unwrap(),
            "changed-content\n"
        );
    }

    #[test]
    fn export_once_preserves_target_identity_for_append_growth() {
        let dir = tempfile::tempdir().unwrap();
        let source_root = dir.path().join("source");
        let target_root = dir.path().join("target").join("sessions");
        std::fs::create_dir_all(&source_root).unwrap();
        let source = source_root.join("session.jsonl");
        std::fs::write(&source, "first\n").unwrap();
        let mut checkpoints = HashMap::new();
        export_once(&source_root, &target_root, 0, &mut checkpoints).unwrap();
        let target = target_root.join("session.jsonl");
        let target_alias = target_root.join("session-alias");
        std::fs::hard_link(&target, &target_alias).unwrap();

        let mut source_file = std::fs::OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap();
        source_file.write_all(b"second\n").unwrap();
        drop(source_file);
        export_once(&source_root, &target_root, 0, &mut checkpoints).unwrap();

        assert_eq!(
            std::fs::read_to_string(target_alias).unwrap(),
            "first\nsecond\n"
        );
    }

    #[test]
    fn export_once_preserves_newest_order_in_target_tree() {
        let dir = tempfile::tempdir().unwrap();
        let source_root = dir.path().join("source");
        let target_root = dir.path().join("target").join("sessions");
        let nested = source_root.join("2026/05/08");
        std::fs::create_dir_all(&nested).unwrap();

        let older = nested.join("older.jsonl");
        let newer = nested.join("newer.jsonl");
        std::fs::write(
            &older,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"older\"}}\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            &newer,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"newer\"}}\n",
        )
        .unwrap();

        let mut checkpoints = HashMap::new();
        export_once(&source_root, &target_root, 0, &mut checkpoints).unwrap();

        let exported = collect_session_files(&target_root, 0).unwrap();
        assert_eq!(exported.len(), 2);
        assert_eq!(
            exported[0].file_name().and_then(|name| name.to_str()),
            Some("newer.jsonl")
        );
        assert_eq!(
            exported[1].file_name().and_then(|name| name.to_str()),
            Some("older.jsonl")
        );
    }

    #[test]
    fn atomic_copy_replaces_existing_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        std::fs::write(&source, "new data\n").unwrap();
        std::fs::write(&target, "old data\n").unwrap();

        copy_session_file_atomically(&source, &target).unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new data\n");
        assert!(!dir.path().join("target.jsonl.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_copy_does_not_follow_dangling_temp_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        let temp = dir.path().join("target.jsonl.tmp");
        let victim = dir.path().join("victim.jsonl");
        std::fs::write(&source, "safe\n").unwrap();
        symlink(&victim, &temp).unwrap();

        copy_session_file_atomically(&source, &target).unwrap();

        assert!(!victim.exists());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "safe\n");
        assert!(std::fs::symlink_metadata(temp).is_err());
    }

    #[test]
    fn session_file_sync_appends_verified_growth() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        std::fs::write(&source, "first\n").unwrap();
        copy_session_file_atomically(&source, &target).unwrap();
        let last_published = local_file_observation(&source).unwrap();

        let mut source_file = std::fs::OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap();
        source_file.write_all(b"second\n").unwrap();
        drop(source_file);

        let published = synchronize_session_file(&source, &target, Some(last_published)).unwrap();

        assert_eq!(published.change, SessionFileChange::Appended);
        assert_eq!(std::fs::read_to_string(target).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn session_file_sync_recovers_append_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        std::fs::write(&source, "first\n").unwrap();
        copy_session_file_atomically(&source, &target).unwrap();

        let mut source_file = std::fs::OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap();
        source_file.write_all(b"second\n").unwrap();
        drop(source_file);

        let published = synchronize_session_file(&source, &target, None).unwrap();

        assert_eq!(published.change, SessionFileChange::Appended);
        assert_eq!(std::fs::read_to_string(target).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn session_file_sync_accepts_identical_migrated_content() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        std::fs::write(&source, "portable\n").unwrap();
        copy_session_file_atomically(&source, &target).unwrap();
        let target_file = std::fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .unwrap();
        target_file
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();

        let published = synchronize_session_file(&source, &target, None).unwrap();

        assert_eq!(published.change, SessionFileChange::Unchanged);
        assert_eq!(
            std::fs::metadata(target).unwrap().modified().unwrap(),
            std::fs::metadata(source).unwrap().modified().unwrap()
        );
    }

    #[test]
    fn session_file_sync_verifies_cold_start_metadata_match() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        std::fs::write(&source, "source\n").unwrap();
        std::fs::write(&target, "target\n").unwrap();
        let source_modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        let target_file = std::fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .unwrap();
        target_file
            .set_times(std::fs::FileTimes::new().set_modified(source_modified))
            .unwrap();

        let published = synchronize_session_file(&source, &target, None).unwrap();

        assert_eq!(published.change, SessionFileChange::Replaced);
        assert_eq!(std::fs::read_to_string(target).unwrap(), "source\n");
    }

    #[test]
    fn session_file_sync_replaces_same_length_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        std::fs::write(&source, "before\n").unwrap();
        copy_session_file_atomically(&source, &target).unwrap();
        std::fs::write(&source, "after!\n").unwrap();

        let published = synchronize_session_file(&source, &target, None).unwrap();

        assert_eq!(published.change, SessionFileChange::Replaced);
        assert_eq!(std::fs::read_to_string(target).unwrap(), "after!\n");
    }

    #[test]
    fn session_file_sync_replaces_truncated_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        std::fs::write(&source, "first\nsecond\n").unwrap();
        copy_session_file_atomically(&source, &target).unwrap();
        std::fs::write(&source, "first\n").unwrap();

        let published = synchronize_session_file(&source, &target, None).unwrap();

        assert_eq!(published.change, SessionFileChange::Replaced);
        assert_eq!(std::fs::read_to_string(target).unwrap(), "first\n");
    }

    #[cfg(unix)]
    #[test]
    fn session_file_sync_replaces_target_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        let victim = dir.path().join("victim.jsonl");
        std::fs::write(&source, "first\nsecond\n").unwrap();
        std::fs::write(&victim, "first\n").unwrap();
        symlink(&victim, &target).unwrap();

        let published = synchronize_session_file(&source, &target, None).unwrap();

        assert_eq!(published.change, SessionFileChange::Replaced);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "first\n");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "first\nsecond\n");
        assert!(!std::fs::symlink_metadata(target)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn resolve_watch_interval_defaults_to_seconds() {
        let args = CodexExportArgs {
            dest: PathBuf::from("/tmp/shared-codex"),
            limit: 0,
            watch: true,
            interval: 2,
            interval_ms: None,
        };

        let interval = resolve_watch_interval(&args).unwrap();
        assert_eq!(interval, Duration::from_secs(2));
    }

    #[test]
    fn resolve_watch_interval_prefers_milliseconds_override() {
        let args = CodexExportArgs {
            dest: PathBuf::from("/tmp/shared-codex"),
            limit: 0,
            watch: true,
            interval: 10,
            interval_ms: Some(250),
        };

        let interval = resolve_watch_interval(&args).unwrap();
        assert_eq!(interval, Duration::from_millis(250));
    }

    #[test]
    fn resolve_watch_interval_rejects_zero_seconds() {
        let args = CodexExportArgs {
            dest: PathBuf::from("/tmp/shared-codex"),
            limit: 0,
            watch: true,
            interval: 0,
            interval_ms: None,
        };

        let error = resolve_watch_interval(&args).unwrap_err();
        assert!(error
            .to_string()
            .contains("`--interval` must be greater than 0"));
    }

    #[test]
    fn resolve_watch_interval_rejects_zero_milliseconds() {
        let args = CodexExportArgs {
            dest: PathBuf::from("/tmp/shared-codex"),
            limit: 0,
            watch: true,
            interval: 1,
            interval_ms: Some(0),
        };

        let error = resolve_watch_interval(&args).unwrap_err();
        assert!(error
            .to_string()
            .contains("`--interval-ms` must be greater than 0"));
    }

    #[test]
    fn export_wait_wakes_before_reconciliation_timeout() {
        let (sender, receiver) = sync_channel(1);
        sender.send(WatchSignal::Changed).unwrap();

        let wait = wait_for_export_cycle(&receiver, Duration::from_secs(2));

        assert_eq!(wait, ExportWait::Changed);
    }

    #[test]
    fn export_watcher_reports_source_changes() {
        let dir = tempfile::tempdir().unwrap();
        let wakeup = start_export_wakeup(dir.path()).unwrap();

        std::fs::write(dir.path().join("session.jsonl"), "event\n").unwrap();
        let wait = wait_for_export_cycle(&wakeup.receiver, Duration::from_secs(5));

        assert_eq!(wait, ExportWait::Changed);
    }

    #[test]
    fn export_wait_distinguishes_timeout_and_disconnection() {
        let (_sender, receiver) = sync_channel(1);
        assert_eq!(
            wait_for_export_cycle(&receiver, Duration::from_millis(1)),
            ExportWait::ReconcileTimeout
        );

        let (sender, receiver) = sync_channel(1);
        drop(sender);
        assert_eq!(
            wait_for_export_cycle(&receiver, Duration::from_secs(1)),
            ExportWait::WatcherDisconnected
        );
    }
}
