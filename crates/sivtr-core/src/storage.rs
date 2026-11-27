//! Local storage footprint and reclamation for the sivtr home.
//!
//! The CLI, TUI, and MCP surfaces share one accounting model: the archive
//! database (with its WAL sidecars), derived caches, saved WorkSets, workspace
//! session logs, and everything else under the home. Reclamation is limited to
//! derived data: free archive pages and legacy parsed-session caches. Session
//! sources and saved WorkSets are never deleted here.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct StorageReport {
    pub home: String,
    pub total_bytes: u64,
    pub archive: ArchiveFootprint,
    pub cache: CacheFootprint,
    pub sets_bytes: u64,
    pub workspaces_bytes: u64,
    pub other_bytes: u64,
    /// Archive files still at the home root from the pre-`cache/` layout.
    /// `sivtr doctor --fix` migrates them into `cache/` when the target is free.
    pub legacy_archive_bytes: u64,
    /// Free archive pages plus legacy cache files, in bytes.
    pub reclaimable_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveFootprint {
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub shm_bytes: u64,
    pub reclaimable_bytes: u64,
    /// Whether the page statistics could be read. `false` means the archive is
    /// missing or another process blocks a read-only open; the counts are then
    /// zero rather than measured.
    pub available: bool,
}

impl ArchiveFootprint {
    pub fn total_bytes(&self) -> u64 {
        self.database_bytes + self.wal_bytes + self.shm_bytes
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheFootprint {
    pub legacy_bytes: u64,
    pub legacy_files: usize,
    pub bm25_bytes: u64,
    pub listing_bytes: u64,
    pub other_bytes: u64,
}

impl CacheFootprint {
    pub fn total_bytes(&self) -> u64 {
        self.legacy_bytes + self.bm25_bytes + self.listing_bytes + self.other_bytes
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VacuumReport {
    pub path: String,
    pub exists: bool,
    pub before_bytes: u64,
    pub after_bytes: Option<u64>,
    pub reclaimable_bytes: u64,
    pub stats_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheEntry {
    pub name: String,
    pub bytes: u64,
}

/// Measure the sivtr home without mutating it.
pub fn report(home: &Path) -> Result<StorageReport> {
    let archive = archive_footprint(&home.join("cache").join("archive.db"))?;
    let cache = cache_footprint(&home.join("cache"))?;
    let sets_bytes = path_bytes(&home.join("sets"))?;
    let workspaces_bytes = path_bytes(&home.join("workspaces"))?;
    // Measure the remainder directly instead of subtracting from a second
    // whole-home walk: the archive WAL can grow between the two measurements.
    let (other_bytes, legacy_archive_bytes) = home_remainder(home)?;
    let total_bytes = archive.total_bytes()
        + cache.total_bytes()
        + sets_bytes
        + workspaces_bytes
        + other_bytes
        + legacy_archive_bytes;

    Ok(StorageReport {
        home: home.display().to_string(),
        total_bytes,
        reclaimable_bytes: archive.reclaimable_bytes + cache.legacy_bytes,
        archive,
        cache,
        sets_bytes,
        workspaces_bytes,
        other_bytes,
        legacy_archive_bytes,
    })
}

/// Compact the archive database. With `dry_run`, report the reclaimable pages
/// without writing to the database.
pub fn vacuum(db_path: &Path, dry_run: bool) -> Result<VacuumReport> {
    let before_bytes = archive_total_bytes(db_path)?;
    if !db_path.exists() {
        return Ok(VacuumReport {
            path: db_path.display().to_string(),
            exists: false,
            before_bytes,
            after_bytes: None,
            reclaimable_bytes: 0,
            stats_available: false,
        });
    }

    let pragmas = read_archive_pragmas(db_path);
    let reclaimable_bytes = pragmas.unwrap_or(0);
    let stats_available = pragmas.is_some();

    if dry_run {
        return Ok(VacuumReport {
            path: db_path.display().to_string(),
            exists: true,
            before_bytes,
            after_bytes: None,
            reclaimable_bytes,
            stats_available,
        });
    }

    let conn = Connection::open(db_path)
        .with_context(|| format!("Failed to open archive database {}", db_path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))
        .context("Failed to set archive busy timeout")?;
    checkpoint_wal(&conn)?;
    conn.execute_batch("VACUUM")
        .context("Failed to compact the archive; close other sivtr processes and retry")?;
    checkpoint_wal(&conn)?;
    drop(conn);

    let after_bytes = archive_total_bytes(db_path)?;
    Ok(VacuumReport {
        path: db_path.display().to_string(),
        exists: true,
        before_bytes,
        after_bytes: Some(after_bytes),
        reclaimable_bytes: before_bytes.saturating_sub(after_bytes),
        stats_available,
    })
}

fn checkpoint_wal(conn: &Connection) -> Result<()> {
    let (busy, _log, _checkpointed): (i64, i64, i64) = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .context("Failed to checkpoint the archive WAL")?;
    if busy != 0 {
        anyhow::bail!("archive is in use; close other sivtr processes and retry");
    }
    Ok(())
}

fn archive_total_bytes(path: &Path) -> Result<u64> {
    Ok(path_bytes(path)?
        + path_bytes(&sidecar(path, "-wal"))?
        + path_bytes(&sidecar(path, "-shm"))?)
}

/// BM25 index caches that search rebuilds on demand.
pub fn bm25_cache_files(cache_dir: &Path) -> Result<Vec<CacheEntry>> {
    cache_files_matching(cache_dir, CacheKind::Bm25)
}

/// Caches selected by `prune-cache`: legacy files always, BM25 indexes only
/// when the caller opts in explicitly.
pub fn prune_cache_files(cache_dir: &Path, include_bm25: bool) -> Result<Vec<CacheEntry>> {
    let mut files = cache_files_matching(cache_dir, CacheKind::Legacy)?;
    if include_bm25 {
        files.extend(bm25_cache_files(cache_dir)?);
    }
    Ok(files)
}

fn cache_files_matching(cache_dir: &Path, kind: CacheKind) -> Result<Vec<CacheEntry>> {
    if !cache_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(cache_dir)
        .with_context(|| format!("Failed to read cache directory {}", cache_dir.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "Failed to read an entry of cache directory {}",
                cache_dir.display()
            )
        })?;
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if classify_cache_file(&name) == kind {
            files.push(CacheEntry {
                name,
                bytes: metadata.len(),
            });
        }
    }
    Ok(files)
}

pub fn remove_cache_files(cache_dir: &Path, entries: &[CacheEntry]) -> Result<(usize, u64)> {
    let mut removed = 0usize;
    let mut freed = 0u64;
    for entry in entries {
        let path = cache_dir.join(&entry.name);
        fs::remove_file(&path)
            .with_context(|| format!("Failed to remove cache file {}", path.display()))?;
        removed += 1;
        freed += entry.bytes;
    }
    Ok((removed, freed))
}

/// Top-level bytes outside the measured categories. Archive files at the home
/// root are returned separately: they are leftovers from the pre-`cache/`
/// layout that `sivtr doctor --fix` moves.
fn home_remainder(home: &Path) -> Result<(u64, u64)> {
    if !home.exists() {
        return Ok((0, 0));
    }
    let entries = fs::read_dir(home)
        .with_context(|| format!("Failed to read home directory {}", home.display()))?;
    let mut other_bytes = 0u64;
    let mut legacy_archive_bytes = 0u64;
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "Failed to read an entry of home directory {}",
                home.display()
            )
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(name.as_ref(), "cache" | "sets" | "workspaces") {
            continue;
        }
        let bytes = path_bytes(&entry.path())?;
        if matches!(
            name.as_ref(),
            "archive.db" | "archive.db-wal" | "archive.db-shm"
        ) {
            legacy_archive_bytes += bytes;
        } else {
            other_bytes += bytes;
        }
    }
    Ok((other_bytes, legacy_archive_bytes))
}

fn archive_footprint(path: &Path) -> Result<ArchiveFootprint> {
    let database_bytes = path_bytes(path)?;
    let wal_bytes = path_bytes(&sidecar(path, "-wal"))?;
    let shm_bytes = path_bytes(&sidecar(path, "-shm"))?;
    let pragmas = read_archive_pragmas(path);
    let reclaimable_bytes = pragmas.unwrap_or(0);
    Ok(ArchiveFootprint {
        database_bytes,
        wal_bytes,
        shm_bytes,
        reclaimable_bytes,
        available: pragmas.is_some(),
    })
}

fn cache_footprint(cache_dir: &Path) -> Result<CacheFootprint> {
    let mut footprint = CacheFootprint {
        legacy_bytes: 0,
        legacy_files: 0,
        bm25_bytes: 0,
        listing_bytes: 0,
        other_bytes: 0,
    };
    if !cache_dir.exists() {
        return Ok(footprint);
    }
    let entries = fs::read_dir(cache_dir)
        .with_context(|| format!("Failed to read cache directory {}", cache_dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "Failed to read an entry of cache directory {}",
                cache_dir.display()
            )
        })?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // The archive and its sidecars are reported separately.
        if matches!(
            name.as_str(),
            "archive.db" | "archive.db-wal" | "archive.db-shm"
        ) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("Failed to stat cache entry {}", path.display()))?;
        if metadata.is_dir() {
            footprint.other_bytes += path_bytes(&path)?;
            continue;
        }
        match classify_cache_file(&name) {
            CacheKind::Legacy => {
                footprint.legacy_bytes += metadata.len();
                footprint.legacy_files += 1;
            }
            CacheKind::Bm25 => footprint.bm25_bytes += metadata.len(),
            CacheKind::Listing => footprint.listing_bytes += metadata.len(),
            CacheKind::Other => footprint.other_bytes += metadata.len(),
        }
    }
    Ok(footprint)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheKind {
    Legacy,
    Bm25,
    Listing,
    Other,
}

