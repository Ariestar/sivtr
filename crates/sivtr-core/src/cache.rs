//! Shared on-disk cache helpers for parsed agent data.
//!
//! Both caches use the same stamp-validated pattern: a payload is stored
//! alongside the source file's `(mtime, size)` fingerprint, and reads only
//! pay the cost of re-parsing when the fingerprint no longer matches. The
//! session record cache ([`crate::query`]) applies it to parse output; the
//! listing cache ([`crate::agents::jsonl`]) applies it one level up so that
//! session discovery is a stat sweep instead of a read + parse of every file.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::agents::AgentProvider;

/// Cache root under the platform data dir (`SIVTR_DATA_DIR` override).
pub fn cache_dir() -> PathBuf {
    crate::workspace::data_dir().join("cache")
}

/// `(mtime secs, mtime nanos, size)` fingerprint of a file; `None` when the
/// file cannot be stamped (missing, unreadable, clock before epoch).
pub fn file_stamp(path: &Path) -> Option<(u64, u32, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let duration = meta
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?;
    Some((duration.as_secs(), duration.subsec_nanos(), meta.len()))
}

/// Cache file for one parsed agent session, keyed by provider + session path.
pub fn session_cache_path(provider: AgentProvider, path: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    provider.hash(&mut hasher);
    path.hash(&mut hasher);
    cache_dir().join(format!("agent-{:016x}.bin", hasher.finish()))
}

/// Cache file for a session listing, keyed by provider name + root directory.
pub fn listing_cache_path(provider: &str, root: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    provider.hash(&mut hasher);
    root.hash(&mut hasher);
    cache_dir().join(format!("listing-{:016x}.bin", hasher.finish()))
}

/// Best-effort atomic write (tmp + rename, same volume); callers ignore
/// failures because a stale cache only costs a re-parse on the next run.
pub fn write_cache_atomic(target: &Path, bytes: &[u8]) {
    let _ = std::fs::create_dir_all(cache_dir());
    let tmp = target.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, target);
    }
}
