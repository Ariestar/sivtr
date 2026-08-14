//! Group store: the `groups`, `group_members`, and `group_shares` tables,
//! roster convergence, and group invites. `StateStore` impl blocks split
//! across this module and `state::mod`; the schema lives in `state::mod`.
//!
//! Invariant: for every contribution a member adds, every other member holds
//! a read-memory grant on that share.

use std::str::FromStr;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{
    hash_secret, now, random_id, random_secret, validate_alias, validate_identifier, GroupInfo,
    GroupMemberInfo, GroupRole, GroupShareInfo, InviteRecord, JoinerInfo, RedeemedGroup, RosterRow,
    StateStore, PERMISSION_READ_MEMORY,
};

impl StateStore {
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

    /// Create a group with the owner's first contribution in one transaction.
    /// A group without its owner's contribution is an invalid state, so the
    /// three rows (group, owner membership, owner share) commit or roll back
    /// together.
    pub fn create_group_with_owner_share(
        &self,
        name: &str,
        self_peer_id: &str,
        self_peer_name: &str,
        share_id: &str,
        share_name: &str,
    ) -> Result<GroupInfo> {
        validate_alias(name, "group name")?;
        validate_alias(share_name, "contributed share name")?;
        // The local device is a peer of itself; peers(id) is a FK target.
        self.save_remote_peer(self_peer_id, self_peer_name, "{}")?;
        let id = random_id("grp");
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO groups(id, name, created_at) VALUES (?1, ?2, ?3)",
            params![id, name.to_ascii_lowercase(), now()],
        )?;
        transaction.execute(
            "INSERT INTO group_members(group_id, peer_id, role, joined_at) VALUES (?1, ?2, 'owner', ?3)",
            params![id, self_peer_id, now()],
        )?;
        transaction.execute(
            "INSERT INTO group_shares(group_id, peer_id, share_id, share_name, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, self_peer_id, share_id, share_name, now()],
        )?;
        transaction.commit()?;
        self.group(&id)
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
        // The group's own id is a valid identifier and resolves through the
        // same lookup, so exclude the target row from the collision check.
        let collides = self
            .group_opt(&new_name)?
            .is_some_and(|other| other.id != group.id);
        if new_name != group.name && collides {
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
    pub fn is_group_owner(&self, group_name_or_id: &str, peer_id: &str) -> Result<bool> {
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
                // Remotely supplied roster entries land here; reject reserved
                // scope segments before they produce non-round-trippable refs.
                validate_alias(share_name, "contributed share name")?;
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

    /// Register one contributed workspace for a member (idempotent). The name
    /// becomes a scope segment in group refs, so it must not use a reserved
    /// scheme name (`local`/`sivtr`) that ref parsing rejects.
    pub fn add_group_share(
        &self,
        group_name_or_id: &str,
        peer_id: &str,
        share_id: &str,
        share_name: &str,
    ) -> Result<()> {
        validate_alias(share_name, "contributed share name")?;
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
        if expected_hash != hash_secret(secret) {
            bail!("Invitation is invalid or expired");
        }
        let timestamp = now();
        transaction.execute(
            "INSERT INTO peers(id, name, endpoint_json, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4) ON CONFLICT(id) DO UPDATE SET name = excluded.name, endpoint_json = excluded.endpoint_json, last_seen_at = excluded.last_seen_at",
            params![joiner.peer_id, joiner.peer_name, joiner.endpoint_json, timestamp],
        )?;
        // 0 rows = the peer was already admitted; a retry (lost `GroupJoined`,
        // or a crash before the joiner saved its local group) must complete
        // without consuming another use or being blocked by the cap.
        let admitted = transaction.execute(
            "INSERT INTO group_members(group_id, peer_id, role, joined_at) VALUES (?1, ?2, 'member', ?3) ON CONFLICT(group_id, peer_id) DO NOTHING",
            params![group_id, joiner.peer_id, timestamp],
        )? > 0;
        if admitted {
            if let Some(max_uses) = max_uses {
                if used_count >= max_uses {
                    bail!("Invitation has reached its usage limit");
                }
            }
        }
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
        if admitted {
            transaction.execute(
                "UPDATE invites SET used_count = used_count + 1 WHERE id = ?1",
                [invite_id],
            )?;
        }
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
