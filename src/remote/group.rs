//! Group mode domain: membership choreography, roster convergence, and query
//! fan-out.
//!
//! A group is a roster overlay on the share/grant/mount model. The owner is
//! the roster's source of truth; members pull-sync on a TTL, and membership
//! changes are broadcast so peers converge between syncs. Every local group
//! behavior lives here - the daemon module only routes wire messages in and
//! out, and the state module only stores.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use iroh::EndpointAddr;
use tokio::task::JoinSet;

use super::context::DaemonContext;
use super::net;
use super::protocol::{
    qualify_query_scope, GroupQueryResponse, InviteTicket, MemberInfo, QueryResponse,
    RemoteRequest, RemoteResponse,
};
use super::state::{GroupInfo, GroupMemberInfo, StateStore};
use crate::commands::memory::filter::Filter;

/// Require `sender` to be the group's owner before owner-only requests
/// (roster changes, renames). Binding them to the transport-authenticated
/// sender prevents a member from forging additions (which would grant an
/// attacker read access to other members' contributions) or removals.
pub(crate) fn require_group_owner(store: &StateStore, group_id: &str, sender: &str) -> Result<()> {
    let owner = store
        .members(group_id)?
        .into_iter()
        .find(|member| member.role == "owner")
        .context("Group has no owner")?;
    if owner.peer_id != sender {
        bail!("Only the group owner may perform this operation");
    }
    Ok(())
}

/// Revoke `target`'s grants on every share `contributor` contributes to the
/// group.
fn revoke_member_access(
    store: &StateStore,
    group_id: &str,
    contributor: &str,
    target: &str,
) -> Result<()> {
    for share in store.group_shares(group_id, contributor)? {
        store.revoke_group_grant(group_id, &share.share_id, target)?;
    }
    Ok(())
}

fn member_info_from_store(
    store: &StateStore,
    group_id: &str,
    member: &GroupMemberInfo,
) -> Result<MemberInfo> {
    let shares = store
        .group_shares(group_id, &member.peer_id)?
        .into_iter()
        .map(|share| (share.share_id, share.share_name))
        .collect();
    let id: iroh::EndpointId = member.peer_id.parse().context("Invalid stored node id")?;
    let endpoint = member
        .endpoint
        .as_deref()
        .filter(|json| !json.is_empty())
        .and_then(|json| serde_json::from_str::<EndpointAddr>(json).ok())
        .unwrap_or_else(|| EndpointAddr::new(id));
    Ok(MemberInfo {
        peer_id: member.peer_id.clone(),
        peer_name: member.peer_name.clone(),
        shares,
        role: member.role.clone(),
        endpoint,
    })
}

/// The authoritative roster for `requester`, gating membership reads: `None`
/// when the group is unknown, an empty roster when the requester is not a
/// member (non-members learn only that they are not members), and the full
/// roster otherwise. The owner's own entry carries its live endpoint so
/// joiners can dial back without relying on discovery alone.
pub(crate) fn roster_for(
    context: &Arc<DaemonContext>,
    group_id: &str,
    requester: &str,
) -> Result<Option<(String, Vec<MemberInfo>)>> {
    let Some(group) = context.store.group_opt(group_id)? else {
        return Ok(None);
    };
    let roster: Vec<MemberInfo> = context
        .store
        .members(&group.id)?
        .iter()
        .map(|member| member_info_from_store(&context.store, &group.id, member))
        .collect::<Result<_>>()?;
    if !roster.iter().any(|row| row.peer_id == requester) {
        return Ok(Some((group.name, Vec::new())));
    }
    let owner_addr = context.endpoint.addr();
    let roster = roster
        .into_iter()
        .map(|mut row| {
            if row.peer_id == context.identity.id() {
                row.endpoint = owner_addr.clone();
            }
            row
        })
        .collect();
    Ok(Some((group.name, roster)))
}

/// Upsert peers, membership, contributions, and grants for every member in
/// `roster`. Add-only and idempotent: nothing local is ever removed. Used on
/// the snapshot paths (join mirror, member-added push), where the sender's
/// own share broadcasts race the snapshot - pruning there would let a stale
/// push regress a share that was just added.
fn upsert_roster(
    context: &Arc<DaemonContext>,
    group_id: &str,
    roster: &[MemberInfo],
) -> Result<()> {
    let self_id = context.identity.id();
    let self_shares = context.store.group_shares(group_id, &self_id)?;
    for remote in roster {
        context.store.save_remote_peer(
            &remote.peer_id,
            &remote.peer_name,
            &serde_json::to_string(&remote.endpoint)?,
        )?;
        context
            .store
            .add_member(group_id, &remote.peer_id, &remote.role)?;
        for (share_id, share_name) in &remote.shares {
            context
                .store
                .add_group_share(group_id, &remote.peer_id, share_id, share_name)?;
        }
        // Grant every member read access to our own contributions.
        if remote.peer_id != self_id {
            for share in &self_shares {
                context
                    .store
                    .group_grant(group_id, &share.share_id, &remote.peer_id)?;
            }
        }
    }
    Ok(())
}

