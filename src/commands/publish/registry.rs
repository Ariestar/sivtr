//! Local publication registry and lifecycle state.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sivtr_core::workspace;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicationStatus {
    Pending,
    Active,
    Revoked,
    Expired,
    Failed,
}

impl PublicationStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for PublicationStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            "expired" => Ok(Self::Expired),
            "failed" => Ok(Self::Failed),
            _ => bail!("unknown publication status `{value}`"),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PublicationRow {
    pub(super) id: String,
    pub(super) endpoint: String,
    pub(super) viewer_key: String,
    pub(super) management_token: String,
    pub(super) title: String,
    pub(super) provider: String,
    pub(super) source_refs: String,
    pub(super) content_sha256: String,
    pub(super) redaction_count: i64,
    pub(super) warning_count: i64,
    pub(super) created_at: String,
    pub(super) expires_at: String,
    pub(super) status: PublicationStatus,
    pub(super) last_error: Option<String>,
}

pub(super) struct PublicationDb {
    connection: Connection,
}

impl PublicationDb {
    pub(super) fn open() -> Result<Self> {
        let dir = workspace::data_dir();
        std::fs::create_dir_all(&dir).context("failed to create publication data directory")?;
        restrict_directory(&dir).context("failed to restrict publication data directory")?;
        let path = dir.join("publication-state.db");
        let connection = Connection::open(&path).context("failed to open publication database")?;
        restrict_file(&path).context("failed to restrict publication database")?;
        Self::from_connection(connection)
    }

    pub(super) fn from_connection(connection: Connection) -> Result<Self> {
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS publications (
                publication_id TEXT PRIMARY KEY,
                endpoint TEXT NOT NULL,
                viewer_key TEXT NOT NULL,
                management_token TEXT NOT NULL,
                title TEXT NOT NULL,
                provider TEXT NOT NULL,
                source_refs TEXT NOT NULL,
                content_sha256 TEXT NOT NULL,
                redaction_count INTEGER NOT NULL,
                warning_count INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                status TEXT NOT NULL,
                last_error TEXT
            );",
            )
            .context("failed to initialize publication database schema")?;
        Ok(Self { connection })
    }

    pub(super) fn insert_pending(&mut self, row: &PublicationRow) -> Result<()> {
        self.connection.execute(
            "INSERT INTO publications (publication_id, endpoint, viewer_key, management_token, title, provider, source_refs, content_sha256, redaction_count, warning_count, created_at, expires_at, status, last_error) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![row.id, row.endpoint, row.viewer_key, row.management_token, row.title, row.provider, row.source_refs, row.content_sha256, row.redaction_count, row.warning_count, row.created_at, row.expires_at, row.status.as_str(), row.last_error],
        )
        .context("failed to insert pending publication")?;
        Ok(())
    }

    fn update_status(
        &self,
        id: &str,
        status: PublicationStatus,
        error: Option<&str>,
    ) -> Result<()> {
        self.connection
            .execute(
                "UPDATE publications SET status = ?1, last_error = ?2 WHERE publication_id = ?3",
                params![status.as_str(), error, id],
            )
            .context("failed to update publication status")?;
        Ok(())
    }

    pub(super) fn mark_active(&self, id: &str) -> Result<()> {
        self.update_status(id, PublicationStatus::Active, None)
    }

    pub(super) fn mark_failed(&self, id: &str, error: &str) -> Result<()> {
        self.update_status(id, PublicationStatus::Failed, Some(error))
    }

    pub(super) fn mark_revoked(&self, id: &str) -> Result<()> {
        self.update_status(id, PublicationStatus::Revoked, None)
    }

    pub(super) fn record_error(&self, id: &str, error: &str) -> Result<()> {
        self.connection
            .execute(
                "UPDATE publications SET last_error = ?1 WHERE publication_id = ?2",
                params![error, id],
            )
            .context("failed to record publication error")?;
        Ok(())
    }

    pub(super) fn find(&self, id: &str) -> Result<Option<PublicationRow>> {
        self.connection
            .query_row(
                "SELECT publication_id, endpoint, viewer_key, management_token, title, provider, source_refs, content_sha256, redaction_count, warning_count, created_at, expires_at, status, last_error FROM publications WHERE publication_id = ?1",
                params![id],
                row_from_query,
            )
            .optional()
            .context("failed to query publication")
    }

    pub(super) fn rows(&self) -> Result<Vec<PublicationRow>> {
        let mut statement = self
            .connection
            .prepare("SELECT publication_id, endpoint, viewer_key, management_token, title, provider, source_refs, content_sha256, redaction_count, warning_count, created_at, expires_at, status, last_error FROM publications ORDER BY created_at DESC")
            .context("failed to prepare publication list query")?;
        let rows = statement
            .query_map([], row_from_query)
            .context("failed to query publication list")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read publication list")?;
        Ok(rows)
    }

    pub(super) fn refresh_expired(&mut self) -> Result<()> {
        let now = Utc::now();
        let rows = self
            .connection
            .prepare("SELECT publication_id, expires_at FROM publications WHERE status IN ('pending', 'active')")
            .context("failed to prepare publication expiry query")?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .context("failed to query publication expiry")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read publication expiry rows")?;
        for (id, expires_at) in rows {
            if is_expired_at(&expires_at, now)? {
                self.connection
                    .execute(
                        "UPDATE publications SET status = 'expired' WHERE publication_id = ?1",
                        params![id],
                    )
                    .context("failed to mark expired publication")?;
            }
        }
        Ok(())
    }
}

pub(super) fn is_expired(value: &str) -> Result<bool> {
    is_expired_at(value, Utc::now())
}

fn is_expired_at(value: &str, now: DateTime<Utc>) -> Result<bool> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc) <= now)
        .with_context(|| format!("invalid publication expiry `{value}`"))
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn row_from_query(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublicationRow> {
    Ok(PublicationRow {
        id: row.get(0)?,
        endpoint: row.get(1)?,
        viewer_key: row.get(2)?,
        management_token: row.get(3)?,
        title: row.get(4)?,
        provider: row.get(5)?,
        source_refs: row.get(6)?,
        content_sha256: row.get(7)?,
        redaction_count: row.get(8)?,
        warning_count: row.get(9)?,
        created_at: row.get(10)?,
        expires_at: row.get(11)?,
        status: row
            .get::<_, String>(12)?
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        last_error: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_expiry_is_reported() {
        assert!(is_expired("not-a-timestamp").is_err());
    }
}
