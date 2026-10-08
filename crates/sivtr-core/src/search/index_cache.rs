//! On-disk cache for built BM25 indexes.
//!
//! Building an index re-tokenizes every passage of the corpus (measured:
//! ~3s on a full agent corpus), so a fresh process should reuse the previous
//! process's index instead of rebuilding it. The cache key is the corpus
//! fingerprint ([`crate::cache::records_fingerprint`]) plus an index-version
//! tag; a fingerprint change (new/changed records) or a layout/scoring change
//! naturally misses and rebuilds.
//!
//! Every distinct corpus gets its own file, and a corpus changes whenever a
//! record is added, so superseded files pile up. Storing an index therefore
//! prunes the directory down to the [`MAX_CACHED_INDEXES`] most recently used
//! files; a cache hit refreshes the file's mtime so "recently used" means
//! read or written.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cache::{cache_dir, index_cache_path, records_fingerprint, write_cache_atomic};
use crate::record::WorkRef;
use crate::search::bm25::{Bm25Index, INDEX_CACHE_VERSION};

/// Index files kept on disk. Several corpora are live at once (one per
/// source/workspace/filter combination in use), so this is a small LRU rather
/// than a single slot; evicting a live index only costs one rebuild.
pub const MAX_CACHED_INDEXES: usize = 8;

fn fingerprint(records: &[crate::record::WorkRecord]) -> u64 {
    let refs: Vec<WorkRef> = records
        .iter()
        .map(|record| record.work_ref.whole())
        .collect();
    records_fingerprint(&refs)
}

/// Cache envelope: version tag plus the fingerprint, so a stale file from a
/// different corpus or code revision is rejected before deserialization.
#[derive(Serialize, Deserialize)]
struct CachedIndex {
    version: u32,
    fingerprint: u64,
    index: Bm25Index,
}

/// Load the cached index for a corpus, or `None` on any mismatch — a stale or
/// corrupt cache must never fail the search, only cost a rebuild. Expected
/// misses (no cache file, version/fingerprint mismatch) stay silent; real
/// I/O or decode failures on an existing file are diagnosed so persistent
/// problems (permissions, disk state) surface instead of silently re-building
/// every run.
pub fn load_index(records: &[crate::record::WorkRecord]) -> Option<Bm25Index> {
    let fingerprint = fingerprint(records);
    let path = index_cache_path(fingerprint);
    if !path.exists() {
        return None;
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            crate::diagnostics::warn(format!(
                "failed to read cached BM25 index {}: {error}",
                path.display()
            ));
            return None;
        }
    };
    let cached: CachedIndex = match rmp_serde::from_slice(&bytes) {
        Ok(cached) => cached,
        Err(error) => {
            crate::diagnostics::warn(format!(
                "cached BM25 index {} is corrupt: {error}",
                path.display()
            ));
            return None;
        }
    };
    if cached.version != INDEX_CACHE_VERSION || cached.fingerprint != fingerprint {
        return None;
    }
    // Best-effort: a missed touch only makes the file an earlier eviction
    // candidate.
    if let Err(error) = touch(&path) {
        crate::diagnostics::warn(format!("failed to refresh cached BM25 index: {error:#}"));
    }
    Some(cached.index)
}

/// Mark an index file as just used so pruning keeps it.
fn touch(path: &Path) -> Result<()> {
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open {} for writing", path.display()))?;
    file.set_modified(SystemTime::now())
        .with_context(|| format!("failed to update the modified time of {}", path.display()))
}

/// Whether `path` is named the way [`index_cache_path`] names an index:
/// `bm25-`, 16 lowercase hex digits, `.bin`. Anything else in the cache
/// directory is not ours to prune.
fn is_index_cache_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("bm25-"))
        .and_then(|name| name.strip_suffix(".bin"))
        .is_some_and(|digits| {
            digits.len() == 16
                && digits
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

/// Remove all but the [`MAX_CACHED_INDEXES`] most recently used index files,
/// always keeping `keep` (the one just written). A file another process
/// removed first is not an error. A file that cannot be removed does not stop
/// the rest from being pruned; the first such failure is returned.
fn prune_stale_indexes(keep: &Path) -> Result<()> {
    let dir = cache_dir();
    let entries = std::fs::read_dir(&dir)
        .with_context(|| format!("failed to read cache directory {}", dir.display()))?;
    let mut indexes: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("failed to read an entry of {}", dir.display()))?
            .path();
        if path == keep || !is_index_cache_file(&path) {
            continue;
        }
        match std::fs::metadata(&path).and_then(|metadata| metadata.modified()) {
            Ok(modified) => indexes.push((modified, path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to read the modified time of {}", path.display())
                });
            }
        }
    }
    indexes.sort_by(|left, right| right.cmp(left));
    let mut first_failure = None;
    for (_, path) in indexes.into_iter().skip(MAX_CACHED_INDEXES - 1) {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_failure.get_or_insert(anyhow::Error::new(error).context(format!(
                    "failed to remove stale BM25 index cache {}",
                    path.display()
                )));
            }
        }
    }
    first_failure.map_or(Ok(()), Err)
}