/// Make local group state match the owner's authoritative `roster`: upsert,
/// prune each member's stale contributions (except our own - they are local
/// authority), and remove members the roster no longer lists (with their
/// contribution rows and grants). The roster always lists every member
/// including us, so a member missing from it has been removed. Pull-only: the
/// owner sees every share change, so its roster may prune; snapshot pushes
/// must stay add-only via [`upsert_roster`].
fn reconcile_roster(
    context: &Arc<DaemonContext>,
    group_id: &str,
    roster: &[MemberInfo],
) -> Result<()> {
    let self_id = context.identity.id();
    upsert_roster(context, group_id, roster)?;
    for remote in roster {
        if remote.peer_id == self_id {
            continue;
        }
        let local = context.store.group_shares(group_id, &remote.peer_id)?;
        for existing in local {
            if !remote.shares.iter().any(|(id, _)| *id == existing.share_id) {
                context
                    .store
                    .remove_group_share(group_id, &remote.peer_id, &existing.share_id)?;
            }
        }
    }
    let local_members = context.store.members(group_id)?;
    for local in local_members {
        if !roster.iter().any(|remote| remote.peer_id == local.peer_id) {
            context.store.remove_member(group_id, &local.peer_id)?;
            for share in &context.store.group_shares(group_id, &self_id)? {
                context
                    .store
                    .revoke_group_grant(group_id, &share.share_id, &local.peer_id)?;
            }
        }
    }
    Ok(())
}

/// Merge one newcomer into the local roster (the `GroupMemberAdded` push):
/// register the member, its contributions, and grant it our own shares. The
/// push is a join-time snapshot, so it is add-only - it must never prune a
/// share the newcomer broadcast directly (the two are unordered).
pub(crate) fn merge_member(
    context: &Arc<DaemonContext>,
    group_id: &str,
    member: &MemberInfo,
) -> Result<()> {
    upsert_roster(context, group_id, std::slice::from_ref(member))
}

/// Pull the authoritative roster from the owner and reconcile membership,
/// contributions, and grants. If the owner says we are no longer a member,
/// drop the group.
pub(crate) async fn sync_group(context: &Arc<DaemonContext>, group_name_or_id: &str) -> Result<()> {
    let group = context.store.group(group_name_or_id)?;
    let local_members = context.store.members(&group.id)?;
    let owner = local_members
        .iter()
        .find(|member| member.role == "owner")
        .context("Group has no owner")?;
    if owner.peer_id == context.identity.id() {
        // The owner is the roster source of truth; nothing to pull.
        context.store.touch_group_sync(&group.id)?;
        return Ok(());
    }
    // Pulling the roster must never hang a local operation on an unreachable
    // owner; callers fall back to the cached roster after this bound.
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        net::exchange_with_peer(
            &context.store,
            &context.endpoint,
            &owner.peer_id,
            RemoteRequest::GroupSync {
                group_id: group.id.clone(),
            },
        ),
    )
    .await
    .context("Group roster sync with the owner timed out")??;
    match response {
        RemoteResponse::GroupSynced {
            group_name,
            member,
            members,
        } => {
            if !member {
                drop_group(context, &group.id).await?;
                return Ok(());
            }
            // The owner's name is authoritative: apply a rename this device
            // has not synced yet. A collision with another local group keeps
            // the stale name but must not block the roster reconciliation.
            if group_name != group.name {
                if let Err(error) = context.store.rename_group(&group.id, &group_name) {
                    crate::output::error(format!(
                        "kept local name for group `{}`: {error:#}",
                        group.name
                    ));
                }
            }
            reconcile_roster(context, &group.id, &members)?;
            context.store.touch_group_sync(&group.id)?;
            Ok(())
        }
        response => bail!("Unexpected group sync response: {response:?}"),
    }
}

/// Best-effort sync when the cached roster is stale; never blocks callers.
/// The owner short-circuits inside [`sync_group`]. Runs in the background so
/// IPC handlers return the cached roster immediately; staleness is bounded by
/// the sync TTL, and a manual `sivtr group sync` forces a pull.
pub(crate) fn maybe_sync_group(context: &Arc<DaemonContext>, group_name_or_id: &str) {
    let stale = match context.store.sync_stale(group_name_or_id, 300) {
        Ok(stale) => stale,
        Err(_) => return,
    };
    if !stale {
        return;
    }
    let context = context.clone();
    let group = group_name_or_id.to_string();
    tokio::spawn(async move {
        if let Err(error) = sync_group(&context, &group).await {
            // Owner unreachable - the cached roster stays in effect.
            crate::output::error(format!("group sync failed for `{group}`: {error:#}"));
        }
    });
}

