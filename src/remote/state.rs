use std::path::{Path, PathBuf};
use std::str::FromStr;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInfo {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub created_at: String,
}

/// A member's role inside a group. Stored as TEXT in SQLite and carried as a
/// lowercase string on the wire; typed here so role comparisons cannot
/// silently drift (`"owner"` vs `"Owner"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupRole {
    Owner,
    Member,
}

impl GroupRole {
    pub fn is_owner(self) -> bool {
        matches!(self, Self::Owner)
    }

    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
        }
    }
}

impl std::str::FromStr for GroupRole {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "owner" => Ok(Self::Owner),
            "member" => Ok(Self::Member),
            _ => bail!("Unknown group role `{value}`"),
        }
    }
}

impl rusqlite::types::FromSql for GroupRole {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let text = value.as_str()?;
        text.parse()
            .map_err(|_| rusqlite::types::FromSqlError::InvalidType)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMemberInfo {
    pub peer_id: String,
    pub peer_name: String,
    pub role: GroupRole,
    pub joined_at: String,
    pub last_seen_at: Option<String>,
    /// Number of workspaces this member contributes to the group.
    pub share_count: i64,
    /// Stored endpoint JSON (iroh `EndpointAddr`); dialable members have one.
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupShareInfo {
    pub share_id: String,
    pub share_name: String,
    pub added_at: String,
}

/// One roster row as it converges into the store: peer identity, role, and
/// contributed shares, with the wire endpoint already serialized.
#[derive(Debug, Clone)]
pub struct RosterRow {
    pub peer_id: String,
    pub peer_name: String,
    pub role: String,
    pub shares: Vec<(String, String)>,
    pub endpoint_json: String,
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

/// A redeemed group invite: the authoritative group id (read from the invite
/// row, never from the joiner's request) plus the post-join roster for the
/// joiner to mirror.
#[derive(Debug)]
pub struct RedeemedGroup {
    pub group_id: String,
    pub roster: Vec<GroupMemberInfo>,
}

/// A peer joining a group, as seen by the group owner. `shares` lists every
/// workspace the joiner contributes (multi-select join).
#[derive(Debug)]
pub struct JoinerInfo<'a> {
    pub peer_id: &'a str,
    pub peer_name: &'a str,
    pub shares: &'a [(String, String)],
    pub endpoint_json: &'a str,
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

            -- One row per (peer, share) authorization source. `grants` keeps a
            -- single row regardless of how many sources issued it, so each
            -- group grant and each direct redeem records its own source here;
            -- a grant row survives until its last source is removed.
            CREATE TABLE IF NOT EXISTS grant_sources (
                share_id    TEXT NOT NULL,
                peer_id     TEXT NOT NULL,
                via         TEXT NOT NULL CHECK (via IN ('group', 'direct')),
                group_id    TEXT,
                created_at  TEXT NOT NULL,
                PRIMARY KEY(share_id, peer_id, via, group_id)
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

            CREATE TABLE IF NOT EXISTS groups (
                id              TEXT PRIMARY KEY,
                name            TEXT NOT NULL UNIQUE,
                last_synced_at  TEXT,
                created_at      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS group_members (
                group_id    TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                peer_id     TEXT NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
                role        TEXT NOT NULL DEFAULT 'member',
                joined_at   TEXT NOT NULL,
                PRIMARY KEY(group_id, peer_id)
            );

            -- One row per (member, contributed share); a member contributes
            -- many workspaces. `share_id` intentionally has NO foreign key:
            -- mirrored members' shares live on their own devices. Deleting a
            -- local share cleans its contribution rows in `remove_share`.
            CREATE TABLE IF NOT EXISTS group_shares (
                group_id    TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                peer_id     TEXT NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
                share_id    TEXT NOT NULL,
                share_name  TEXT NOT NULL,
                added_at    TEXT NOT NULL,
                PRIMARY KEY(group_id, peer_id, share_id)
            );
            "#,
        )?;
        // Idempotent migration for installs that predate group invites.
        // `CREATE TABLE IF NOT EXISTS` cannot add columns, so ALTER and ignore
        // the "duplicate column name" error on already-migrated databases.
        let connection = self.connect()?;
        for statement in [
            "ALTER TABLE invites ADD COLUMN kind TEXT NOT NULL DEFAULT 'share'",
            "ALTER TABLE invites ADD COLUMN group_id TEXT",
            "ALTER TABLE invites ADD COLUMN max_uses INTEGER",
            "ALTER TABLE invites ADD COLUMN used_count INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE groups ADD COLUMN roster_epoch INTEGER NOT NULL DEFAULT 0",
        ] {
            // Only the expected "column already exists" failure is benign for
            // an idempotent ALTER; anything else is a real migration error.
            match connection.execute(statement, []) {
                Ok(_) => {}
                Err(error) if Self::is_duplicate_column(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }
        // Migrate the pre-split `group_members` (one share per member) into the
        // member/share split: contributions move to `group_shares`, the member
        // table keeps only membership + role.
        let columns: Vec<String> = connection
            .prepare("PRAGMA table_info(group_members)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if columns.iter().any(|column| column == "share_id") {
            connection.execute_batch(
                r#"
                INSERT INTO group_shares(group_id, peer_id, share_id, share_name, added_at)
                    SELECT group_id, peer_id, share_id, share_name, joined_at
                    FROM group_members WHERE share_id IS NOT NULL AND share_id != '';
                ALTER TABLE group_members RENAME TO group_members_old;
                CREATE TABLE group_members (
                    group_id    TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                    peer_id     TEXT NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
                    role        TEXT NOT NULL DEFAULT 'member',
                    joined_at   TEXT NOT NULL,
                    PRIMARY KEY(group_id, peer_id)
                );
                INSERT INTO group_members(group_id, peer_id, role, joined_at)
                    SELECT group_id, peer_id, role, joined_at FROM group_members_old;
                DROP TABLE group_members_old;
                "#,
            )?;
        }
        // Rebuild `group_shares` without the share_id foreign key: mirrored
        // members' shares live on their own devices, so they never exist in
        // the local shares table and a FK would reject them. Only the obsolete
        // `share_id` FK triggers the rebuild — the `group_id`/`peer_id` FKs are
        // intentional, so counting every FK would re-run this migration on
        // every startup.
        let share_fks: i64 = connection.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('group_shares') WHERE \"from\" = 'share_id'",
            [],
            |row| row.get(0),
        )?;
        if share_fks > 0 {
            connection.execute_batch(
                r#"
                ALTER TABLE group_shares RENAME TO group_shares_old;
                CREATE TABLE group_shares (
                    group_id    TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                    peer_id     TEXT NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
                    share_id    TEXT NOT NULL,
                    share_name  TEXT NOT NULL,
                    added_at    TEXT NOT NULL,
                    PRIMARY KEY(group_id, peer_id, share_id)
                );
                INSERT INTO group_shares(group_id, peer_id, share_id, share_name, added_at)
                    SELECT group_id, peer_id, share_id, share_name, added_at FROM group_shares_old;
                DROP TABLE group_shares_old;
                "#,
            )?;
        }
        Ok(())
    }

    /// True for the only benign `ALTER TABLE ADD COLUMN` failure on an
    /// already-migrated database: the column already exists.
    fn is_duplicate_column(error: &rusqlite::Error) -> bool {
        matches!(
            error,
            rusqlite::Error::SqliteFailure(_, Some(message))
                if message.contains("duplicate column name")
        )
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
        // A share that is still contributed to a group cannot be deleted: the
        // group's roster elsewhere keeps dialing it and a later sync would
        // resurrect the stale contribution. Withdraw it from the group first.
        let groups: Vec<String> = self
            .connect()?
            .prepare(
                "SELECT g.name FROM group_shares gs JOIN groups g ON g.id = gs.group_id WHERE gs.share_id = ?1 ORDER BY g.name",
            )?
            .query_map([&share.id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !groups.is_empty() {
            bail!(
                "Share `{}` is contributed to group(s) {}; withdraw it from the group(s) first",
                share.name,
                groups.join(", ")
            );
        }
        let connection = self.connect()?;
        connection.execute("DELETE FROM group_shares WHERE share_id = ?1", [&share.id])?;
        connection.execute("DELETE FROM shares WHERE id = ?1", [&share.id])?;
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
        // Only share tickets are redeemable here; a group ticket carries an
        // owner share id and must go through `redeem_group_invite`, which
        // joins the roster and grants the whole mesh instead of a direct grant.
        let invite = transaction
            .query_row(
                "SELECT i.share_id, s.name, i.secret_hash, i.expires_at, i.used_at, s.enabled FROM invites i JOIN shares s ON s.id = i.share_id WHERE i.id = ?1 AND i.kind = 'share'",
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
        // A direct redeem is its own grant source, independent of any group:
        // withdrawing the same share from a group later must keep this grant.
        transaction.execute(
            "INSERT INTO grant_sources(share_id, peer_id, via, group_id, created_at) VALUES (?1, ?2, 'direct', NULL, ?3) ON CONFLICT DO NOTHING",
            params![share_id, peer_id, timestamp],
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

    /// Refresh reachability hints for a known peer. Identity (id) is stable; addresses expire.
    pub fn refresh_peer_endpoint(&self, peer_id: &str, endpoint_json: &str) -> Result<()> {
        let updated = self.connect()?.execute(
            "UPDATE peers SET endpoint_json = ?1, last_seen_at = ?2 WHERE id = ?3",
            params![endpoint_json, now(), peer_id],
        )?;
        if updated == 0 {
            bail!("Unknown peer `{peer_id}`");
        }
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
        // group_members cascades on peers(id): forgetting a peer that still
        // participates in a group would silently delete the membership and
        // leave the group without an owner, unreachable by roster sync.
        let groups: Vec<String> = self
            .connect()?
            .prepare(
                "SELECT g.name FROM group_members gm JOIN groups g ON g.id = gm.group_id WHERE gm.peer_id = ?1 ORDER BY g.name",
            )?
            .query_map([&peer.id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !groups.is_empty() {
            bail!(
                "Peer `{}` participates in group(s) {}; remove it from the group(s) before forgetting it",
                peer.name,
                groups.join(", ")
            );
        }
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

    /// Revoke a grant, returning it when one was active.
    ///
    /// Revoking a grant that does not exist is an idempotent no-op (`Ok(None)`):
    /// the group paths rely on it, since kick/leave must not fail just because
    /// a peer already lost access. Every other failure (missing share/peer,
    /// database error) is propagated.
    pub fn revoke(
        &self,
        share_name_or_id: &str,
        peer_name_or_id: &str,
    ) -> Result<Option<GrantInfo>> {
        let share = self.share(share_name_or_id)?;
        let peer = self.peer(peer_name_or_id)?;
        let grant = self
            .grants(&share.id)?
            .into_iter()
            .find(|grant| grant.peer_id == peer.id);
        let Some(grant) = grant else {
            return Ok(None);
        };
        let connection = self.connect()?;
        connection.execute(
            "UPDATE grants SET revoked_at = ?1 WHERE peer_id = ?2 AND share_id = ?3",
            params![now(), peer.id, share.id],
        )?;
        // An explicit revoke removes the grant for good, so its sources no
        // longer justify anything.
        connection.execute(
            "DELETE FROM grant_sources WHERE peer_id = ?1 AND share_id = ?2",
            params![peer.id, share.id],
        )?;
        Ok(Some(grant))
    }

    // ------------------------------------------------------------------
    // Groups: a named set of devices that expose their memory to each other.
    // Membership (`group_members`) and contributed workspaces (`group_shares`,
    // many per member) are separate. Invariant: for every contribution a
    // member adds, every other member holds a read-memory grant on that share.
    // ------------------------------------------------------------------

    pub fn add_group(
        &self,
        name: &str,
        self_peer_id: &str,
        self_peer_name: &str,
    ) -> Result<GroupInfo> {
        validate_alias(name, "group name")?;
        // The local device is a peer of itself; peers(id) is a FK target.
        self.save_remote_peer(self_peer_id, self_peer_name, "{}")?;
        let id = random_id("grp");
        let info = self.register_group(&id, name)?;
        self.connect()?.execute(
            "INSERT INTO group_members(group_id, peer_id, role, joined_at) VALUES (?1, ?2, 'owner', ?3)",
            params![id, self_peer_id, now()],
        )?;
        Ok(info)
    }

    /// Register a group row with an explicit id. Joiners use this to mirror the
    /// owner's group identity; idempotent on re-join.
    pub fn register_group(&self, id: &str, name: &str) -> Result<GroupInfo> {
        validate_alias(name, "group name")?;
        self.connect()?.execute(
            "INSERT INTO groups(id, name, created_at) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO NOTHING",
            params![id, name.to_ascii_lowercase(), now()],
        )?;
        self.group(name)
    }

    /// Rename the group. The name is the ref segment (`team:...`) and is
    /// stored once per device in `groups.name`; the owner renames and members
    /// mirror the new name on their next roster sync. Collisions with another
    /// local group are rejected (the `UNIQUE` constraint backs the check).
    pub fn rename_group(&self, group_name_or_id: &str, new_name: &str) -> Result<GroupInfo> {
        // The same rules as creation: a rename must not smuggle in a name
        // `add_group` would reject (including reserved scheme names).
        validate_alias(new_name, "group name")?;
        let new_name = new_name.to_ascii_lowercase();
        let group = self.group(group_name_or_id)?;
        if new_name != group.name && self.group_opt(&new_name)?.is_some() {
            bail!("A group named `{new_name}` already exists");
        }
        self.connect()?.execute(
            "UPDATE groups SET name = ?1 WHERE id = ?2",
            params![new_name, group.id],
        )?;
        self.group(&group.id)
    }

    pub fn groups(&self) -> Result<Vec<GroupInfo>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT g.id, g.name, (SELECT COUNT(*) FROM group_members m WHERE m.group_id = g.id), g.created_at FROM groups g ORDER BY g.name",
        )?;
        let rows = statement.query_map([], group_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn group_opt(&self, name_or_id: &str) -> Result<Option<GroupInfo>> {
        self.connect()?
            .query_row(
                "SELECT g.id, g.name, (SELECT COUNT(*) FROM group_members m WHERE m.group_id = g.id), g.created_at FROM groups g WHERE g.id = ?1 OR lower(g.name) = lower(?1)",
                [name_or_id],
                group_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn group(&self, name_or_id: &str) -> Result<GroupInfo> {
        self.group_opt(name_or_id)?
            .with_context(|| format!("Unknown group `{name_or_id}`"))
    }

    /// Insert or refresh one member row. Endpoint/name live in `peers`.
    /// `role` is parsed at this single boundary; comparisons use [`GroupRole`].
    pub fn add_member(
        &self,
        group_name_or_id: &str,
        peer_id: &str,
        role: &str,
    ) -> Result<GroupMemberInfo> {
        let role = GroupRole::from_str(role)?;
        let group = self.group(group_name_or_id)?;
        self.connect()?.execute(
            "INSERT INTO group_members(group_id, peer_id, role, joined_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(group_id, peer_id) DO UPDATE SET role = excluded.role",
            params![group.id, peer_id, role.as_wire(), now()],
        )?;
        self.member(&group.id, peer_id)
    }

    pub fn remove_member(&self, group_name_or_id: &str, peer_id: &str) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The member's contribution rows describe access on its own device and
        // are not covered by the `group_members` cascade; leaving them behind
        // would resurrect stale shares if the peer ever rejoins with a
        // different set, so removal is one operation over both tables.
        transaction.execute(
            "DELETE FROM group_shares WHERE group_id = ?1 AND peer_id = ?2",
            params![group.id, peer_id],
        )?;
        transaction.execute(
            "DELETE FROM group_members WHERE group_id = ?1 AND peer_id = ?2",
            params![group.id, peer_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn member(&self, group_name_or_id: &str, peer_id: &str) -> Result<GroupMemberInfo> {
        let group = self.group(group_name_or_id)?;
        self.connect()?
            .query_row(
                "SELECT gm.peer_id, p.name, gm.role, gm.joined_at, p.last_seen_at, p.endpoint_json, (SELECT COUNT(*) FROM group_shares gs WHERE gs.group_id = gm.group_id AND gs.peer_id = gm.peer_id) FROM group_members gm JOIN peers p ON p.id = gm.peer_id WHERE gm.group_id = ?1 AND gm.peer_id = ?2",
                params![group.id, peer_id],
                group_member_from_row,
            )
            .optional()?
            .with_context(|| format!("Peer `{peer_id}` is not a member of `{}`", group.name))
    }

    pub fn members(&self, group_name_or_id: &str) -> Result<Vec<GroupMemberInfo>> {
        let group = self.group(group_name_or_id)?;
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT gm.peer_id, p.name, gm.role, gm.joined_at, p.last_seen_at, p.endpoint_json, (SELECT COUNT(*) FROM group_shares gs WHERE gs.group_id = gm.group_id AND gs.peer_id = gm.peer_id) FROM group_members gm JOIN peers p ON p.id = gm.peer_id WHERE gm.group_id = ?1 ORDER BY p.name",
        )?;
        let rows = statement.query_map([group.id], group_member_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// The owner's membership row; the single authority for "who owns this
    /// group". A group without an owner row is treated as an error.
    pub fn owner(&self, group_name_or_id: &str) -> Result<GroupMemberInfo> {
        self.members(group_name_or_id)?
            .into_iter()
            .find(|member| member.role.is_owner())
            .context("Group has no owner")
    }

    /// True when `peer_id` is the group's owner.
    pub fn is_owner(&self, group_name_or_id: &str, peer_id: &str) -> Result<bool> {
        Ok(self.owner(group_name_or_id)?.peer_id == peer_id)
    }

    /// True when `peer_id` is a member of the group (any role).
    pub fn is_member(&self, group_name_or_id: &str, peer_id: &str) -> Result<bool> {
        Ok(self
            .members(group_name_or_id)?
            .iter()
            .any(|member| member.peer_id == peer_id))
    }

    /// Upsert peers, membership, contributions, and grants for every row in
    /// `roster`, all in one transaction. Add-only and idempotent: nothing
    /// local is ever removed. Used on the snapshot paths (join mirror,
    /// member-added push), where the sender's own share broadcasts race the
    /// snapshot - pruning there would let a stale push regress a share that
    /// was just added.
    pub fn apply_roster_add_only(
        &self,
        group_name_or_id: &str,
        self_id: &str,
        roster: &[RosterRow],
    ) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.upsert_roster_sql(&transaction, &group.id, self_id, roster)?;
        transaction.commit()?;
        Ok(())
    }

    /// Make local group state match the owner's authoritative `roster`, in one
    /// transaction: upsert, prune each member's stale contributions (except
    /// our own - they are local authority), and remove members the roster no
    /// longer lists (with their contribution rows and grants). The roster
    /// always lists every member including us, so a member missing from it has
    /// been removed. Pull-only: the owner sees every share change, so its
    /// roster may prune; snapshot pushes must stay add-only via
    /// [`Self::apply_roster_add_only`].
    pub fn apply_roster_reconcile(
        &self,
        group_name_or_id: &str,
        self_id: &str,
        roster: &[RosterRow],
    ) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        let group_id = &group.id;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.upsert_roster_sql(&transaction, group_id, self_id, roster)?;
        // Prune stale shares for every non-self member.
        for row in roster {
            if row.peer_id == self_id {
                continue;
            }
            let local = self.group_shares_sql(&transaction, group_id, &row.peer_id)?;
            for existing in local {
                if !row.shares.iter().any(|(id, _)| *id == existing) {
                    transaction.execute(
                        "DELETE FROM group_shares WHERE group_id = ?1 AND peer_id = ?2 AND share_id = ?3",
                        params![group_id, row.peer_id, existing],
                    )?;
                }
            }
        }
        // Drop members absent from the roster, revoking their grants on our
        // shares. Read our shares after the upsert so newly added self
        // contributions are covered too.
        let self_shares = self.group_shares_sql(&transaction, group_id, self_id)?;
        let local_members = self.members_sql(&transaction, group_id)?;
        for local in local_members {
            if !roster.iter().any(|row| row.peer_id == local) {
                transaction.execute(
                    "DELETE FROM group_shares WHERE group_id = ?1 AND peer_id = ?2",
                    params![group_id, local],
                )?;
                transaction.execute(
                    "DELETE FROM group_members WHERE group_id = ?1 AND peer_id = ?2",
                    params![group_id, local],
                )?;
                for share_id in &self_shares {
                    self.revoke_group_grant_sql(&transaction, group_id, share_id, &local)?;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Transaction-scoped upsert shared by the add-only and reconcile paths.
    fn upsert_roster_sql(
        &self,
        transaction: &rusqlite::Connection,
        group_id: &str,
        self_id: &str,
        roster: &[RosterRow],
    ) -> Result<()> {
        let self_shares = self.group_shares_sql(transaction, group_id, self_id)?;
        let timestamp = now();
        for row in roster {
            transaction.execute(
                "INSERT INTO peers(id, name, endpoint_json, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4) ON CONFLICT(id) DO UPDATE SET name = excluded.name, endpoint_json = excluded.endpoint_json, last_seen_at = excluded.last_seen_at",
                params![row.peer_id, row.peer_name, row.endpoint_json, timestamp],
            )?;
            transaction.execute(
                "INSERT INTO group_members(group_id, peer_id, role, joined_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(group_id, peer_id) DO UPDATE SET role = excluded.role",
                params![group_id, row.peer_id, row.role, timestamp],
            )?;
            for (share_id, share_name) in &row.shares {
                transaction.execute(
                    "INSERT INTO group_shares(group_id, peer_id, share_id, share_name, added_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(group_id, peer_id, share_id) DO NOTHING",
                    params![group_id, row.peer_id, share_id, share_name, timestamp],
                )?;
            }
            if row.peer_id != self_id {
                for share_id in &self_shares {
                    self.group_grant_sql(
                        transaction,
                        group_id,
                        share_id,
                        &row.peer_id,
                        &timestamp,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Transaction-scoped grant of `peer`'s read access to `share` on behalf
    /// of `group` (idempotent; mirrors [`Self::group_grant`]).
    fn group_grant_sql(
        &self,
        transaction: &rusqlite::Connection,
        group_id: &str,
        share_id: &str,
        peer_id: &str,
        timestamp: &str,
    ) -> Result<()> {
        transaction.execute(
            "INSERT INTO grant_sources(share_id, peer_id, via, group_id, created_at) VALUES (?1, ?2, 'group', ?3, ?4) ON CONFLICT DO NOTHING",
            params![share_id, peer_id, group_id, timestamp],
        )?;
        transaction.execute(
            "INSERT INTO grants(peer_id, share_id, permission, created_at, revoked_at) VALUES (?1, ?2, ?3, ?4, NULL) ON CONFLICT(peer_id, share_id) DO UPDATE SET permission = excluded.permission, revoked_at = NULL",
            params![peer_id, share_id, PERMISSION_READ_MEMORY, timestamp],
        )?;
        Ok(())
    }

    /// Transaction-scoped revocation of a member's grant on a group share
    /// (mirrors [`Self::revoke_group_grant`]: the group source is removed
    /// first; the grant survives while another source justifies it).
    fn revoke_group_grant_sql(
        &self,
        transaction: &rusqlite::Connection,
        group_id: &str,
        share_id: &str,
        peer_id: &str,
    ) -> Result<()> {
        transaction.execute(
            "DELETE FROM grant_sources WHERE share_id = ?1 AND peer_id = ?2 AND via = 'group' AND group_id = ?3",
            params![share_id, peer_id, group_id],
        )?;
        let other: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM grant_sources
                WHERE share_id = ?1 AND peer_id = ?2
                  AND NOT (via = 'group' AND group_id = ?3)
            )",
            params![share_id, peer_id, group_id],
            |row| row.get(0),
        )?;
        if !other {
            transaction.execute(
                "UPDATE grants SET revoked_at = ?1 WHERE peer_id = ?2 AND share_id = ?3",
                params![now(), peer_id, share_id],
            )?;
            transaction.execute(
                "DELETE FROM grant_sources WHERE peer_id = ?1 AND share_id = ?2",
                params![peer_id, share_id],
            )?;
        }
        Ok(())
    }

    /// Transaction-scoped share-id list for one member of a group.
    fn group_shares_sql(
        &self,
        transaction: &rusqlite::Connection,
        group_id: &str,
        peer_id: &str,
    ) -> Result<Vec<String>> {
        let mut statement = transaction
            .prepare("SELECT share_id FROM group_shares WHERE group_id = ?1 AND peer_id = ?2")?;
        let rows = statement.query_map(params![group_id, peer_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Transaction-scoped member id list for a group.
    fn members_sql(
        &self,
        transaction: &rusqlite::Connection,
        group_id: &str,
    ) -> Result<Vec<String>> {
        let mut statement =
            transaction.prepare("SELECT peer_id FROM group_members WHERE group_id = ?1")?;
        let rows = statement.query_map([group_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Register one contributed workspace for a member (idempotent).
    pub fn add_group_share(
        &self,
        group_name_or_id: &str,
        peer_id: &str,
        share_id: &str,
        share_name: &str,
    ) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        self.connect()?.execute(
            "INSERT INTO group_shares(group_id, peer_id, share_id, share_name, added_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(group_id, peer_id, share_id) DO NOTHING",
            params![group.id, peer_id, share_id, share_name, now()],
        )?;
        Ok(())
    }

    pub fn remove_group_share(
        &self,
        group_name_or_id: &str,
        peer_id: &str,
        share_id: &str,
    ) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        self.connect()?.execute(
            "DELETE FROM group_shares WHERE group_id = ?1 AND peer_id = ?2 AND share_id = ?3",
            params![group.id, peer_id, share_id],
        )?;
        Ok(())
    }

    pub fn group_shares(
        &self,
        group_name_or_id: &str,
        peer_id: &str,
    ) -> Result<Vec<GroupShareInfo>> {
        let group = self.group(group_name_or_id)?;
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT share_id, share_name, added_at FROM group_shares WHERE group_id = ?1 AND peer_id = ?2 ORDER BY share_name",
        )?;
        let rows = statement.query_map(params![group.id, peer_id], |row| {
            Ok(GroupShareInfo {
                share_id: row.get(0)?,
                share_name: row.get(1)?,
                added_at: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Resolve a member's contributed share by name (three-segment scope
    /// `team/alice/proj-b` lands here).
    pub fn group_share_by_name(
        &self,
        group_name_or_id: &str,
        peer_id: &str,
        share_name: &str,
    ) -> Result<Option<GroupShareInfo>> {
        let group = self.group(group_name_or_id)?;
        self.connect()?
            .query_row(
                "SELECT share_id, share_name, added_at FROM group_shares WHERE group_id = ?1 AND peer_id = ?2 AND share_name = ?3",
                params![group.id, peer_id, share_name],
                |row| {
                    Ok(GroupShareInfo {
                        share_id: row.get(0)?,
                        share_name: row.get(1)?,
                        added_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Grant `peer` read access to `share` on behalf of `group` (idempotent).
    /// Also clears a previous revocation, so rejoining a group reactivates
    /// the grant. The group source is recorded in `grant_sources` under the
    /// group's real id (the argument may be a name), so a later withdrawal
    /// from this group alone does not revoke access still justified by
    /// another source.
    pub fn group_grant(&self, group_name_or_id: &str, share_id: &str, peer_id: &str) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        let connection = self.connect()?;
        self.group_grant_sql(&connection, &group.id, share_id, peer_id, &now())
    }

    /// Revoke a member's grant on a group share. The group's source row is
    /// removed first; the grant itself survives while another source still
    /// justifies it (see [`Self::revoke_group_grant_sql`]).
    pub fn revoke_group_grant(
        &self,
        group_name_or_id: &str,
        share_id: &str,
        peer_id: &str,
    ) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        let connection = self.connect()?;
        self.revoke_group_grant_sql(&connection, &group.id, share_id, peer_id)
    }

    /// Revoke every other member's grant on `share` (the contributor withdrew
    /// that workspace from the group). A grant survives when the peer still
    /// holds access from another source, because `grants` keeps one row per
    /// (peer, share) regardless of how many sources issued it.
    pub fn revoke_group_share(
        &self,
        group_name_or_id: &str,
        share_id: &str,
        except_peer_id: &str,
    ) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        let peers: Vec<String> = self
            .connect()?
            .prepare("SELECT peer_id FROM group_members WHERE group_id = ?1 AND peer_id != ?2")?
            .query_map(params![group.id, except_peer_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for peer_id in peers {
            self.revoke_group_grant(&group.id, share_id, &peer_id)?;
        }
        Ok(())
    }

    pub fn remove_group(&self, name_or_id: &str) -> Result<GroupInfo> {
        let group = self.group(name_or_id)?;
        self.connect()?
            .execute("DELETE FROM groups WHERE id = ?1", [&group.id])?;
        Ok(group)
    }

    pub fn create_group_invite(
        &self,
        group_name_or_id: &str,
        valid_for_seconds: i64,
        max_uses: Option<i64>,
    ) -> Result<InviteRecord> {
        if valid_for_seconds <= 0 {
            bail!("Invite expiration must be positive");
        }
        let group = self.group(group_name_or_id)?;
        // invites.share_id references shares(id); group invites point at the
        // owner's first contributed share so the FK holds.
        let owner_share: String = self
            .connect()?
            .query_row(
                "SELECT gs.share_id FROM group_shares gs JOIN group_members gm ON gm.group_id = gs.group_id AND gm.peer_id = gs.peer_id WHERE gs.group_id = ?1 AND gm.role = 'owner' LIMIT 1",
                [&group.id],
                |row| row.get(0),
            )
            .optional()?
            .context("Group has no shared workspace to invite with")?;
        let id = random_id("iv");
        let secret = random_secret();
        let expires_at = Utc::now().timestamp() + valid_for_seconds;
        self.connect()?.execute(
            "INSERT INTO invites(id, share_id, secret_hash, permission, expires_at, used_at, created_at, kind, group_id, max_uses, used_count) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, 'group', ?7, ?8, 0)",
            params![id, owner_share, hash_secret(&secret), PERMISSION_READ_MEMORY, expires_at, now(), group.id, max_uses],
        )?;
        Ok(InviteRecord {
            id,
            share_id: group.id,
            share_name: group.name,
            secret,
            expires_at,
        })
    }

    /// Redeem a group invite: validates the multi-use ticket, adds the joiner
    /// to the roster with every contributed workspace, grants the joiner read
    /// access to the owner's contributions, and returns the authoritative
    /// group id with the current roster for the joiner to reconcile.
    pub fn redeem_group_invite(
        &self,
        invite_id: &str,
        secret: &str,
        joiner: &JoinerInfo<'_>,
    ) -> Result<RedeemedGroup> {
        validate_identifier(joiner.peer_name, "peer name")?;
        // Group mode promises every member publishes memory to the mesh; a
        // joiner with no contribution would consume the roster and owner
        // shares without contributing anything back.
        if joiner.shares.is_empty() {
            bail!("A group member must contribute at least one workspace");
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let invite = transaction
            .query_row(
                "SELECT i.group_id, i.secret_hash, i.expires_at, i.used_count, i.max_uses, i.used_at FROM invites i WHERE i.id = ?1 AND i.kind = 'group'",
                [invite_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?
            .context("Invitation is invalid or expired")?;
        let (group_id, expected_hash, expires_at, used_count, max_uses, used_at) = invite;
        let group_id = group_id.context("Invitation is invalid or expired")?;
        if expires_at < Utc::now().timestamp() || used_at.is_some() {
            bail!("Invitation is invalid or expired");
        }
        if let Some(max_uses) = max_uses {
            if used_count >= max_uses {
                bail!("Invitation has reached its usage limit");
            }
        }
        if expected_hash != hash_secret(secret) {
            bail!("Invitation is invalid or expired");
        }
        let timestamp = now();
        transaction.execute(
            "INSERT INTO peers(id, name, endpoint_json, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4) ON CONFLICT(id) DO UPDATE SET name = excluded.name, endpoint_json = excluded.endpoint_json, last_seen_at = excluded.last_seen_at",
            params![joiner.peer_id, joiner.peer_name, joiner.endpoint_json, timestamp],
        )?;
        transaction.execute(
            "INSERT INTO group_members(group_id, peer_id, role, joined_at) VALUES (?1, ?2, 'member', ?3) ON CONFLICT(group_id, peer_id) DO NOTHING",
            params![group_id, joiner.peer_id, timestamp],
        )?;
        for (share_id, share_name) in joiner.shares {
            transaction.execute(
                "INSERT INTO group_shares(group_id, peer_id, share_id, share_name, added_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(group_id, peer_id, share_id) DO NOTHING",
                params![group_id, joiner.peer_id, share_id, share_name, timestamp],
            )?;
        }
        // Owner grants the newcomer on every owner contribution.
        let owner_shares: Vec<String> = {
            let mut statement = transaction.prepare(
                "SELECT gs.share_id FROM group_shares gs JOIN group_members gm ON gm.group_id = gs.group_id AND gm.peer_id = gs.peer_id WHERE gs.group_id = ?1 AND gm.role = 'owner'",
            )?;
            let rows = statement.query_map([&group_id], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for owner_share in &owner_shares {
            transaction.execute(
                "INSERT INTO grant_sources(share_id, peer_id, via, group_id, created_at) VALUES (?1, ?2, 'group', ?3, ?4) ON CONFLICT DO NOTHING",
                params![owner_share, joiner.peer_id, group_id, timestamp],
            )?;
            transaction.execute(
                "INSERT INTO grants(peer_id, share_id, permission, created_at, revoked_at) VALUES (?1, ?2, ?3, ?4, NULL) ON CONFLICT(peer_id, share_id) DO UPDATE SET permission = excluded.permission, revoked_at = NULL",
                params![joiner.peer_id, owner_share, PERMISSION_READ_MEMORY, timestamp],
            )?;
        }
        transaction.execute(
            "UPDATE invites SET used_count = used_count + 1 WHERE id = ?1",
            [invite_id],
        )?;
        transaction.commit()?;
        let roster = self.members(&group_id)?;
        Ok(RedeemedGroup { group_id, roster })
    }

    pub fn touch_group_sync(&self, group_name_or_id: &str) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        self.connect()?.execute(
            "UPDATE groups SET last_synced_at = ?1 WHERE id = ?2",
            params![now(), group.id],
        )?;
        Ok(())
    }

    /// The highest roster version this device has adopted. The owner bumps it
    /// on every membership change it processes; members adopt it from owner
    /// broadcasts and pulls. Never regresses, so it doubles as the stale-
    /// message watermark on the receiving side.
    pub fn roster_epoch(&self, group_name_or_id: &str) -> Result<i64> {
        let group = self.group(group_name_or_id)?;
        self.connect()?
            .query_row(
                "SELECT roster_epoch FROM groups WHERE id = ?1",
                [&group.id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Owner-side: record one roster change and return the new epoch, which
    /// the caller carries on the accompanying broadcast or sync response.
    pub fn bump_roster_epoch(&self, group_name_or_id: &str) -> Result<i64> {
        let group = self.group(group_name_or_id)?;
        self.connect()?
            .query_row(
                "UPDATE groups SET roster_epoch = roster_epoch + 1 WHERE id = ?1 RETURNING roster_epoch",
                [&group.id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Adopt a roster version observed from the owner (broadcast or pull).
    /// Monotonic: a stale observation never regresses the local watermark.
    pub fn adopt_roster_epoch(&self, group_name_or_id: &str, epoch: i64) -> Result<()> {
        let group = self.group(group_name_or_id)?;
        self.connect()?.execute(
            "UPDATE groups SET roster_epoch = MAX(roster_epoch, ?1) WHERE id = ?2",
            params![epoch, group.id],
        )?;
        Ok(())
    }

    pub fn sync_stale(&self, group_name_or_id: &str, ttl_secs: i64) -> Result<bool> {
        let group = self.group(group_name_or_id)?;
        let last: Option<String> = self
            .connect()?
            .query_row(
                "SELECT last_synced_at FROM groups WHERE id = ?1",
                [&group.id],
                |row| row.get(0),
            )
            .optional()?;
        match last {
            None => Ok(true),
            Some(timestamp) => {
                let last = chrono::DateTime::parse_from_rfc3339(&timestamp)
                    .map_err(|_| anyhow::anyhow!("Invalid last_synced_at timestamp"))?
                    .with_timezone(&Utc);
                Ok(Utc::now() - last > chrono::Duration::seconds(ttl_secs))
            }
        }
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

fn group_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GroupInfo> {
    Ok(GroupInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        member_count: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn group_member_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GroupMemberInfo> {
    Ok(GroupMemberInfo {
        peer_id: row.get(0)?,
        peer_name: row.get(1)?,
        role: row.get(2)?,
        joined_at: row.get(3)?,
        last_seen_at: row.get(4)?,
        endpoint: row.get(5)?,
        share_count: row.get(6)?,
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
    getrandom::fill(&mut bytes).expect("OS RNG unavailable");
    format!("{prefix}_{}", hex(&bytes))
}

fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS RNG unavailable");
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
    fn peer_endpoint_hints_refresh_without_renaming() {
        let temp = tempfile::tempdir().unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        store
            .save_remote_peer("peer-1", "alice", r#"{"id":"old","addrs":[]}"#)
            .unwrap();
        store
            .refresh_peer_endpoint("peer-1", r#"{"id":"new","addrs":[]}"#)
            .unwrap();
        assert_eq!(
            store.peer_endpoint("peer-1").unwrap(),
            r#"{"id":"new","addrs":[]}"#
        );
        assert!(store.refresh_peer_endpoint("missing", "{}").is_err());
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

    fn group_store() -> (tempfile::TempDir, StateStore, ShareInfo) {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        let share = store
            .add_share("workspace-key", &workspace, "project", true)
            .unwrap();
        store.add_group("team", "self-1", "self").unwrap();
        store
            .add_group_share("team", "self-1", &share.id, &share.name)
            .unwrap();
        (temp, store, share)
    }

    fn joiner<'a>(
        peer_id: &'a str,
        peer_name: &'a str,
        shares: &'a [(String, String)],
    ) -> JoinerInfo<'a> {
        JoinerInfo {
            peer_id,
            peer_name,
            shares,
            endpoint_json: "{}",
        }
    }

    #[test]
    fn is_owner_and_is_member_gate_membership() {
        let (_temp, store, _share) = group_store();
        store.save_remote_peer("peer-2", "bob", "{}").unwrap();
        store.add_member("team", "peer-2", "member").unwrap();
        assert!(store.is_owner("team", "self-1").unwrap());
        assert!(!store.is_owner("team", "peer-2").unwrap());
        assert!(!store.is_owner("team", "stranger").unwrap());
        assert!(store.is_member("team", "self-1").unwrap());
        assert!(store.is_member("team", "peer-2").unwrap());
        assert!(!store.is_member("team", "stranger").unwrap());
        // A group without an owner row has no owner authority.
        assert!(store.owner("missing").is_err());
    }

    /// Create a real share under `temp` (group_shares.share_id references shares.id).
    fn real_share(store: &StateStore, temp: &Path, key: &str, name: &str) -> ShareInfo {
        let root = temp.join(format!("ws-{key}"));
        std::fs::create_dir(&root).unwrap();
        store.add_share(key, &root, name, true).unwrap()
    }

    #[test]
    fn group_invite_is_multi_use_until_expiry_or_max_uses() {
        let (temp, store, _share) = group_store();
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        let bob_share = real_share(&store, temp.path(), "bob", "bob-ws");
        let dan_share = real_share(&store, temp.path(), "dan", "dan-ws");
        let erin_share = real_share(&store, temp.path(), "erin", "erin-ws");

        let invite = store.create_group_invite("team", 60, None).unwrap();
        let redeemed = store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &joiner(
                    "peer-1",
                    "alice",
                    &[(alice_share.id.clone(), alice_share.name.clone())],
                ),
            )
            .unwrap();
        assert_eq!(
            redeemed.group_id,
            store.group("team").unwrap().id,
            "the group is derived from the invite row"
        );
        assert_eq!(redeemed.roster.len(), 2, "owner + alice");
        // The same ticket admits another peer — group invites are multi-use.
        let redeemed = store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &joiner(
                    "peer-2",
                    "bob",
                    &[(bob_share.id.clone(), bob_share.name.clone())],
                ),
            )
            .unwrap();
        assert_eq!(redeemed.roster.len(), 3);
        assert!(store
            .redeem_group_invite(
                &invite.id,
                "wrong-secret",
                &joiner(
                    "peer-3",
                    "carol",
                    &[(alice_share.id.clone(), alice_share.name.clone())],
                ),
            )
            .is_err());

        // max_uses caps redemption even before expiry.
        let limited = store.create_group_invite("team", 60, Some(1)).unwrap();
        store
            .redeem_group_invite(
                &limited.id,
                &limited.secret,
                &joiner(
                    "peer-4",
                    "dan",
                    &[(dan_share.id.clone(), dan_share.name.clone())],
                ),
            )
            .unwrap();
        assert!(store
            .redeem_group_invite(
                &limited.id,
                &limited.secret,
                &joiner(
                    "peer-5",
                    "erin",
                    &[(erin_share.id.clone(), erin_share.name.clone())],
                ),
            )
            .is_err());
    }

    #[test]
    fn join_grants_owner_and_roster_roundtrips() {
        let (temp, store, share) = group_store();
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        let invite = store.create_group_invite("team", 60, None).unwrap();
        let redeemed = store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &JoinerInfo {
                    peer_id: "peer-1",
                    peer_name: "alice",
                    shares: &[(alice_share.id.clone(), alice_share.name.clone())],
                    endpoint_json: r#"{"id":"alice"}"#,
                },
            )
            .unwrap();
        // Owner's share is now authorized for the newcomer.
        store.authorize("peer-1", &share.id, "query").unwrap();
        // Roster carries the member's endpoint hint for dialing back.
        let alice = redeemed
            .roster
            .iter()
            .find(|member| member.peer_id == "peer-1")
            .expect("alice in roster");
        assert_eq!(alice.endpoint.as_deref(), Some(r#"{"id":"alice"}"#));
        assert_eq!(alice.role, GroupRole::Member);
        assert_eq!(alice.share_count, 1);
    }

    #[test]
    fn redeem_invite_rejects_group_tickets() {
        let (temp, store, _share) = group_store();
        let invite = store.create_group_invite("team", 60, None).unwrap();
        let error = store
            .redeem_invite(&invite.id, &invite.secret, "peer-1", "alice")
            .expect_err("a group ticket must not grant a direct share");
        assert!(error.to_string().contains("invalid or expired"));
        // The failed attempt grants nothing and leaves the multi-use ticket
        // valid, so legitimate joiners are unaffected.
        assert!(store
            .authorize("peer-1", &invite.share_id, "query")
            .is_err());
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &joiner("peer-1", "alice", &[(alice_share.id, alice_share.name)]),
            )
            .expect("group ticket still redeemable");
    }

    #[test]
    fn group_join_requires_a_contributed_workspace() {
        let (_temp, store, _share) = group_store();
        let invite = store.create_group_invite("team", 60, None).unwrap();
        let error = store
            .redeem_group_invite(&invite.id, &invite.secret, &joiner("peer-1", "alice", &[]))
            .expect_err("an empty contribution set must be rejected");
        assert!(error.to_string().contains("contribute at least one"));
        // The peer was not admitted and holds no grant.
        assert!(store
            .members("team")
            .unwrap()
            .iter()
            .all(|member| member.peer_id != "peer-1"));
        assert!(store
            .authorize("peer-1", &invite.share_id, "query")
            .is_err());
    }

    #[test]
    fn group_shares_keeps_only_intentional_foreign_keys() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.db");
        StateStore::open(db.clone()).unwrap();
        let store = StateStore::open(db).unwrap();
        let connection = store.connect().unwrap();
        let mut fks: Vec<String> = connection
            .prepare("SELECT \"from\" FROM pragma_foreign_key_list('group_shares')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        fks.sort();
        assert_eq!(
            fks,
            vec!["group_id".to_string(), "peer_id".to_string()],
            "the obsolete share_id FK stays gone across reopens"
        );
    }

    #[test]
    fn member_add_reconciles_newcomer_grant() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        let owner_share = store
            .add_share("workspace-key", &workspace, "project", true)
            .unwrap();
        let bob_share = store
            .add_share("workspace-bob", &workspace, "bob-project", true)
            .unwrap();
        let carol_share = real_share(&store, temp.path(), "carol", "carol-ws");
        store.add_group("team", "self-1", "self").unwrap();
        store
            .add_group_share("team", "self-1", &owner_share.id, &owner_share.name)
            .unwrap();
        store.save_remote_peer("peer-2", "bob", "{}").unwrap();
        store.add_member("team", "peer-2", "member").unwrap();
        store
            .add_group_share("team", "peer-2", &bob_share.id, &bob_share.name)
            .unwrap();
        // Bob reconciles a GroupMemberAdded push: the newcomer gets read access
        // to Bob's group share.
        store.save_remote_peer("peer-3", "carol", "{}").unwrap();
        store.group_grant("team", &bob_share.id, "peer-3").unwrap();
        store.add_member("team", "peer-3", "member").unwrap();
        store
            .add_group_share("team", "peer-3", &carol_share.id, &carol_share.name)
            .unwrap();
        assert!(store.authorize("peer-3", &bob_share.id, "query").is_ok());
    }

    #[test]
    fn remove_member_revokes_grants_and_roster() {
        let (temp, store, share) = group_store();
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        let invite = store.create_group_invite("team", 60, None).unwrap();
        store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &joiner(
                    "peer-1",
                    "alice",
                    &[(alice_share.id.clone(), alice_share.name.clone())],
                ),
            )
            .unwrap();
        store.remove_member("team", "peer-1").unwrap();
        assert!(
            store.revoke(&share.name, "peer-1").unwrap().is_some(),
            "an active grant was revoked"
        );
        assert!(store.authorize("peer-1", &share.id, "query").is_err());
        assert!(!store
            .members("team")
            .unwrap()
            .iter()
            .any(|member| member.peer_id == "peer-1"));
    }

    #[test]
    fn revoke_without_grant_is_an_idempotent_noop() {
        let (_temp, store, share) = group_store();
        store.save_remote_peer("peer-1", "alice", "{}").unwrap();
        assert!(store.revoke(&share.name, "peer-1").unwrap().is_none());
        // Repeating the revoke stays a success: kick/leave must not fail just
        // because a peer already lost access.
        assert!(store.revoke(&share.name, "peer-1").unwrap().is_none());
    }

    #[test]
    fn group_rename_updates_name_and_keeps_id() {
        let (_temp, store, _share) = group_store();
        let renamed = store.rename_group("team", "dev").unwrap();
        assert_eq!(renamed.name, "dev");
        assert_eq!(renamed.id, store.group("dev").unwrap().id);
        assert!(store.group("team").is_err(), "old name no longer resolves");
        // The renamed group still owns its membership and contributions.
        assert_eq!(store.members("dev").unwrap().len(), 1);
        assert_eq!(store.group_shares("dev", "self-1").unwrap().len(), 1);
        // Renaming to the same name is a no-op.
        store.rename_group("dev", "dev").unwrap();
        // A collision with another local group is rejected.
        store.add_group("docs", "self-1", "self").unwrap();
        let error = store.rename_group("dev", "docs").expect_err("collision");
        assert!(error.to_string().contains("already exists"));
        // Identifier rules still apply.
        let error = store.rename_group("dev", "bad name").expect_err("invalid");
        assert!(error.to_string().contains("must be"));
        // Reserved scheme names are rejected, same as creation.
        let error = store.rename_group("dev", "sivtr").expect_err("reserved");
        assert!(error.to_string().contains("reserved"));
    }

    #[test]
    fn group_opt_is_none_for_unknown_group() {
        let temp = tempfile::tempdir().unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        assert!(store.group_opt("missing").unwrap().is_none());
        store.add_group("team", "self-1", "self").unwrap();
        let group = store.group_opt("team").unwrap().expect("known group");
        assert_eq!(group.name, "team");
    }

    #[test]
    fn member_list_includes_last_seen_and_endpoint() {
        let (temp, store, _share) = group_store();
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        store
            .save_remote_peer("peer-1", "alice", r#"{"id":"alice"}"#)
            .unwrap();
        store.add_member("team", "peer-1", "member").unwrap();
        store
            .add_group_share("team", "peer-1", &alice_share.id, &alice_share.name)
            .unwrap();
        let roster = store.members("team").unwrap();
        assert_eq!(roster.len(), 2, "owner + alice");
        let alice = roster
            .iter()
            .find(|member| member.peer_id == "peer-1")
            .unwrap();
        assert_eq!(alice.role, GroupRole::Member);
        assert_eq!(alice.endpoint.as_deref(), Some(r#"{"id":"alice"}"#));
        assert_eq!(alice.share_count, 1);
        assert!(
            alice.last_seen_at.is_some(),
            "peer upsert sets last_seen_at"
        );
        let team = store.group("team").unwrap();
        assert_eq!(team.member_count, 2);
    }

    #[test]
    fn share_reused_across_groups() {
        let (temp, store, share) = group_store();
        let a_share = real_share(&store, temp.path(), "a", "a-ws");
        let b_share = real_share(&store, temp.path(), "b", "b-ws");
        store.add_group("team-b", "self-1", "self").unwrap();
        store.save_remote_peer("peer-1", "alice", "{}").unwrap();
        store.add_member("team", "peer-1", "member").unwrap();
        store.add_member("team-b", "peer-1", "member").unwrap();
        store
            .add_group_share("team", "peer-1", &a_share.id, &a_share.name)
            .unwrap();
        store
            .add_group_share("team-b", "peer-1", &b_share.id, &b_share.name)
            .unwrap();
        // Same share granted to the same peer twice is a no-op, not a conflict.
        store.group_grant("team", &share.id, "peer-1").unwrap();
        store.group_grant("team", &share.id, "peer-1").unwrap();
        assert!(store.authorize("peer-1", &share.id, "query").is_ok());
        assert_eq!(store.groups().unwrap().len(), 2);
    }

    #[test]
    fn member_contributes_multiple_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let root_a = temp.path().join("a");
        let root_b = temp.path().join("b");
        std::fs::create_dir(&root_a).unwrap();
        std::fs::create_dir(&root_b).unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        let first = store
            .add_share("workspace-key-1", &root_a, "project-1", true)
            .unwrap();
        let second = store
            .add_share("workspace-key-2", &root_b, "project-2", true)
            .unwrap();
        store.add_group("team", "self-1", "self").unwrap();
        store
            .add_group_share("team", "self-1", &first.id, &first.name)
            .unwrap();
        store
            .add_group_share("team", "self-1", &second.id, &second.name)
            .unwrap();
        assert_eq!(store.group_shares("team", "self-1").unwrap().len(), 2);
        let owner = store.members("team").unwrap().remove(0);
        assert_eq!(owner.share_count, 2);
        // Name-resolved lookup powers `team/self/project-2` three-segment refs.
        let found = store
            .group_share_by_name("team", "self-1", "project-2")
            .unwrap()
            .expect("share by name");
        assert_eq!(found.share_id, second.id);
        // Withdrawing one contribution keeps the rest.
        store
            .remove_group_share("team", "self-1", &second.id)
            .unwrap();
        assert_eq!(store.group_shares("team", "self-1").unwrap().len(), 1);
        assert!(store
            .group_share_by_name("team", "self-1", "project-2")
            .unwrap()
            .is_none());
    }

    #[test]
    fn revoke_group_share_withdraws_member_grants() {
        let (temp, store, share) = group_store();
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        let invite = store.create_group_invite("team", 60, None).unwrap();
        store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &joiner(
                    "peer-1",
                    "alice",
                    &[(alice_share.id.clone(), alice_share.name.clone())],
                ),
            )
            .unwrap();
        // The owner withdraws their contribution; alice loses access to it.
        store
            .revoke_group_share("team", &share.id, "self-1")
            .unwrap();
        assert!(store.authorize("peer-1", &share.id, "query").is_err());
        // Re-adding the share re-grants.
        store.group_grant("team", &share.id, "peer-1").unwrap();
        assert!(store.authorize("peer-1", &share.id, "query").is_ok());
    }

    #[test]
    fn group_revocation_preserves_access_from_other_groups() {
        let (_temp, store, share) = group_store();
        // Alice is a member of `team` and `team-b`; the owner contributes the
        // same workspace to both groups.
        store.add_group("team-b", "self-1", "self").unwrap();
        store.save_remote_peer("peer-1", "alice", "{}").unwrap();
        store.add_member("team", "peer-1", "member").unwrap();
        store.add_member("team-b", "peer-1", "member").unwrap();
        store
            .add_group_share("team", "self-1", &share.id, &share.name)
            .unwrap();
        store
            .add_group_share("team-b", "self-1", &share.id, &share.name)
            .unwrap();
        // Both groups granted access; each records its own source.
        store.group_grant("team", &share.id, "peer-1").unwrap();
        store.group_grant("team-b", &share.id, "peer-1").unwrap();
        // Withdraw from `team` (contribution row first, as the daemon does):
        // the grant survives because `team-b` still lists the share.
        store
            .remove_group_share("team", "self-1", &share.id)
            .unwrap();
        store
            .revoke_group_share("team", &share.id, "self-1")
            .unwrap();
        assert!(
            store.authorize("peer-1", &share.id, "query").is_ok(),
            "the second group still justifies the grant"
        );
        // Withdrawing from the last group revokes it.
        store
            .remove_group_share("team-b", "self-1", &share.id)
            .unwrap();
        store
            .revoke_group_share("team-b", &share.id, "self-1")
            .unwrap();
        assert!(store.authorize("peer-1", &share.id, "query").is_err());
    }

    #[test]
    fn group_revocation_preserves_direct_redeem_access() {
        let (temp, store, share) = group_store();
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        // Alice first redeems a direct share invite for the owner workspace
        // (the owner's store records the 'direct' source), then joins the
        // group too.
        let direct = store.create_invite("project", 60).unwrap();
        store
            .redeem_invite(&direct.id, &direct.secret, "peer-1", "alice")
            .unwrap();
        let invite = store.create_group_invite("team", 60, None).unwrap();
        store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &joiner(
                    "peer-1",
                    "alice",
                    &[(alice_share.id.clone(), alice_share.name.clone())],
                ),
            )
            .unwrap();
        // Withdrawing the workspace from the group must keep the direct grant.
        store
            .revoke_group_share("team", &share.id, "self-1")
            .unwrap();
        assert!(
            store.authorize("peer-1", &share.id, "query").is_ok(),
            "the direct redeem still grants access"
        );
        // An explicit revoke removes it for good.
        store.revoke(&share.name, "peer-1").unwrap();
        assert!(store.authorize("peer-1", &share.id, "query").is_err());
    }

    #[test]
    fn remove_member_cleans_contribution_rows() {
        let (temp, store, _share) = group_store();
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        let invite = store.create_group_invite("team", 60, None).unwrap();
        store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &joiner("peer-1", "alice", &[(alice_share.id, alice_share.name)]),
            )
            .unwrap();
        assert_eq!(store.group_shares("team", "peer-1").unwrap().len(), 1);
        store.remove_member("team", "peer-1").unwrap();
        assert!(
            store.group_shares("team", "peer-1").unwrap().is_empty(),
            "contributions are removed with the membership"
        );
    }

    #[test]
    fn removing_a_contributed_share_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        let share = store
            .add_share("workspace-key", &workspace, "project", true)
            .unwrap();
        store.add_group("team", "self-1", "self").unwrap();
        store
            .add_group_share("team", "self-1", &share.id, &share.name)
            .unwrap();
        let error = store.remove_share(&share.name).expect_err("contributed");
        assert!(error.to_string().contains("contributed to group"));
        // Once the contribution is withdrawn, removal succeeds.
        store
            .remove_group_share("team", "self-1", &share.id)
            .unwrap();
        store.remove_share(&share.name).unwrap();
        assert!(store.share(&share.id).is_err());
    }

    #[test]
    fn forgetting_a_group_member_is_rejected() {
        let (_temp, store, _share) = group_store();
        // self-1 is the group owner; forgetting it would orphan the group.
        let error = store.forget_peer("self-1").expect_err("group member");
        assert!(error.to_string().contains("participates in group"));
        // A peer that is not in any group is forgettable.
        store.save_remote_peer("peer-9", "zoe", "{}").unwrap();
        store.forget_peer("peer-9").unwrap();
        assert!(store.peer("peer-9").is_err());
    }

    #[test]
    fn group_names_reject_reserved_schemes() {
        let temp = tempfile::tempdir().unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        let error = store
            .add_group("local", "self-1", "self")
            .expect_err("reserved");
        assert!(error.to_string().contains("reserved"));
        let error = store
            .add_group("sivtr", "self-1", "self")
            .expect_err("reserved");
        assert!(error.to_string().contains("reserved"));
        // Ordinary names still work.
        store.add_group("team", "self-1", "self").unwrap();
    }
}
