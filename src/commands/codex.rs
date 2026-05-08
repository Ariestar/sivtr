use crate::cli::{CodexAction, CodexCommand, CodexExportArgs};
use anyhow::{Context, Result};
use sivtr_core::codex::local_codex_sessions_dir;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use std::time::SystemTime;

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

    loop {
        let copied = export_once(&source_root, &target_root, args.limit)?;
        eprintln!(
            "sivtr: mirrored {} Codex session file(s) to {}",
            copied,
            target_root.display()
        );

        if !args.watch {
            return Ok(());
        }

        thread::sleep(Duration::from_secs(args.interval));
    }
}

fn export_once(source_root: &Path, target_root: &Path, limit: usize) -> Result<usize> {
    let files = collect_session_files(source_root, limit)?;
    if files.is_empty() {
        anyhow::bail!("No local Codex sessions found at {}", source_root.display());
    }

    fs::create_dir_all(target_root)
        .with_context(|| format!("Failed to create {}", target_root.display()))?;
    set_shared_read_permissions(target_root)?;

    let mut kept = HashSet::new();
    for source in &files {
        let relative = source
            .strip_prefix(source_root)
            .with_context(|| format!("Failed to relativize {}", source.display()))?;
        kept.insert(relative.to_path_buf());

        let target = target_root.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
            set_shared_read_permissions_recursive(target_root, parent)?;
        }
        copy_session_file_atomically(source, &target)?;
    }

    remove_stale_exported_files(target_root, &kept)?;
    Ok(files.len())
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

fn copy_session_file_atomically(source: &Path, target: &Path) -> Result<()> {
    let temp = target.with_extension("jsonl.tmp");
    if temp.exists() {
        fs::remove_file(&temp).with_context(|| format!("Failed to remove {}", temp.display()))?;
    }

    fs::copy(source, &temp).with_context(|| {
        format!(
            "Failed to copy Codex session from {} to {}",
            source.display(),
            temp.display()
        )
    })?;
    set_shared_read_permissions(&temp)?;

    if target.exists() {
        fs::remove_file(target)
            .with_context(|| format!("Failed to replace {}", target.display()))?;
    }

    fs::rename(&temp, target).with_context(|| format!("Failed to publish {}", target.display()))?;
    set_shared_read_permissions(target)?;
    Ok(())
}

fn remove_stale_exported_files(root: &Path, kept: &HashSet<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    remove_stale_exported_files_inner(root, root, kept)
}

fn remove_stale_exported_files_inner(
    root: &Path,
    dir: &Path,
    kept: &HashSet<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            remove_stale_exported_files_inner(root, &path, kept)?;
            if fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
            }
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("Failed to relativize {}", path.display()))?;
        if !kept.contains(relative) {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove stale export {}", path.display()))?;
        }
    }

    Ok(())
}

#[cfg(unix)]
fn set_shared_read_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if path.is_dir() { 0o755 } else { 0o644 };
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
    use super::{collect_session_files, copy_session_file_atomically, export_once};
    #[cfg(unix)]
    use std::path::Path;

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

        export_once(&source_root, &target_root, 1).unwrap();

        assert!(target_root.join("2026/05/08/newest.jsonl").exists());
        assert!(!target_root.join("2026/05/08/older.jsonl").exists());
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
}