/// Remove the group locally: revoke the grants we handed out on every
/// contribution, then drop the group row (members + contributions cascade).
pub(crate) async fn drop_group(context: &Arc<DaemonContext>, group_name_or_id: &str) -> Result<()> {
    let group = context.store.group(group_name_or_id)?;
    let members = context.store.members(&group.id)?;
    let self_id = context.identity.id();
    for member in &members {
        if member.peer_id != self_id {
            revoke_member_access(&context.store, &group.id, &self_id, &member.peer_id)?;
        }
    }
    context.store.remove_group(&group.id)?;
    Ok(())
}

pub(crate) async fn leave_group(
    context: &Arc<DaemonContext>,
    group_name_or_id: &str,
) -> Result<()> {
    let group = context.store.group(group_name_or_id)?;
    let members = context.store.members(&group.id)?;
    let self_id = context.identity.id();
    let self_member = members
        .iter()
        .find(|member| member.peer_id == self_id)
        .context("You are not a member of this group")?;
    let is_owner = self_member.role == "owner";
    // Leaving revokes the grants we handed out on our shares and drops the
    // local group row (membership and contributions cascade), so `group list`
    // stops showing it immediately - even while the owner is offline.
    drop_group(context, &group.id).await?;
    if is_owner {
        // Owner leaving disbands the group: kick every remaining member. Each
        // request names its own target (`peer_id` differs per recipient), so
        // broadcast's shared template does not fit - send them individually.
        for member in &members {
            if member.peer_id == self_id {
                continue;
            }
            let _ = tokio::time::timeout(
                Duration::from_secs(3),
                net::exchange_with_peer(
                    &context.store,
                    &context.endpoint,
                    &member.peer_id,
                    RemoteRequest::GroupMemberRemoved {
                        group_id: group.id.clone(),
                        peer_id: member.peer_id.clone(),
                        peer_name: String::new(),
                    },
                ),
            )
            .await;
        }
    } else if let Some(owner) = members.iter().find(|member| member.role == "owner") {
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            net::exchange_with_peer(
                &context.store,
                &context.endpoint,
                &owner.peer_id,
                RemoteRequest::GroupLeave {
                    group_id: group.id.clone(),
                },
            ),
        )
        .await;
    }
    Ok(())
}

pub(crate) async fn remove_group_member(
    context: &Arc<DaemonContext>,
    group_name_or_id: &str,
    peer_name_or_id: &str,
) -> Result<()> {
    let group = context.store.group(group_name_or_id)?;
    let members = context.store.members(&group.id)?;
    let self_id = context.identity.id();
    let self_member = members
        .iter()
        .find(|member| member.peer_id == self_id)
        .context("You are not a member of this group")?;
    if self_member.role != "owner" {
        bail!("Only the group owner can remove members");
    }
    let target = members
        .iter()
        .find(|member| {
            member.peer_id == peer_name_or_id
                || member.peer_name.eq_ignore_ascii_case(peer_name_or_id)
        })
        .context("Unknown group member")?;
    if target.peer_id == self_id {
        bail!("The group owner cannot remove itself; leave the group to disband it");
    }
    revoke_member_access(&context.store, &group.id, &self_id, &target.peer_id)?;
    // Notify everyone, including the removed peer, before dropping it from
    // the roster: broadcast reads the member list from the store, so the
    // target must still be listed to receive its own removal and clear the
    // group locally. Offline peers converge on their next sync.
    broadcast(
        context,
        &group.id,
        RemoteRequest::GroupMemberRemoved {
            group_id: group.id.clone(),
            peer_id: target.peer_id.clone(),
            peer_name: target.peer_name.clone(),
        },
        None,
    )
    .await;
    context.store.remove_member(&group.id, &target.peer_id)?;
    Ok(())
}

/// Best-effort send `request` to every group member except `skip`. The caller
/// builds the request once from borrowed data; each member task owns a cloned
/// copy ('static tasks), which is the only clone per member.
async fn broadcast(
    context: &Arc<DaemonContext>,
    group_name_or_id: &str,
    request: RemoteRequest,
    skip: Option<&str>,
) {
    let Ok(members) = context.store.members(group_name_or_id) else {
        return;
    };
    let mut tasks = JoinSet::new();
    for member in members {
        if skip == Some(member.peer_id.as_str()) {
            continue;
        }
        let context = context.clone();
        let peer_id = member.peer_id.clone();
        let request = request.clone();
        tasks.spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(3),
                net::exchange_with_peer(&context.store, &context.endpoint, &peer_id, request),
            )
            .await;
        });
    }
    while tasks.join_next().await.is_some() {}
}