fn classify_cache_file(name: &str) -> CacheKind {
    if is_legacy_cache_name(name) {
        CacheKind::Legacy
    } else if is_hex_cache_name(name, "bm25-", ".bin") {
        CacheKind::Bm25
    } else if is_hex_cache_name(name, "listing-", ".bin") {
        CacheKind::Listing
    } else {
        CacheKind::Other
    }
}

/// Legacy parsed-session caches: `<prefix>-<16 hex>.bin` and the matching
/// `.meta.bin` sidecar. Current builds write `bm25-` and `listing-` only.
fn is_legacy_cache_name(name: &str) -> bool {
    ["agent-", "sess-"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(".bin"))
            .map(|middle| middle.strip_suffix(".meta").unwrap_or(middle))
            .is_some_and(is_hex_16)
    })
}

fn is_hex_cache_name(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
        .is_some_and(is_hex_16)
}

fn is_hex_16(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Reclaimable archive bytes; `None` when the file is missing or cannot be
/// opened read-only.
fn read_archive_pragmas(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).ok()?;
    let free_pages = conn
        .query_row("PRAGMA freelist_count", [], |row| row.get::<_, i64>(0))
        .ok()?;
    let page_size = conn
        .query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))
        .ok()?;
    (free_pages >= 0 && page_size > 0).then(|| (free_pages as u64).saturating_mul(page_size as u64))
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

