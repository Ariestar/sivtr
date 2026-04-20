use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

use super::schema;

/// Capture source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSource {
    Run,
    Pipe,
    Import,
}

impl std::fmt::Display for CaptureSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureSource::Run => write!(f, "run"),
            CaptureSource::Pipe => write!(f, "pipe"),
            CaptureSource::Import => write!(f, "import"),
        }
    }
}

/// A history entry stored in the database.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub content: String,
    pub command: Option<String>,
    pub timestamp: String,
    pub hostname: String,
    pub session_id: String,
    pub source: String,
}

/// Manages the SQLite history database.
pub struct HistoryStore {
    pub(crate) conn: Connection,
}

impl HistoryStore {
    /// Open or create the history database at the default location.
    pub fn open_default() -> Result<Self> {
        let path = Self::default_db_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::open(&path)
    }

    /// Open or create the history database at a specific path.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        schema::init_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::init_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Insert a new history entry.
    pub fn insert(
        &self,
        content: &str,
        command: Option<&str>,
        source: CaptureSource,
    ) -> Result<i64> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unknown".to_string());
        let session_id = uuid::Uuid::new_v4().to_string();

        self.conn.execute(
            "INSERT INTO history (content, command, timestamp, hostname, session_id, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                content,
                command,
                timestamp,
                hostname,
                session_id,
                source.to_string()
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Get default database path.
    fn default_db_path() -> Result<PathBuf> {
        let data_dir =
            dirs::data_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine data directory"))?;
        let current = data_dir.join("sivtr").join("history.db");
        if current.exists() {
            return Ok(current);
        }

        let legacy = data_dir.join("sift").join("history.db");
        if legacy.exists() {
            return Ok(legacy);
        }

        Ok(current)
    }
}