/// Client-side join (first time): redeem the invite with the owner, mirror
/// the roster, register our contributed shares, and grant members.
pub(crate) async fn redeem_group_remote(
    context: &Arc<DaemonContext>,
    encoded_invite: &str,
    shares: &[(String, String)],
) -> Result<(String, usize)> {
    let invite = InviteTicket::parse(encoded_invite)?;
    if invite.expires_at < Utc::now().timestamp() {
        bail!("Invitation is expired");
    }
    if invite.group_id.is_none() {
        bail!("Invitation is not a group invite");
    }
    let (response, _observed) = net::exchange(
        &context.endpoint,
        invite.endpoint,
        RemoteRequest::RedeemGroupInvite {
            invite_id: invite.invite_id,
            secret: invite.secret,
            peer_name: context.identity.name.clone(),
            shares: shares.to_vec(),
            endpoint: context.endpoint.addr(),
        },
    )
    .await?;
    // The owner derives the group from the invite row; register under that id.
    let (group_id, group_name, members) = match response {
        RemoteResponse::GroupJoined {
            group_id,
            group_name,
            members,
        } => (group_id, group_name, members),
        response => bail!("Unexpected invitation response: {response:?}"),
    };
    // Our own device is a peer of itself (FK target in group_members).
    context
        .store
        .save_remote_peer(&context.identity.id(), &context.identity.name, "{}")?;
    // Mirror the owner-assigned group identity and join it.
    context.store.register_group(&group_id, &group_name)?;
    context
        .store
        .add_member(&group_id, &context.identity.id(), "member")?;
    for (share_id, share_name) in shares {
        context
            .store
            .add_group_share(&group_id, &context.identity.id(), share_id, share_name)?;
    }
    // Converge on the returned roster and grant every member our shares. The
    // mirror is a join-time snapshot: add-only, never prune.
    upsert_roster(context, &group_id, &members)?;
    context.store.touch_group_sync(&group_id)?;
    Ok((group_name, members.len()))
}

/// Owner side of a group join: validate the ticket, add the joiner and its
/// contributions, grant it the owner's shares, notify the other members, and
/// return the authoritative group id with the current roster.
pub(crate) async fn handle_redeem_group_invite(
    context: &Arc<DaemonContext>,
    sender: &str,
    invite_id: &str,
    secret: &str,
    peer_name: String,
    shares: Vec<(String, String)>,
    endpoint: EndpointAddr,
) -> Result<RemoteResponse> {
    let endpoint_json = serde_json::to_string(&endpoint)?;
    let joiner = super::state::JoinerInfo {
        peer_id: sender,
        peer_name: &peer_name,
        shares: &shares,
        endpoint_json: &endpoint_json,
    };
    let redeemed = context
        .store
        .redeem_group_invite(invite_id, secret, &joiner)?;
    // The invite row is the authority: the group is derived from the ticket,
    // never from the joiner's request, and used for every follow-up (name
    // lookup, roster, broadcast).
    let group_id = redeemed.group_id;
    let group_name = context.store.group(&group_id)?.name;
    let members: Vec<MemberInfo> = redeemed
        .roster
        .iter()
        .map(|member| member_info_from_store(&context.store, &group_id, member))
        .collect::<Result<_>>()?;
    // Ensure the owner's own roster entry carries the live endpoint so the
    // joiner can dial back without relying on discovery alone.
    let owner_addr = context.endpoint.addr();
    let members = members
        .into_iter()
        .map(|mut member| {
            if member.peer_id == context.identity.id() {
                member.endpoint = owner_addr.clone();
            }
            member
        })
        .collect();
    // Notify existing members about the newcomer so they can grant the
    // newcomer access. Offline members reconcile on their next sync.
    let newcomer = MemberInfo {
        peer_id: sender.to_string(),
        peer_name,
        shares,
        role: "member".to_string(),
        endpoint,
    };
    let context = context.clone();
    let newcomer_id = newcomer.peer_id.clone();
    let broadcast_group_id = group_id.clone();
    tokio::spawn(async move {
        broadcast(
            &context,
            &broadcast_group_id,
            RemoteRequest::GroupMemberAdded {
                group_id: broadcast_group_id.clone(),
                member: newcomer,
            },
            Some(&newcomer_id),
        )
        .await;
    });
    Ok(RemoteResponse::GroupJoined {
        group_id,
        group_name,
        members,
    })
}