/// Best-effort write of a built index; failures only cost a rebuild next run.
/// Failures are diagnosed (serialize error, write error) because the rebuild
/// they cause is expensive (~3s).
pub fn store_index(records: &[crate::record::WorkRecord], index: &Bm25Index) {
    let cached = CachedIndex {
        version: INDEX_CACHE_VERSION,
        fingerprint: fingerprint(records),
        index: index.clone(),
    };
    let bytes = match rmp_serde::to_vec(&cached) {
        Ok(bytes) => bytes,
        Err(error) => {
            crate::diagnostics::warn(format!("failed to serialize BM25 index cache: {error}"));
            return;
        }
    };
    let path = index_cache_path(cached.fingerprint);
    if !write_cache_atomic(&path, &bytes) {
        crate::diagnostics::warn(format!(
            "failed to write BM25 index cache {}",
            path.display()
        ));
        return;
    }
    // Best-effort: whatever is not pruned now is left for the next store.
    if let Err(error) = prune_stale_indexes(&path) {
        crate::diagnostics::warn(format!("failed to prune BM25 index cache: {error:#}"));
    }
}

/// Build or load the index for a corpus: reuse the cached index when it
/// matches, otherwise build and persist.
pub fn build_or_load(records: &[crate::record::WorkRecord]) -> Bm25Index {
    if let Some(index) = load_index(records) {
        return index;
    }
    let index = Bm25Index::build(records);
    store_index(records, &index);
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{MessageRole, WorkRecord};
    use crate::search::bm25::Bm25Index;
    use crate::test_fixtures::{message_part, terminal_record, EnvGuard};
    use std::time::Duration;

    fn record(session: &str, index: usize, title: &str, text: &str) -> WorkRecord {
        let mut record = terminal_record(session, index, title, "");
        record.parts = vec![message_part(1, MessageRole::Assistant, text)];
        record
    }

    /// Point the cache at a throwaway home: storing an index prunes the cache
    /// directory, which must never be the developer's real one.
    fn temp_home() -> (EnvGuard, tempfile::TempDir) {
        let env = EnvGuard::capture(&["SIVTR_HOME"]);
        let home = tempfile::tempdir().expect("temp home");
        std::env::set_var("SIVTR_HOME", home.path());
        (env, home)
    }

    #[test]
    fn cache_round_trips_a_built_index() {
        let (_env, _home) = temp_home();
        let records = vec![
            record("s1", 1, "cargo install", "building project"),
            record("s1", 2, "run tests", "tests passed"),
        ];
        let built = Bm25Index::build(&records);
        store_index(&records, &built);
        let loaded = load_index(&records).expect("cache hit");
        assert_eq!(loaded, built);
    }

    #[test]
    fn cache_misses_on_different_corpus() {
        let (_env, _home) = temp_home();
        let records = vec![record("s1", 1, "a", "one")];
        let built = Bm25Index::build(&records);
        store_index(&records, &built);
        let other = vec![record("s9", 9, "b", "two")];
        assert!(load_index(&other).is_none());
    }

    fn set_age(path: &Path, seconds_ago: u64) {
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open cache file");
        file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago))
            .expect("set mtime");
    }

    fn index_file_count() -> usize {
        std::fs::read_dir(cache_dir())
            .expect("read cache dir")
            .flatten()
            .filter(|entry| is_index_cache_file(&entry.path()))
            .count()
    }

    #[test]
    fn storing_prunes_least_recently_used_indexes() {
        let (_env, _home) = temp_home();

        let corpora: Vec<Vec<WorkRecord>> = (0..MAX_CACHED_INDEXES + 3)
            .map(|n| vec![record(&format!("s{n}"), 1, "title", "body")])
            .collect();
        let paths: Vec<PathBuf> = corpora
            .iter()
            .map(|records| index_cache_path(fingerprint(records)))
            .collect();
        // Oldest first; each store happens "now", so back-date it afterwards.
        for (n, records) in corpora.iter().enumerate() {
            store_index(records, &Bm25Index::build(records));
            set_age(&paths[n], 1000 - n as u64);
        }
        assert_eq!(index_file_count(), MAX_CACHED_INDEXES);
        for path in &paths[..3] {
            assert!(!path.exists(), "oldest indexes are pruned");
        }

        // A hit on the oldest survivor refreshes it, so the next store evicts
        // the second-oldest instead.
        assert!(load_index(&corpora[3]).is_some());
        let other = cache_dir().join("listing-0000000000000000.bin");
        std::fs::write(&other, b"x").expect("write unrelated cache file");
        set_age(&other, 5000);
        // Starts and ends like an index file but is not one sivtr wrote.
        let lookalike = cache_dir().join("bm25-notes.bin");
        std::fs::write(&lookalike, b"x").expect("write lookalike file");
        set_age(&lookalike, 5000);
        let fresh = vec![record("fresh", 1, "title", "body")];
        store_index(&fresh, &Bm25Index::build(&fresh));

        assert_eq!(index_file_count(), MAX_CACHED_INDEXES);
        assert!(paths[3].exists(), "recently read index survives");
        assert!(!paths[4].exists(), "least recently used index is pruned");
        assert!(index_cache_path(fingerprint(&fresh)).exists());
        assert!(other.exists(), "non-index cache files are left alone");
        assert!(lookalike.exists(), "only generated index names are pruned");
    }
}
