use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Open a SQLite database read-only for agent session stores.
pub fn open_readonly_db(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("Failed to open agent database {}", path.display()))
}

/// Convert a millisecond epoch timestamp into `SystemTime`.
pub fn system_time_from_millis(value: i64) -> SystemTime {
    if value <= 0 {
        return UNIX_EPOCH;
    }
    UNIX_EPOCH + Duration::from_millis(value as u64)
}