/// An existing member re-runs join with the final checkbox list: register new
/// contributions (granting every member), withdraw unchecked ones (revoking
/// grants), and broadcast both directions so peers stay in sync.
pub(crate) async fn adjust_group_shares(
    context: &Arc<DaemonContext>,
    group_name_or_id: &str,
    shares: &[(String, String)],
) -> Result<()> {
    // The mesh contract requires at least one contribution; an empty final
    // set would keep the membership while consuming everyone else's memory.
    if shares.is_empty() {
        bail!("A group member must contribute at least one workspace");
    }
    let group = context.store.group(group_name_or_id)?;
    let self_id = context.identity.id();
    let members = context.store.members(&group.id)?;
    let current = context.store.group_shares(&group.id, &self_id)?;
    let wanted: &[(String, String)] = shares;

    for (share_id, share_name) in wanted {
        if !current
            .iter()
            .any(|existing| existing.share_id == *share_id)
        {
            context
                .store
                .add_group_share(&group.id, &self_id, share_id, share_name)?;
            for member in &members {
                if member.peer_id != self_id {
                    context
                        .store
                        .group_grant(&group.id, share_id, &member.peer_id)?;
                }
            }
            broadcast(
                context,
                &group.id,
                RemoteRequest::GroupShareAdded {
                    group_id: group.id.clone(),
                    peer_id: self_id.clone(),
                    peer_name: String::new(),
                    share_id: share_id.clone(),
                    share_name: share_name.clone(),
                },
                Some(&self_id),
            )
            .await;
        }
    }
    for existing in current {
        if !wanted.iter().any(|(id, _)| *id == existing.share_id) {
            context
                .store
                .remove_group_share(&group.id, &self_id, &existing.share_id)?;
            context
                .store
                .revoke_group_share(&group.id, &existing.share_id, &self_id)?;
            broadcast(
                context,
                &group.id,
                RemoteRequest::GroupShareRemoved {
                    group_id: group.id.clone(),
                    peer_id: self_id.clone(),
                    share_id: existing.share_id.clone(),
                },
                Some(&self_id),
            )
            .await;
        }
    }
    Ok(())
}

/// Receiver side of a `GroupMemberRemoved` push: the removed peer drops the
/// group locally; everyone else revokes that peer's grants and removes it.
/// The router has already verified the sender is the group owner.
pub(crate) async fn handle_member_removed(
    context: &Arc<DaemonContext>,
    group_id: &str,
    removed_peer: &str,
) -> Result<RemoteResponse> {
    if removed_peer == context.identity.id() {
        // We were kicked (or the owner disbanded the group).
        drop_group(context, group_id).await?;
    } else {
        revoke_member_access(
            &context.store,
            group_id,
            &context.identity.id(),
            removed_peer,
        )?;
        context.store.remove_member(group_id, removed_peer)?;
    }
    Ok(RemoteResponse::GroupAck)
}

/// Owner side of a member leave: drop the leaver from the authoritative
/// roster, revoke its grants on owner contributions, and broadcast the
/// removal so every member revokes the leaver too.
pub(crate) async fn handle_leave(
    context: &Arc<DaemonContext>,
    group_id: &str,
    sender: &str,
) -> Result<()> {
    let members_before = context.store.members(group_id)?;
    let leaver = members_before
        .iter()
        .find(|row| row.peer_id == sender)
        .cloned()
        .context("Unknown group member")?;
    context.store.remove_member(group_id, sender)?;
    // Revoke every owner contribution from the leaver.
    if let Some(owner) = members_before.iter().find(|row| row.role == "owner") {
        revoke_member_access(&context.store, group_id, &owner.peer_id, sender)?;
    }
    broadcast(
        context,
        group_id,
        RemoteRequest::GroupMemberRemoved {
            group_id: group_id.to_string(),
            peer_id: leaver.peer_id.clone(),
            peer_name: leaver.peer_name.clone(),
        },
        None,
    )
    .await;
    Ok(())
}

