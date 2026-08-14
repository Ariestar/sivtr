use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use sivtr_core::workspace;

mod group;

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
        let connection = self.connect()?;
        // Detect installs that predate the group-domain grant-sources table
        // (the schema below would create it) so the migration can backfill
        // existing grants exactly once, on the pass that introduces it.
        let had_grant_sources: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'grant_sources'",
            [],
            |row| row.get(0),
        )?;
        connection.execute_batch(
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
                -- Direct rows use '' as the sentinel so the primary key stays
                -- unique (SQLite treats NULLs as distinct in UNIQUE indexes).
                group_id    TEXT NOT NULL DEFAULT '',
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
        // `share_id` FK triggers the rebuild 鈥?the `group_id`/`peer_id` FKs are
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
        // The pass that introduces `grant_sources` backfills the already-active
        // grants as direct sources: without a source row, withdrawing the same
        // share from a later group would revoke a still-valid direct grant.
        if had_grant_sources == 0 {
            connection.execute_batch(
                "INSERT INTO grant_sources(share_id, peer_id, via, group_id, created_at)
                 SELECT share_id, peer_id, 'direct', '', created_at FROM grants
                 WHERE revoked_at IS NULL",
            )?;
        }
        // Legacy `direct` rows stored NULL for group_id, which the primary key
        // treats as distinct: normalize them to the sentinel so repeated
        // direct redemptions deduplicate.
        connection.execute(
            "UPDATE grant_sources SET group_id = '' WHERE group_id IS NULL",
            [],
        )?;
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
            "INSERT INTO grant_sources(share_id, peer_id, via, group_id, created_at) VALUES (?1, ?2, 'direct', '', ?3) ON CONFLICT DO NOTHING",
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
        // `grant_sources` has no foreign key on peers, so the cascade cannot
        // clean it: drop every source this peer authorized through. Otherwise
        // a later rejoin + kick would see the stale source and preserve the
        // grant, leaving the forgotten peer queryable forever.
        let connection = self.connect()?;
        connection.execute("DELETE FROM grant_sources WHERE peer_id = ?1", [&peer.id])?;
        connection.execute("DELETE FROM peers WHERE id = ?1", [&peer.id])?;
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

    #[test]
    fn create_group_with_owner_share_writes_all_three_rows() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        let share = store
            .add_share("workspace-key", &workspace, "project", true)
            .unwrap();
        let group = store
            .create_group_with_owner_share("team", "self-1", "self", &share.id, &share.name)
            .unwrap();
        assert!(store
            .members(&group.id)
            .unwrap()
            .iter()
            .any(|member| member.peer_id == "self-1"));
        let contributions = store.group_shares(&group.id, "self-1").unwrap();
        assert_eq!(contributions.len(), 1);
        assert_eq!(contributions[0].share_id, share.id);
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
        assert!(store.is_group_owner("team", "self-1").unwrap());
        assert!(!store.is_group_owner("team", "peer-2").unwrap());
        assert!(!store.is_group_owner("team", "stranger").unwrap());
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
        // The same ticket admits another peer 鈥?group invites are multi-use.
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
    fn group_invite_retry_by_an_admitted_peer_is_idempotent() {
        let (temp, store, _share) = group_store();
        let alice_share = real_share(&store, temp.path(), "alice", "alice-ws");
        let bob_share = real_share(&store, temp.path(), "bob", "bob-ws");

        // max_uses = 1: a single admission.
        let invite = store.create_group_invite("team", 60, Some(1)).unwrap();
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

        // A retry by the same peer (lost `GroupJoined` response, or a crash
        // before the joiner saved its local group) completes instead of being
        // blocked by the usage cap, and does not consume another use.
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

        // The retry did not spend the invite: one different peer still fits.
        let error = store
            .redeem_group_invite(
                &invite.id,
                &invite.secret,
                &joiner(
                    "peer-2",
                    "bob",
                    &[(bob_share.id.clone(), bob_share.name.clone())],
                ),
            )
            .expect_err("max_uses=1 is spent by the first admission");
        assert!(error.to_string().contains("usage limit"));
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
        // Withdrawing one contribution keeps the rest.
        store
            .remove_group_share("team", "self-1", &second.id)
            .unwrap();
        assert_eq!(store.group_shares("team", "self-1").unwrap().len(), 1);
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
    fn repeated_direct_redemption_does_not_duplicate_grant_sources() {
        let (_temp, store, _share) = group_store();
        let first = store.create_invite("project", 60).unwrap();
        let second = store.create_invite("project", 60).unwrap();
        store
            .redeem_invite(&first.id, &first.secret, "peer-1", "alice")
            .unwrap();
        store
            .redeem_invite(&second.id, &second.secret, "peer-1", "alice")
            .unwrap();
        let count: i64 = store
            .connect()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM grant_sources WHERE peer_id = 'peer-1' AND via = 'direct'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "a second direct redeem of the same share must not append a duplicate source row"
        );
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
    fn forgetting_a_peer_clears_its_grant_sources() {
        let (_temp, store, share) = group_store();
        // A direct redeem records a `grant_sources` row that has no foreign
        // key on peers; forgetting the peer must drop it or the stale source
        // would keep a later rejoin's grant alive after a kick.
        store.save_remote_peer("peer-9", "zoe", "{}").unwrap();
        let invite = store.create_invite(&share.name, 60).unwrap();
        store
            .redeem_invite(&invite.id, &invite.secret, "peer-9", "zoe")
            .unwrap();
        store.forget_peer("peer-9").unwrap();
        let remaining: i64 = store
            .connect()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM grant_sources WHERE peer_id = ?1",
                ["peer-9"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "forgetting a peer clears its grant sources");
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

    #[test]
    fn rename_to_own_id_is_not_a_collision() {
        let temp = tempfile::tempdir().unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        let group = store.add_group("team", "self-1", "self").unwrap();
        // The group's own id satisfies the identifier rules and resolves
        // through the same lookup; renaming to it is not a collision.
        let renamed = store.rename_group("team", &group.id).unwrap();
        assert_eq!(renamed.name, group.id);
        // A real collision is still rejected.
        store.add_group("other", "self-1", "self").unwrap();
        let error = store
            .rename_group(&group.id, "other")
            .expect_err("collision");
        assert!(error.to_string().contains("already exists"));
    }

    #[test]
    fn contributed_share_names_reject_reserved_schemes() {
        let temp = tempfile::tempdir().unwrap();
        let store = StateStore::open(temp.path().join("state.db")).unwrap();
        store.add_group("team", "self-1", "self").unwrap();
        let error = store
            .add_group_share("team", "self-1", "sh-1", "local")
            .expect_err("reserved");
        assert!(error.to_string().contains("reserved"));
        let error = store
            .add_group_share("team", "self-1", "sh-1", "sivtr")
            .expect_err("reserved");
        assert!(error.to_string().contains("reserved"));
        store
            .add_group_share("team", "self-1", "sh-1", "bob-ws")
            .unwrap();
    }

    #[test]
    fn migration_backfills_legacy_direct_grants() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.db");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        // Simulate an install from before `grant_sources`: no table, but an
        // active grant issued by a direct redeem.
        let store = StateStore::open(db.clone()).unwrap();
        let share = store.add_share("ws-key", &workspace, "proj", true).unwrap();
        store.save_remote_peer("peer-1", "bob", "{}").unwrap();
        {
            let connection = store.connect().unwrap();
            connection
                .execute_batch(&format!(
                    "DROP TABLE grant_sources;
                     INSERT INTO grants(peer_id, share_id, permission, created_at)
                     VALUES ('peer-1', '{}', 'read_memory', '2026-01-01T00:00:00Z');",
                    share.id
                ))
                .unwrap();
        }
        // Reopening runs the migration: the table is recreated and the active
        // grant gets a direct source, so a later group withdrawal cannot
        // revoke it.
        let store = StateStore::open(db.clone()).unwrap();
        let backfilled: i64 = store
            .connect()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM grant_sources WHERE peer_id = 'peer-1' AND via = 'direct'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(backfilled, 1, "legacy active grants become direct sources");
    }
}
