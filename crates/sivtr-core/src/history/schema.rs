use anyhow::Result;
use rusqlite::Connection;

/// Initialize the database schema, creating tables if they don't exist.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        -- y table: raw history captures (existing)
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

        -- Triggers to keep history FTS in sync
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

        -- i table: parsed input entries with 5-layer hierarchy
        -- Layers: workspace -> source -> session -> dialogue -> content
        CREATE TABLE IF NOT EXISTS input (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            workspace TEXT NOT NULL DEFAULT '',
            source TEXT NOT NULL DEFAULT 'terminal',
            session_id TEXT NOT NULL,
            dialogue_id TEXT NOT NULL,
            content TEXT NOT NULL,
            content_type TEXT NOT NULL DEFAULT 'command',
            timestamp TEXT NOT NULL,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_input_workspace ON input(workspace);
        CREATE INDEX IF NOT EXISTS idx_input_source ON input(workspace, source);
        CREATE INDEX IF NOT EXISTS idx_input_session ON input(workspace, source, session_id);
        CREATE INDEX IF NOT EXISTS idx_input_dialogue ON input(workspace, source, session_id, dialogue_id);
        CREATE INDEX IF NOT EXISTS idx_input_timestamp ON input(timestamp);

        CREATE VIRTUAL TABLE IF NOT EXISTS input_fts USING fts5(
            content, content_type, workspace, source,
            content='input', content_rowid='id'
        );

        CREATE TRIGGER IF NOT EXISTS input_ai AFTER INSERT ON input BEGIN
            INSERT INTO input_fts(rowid, content, content_type, workspace, source)
            VALUES (new.id, new.content, new.content_type, new.workspace, new.source);
        END;

        CREATE TRIGGER IF NOT EXISTS input_ad AFTER DELETE ON input BEGIN
            INSERT INTO input_fts(input_fts, rowid, content, content_type, workspace, source)
            VALUES ('delete', old.id, old.content, old.content_type, old.workspace, old.source);
        END;

        CREATE TRIGGER IF NOT EXISTS input_au AFTER UPDATE ON input BEGIN
            INSERT INTO input_fts(input_fts, rowid, content, content_type, workspace, source)
            VALUES ('delete', old.id, old.content, old.content_type, old.workspace, old.source);
            INSERT INTO input_fts(rowid, content, content_type, workspace, source)
            VALUES (new.id, new.content, new.content_type, new.workspace, new.source);
        END;

        -- o table: parsed output entries with 5-layer hierarchy
        CREATE TABLE IF NOT EXISTS output (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            workspace TEXT NOT NULL DEFAULT '',
            source TEXT NOT NULL DEFAULT 'terminal',
            session_id TEXT NOT NULL,
            dialogue_id TEXT NOT NULL,
            content TEXT NOT NULL,
            content_type TEXT NOT NULL DEFAULT 'text',
            timestamp TEXT NOT NULL,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_output_workspace ON output(workspace);
        CREATE INDEX IF NOT EXISTS idx_output_source ON output(workspace, source);
        CREATE INDEX IF NOT EXISTS idx_output_session ON output(workspace, source, session_id);
        CREATE INDEX IF NOT EXISTS idx_output_dialogue ON output(workspace, source, session_id, dialogue_id);
        CREATE INDEX IF NOT EXISTS idx_output_timestamp ON output(timestamp);

        CREATE VIRTUAL TABLE IF NOT EXISTS output_fts USING fts5(
            content, content_type, workspace, source,
            content='output', content_rowid='id'
        );

        CREATE TRIGGER IF NOT EXISTS output_ai AFTER INSERT ON output BEGIN
            INSERT INTO output_fts(rowid, content, content_type, workspace, source)
            VALUES (new.id, new.content, new.content_type, new.workspace, new.source);
        END;

        CREATE TRIGGER IF NOT EXISTS output_ad AFTER DELETE ON output BEGIN
            INSERT INTO output_fts(output_fts, rowid, content, content_type, workspace, source)
            VALUES ('delete', old.id, old.content, old.content_type, old.workspace, old.source);
        END;

        CREATE TRIGGER IF NOT EXISTS output_au AFTER UPDATE ON output BEGIN
            INSERT INTO output_fts(output_fts, rowid, content, content_type, workspace, source)
            VALUES ('delete', old.id, old.content, old.content_type, old.workspace, old.source);
            INSERT INTO output_fts(rowid, content, content_type, workspace, source)
            VALUES (new.id, new.content, new.content_type, new.workspace, new.source);
        END;
        ",
    )?;
    Ok(())
}
