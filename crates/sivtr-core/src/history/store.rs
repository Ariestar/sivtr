use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

use super::layer::{
    DialogueInfo, InputEntry, LayerPath, OutputEntry, SessionInfo, SourceInfo, WorkspaceInfo,
};
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

/// A history entry stored in the database (y table).
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

    // ── y table: raw history ──

    /// Insert a new history entry into the y table.
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

    // ── i table: input ──

    /// Insert an input entry with full layer path.
    pub fn insert_input(
        &self,
        path: &LayerPath,
        content: &str,
        content_type: &str,
        timestamp: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO input (workspace, source, session_id, dialogue_id, content, content_type, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                path.workspace,
                path.source,
                path.session_id,
                path.dialogue_id,
                content,
                content_type,
                timestamp
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get an input entry by ID.
    pub fn get_input(&self, id: i64) -> Result<Option<InputEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace, source, session_id, dialogue_id, content, content_type, timestamp
             FROM input WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], map_input_row)?;
        match rows.next() {
            Some(Ok(entry)) => Ok(Some(entry)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Search input by FTS across content (ASCII) or LIKE fallback (CJK/mixed).
    pub fn search_input(&self, query: &str, limit: usize) -> Result<Vec<InputEntry>> {
        if query_needs_like_fallback(query) {
            return self.search_input_like(query, limit);
        }
        let fts_query = build_fts_query(query);
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.workspace, i.source, i.session_id, i.dialogue_id, i.content, i.content_type, i.timestamp
             FROM input i
             JOIN input_fts fts ON i.id = fts.rowid
             WHERE input_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let entries = stmt
            .query_map(rusqlite::params![fts_query, limit], map_input_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// Search input filtered by workspace and source.
    pub fn search_input_scoped(
        &self,
        workspace: &str,
        source: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<InputEntry>> {
        if query_needs_like_fallback(query) {
            return self.search_input_scoped_like(workspace, source, query, limit);
        }
        let fts_query = build_fts_query(query);
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.workspace, i.source, i.session_id, i.dialogue_id, i.content, i.content_type, i.timestamp
             FROM input i
             JOIN input_fts fts ON i.id = fts.rowid
             WHERE input_fts MATCH ?1 AND i.workspace = ?2 AND i.source = ?3
             ORDER BY rank
             LIMIT ?4",
        )?;
        let entries = stmt
            .query_map(
                rusqlite::params![fts_query, workspace, source, limit],
                map_input_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn search_input_like(&self, query: &str, limit: usize) -> Result<Vec<InputEntry>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace, source, session_id, dialogue_id, content, content_type, timestamp
             FROM input WHERE content LIKE ?1
             ORDER BY timestamp DESC LIMIT ?2",
        )?;
        let entries = stmt
            .query_map(rusqlite::params![pattern, limit], map_input_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn search_input_scoped_like(
        &self,
        workspace: &str,
        source: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<InputEntry>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace, source, session_id, dialogue_id, content, content_type, timestamp
             FROM input WHERE content LIKE ?1 AND workspace = ?2 AND source = ?3
             ORDER BY timestamp DESC LIMIT ?4",
        )?;
        let entries = stmt
            .query_map(
                rusqlite::params![pattern, workspace, source, limit],
                map_input_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// List input entries for a specific dialogue.
    pub fn list_dialogue_inputs(&self, path: &LayerPath) -> Result<Vec<InputEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace, source, session_id, dialogue_id, content, content_type, timestamp
             FROM input
             WHERE workspace = ?1 AND source = ?2 AND session_id = ?3 AND dialogue_id = ?4
             ORDER BY id",
        )?;
        let entries = stmt
            .query_map(
                rusqlite::params![path.workspace, path.source, path.session_id, path.dialogue_id],
                map_input_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    // ── o table: output ──

    /// Insert an output entry with full layer path.
    pub fn insert_output(
        &self,
        path: &LayerPath,
        content: &str,
        content_type: &str,
        timestamp: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO output (workspace, source, session_id, dialogue_id, content, content_type, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                path.workspace,
                path.source,
                path.session_id,
                path.dialogue_id,
                content,
                content_type,
                timestamp
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get an output entry by ID.
    pub fn get_output(&self, id: i64) -> Result<Option<OutputEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace, source, session_id, dialogue_id, content, content_type, timestamp
             FROM output WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], map_output_row)?;
        match rows.next() {
            Some(Ok(entry)) => Ok(Some(entry)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Search output by FTS across content (ASCII) or LIKE fallback (CJK/mixed).
    pub fn search_output(&self, query: &str, limit: usize) -> Result<Vec<OutputEntry>> {
        if query_needs_like_fallback(query) {
            return self.search_output_like(query, limit);
        }
        let fts_query = build_fts_query(query);
        let mut stmt = self.conn.prepare(
            "SELECT o.id, o.workspace, o.source, o.session_id, o.dialogue_id, o.content, o.content_type, o.timestamp
             FROM output o
             JOIN output_fts fts ON o.id = fts.rowid
             WHERE output_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let entries = stmt
            .query_map(rusqlite::params![fts_query, limit], map_output_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// Search output filtered by workspace and source.
    pub fn search_output_scoped(
        &self,
        workspace: &str,
        source: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<OutputEntry>> {
        if query_needs_like_fallback(query) {
            return self.search_output_scoped_like(workspace, source, query, limit);
        }
        let fts_query = build_fts_query(query);
        let mut stmt = self.conn.prepare(
            "SELECT o.id, o.workspace, o.source, o.session_id, o.dialogue_id, o.content, o.content_type, o.timestamp
             FROM output o
             JOIN output_fts fts ON o.id = fts.rowid
             WHERE output_fts MATCH ?1 AND o.workspace = ?2 AND o.source = ?3
             ORDER BY rank
             LIMIT ?4",
        )?;
        let entries = stmt
            .query_map(
                rusqlite::params![fts_query, workspace, source, limit],
                map_output_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn search_output_like(&self, query: &str, limit: usize) -> Result<Vec<OutputEntry>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace, source, session_id, dialogue_id, content, content_type, timestamp
             FROM output WHERE content LIKE ?1
             ORDER BY timestamp DESC LIMIT ?2",
        )?;
        let entries = stmt
            .query_map(rusqlite::params![pattern, limit], map_output_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn search_output_scoped_like(
        &self,
        workspace: &str,
        source: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<OutputEntry>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace, source, session_id, dialogue_id, content, content_type, timestamp
             FROM output WHERE content LIKE ?1 AND workspace = ?2 AND source = ?3
             ORDER BY timestamp DESC LIMIT ?4",
        )?;
        let entries = stmt
            .query_map(
                rusqlite::params![pattern, workspace, source, limit],
                map_output_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// List output entries for a specific dialogue.
    pub fn list_dialogue_outputs(&self, path: &LayerPath) -> Result<Vec<OutputEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace, source, session_id, dialogue_id, content, content_type, timestamp
             FROM output
             WHERE workspace = ?1 AND source = ?2 AND session_id = ?3 AND dialogue_id = ?4
             ORDER BY id",
        )?;
        let entries = stmt
            .query_map(
                rusqlite::params![path.workspace, path.source, path.session_id, path.dialogue_id],
                map_output_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    // ── Layer traversal queries ──

    /// List distinct workspaces.
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceInfo>> {
        // Use UNION to collect all distinct workspaces, then aggregate per workspace.
        let mut stmt = self.conn.prepare(
            "SELECT
                w.ws as workspace,
                (SELECT count(distinct source) FROM input WHERE workspace = w.ws)
                  + (SELECT count(distinct source) FROM output WHERE workspace = w.ws) as source_count,
                (SELECT count(distinct session_id) FROM input WHERE workspace = w.ws)
                  + (SELECT count(distinct session_id) FROM output WHERE workspace = w.ws) as session_count,
                (SELECT count(distinct dialogue_id) FROM input WHERE workspace = w.ws)
                  + (SELECT count(distinct dialogue_id) FROM output WHERE workspace = w.ws) as dialogue_count,
                coalesce(
                  (SELECT max(timestamp) FROM input WHERE workspace = w.ws),
                  (SELECT max(timestamp) FROM output WHERE workspace = w.ws),
                  ''
                ) as last_active
             FROM (SELECT workspace as ws FROM input
                   UNION SELECT workspace FROM output) w
             ORDER BY last_active DESC",
        )?;
        let entries = stmt
            .query_map([], |row| {
                Ok(WorkspaceInfo {
                    name: row.get(0)?,
                    source_count: row.get(1)?,
                    session_count: row.get(2)?,
                    dialogue_count: row.get(3)?,
                    last_active: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// List sources for a given workspace.
    pub fn list_sources(&self, workspace: &str) -> Result<Vec<SourceInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                source,
                (SELECT count(distinct session_id) FROM input WHERE workspace = ?1 AND source = s.source) as session_count,
                (SELECT count(distinct dialogue_id) FROM input WHERE workspace = ?1 AND source = s.source) as dialogue_count,
                coalesce(
                  (SELECT max(timestamp) FROM input WHERE workspace = ?1 AND source = s.source),
                  ''
                ) as last_active
             FROM (SELECT source FROM input WHERE workspace = ?1
                   UNION SELECT source FROM output WHERE workspace = ?1) s
             ORDER BY last_active DESC",
        )?;
        let entries = stmt
            .query_map(rusqlite::params![workspace], |row| {
                Ok(SourceInfo {
                    name: row.get(0)?,
                    session_count: row.get(1)?,
                    dialogue_count: row.get(2)?,
                    last_active: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// List sessions for a given workspace + source.
    pub fn list_sessions(&self, workspace: &str, source: &str) -> Result<Vec<SessionInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                session_id,
                count(distinct dialogue_id) as dialogue_count,
                min(timestamp) as first_at,
                max(timestamp) as last_at
             FROM input
             WHERE workspace = ?1 AND source = ?2
             GROUP BY session_id
             ORDER BY last_at DESC",
        )?;
        let entries = stmt
            .query_map(rusqlite::params![workspace, source], |row| {
                Ok(SessionInfo {
                    id: row.get(0)?,
                    dialogue_count: row.get(1)?,
                    first_at: row.get(2)?,
                    last_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// List dialogues for a given workspace + source + session.
    pub fn list_dialogues(
        &self,
        workspace: &str,
        source: &str,
        session_id: &str,
    ) -> Result<Vec<DialogueInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                dialogue_id,
                count(*) as input_count,
                0 as output_count,
                min(timestamp) as first_at,
                max(timestamp) as last_at
             FROM input
             WHERE workspace = ?1 AND source = ?2 AND session_id = ?3
             GROUP BY dialogue_id
             ORDER BY last_at DESC",
        )?;
        let mut entries: Vec<DialogueInfo> = stmt
            .query_map(
                rusqlite::params![workspace, source, session_id],
                |row| {
                    Ok(DialogueInfo {
                        id: row.get(0)?,
                        input_count: row.get(1)?,
                        output_count: row.get(2)?,
                        first_at: row.get(3)?,
                        last_at: row.get(4)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Merge output counts
        let mut output_stmt = self.conn.prepare(
            "SELECT dialogue_id, count(*) as output_count
             FROM output
             WHERE workspace = ?1 AND source = ?2 AND session_id = ?3
             GROUP BY dialogue_id",
        )?;
        let output_counts: Vec<(String, i64)> = output_stmt
            .query_map(
                rusqlite::params![workspace, source, session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for entry in &mut entries {
            if let Some((_, count)) = output_counts.iter().find(|(id, _)| *id == entry.id) {
                entry.output_count = *count;
            }
        }

        Ok(entries)
    }

    // ── Helpers ──

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

fn map_input_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InputEntry> {
    Ok(InputEntry {
        id: row.get(0)?,
        workspace: row.get(1)?,
        source: row.get(2)?,
        session_id: row.get(3)?,
        dialogue_id: row.get(4)?,
        content: row.get(5)?,
        content_type: row.get(6)?,
        timestamp: row.get(7)?,
    })
}

fn map_output_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutputEntry> {
    Ok(OutputEntry {
        id: row.get(0)?,
        workspace: row.get(1)?,
        source: row.get(2)?,
        session_id: row.get(3)?,
        dialogue_id: row.get(4)?,
        content: row.get(5)?,
        content_type: row.get(6)?,
        timestamp: row.get(7)?,
    })
}

pub(crate) fn build_fts_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return "\"\"".to_string();
    }

    let terms: Vec<String> = trimmed
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(|term| {
            // Only ASCII terms get phrase-quoted. CJK/mixed terms
            // create false positives with FTS5 unicode61 bigrams,
            // so we fall back to LIKE search for those.
            if term.is_ascii() && !term.contains('"') {
                format!("\"{}\"", term.replace('"', "\"\""))
            } else {
                // Return empty string — caller must use LIKE for this term
                String::new()
            }
        })
        .filter(|term| !term.is_empty())
        .collect();

    if terms.is_empty() {
        "\"\"".to_string()
    } else {
        terms.join(" ")
    }
}

/// Check if a query contains only FTS-safe terms (ASCII, no CJK).
pub(crate) fn query_needs_like_fallback(query: &str) -> bool {
    query.trim().split_whitespace().any(|term| !term.is_ascii())
}