/// Fan out a group query: the caller's own contributions run in-process (a
/// failure is a real error), every remote (member, share) is dialed in parallel
/// under a per-peer budget, and results are merged qualified per member and
/// share. Members that did not answer are reported as skipped.
pub(crate) async fn group_fan_out(
    context: &Arc<DaemonContext>,
    group_name: &str,
    members: &[MemberInfo],
    source: &str,
    filter: Filter,
) -> Result<GroupQueryResponse> {
    const PER_PEER_TIMEOUT: Duration = Duration::from_millis(2500);
    // Shares only bound the set (pattern/status/time/...). Ordering, the
    // `latest` window, and `limit` are group-wide: pushed down, they would
    // return a per-share top-k (five per share for `--latest 5`) instead of
    // the group's global top results, so they are applied once on the merged
    // corpus below.
    let full = filter.for_remote_peer();
    let mut bounds = full.clone();
    bounds.rank = None;
    bounds.latest = None;
    bounds.limit = None;

    let self_id = context.identity.id();
    let mut records = Vec::new();
    let mut anchors = Vec::new();
    // Every result is scoped `team/<peer-id>/proj-b` so members stay apart and
    // records round-trip through show/zoom/nav. The member segment is the
    // stable peer id, not the display name: two devices can share a hostname
    // (and a workspace name), and a name-based scope would collide and make
    // the refs ambiguous.
    let mut merge = |peer_id: &str, share_name: &str, mut query: QueryResponse| {
        qualify_query_scope(&format!("{group_name}/{peer_id}/{share_name}"), &mut query);
        records.extend(query.records);
        anchors.extend(query.anchors);
    };

    // The local member's contributions are part of the group, so they are
    // queried like any other share - just in-process instead of over the wire.
    // Local failures propagate; they are not "offline" peers.
    for member in members.iter().filter(|member| member.peer_id == self_id) {
        for (share_id, share_name) in &member.shares {
            let share = context.store.share(share_id)?;
            let query = tokio::task::spawn_blocking({
                let root = share.root.clone();
                let source = source.to_string();
                let filter = bounds.clone();
                move || {
                    let (records, anchors) = crate::commands::memory::workset::run_on_share(
                        std::path::Path::new(&root),
                        &source,
                        filter,
                        share.redact,
                    )?;
                    Ok::<_, anyhow::Error>(QueryResponse { records, anchors })
                }
            })
            .await??;
            merge(&member.peer_id, share_name, query);
        }
    }

    let mut tasks = JoinSet::new();
    for member in members.iter().filter(|member| member.peer_id != self_id) {
        for (share_id, share_name) in &member.shares {
            let context = context.clone();
            let peer_id = member.peer_id.clone();
            let share_id = share_id.clone();
            let share_name = share_name.clone();
            let source = source.to_string();
            let filter = bounds.clone();
            tasks.spawn(async move {
                let result = tokio::time::timeout(
                    PER_PEER_TIMEOUT,
                    net::exchange_with_peer(
                        &context.store,
                        &context.endpoint,
                        &peer_id,
                        RemoteRequest::Query {
                            share_id,
                            source,
                            filter,
                        },
                    ),
                )
                .await;
                (peer_id, share_name, result)
            });
        }
    }
    // A member is online when any of its shares answered, decided only after
    // every share task completes so a later failure cannot reclassify a peer
    // whose records were already merged.
    let mut online: HashSet<String> = HashSet::new();
    while let Some(joined) = tasks.join_next().await {
        let Ok((peer_id, share_name, result)) = joined else {
            continue;
        };
        if let Ok(Ok(RemoteResponse::Query(query))) = result {
            merge(&peer_id, &share_name, query);
            online.insert(peer_id);
        }
    }
    let skipped: Vec<String> = members
        .iter()
        .filter(|member| member.peer_id != self_id && !online.contains(&member.peer_id))
        .map(|member| member.peer_name.clone())
        .collect();

    // Order the merged corpus once, as one group: `latest` window -> sort ->
    // `limit`. Re-running the pipeline here is idempotent for the bounds each
    // share already applied; the shared code also ranks the merged corpus as
    // a whole when the sort is relevance.
    let merged = crate::commands::memory::filter::apply(PathBuf::new(), records, anchors, full)?;
    Ok(GroupQueryResponse {
        query: QueryResponse {
            records: merged.records,
            anchors: merged.anchors,
        },
        skipped,
    })
}

