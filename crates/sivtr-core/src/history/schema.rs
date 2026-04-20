use anyhow::Result;
use rusqlite::Connection;

/// Initialize the database schema, creating tables if they don't exist.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            command TEXT,
            timestamp TEXT NOT NULL,
            hostname TEXT NOT NULL,
            session_id TEXT NOT NULL,
            source TEXT NOT NULL,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS history_fts USING fts5(
            content, command,
            content='history', content_rowid='id'
        );

        -- Triggers to keep FTS index in sync
        CREATE TRIGGER IF NOT EXISTS history_ai AFTER INSERT ON history BEGIN
            INSERT INTO history_fts(rowid, content, command)
            VALUES (new.id, new.content, new.command);
        END;

        CREATE TRIGGER IF NOT EXISTS history_ad AFTER DELETE ON history BEGIN
            INSERT INTO history_fts(history_fts, rowid, content, command)
            VALUES ('delete', old.id, old.content, old.command);
        END;

        CREATE TRIGGER IF NOT EXISTS history_au AFTER UPDATE ON history BEGIN
            INSERT INTO history_fts(history_fts, rowid, content, command)
            VALUES ('delete', old.id, old.content, old.command);
            INSERT INTO history_fts(rowid, content, command)
            VALUES (new.id, new.content, new.command);
        END;
        ",
    )?;
    Ok(())
}
