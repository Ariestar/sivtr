use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use sivtr_core::workspace;

const PERMISSION_READ_MEMORY: &str = "read-memory";

#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareInfo {
    pub id: String,
    pub name: String,
    pub workspace_key: String,
    pub root: String,
    pub enabled: bool,
    pub redact: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantInfo {
    pub peer_id: String,
    pub peer_name: String,
    pub share_id: String,
    pub share_name: String,
    pub permission: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountInfo {
    pub workspace_key: String,
    pub alias: String,
    pub peer_id: String,
    pub peer_name: String,
    pub share_id: String,
    pub share_name: String,
}

#[derive(Debug, Clone)]
pub struct InviteRecord {
    pub id: String,
    pub share_id: String,
    pub share_name: String,
    pub secret: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct RedeemedShare {
    pub share_id: String,
    pub share_name: String,
}

impl StateStore {
    pub fn open_default() -> Result<Self> {
        Self::open(workspace::data_dir().join("remote-state.db"))
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
            restrict_directory(parent)?;
        }
        let store = Self { path };
        store.initialize()?;
        Ok(store)
    }

    fn connect(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)
            .with_context(|| format!("Failed to open {}", self.path.display()))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(connection)
    }

    fn initialize(&self) -> Result<()> {
        self.connect()?.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS peers (
                id              TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                endpoint_json   TEXT,
                created_at      TEXT NOT NULL,
                last_seen_at    TEXT
            );

            CREATE TABLE IF NOT EXISTS shares (
                id              TEXT PRIMARY KEY,
                name            TEXT NOT NULL UNIQUE,
                workspace_key   TEXT NOT NULL UNIQUE,
                root            TEXT NOT NULL,
                enabled         INTEGER NOT NULL,
                redact          INTEGER NOT NULL,
                created_at      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS grants (
                peer_id         TEXT NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
                share_id        TEXT NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
                permission      TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                revoked_at      TEXT,
                PRIMARY KEY(peer_id, share_id)
            );

            CREATE TABLE IF NOT EXISTS invites (
                id              TEXT PRIMARY KEY,
                share_id        TEXT NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
                secret_hash     BLOB NOT NULL,
                permission      TEXT NOT NULL,
                expires_at      INTEGER NOT NULL,
                used_at         TEXT,
                created_at      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS mounts (
                workspace_key   TEXT NOT NULL,
                alias           TEXT NOT NULL,
                peer_id         TEXT NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
                share_id        TEXT NOT NULL,
                share_name      TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                PRIMARY KEY(workspace_key, alias),
                UNIQUE(workspace_key, peer_id, share_id)
            );

            CREATE TABLE IF NOT EXISTS audit_events (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                occurred_at     TEXT NOT NULL,
                peer_id         TEXT NOT NULL,
                share_id        TEXT NOT NULL,
                action          TEXT NOT NULL,
                decision        TEXT NOT NULL,
                reason          TEXT
            );
            "#,
        )?;
        Ok(())
    }

    pub fn add_share(
        &self,
        workspace_key: &str,
        root: &Path,
        name: &str,
        redact: bool,
    ) -> Result<ShareInfo> {
        validate_identifier(name, "share name")?;
        let root = root
            .canonicalize()
            .with_context(|| format!("Failed to resolve workspace {}", root.display()))?;
        let id = random_id("sh");
        let created_at = now();
        let connection = self.connect()?;
        connection
            .execute(
                "INSERT INTO shares(id, name, workspace_key, root, enabled, redact, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)",
                params![id, name, workspace_key, root.to_string_lossy(), redact, created_at],
            )
            .with_context(|| format!("Share `{name}` or this workspace already exists"))?;
        self.share(&id)
    }

    pub fn shares(&self) -> Result<Vec<ShareInfo>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, name, workspace_key, root, enabled, redact, created_at FROM shares ORDER BY name",
        )?;
        let rows = statement.query_map([], share_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn share(&self, name_or_id: &str) -> Result<ShareInfo> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT id, name, workspace_key, root, enabled, redact, created_at FROM shares WHERE id = ?1 OR name = ?1",
                [name_or_id],
                share_from_row,
            )
            .optional()?
            .with_context(|| format!("Unknown share `{name_or_id}`"))
    }

    pub fn set_share_enabled(&self, name_or_id: &str, enabled: bool) -> Result<ShareInfo> {
        let share = self.share(name_or_id)?;
        self.connect()?.execute(
            "UPDATE shares SET enabled = ?1 WHERE id = ?2",
            params![enabled, share.id],
        )?;
        self.share(&share.id)
    }

    pub fn remove_share(&self, name_or_id: &str) -> Result<ShareInfo> {
        let share = self.share(name_or_id)?;
        self.connect()?
            .execute("DELETE FROM shares WHERE id = ?1", [&share.id])?;
        Ok(share)
    }

    pub fn create_invite(&self, name_or_id: &str, valid_for_seconds: i64) -> Result<InviteRecord> {
        if valid_for_seconds <= 0 {
            bail!("Invite expiration must be positive");
        }
        let share = self.share(name_or_id)?;
        if !share.enabled {
            bail!("Share `{}` is disabled", share.name);
        }
        let id = random_id("iv");
        let secret = random_secret();
        let expires_at = Utc::now().timestamp() + valid_for_seconds;
        self.connect()?.execute(
            "INSERT INTO invites(id, share_id, secret_hash, permission, expires_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, share.id, hash_secret(&secret), PERMISSION_READ_MEMORY, expires_at, now()],
        )?;
        Ok(InviteRecord {
            id,
            share_id: share.id,
            share_name: share.name,
            secret,
            expires_at,
        })
    }

    pub fn redeem_invite(
        &self,
        invite_id: &str,
        secret: &str,
        peer_id: &str,
        peer_name: &str,
    ) -> Result<RedeemedShare> {
        validate_identifier(peer_name, "peer name")?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let invite = transaction
            .query_row(
                "SELECT i.share_id, s.name, i.secret_hash, i.expires_at, i.used_at, s.enabled FROM invites i JOIN shares s ON s.id = i.share_id WHERE i.id = ?1",
                [invite_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, bool>(5)?,
                    ))
                },
            )
            .optional()?
            .context("Invitation is invalid or expired")?;
        let (share_id, share_name, expected_hash, expires_at, used_at, enabled) = invite;
        if used_at.is_some() || expires_at < Utc::now().timestamp() || !enabled {
            bail!("Invitation is invalid or expired");
        }
        if expected_hash != hash_secret(secret) {
            bail!("Invitation is invalid or expired");
        }
        let timestamp = now();
        transaction.execute(
            "INSERT INTO peers(id, name, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?3) ON CONFLICT(id) DO UPDATE SET name = excluded.name, last_seen_at = excluded.last_seen_at",
            params![peer_id, peer_name, timestamp],
        )?;
        transaction.execute(
            "INSERT INTO grants(peer_id, share_id, permission, created_at, revoked_at) VALUES (?1, ?2, ?3, ?4, NULL) ON CONFLICT(peer_id, share_id) DO UPDATE SET permission = excluded.permission, revoked_at = NULL",
            params![peer_id, share_id, PERMISSION_READ_MEMORY, timestamp],
        )?;
        transaction.execute(
            "UPDATE invites SET used_at = ?1 WHERE id = ?2 AND used_at IS NULL",
            params![timestamp, invite_id],
        )?;
        transaction.commit()?;
        Ok(RedeemedShare {
            share_id,
            share_name,
        })
    }

    pub fn authorize(&self, peer_id: &str, share_id: &str, action: &str) -> Result<ShareInfo> {
        let connection = self.connect()?;
        let share = connection
            .query_row(
                "SELECT s.id, s.name, s.workspace_key, s.root, s.enabled, s.redact, s.created_at FROM shares s JOIN grants g ON g.share_id = s.id WHERE s.id = ?1 AND g.peer_id = ?2 AND g.permission = ?3 AND g.revoked_at IS NULL AND s.enabled = 1",
                params![share_id, peer_id, PERMISSION_READ_MEMORY],
                share_from_row,
            )
            .optional()?;
        match share {
            Some(share) => {
                self.audit(peer_id, share_id, action, "allow", None)?;
                Ok(share)
            }
            None => {
                self.audit(peer_id, share_id, action, "deny", Some("share unavailable"))?;
                bail!("share unavailable")
            }
        }
    }

    pub fn save_remote_peer(
        &self,
        peer_id: &str,
        peer_name: &str,
        endpoint_json: &str,
    ) -> Result<()> {
        let timestamp = now();
        self.connect()?.execute(
            "INSERT INTO peers(id, name, endpoint_json, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4) ON CONFLICT(id) DO UPDATE SET name = excluded.name, endpoint_json = excluded.endpoint_json, last_seen_at = excluded.last_seen_at",
            params![peer_id, peer_name, endpoint_json, timestamp],
        )?;
        Ok(())
    }

    pub fn peer_endpoint(&self, peer_id: &str) -> Result<String> {
        self.connect()?
            .query_row(
                "SELECT endpoint_json FROM peers WHERE id = ?1",
                [peer_id],
                |row| row.get::<_, Option<String>>(0),
            )?
            .context("Remote peer has no known endpoint")
    }

    pub fn peers(&self) -> Result<Vec<PeerInfo>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT id, name, created_at, last_seen_at FROM peers ORDER BY name, id")?;
        let rows = statement.query_map([], |row| {
            Ok(PeerInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                last_seen_at: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn forget_peer(&self, name_or_id: &str) -> Result<PeerInfo> {
        let peer = self.peer(name_or_id)?;
        self.connect()?
            .execute("DELETE FROM peers WHERE id = ?1", [&peer.id])?;
        Ok(peer)
    }

    pub fn add_mount(
        &self,
        workspace_key: &str,
        alias: &str,
        peer_id: &str,
        share_id: &str,
        share_name: &str,
    ) -> Result<MountInfo> {
        validate_alias(alias, "remote alias")?;
        self.connect()
            .and_then(|connection| {
                connection.execute(
                    "INSERT INTO mounts(workspace_key, alias, peer_id, share_id, share_name, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![workspace_key, alias.to_ascii_lowercase(), peer_id, share_id, share_name, now()],
                )?;
                Ok(())
            })
            .with_context(|| format!("Remote alias `{alias}` or this remote share already exists in the workspace"))?;
        self.mount(workspace_key, alias)
    }

    pub fn mounts(&self, workspace_key: &str) -> Result<Vec<MountInfo>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT m.workspace_key, m.alias, m.peer_id, p.name, m.share_id, m.share_name FROM mounts m JOIN peers p ON p.id = m.peer_id WHERE m.workspace_key = ?1 ORDER BY m.alias",
        )?;
        let rows = statement.query_map([workspace_key], mount_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn mount(&self, workspace_key: &str, alias: &str) -> Result<MountInfo> {
        self.connect()?
            .query_row(
                "SELECT m.workspace_key, m.alias, m.peer_id, p.name, m.share_id, m.share_name FROM mounts m JOIN peers p ON p.id = m.peer_id WHERE m.workspace_key = ?1 AND m.alias = ?2",
                params![workspace_key, alias.to_ascii_lowercase()],
                mount_from_row,
            )
            .optional()?
            .with_context(|| format!("Unknown remote `{alias}` in this workspace"))
    }

    pub fn remove_mount(&self, workspace_key: &str, alias: &str) -> Result<MountInfo> {
        let mount = self.mount(workspace_key, alias)?;
        self.connect()?.execute(
            "DELETE FROM mounts WHERE workspace_key = ?1 AND alias = ?2",
            params![workspace_key, mount.alias],
        )?;
        Ok(mount)
    }

    pub fn rename_mount(
        &self,
        workspace_key: &str,
        alias: &str,
        new_alias: &str,
    ) -> Result<MountInfo> {
        validate_alias(new_alias, "remote alias")?;
        let mount = self.mount(workspace_key, alias)?;
        self.connect()?.execute(
            "UPDATE mounts SET alias = ?1 WHERE workspace_key = ?2 AND alias = ?3",
            params![new_alias.to_ascii_lowercase(), workspace_key, mount.alias],
        )?;
        self.mount(workspace_key, new_alias)
    }

    pub fn grants(&self, share_name_or_id: &str) -> Result<Vec<GrantInfo>> {
        let share = self.share(share_name_or_id)?;
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT g.peer_id, p.name, g.share_id, s.name, g.permission, g.created_at FROM grants g JOIN peers p ON p.id = g.peer_id JOIN shares s ON s.id = g.share_id WHERE g.share_id = ?1 AND g.revoked_at IS NULL ORDER BY p.name",
        )?;
        let rows = statement.query_map([share.id], |row| {
            Ok(GrantInfo {
                peer_id: row.get(0)?,
                peer_name: row.get(1)?,
                share_id: row.get(2)?,
                share_name: row.get(3)?,
                permission: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn revoke(&self, share_name_or_id: &str, peer_name_or_id: &str) -> Result<GrantInfo> {
        let share = self.share(share_name_or_id)?;
        let peer = self.peer(peer_name_or_id)?;
        let grant = self
            .grants(&share.id)?
            .into_iter()
            .find(|grant| grant.peer_id == peer.id)
            .with_context(|| {
                format!(
                    "Peer `{}` has no active grant for `{}`",
                    peer.name, share.name
                )
            })?;
        self.connect()?.execute(
            "UPDATE grants SET revoked_at = ?1 WHERE peer_id = ?2 AND share_id = ?3",
            params![now(), peer.id, share.id],
        )?;
        Ok(grant)
    }

    fn peer(&self, name_or_id: &str) -> Result<PeerInfo> {
        self.connect()?
            .query_row(
                "SELECT id, name, created_at, last_seen_at FROM peers WHERE id = ?1 OR name = ?1",
                [name_or_id],
                |row| {
                    Ok(PeerInfo {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        created_at: row.get(2)?,
                        last_seen_at: row.get(3)?,
                    })
                },
            )
            .optional()?
            .with_context(|| format!("Unknown peer `{name_or_id}`"))
    }

    fn audit(
        &self,
        peer_id: &str,
        share_id: &str,
        action: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO audit_events(occurred_at, peer_id, share_id, action, decision, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now(), peer_id, share_id, action, decision, reason],
        )?;
        Ok(())
    }
}

fn share_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShareInfo> {
    Ok(ShareInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        workspace_key: row.get(2)?,
        root: row.get(3)?,
        enabled: row.get(4)?,
        redact: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn mount_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MountInfo> {
    Ok(MountInfo {
        workspace_key: row.get(0)?,
        alias: row.get(1)?,
        peer_id: row.get(2)?,
        peer_name: row.get(3)?,
        share_id: row.get(4)?,
        share_name: row.get(5)?,
    })
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        bail!("{label} must be [a-zA-Z0-9_-]+");
    }
    Ok(())
}

fn validate_alias(value: &str, label: &str) -> Result<()> {
    validate_identifier(value, label)?;
    if value.eq_ignore_ascii_case("local") || value.eq_ignore_ascii_case("sivtr") {
        bail!("{label} must not be reserved scheme name `local` or `sivtr`");
    }
    Ok(())
}

fn random_id(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("OS RNG unavailable");
    format!("{prefix}_{}", hex(&bytes))
}

fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS RNG unavailable");
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

fn hash_secret(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_is_single_use_and_grant_is_share_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        let share = store
            .add_share("workspace-key", &workspace, "project", true)
            .unwrap();
        let invite = store.create_invite("project", 60).unwrap();

        let redeemed = store
            .redeem_invite(&invite.id, &invite.secret, "peer-1", "alice")
            .unwrap();
        assert_eq!(redeemed.share_id, share.id);
        assert!(store
            .redeem_invite(&invite.id, &invite.secret, "peer-2", "bob")
            .is_err());
        assert_eq!(
            store.authorize("peer-1", &share.id, "source").unwrap().id,
            share.id
        );
        assert!(store.authorize("peer-2", &share.id, "source").is_err());
    }

    #[test]
    fn mounts_are_scoped_to_local_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        store.save_remote_peer("peer-1", "alice", "{}").unwrap();
        store
            .add_mount("workspace-a", "desk", "peer-1", "share-a", "project-a")
            .unwrap();

        assert!(store.mount("workspace-a", "desk").is_ok());
        assert!(store.mount("workspace-b", "desk").is_err());
    }

    #[test]
    fn share_name_can_be_sivtr_but_alias_cannot() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        store
            .add_share("workspace-key", &workspace, "sivtr", true)
            .expect("share name sivtr should be allowed");
        store.save_remote_peer("peer-1", "alice", "{}").unwrap();
        assert!(store
            .add_mount("workspace-a", "sivtr", "peer-1", "share-a", "project-a")
            .is_err());
        assert!(store
            .add_mount("workspace-a", "local", "peer-1", "share-a", "project-a")
            .is_err());
    }
}