/// Resolve which (member, share) pairs a group query fans out to. The caller's
/// own contribution is a target like any other member's - the local member is
/// queried in-process by [`group_fan_out`]. `member` and `share` pin the
/// three-segment scopes `team/alice` and `team/alice/proj-b`.
pub(crate) fn group_targets(
    store: &StateStore,
    group: &GroupInfo,
    member: Option<&str>,
    share: Option<&str>,
) -> Result<Vec<MemberInfo>> {
    let all: Vec<MemberInfo> = store
        .members(&group.id)?
        .iter()
        .map(|member| member_info_from_store(store, &group.id, member))
        .collect::<Result<_>>()?;
    let mut targets: Vec<MemberInfo> = match member {
        Some(name) => {
            let needle = name.to_ascii_lowercase();
            let matches: Vec<MemberInfo> = all
                .into_iter()
                .filter(|member| {
                    member.peer_name.to_ascii_lowercase() == needle || member.peer_id == needle
                })
                .collect();
            if matches.is_empty() {
                bail!("No group member named `{name}` in `{}`", group.name);
            }
            matches
        }
        None => all,
    };
    // `team/alice/proj-b` pins one contributed share per member.
    if let Some(share_name) = share {
        for target in &mut targets {
            target
                .shares
                .retain(|(_, name)| name.eq_ignore_ascii_case(share_name));
        }
        targets.retain(|target| !target.shares.is_empty());
        if targets.is_empty() {
            bail!(
                "No member contributes a share named `{share_name}` in `{}`",
                group.name
            );
        }
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::identity::Identity;
    use crate::remote::protocol::REMOTE_ALPN;

    fn group_store() -> (tempfile::TempDir, StateStore) {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = StateStore::open(temp.path().join("state.db")).expect("store");
        store.add_group("team", "self-1", "self").expect("group");
        store.save_remote_peer("peer-2", "bob", "{}").expect("peer");
        store
            .add_member("team", "peer-2", "member")
            .expect("member");
        (temp, store)
    }

    #[test]
    fn only_the_owner_may_change_membership() {
        let (_temp, store) = group_store();
        require_group_owner(&store, "team", "self-1").expect("owner passes");
        let error = require_group_owner(&store, "team", "peer-2").expect_err("member rejected");
        assert!(error.to_string().contains("Only the group owner"));
        let error = require_group_owner(&store, "team", "stranger").expect_err("outsider rejected");
        assert!(error.to_string().contains("Only the group owner"));
    }

    #[test]
    fn unknown_group_has_no_owner() {
        let (_temp, store) = group_store();
        let error = require_group_owner(&store, "missing", "self-1").expect_err("unknown group");
        assert!(error.to_string().contains("Unknown group"));
    }

    #[test]
    fn group_targets_include_the_local_member() {
        let (_temp, store, self_id) = group_with_members();
        let group = store.group("team").expect("group");
        let targets = group_targets(&store, &group, None, None).expect("targets");
        assert!(
            targets.iter().any(|member| member.peer_id == self_id),
            "the caller's own contribution is a fan-out target"
        );
        assert!(targets.iter().any(|member| member.peer_name == "bob"));
    }

    #[test]
    fn group_targets_pin_one_member_by_name_or_id() {
        let (_temp, store, self_id) = group_with_members();
        let group = store.group("team").expect("group");
        let targets = group_targets(&store, &group, Some("self"), None).expect("targets");
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].peer_id, self_id,
            "self-query resolves to the local member"
        );
        let error =
            group_targets(&store, &group, Some("nobody"), None).expect_err("unknown member");
        assert!(error.to_string().contains("No group member named"));
    }

    #[test]
    fn group_targets_pin_one_contributed_share() {
        let (_temp, store, _self_id) = group_with_members();
        let group = store.group("team").expect("group");
        let targets = group_targets(&store, &group, None, Some("project")).expect("targets");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].peer_name, "self");
        assert_eq!(targets[0].shares.len(), 1);

        let error =
            group_targets(&store, &group, None, Some("missing")).expect_err("unknown share");
        assert!(error.to_string().contains("No member contributes a share"));
    }

    /// Group with the owner contributing `project` and a second member `bob`,
    /// using real node ids so `member_info_from_store` can build fan-out targets.
    fn group_with_members() -> (tempfile::TempDir, StateStore, String) {
        let temp = tempfile::tempdir().expect("temp dir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace dir");
        let store = StateStore::open(temp.path().join("state.db")).expect("store");
        let share = store
            .add_share("workspace-key", &workspace, "project", true)
            .expect("share");
        let self_id = iroh::SecretKey::generate().public().to_string();
        let bob_id = iroh::SecretKey::generate().public().to_string();
        store.add_group("team", &self_id, "self").expect("group");
        store
            .add_group_share("team", &self_id, &share.id, &share.name)
            .expect("contribution");
        store.save_remote_peer(&bob_id, "bob", "{}").expect("peer");
        store.add_member("team", &bob_id, "member").expect("member");
        (temp, store, self_id)
    }

    /// Real daemon context (bound endpoint, temp store, generated identity)
    /// plus a group owned by `self` contributing `owner-ws`, and a member
    /// `bob`. Production signatures stay context-based; the seam lives
    /// entirely in this test module.
    async fn context_with_members() -> (tempfile::TempDir, Arc<DaemonContext>, String) {
        let temp = tempfile::tempdir().expect("temp dir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace dir");
        let store = StateStore::open(temp.path().join("state.db")).expect("store");
        let owner_share = store
            .add_share("workspace-key", &workspace, "owner-ws", true)
            .expect("owner share");
        let identity = Identity {
            name: "self".to_string(),
            secret_key: iroh::SecretKey::generate(),
        };
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
            .secret_key(identity.secret_key.clone())
            .alpns(vec![REMOTE_ALPN.to_vec()])
            .bind()
            .await
            .expect("endpoint");
        let self_id = identity.id();
        store.add_group("team", &self_id, "self").expect("group");
        store
            .add_group_share("team", &self_id, &owner_share.id, &owner_share.name)
            .expect("owner contribution");
        let bob_id = iroh::SecretKey::generate().public().to_string();
        store.save_remote_peer(&bob_id, "bob", "{}").expect("peer");
        store.add_member("team", &bob_id, "member").expect("member");
        let context = Arc::new(DaemonContext {
            store,
            endpoint,
            identity,
            started_at: String::new(),
            control_token: String::new(),
        });
        (temp, context, self_id)
    }

    /// A roster row the way it travels on the wire.
    fn member_info(
        peer_id: &str,
        peer_name: &str,
        role: &str,
        shares: Vec<(String, String)>,
    ) -> MemberInfo {
        let id: iroh::EndpointId = peer_id.parse().expect("node id");
        MemberInfo {
            peer_id: peer_id.to_string(),
            peer_name: peer_name.to_string(),
            shares,
            role: role.to_string(),
            endpoint: iroh::EndpointAddr::new(id),
        }
    }

    #[tokio::test]
    async fn member_add_push_never_prunes_concurrent_share() {
        let (temp, context, _self_id) = context_with_members().await;
        let store = &context.store;
        let workspace = temp.path().join("workspace");
        let bob_id = store
            .members("team")
            .expect("members")
            .into_iter()
            .find(|member| member.peer_name == "bob")
            .expect("bob")
            .peer_id;
        let s2 = store
            .add_share("workspace-bob", &workspace, "bob-ws", true)
            .expect("share");
        let s3 = store
            .add_share("workspace-bob-2", &workspace, "bob-ws-2", true)
            .expect("share");
        store
            .add_group_share("team", &bob_id, &s2.id, &s2.name)
            .expect("s2 broadcast");
        store
            .add_group_share("team", &bob_id, &s3.id, &s3.name)
            .expect("s3 broadcast");
        // The owner's join-time snapshot is stale: it predates the S3
        // broadcast and does not list it.
        let stale_push = member_info(&bob_id, "bob", "member", vec![(s2.id.clone(), s2.name.clone())]);
        merge_member(&context, "team", &stale_push).expect("merge");
        let shares = store.group_shares("team", &bob_id).expect("shares");
        assert!(
            shares.iter().any(|share| share.share_id == s3.id),
            "an add-only push must never prune a share broadcast concurrently"
        );
        assert!(shares.iter().any(|share| share.share_id == s2.id));
    }

    #[tokio::test]
    async fn reconcile_prunes_stale_shares_on_pull() {
        let (temp, context, self_id) = context_with_members().await;
        let store = &context.store;
        let workspace = temp.path().join("workspace");
        let owner_share = store.share("owner-ws").expect("owner share");
        let bob_id = store
            .members("team")
            .expect("members")
            .into_iter()
            .find(|member| member.peer_name == "bob")
            .expect("bob")
            .peer_id;
        let s2 = store
            .add_share("workspace-bob", &workspace, "bob-ws", true)
            .expect("share");
        let s3 = store
            .add_share("workspace-bob-2", &workspace, "bob-ws-2", true)
            .expect("share");
        store
            .add_group_share("team", &bob_id, &s2.id, &s2.name)
            .expect("s2");
        store
            .add_group_share("team", &bob_id, &s3.id, &s3.name)
            .expect("s3");
        // The owner's authoritative roster no longer lists S3 (Bob withdrew
        // it); the pull must prune the local copy.
        let roster = vec![
            member_info(
                &self_id,
                "self",
                "owner",
                vec![(owner_share.id.clone(), owner_share.name.clone())],
            ),
            member_info(&bob_id, "bob", "member", vec![(s2.id.clone(), s2.name.clone())]),
        ];
        reconcile_roster(&context, "team", &roster).expect("reconcile");
        let shares = store.group_shares("team", &bob_id).expect("shares");
        assert_eq!(shares.len(), 1, "withdrawn share pruned by the pull");
        assert_eq!(shares[0].share_id, s2.id);
    }

    #[tokio::test]
    async fn reconcile_drops_members_absent_from_roster() {
        let (temp, context, self_id) = context_with_members().await;
        let store = &context.store;
        let workspace = temp.path().join("workspace");
        let owner_share = store.share("owner-ws").expect("owner share");
        let bob_id = store
            .members("team")
            .expect("members")
            .into_iter()
            .find(|member| member.peer_name == "bob")
            .expect("bob")
            .peer_id;
        // Carol joined after the last pull; the owner then kicked her.
        let carol_id = iroh::SecretKey::generate().public().to_string();
        store.save_remote_peer(&carol_id, "carol", "{}").expect("peer");
        store.add_member("team", &carol_id, "member").expect("member");
        let carol_share = store
            .add_share("workspace-carol", &workspace, "carol-ws", true)
            .expect("share");
        store
            .add_group_share("team", &carol_id, &carol_share.id, &carol_share.name)
            .expect("carol contribution");
        store
            .group_grant("team", &owner_share.id, &carol_id)
            .expect("grant");
        // The authoritative roster lists every remaining member (self + bob);
        // carol is absent, so the pull must drop her and revoke her grants.
        let roster = vec![
            member_info(
                &self_id,
                "self",
                "owner",
                vec![(owner_share.id.clone(), owner_share.name.clone())],
            ),
            member_info(&bob_id, "bob", "member", Vec::new()),
        ];
        reconcile_roster(&context, "team", &roster).expect("reconcile");
        let members = store.members("team").expect("members");
        assert!(
            !members.iter().any(|member| member.peer_id == carol_id),
            "member absent from the owner roster is dropped"
        );
        assert!(
            store.group_shares("team", &carol_id).expect("shares").is_empty(),
            "dropped member's contribution rows are cleaned"
        );
        assert!(
            store
                .grants(&owner_share.id)
                .expect("grants")
                .iter()
                .all(|grant| grant.peer_id != carol_id),
            "dropped member's grants on our shares are revoked"
        );
    }
}
