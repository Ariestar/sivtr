use super::store::{HistoryEntry, HistoryStore};
use anyhow::Result;

impl HistoryStore {
    /// List recent history entries, most recent first.
    pub fn list_recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, command, timestamp, hostname, session_id, source
             FROM history ORDER BY id DESC LIMIT ?1",
        )?;

        let entries = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    command: row.get(2)?,
                    timestamp: row.get(3)?,
                    hostname: row.get(4)?,
                    session_id: row.get(5)?,
                    source: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Full-text search across history content and commands.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT h.id, h.content, h.command, h.timestamp, h.hostname, h.session_id, h.source
             FROM history h
             JOIN history_fts fts ON h.id = fts.rowid
             WHERE history_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;

        let entries = stmt
            .query_map(rusqlite::params![query, limit], |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    command: row.get(2)?,
                    timestamp: row.get(3)?,
                    hostname: row.get(4)?,
                    session_id: row.get(5)?,
                    source: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Get a single history entry by ID.
    pub fn get_by_id(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, command, timestamp, hostname, session_id, source
             FROM history WHERE id = ?1",
        )?;

        let mut entries = stmt.query_map(rusqlite::params![id], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                content: row.get(1)?,
                command: row.get(2)?,
                timestamp: row.get(3)?,
                hostname: row.get(4)?,
                session_id: row.get(5)?,
                source: row.get(6)?,
            })
        })?;

        match entries.next() {
            Some(Ok(entry)) => Ok(Some(entry)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::store::CaptureSource;

    #[test]
    fn test_insert_and_list() {
        let store = HistoryStore::open_memory().unwrap();
        store
            .insert("hello world output", Some("echo hello"), CaptureSource::Run)
            .unwrap();
        store
            .insert(
                "cargo build output",
                Some("cargo build"),
                CaptureSource::Run,
            )
            .unwrap();

        let entries = store.list_recent(10).unwrap();
        assert_eq!(entries.len(), 2);
        // Most recent first
        assert!(entries[0].content.contains("cargo"));
    }

    #[test]
    fn test_fts_search() {
        let store = HistoryStore::open_memory().unwrap();
        store
            .insert(
                "error: cannot find crate",
                Some("cargo build"),
                CaptureSource::Run,
            )
            .unwrap();
        store
            .insert("all tests passed", Some("cargo test"), CaptureSource::Run)
            .unwrap();

        let results = store.search("error", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("error"));
    }

    #[test]
    fn test_get_by_id() {
        let store = HistoryStore::open_memory().unwrap();
        let id = store
            .insert("some output", None, CaptureSource::Pipe)
            .unwrap();

        let entry = store.get_by_id(id).unwrap().unwrap();
        assert_eq!(entry.content, "some output");
        assert_eq!(entry.source, "pipe");
    }
}