/// Size of one file or a directory tree. Missing paths count as zero; other
/// I/O errors propagate so reports never present partial data as complete.
fn path_bytes(path: &Path) -> Result<u64> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to stat {}", path.display()));
        }
    };
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let entries = fs::read_dir(path)
        .with_context(|| format!("Failed to read directory {}", path.display()))?;
    let mut total = 0u64;
    for entry in entries {
        let entry =
            entry.with_context(|| format!("Failed to read an entry of {}", path.display()))?;
        total += path_bytes(&entry.path())?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_cache_names_are_recognized() {
        assert!(is_legacy_cache_name("agent-000971a2418f9244.bin"));
        assert!(is_legacy_cache_name("agent-000971a2418f9244.meta.bin"));
        assert!(is_legacy_cache_name("sess-d63b3cbec24f8996.bin"));
        assert!(!is_legacy_cache_name("bm25-000971a2418f9244.bin"));
        assert!(!is_legacy_cache_name("listing-000971a2418f9244.bin"));
        assert!(!is_legacy_cache_name("agent-bak-20260820021602"));
        assert!(!is_legacy_cache_name("agent-notes.bin"));
        assert!(!is_legacy_cache_name("agent-000971A2418F9244.bin"));
    }

    #[test]
    fn cache_footprint_separates_legacy_and_current_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("sess-0123456789abcdef.bin"), b"1234").expect("write legacy");
        fs::write(dir.path().join("bm25-0123456789abcdef.bin"), b"12").expect("write bm25");
        fs::write(dir.path().join("listing-0123456789abcdef.bin"), b"1").expect("write listing");
        fs::create_dir(dir.path().join("agent-bak-20260820021602")).expect("create backup dir");
        fs::write(
            dir.path().join("agent-bak-20260820021602").join("old.bin"),
            b"123",
        )
        .expect("write backup file");

        let footprint = cache_footprint(dir.path()).expect("cache footprint");
        assert_eq!(footprint.legacy_bytes, 4);
        assert_eq!(footprint.legacy_files, 1);
        assert_eq!(footprint.bm25_bytes, 2);
        assert_eq!(footprint.listing_bytes, 1);
        assert_eq!(footprint.other_bytes, 3);
        assert_eq!(footprint.total_bytes(), 10);
    }

    #[test]
    fn prune_lists_without_deleting_and_apply_removes_only_legacy_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let legacy = dir.path().join("sess-0123456789abcdef.bin");
        let legacy_meta = dir.path().join("agent-0123456789abcdef.meta.bin");
        let bm25 = dir.path().join("bm25-0123456789abcdef.bin");
        let listing = dir.path().join("listing-0123456789abcdef.bin");
        let backup_dir = dir.path().join("agent-bak-20260820021602");
        for path in [&legacy, &legacy_meta, &bm25, &listing] {
            fs::write(path, b"x").expect("write cache file");
        }
        fs::create_dir(&backup_dir).expect("create backup dir");

        let entries = prune_cache_files(dir.path(), false).expect("list legacy files");
        assert_eq!(entries.len(), 2);
        assert!(legacy.exists());
        assert!(legacy_meta.exists());

        let (removed, freed) = remove_cache_files(dir.path(), &entries).expect("remove legacy");
        assert_eq!(removed, 2);
        assert_eq!(freed, 2);
        assert!(!legacy.exists());
        assert!(!legacy_meta.exists());
        assert!(bm25.exists());
        assert!(listing.exists());
        assert!(backup_dir.exists());
    }

    #[test]
    fn report_counts_known_categories_and_leaves_the_remainder_as_other() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir(dir.path().join("cache")).expect("create cache");
        fs::create_dir(dir.path().join("sets")).expect("create sets");
        fs::create_dir(dir.path().join("workspaces")).expect("create workspaces");
        fs::write(dir.path().join("cache/sess-0123456789abcdef.bin"), b"1234")
            .expect("write cache");
        fs::write(dir.path().join("cache/bm25-0123456789abcdef.bin"), b"12").expect("write bm25");
        fs::write(dir.path().join("sets/set.json"), b"123").expect("write set");
        fs::write(dir.path().join("workspaces/log.jsonl"), b"12345").expect("write workspace");
        fs::write(dir.path().join("config.toml"), b"1").expect("write config");

        let report = report(dir.path()).expect("storage report");
        assert_eq!(report.total_bytes, 15);
        assert_eq!(report.cache.legacy_bytes, 4);
        assert_eq!(report.cache.bm25_bytes, 2);
        assert_eq!(report.sets_bytes, 3);
        assert_eq!(report.workspaces_bytes, 5);
        assert_eq!(report.other_bytes, 1);
        assert_eq!(report.reclaimable_bytes, 4);
    }

    #[test]
    fn report_separates_legacy_archive_files_at_the_home_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("archive.db"), b"123").expect("write legacy archive");
        fs::write(dir.path().join("archive.db-wal"), b"12").expect("write legacy wal");
        fs::write(dir.path().join("config.toml"), b"1").expect("write config");

        let report = report(dir.path()).expect("storage report");
        assert!(!report.archive.available);
        assert_eq!(report.legacy_archive_bytes, 5);
        assert_eq!(report.other_bytes, 1);
        assert_eq!(report.total_bytes, 6);
    }

    #[test]
    fn archive_footprint_handles_a_missing_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let report = archive_footprint(&dir.path().join("archive.db")).expect("archive footprint");
        assert!(!report.available);
        assert_eq!(report.database_bytes, 0);
        assert_eq!(report.reclaimable_bytes, 0);
    }

    #[test]
    fn vacuum_reclaims_free_pages_in_a_temporary_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("archive.db");
        {
            let conn = Connection::open(&path).expect("open database");
            conn.execute_batch("CREATE TABLE t(x BLOB);")
                .expect("create table");
            for _ in 0..64 {
                conn.execute("INSERT INTO t VALUES (randomblob(4096))", [])
                    .expect("insert blob");
            }
            conn.execute("DELETE FROM t", []).expect("delete rows");
        }
        let before = fs::metadata(&path).expect("stat database").len();
        let report = vacuum(&path, false).expect("vacuum database");
        let after = report.after_bytes.expect("vacuum reports the final size");
        assert!(after < before, "vacuum should reclaim free pages");
        assert_eq!(
            archive_footprint(&path)
                .expect("archive footprint")
                .reclaimable_bytes,
            0
        );
    }

    #[test]
    fn vacuum_truncates_wal_with_an_idle_second_connection() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("archive.db");
        let conn = Connection::open(&path).expect("open database");
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(x BLOB);")
            .expect("create wal database");
        for _ in 0..64 {
            conn.execute("INSERT INTO t VALUES (randomblob(4096))", [])
                .expect("insert blob");
        }
        conn.execute("DELETE FROM t", []).expect("delete rows");
        let idle = Connection::open(&path).expect("open idle connection");
        let before = archive_total_bytes(&path).expect("measure before");
        let report = vacuum(&path, false).expect("vacuum with idle connection");
        let after = report.after_bytes.expect("final size");
        assert!(after < before, "combined archive footprint should shrink");
        assert_eq!(
            path_bytes(&sidecar(&path, "-wal")).expect("measure wal"),
            0,
            "wal should be truncated after vacuum"
        );
        assert_eq!(report.reclaimable_bytes, before - after);
        drop(idle);
        drop(conn);
    }

    #[test]
    fn prune_cache_only_selects_bm25_when_requested() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("sess-0123456789abcdef.bin"), b"1").expect("write legacy");
        fs::write(dir.path().join("bm25-0123456789abcdef.bin"), b"22").expect("write bm25");
        fs::write(dir.path().join("listing-0123456789abcdef.bin"), b"333").expect("write listing");

        let default = prune_cache_files(dir.path(), false).expect("default prune list");
        assert_eq!(default.len(), 1);
        assert_eq!(default[0].name, "sess-0123456789abcdef.bin");

        let with_bm25 = prune_cache_files(dir.path(), true).expect("bm25 prune list");
        assert_eq!(with_bm25.len(), 2);
        assert!(with_bm25
            .iter()
            .any(|entry| entry.name.starts_with("bm25-")));
        assert!(with_bm25
            .iter()
            .all(|entry| !entry.name.starts_with("listing-")));
    }
}
