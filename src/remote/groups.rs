//! Group mode domain: membership choreography and roster convergence.
//!
//! A group is a roster overlay on the share/grant/mount model. The owner is
//! the roster's source of truth; members pull-sync on a TTL, and membership
//! changes are broadcast so peers converge between syncs. Query fan-out over
//! the roster lives in [`super::fanout`]. Every local group behavior lives
//! here - the daemon module only routes wire messages in and out, and the
//! state module only stores.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use iroh::EndpointAddr;
use tokio::task::JoinSet;

use super::context::DaemonContext;
use super::net;
use super::protocol::{InviteTicket, LocalResponse, MemberInfo, RemoteRequest, RemoteResponse};
use super::state::{GroupMemberInfo, RosterRow, StateStore};
use crate::commands::memory::filter::Filter;

/// Time budgets for group convergence. The client-side IPC read timeout
/// (`workset::GROUP_QUERY_TIMEOUT`) must stay at least [`SYNC_PULL_TIMEOUT`]
/// plus the per-share fan-out budget (`fanout::PER_SHARE_QUERY_TIMEOUT`), so
/// a query is never cut off mid-flight.
const SYNC_PULL_TIMEOUT: Duration = Duration::from_secs(5);
const BROADCAST_TIMEOUT: Duration = Duration::from_secs(3);
/// The cached roster is re-pulled from the owner after this many seconds.
const SYNC_TTL_SECS: i64 = 300;

/// Require `sender` to be the group's owner before owner-only requests
/// (roster changes, renames). Binding them to the transport-authenticated
/// sender prevents a member from forging additions (which would grant an
/// attacker read access to other members' contributions) or removals.
pub(crate) fn require_group_owner(store: &StateStore, group_id: &str, sender: &str) -> Result<()> {
    if !store.is_group_owner(group_id, sender)? {
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

pub(crate) fn member_info_from_store(
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
        role: member.role.as_wire().to_string(),
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
/// push regress a share that was just added. Runs in one store transaction.
fn upsert_roster(
    context: &Arc<DaemonContext>,
    group_id: &str,
    roster: &[MemberInfo],
) -> Result<()> {
    let self_id = context.identity.id();
    context
        .store
        .apply_roster_add_only(group_id, &self_id, &roster_rows(roster)?)
}

/// Make local group state match the owner's authoritative `roster`: upsert,
/// prune each member's stale contributions (except our own - they are local
/// authority), and remove members the roster no longer lists (with their
/// contribution rows and grants). The roster always lists every member
/// including us, so a member missing from it has been removed. Pull-only: the
/// owner sees every share change, so its roster may prune; snapshot pushes
/// must stay add-only via [`upsert_roster`]. Runs in one store transaction.
fn reconcile_roster(
    context: &Arc<DaemonContext>,
    group_id: &str,
    roster: &[MemberInfo],
) -> Result<()> {
    let self_id = context.identity.id();
    context
        .store
        .apply_roster_reconcile(group_id, &self_id, &roster_rows(roster)?)
}

/// Convert wire roster rows into the store-level form the transactional
/// converge methods consume.
fn roster_rows(roster: &[MemberInfo]) -> Result<Vec<RosterRow>> {
    roster
        .iter()
        .map(|member| {
            Ok(RosterRow {
                peer_id: member.peer_id.clone(),
                peer_name: member.peer_name.clone(),
                role: member.role.clone(),
                shares: member.shares.clone(),
                endpoint_json: serde_json::to_string(&member.endpoint)?,
            })
        })
        .collect()
}

/// Merge the owner's post-join roster snapshot (the `GroupMemberAdded` push):
/// register every member in it add-only and adopt the watermark. The push
/// carries the full roster, so a snapshot that lost the epoch race to a newer
/// one is dropped wholesale - a member registered in between the two joins is
/// already present in the newer snapshot, never lost to the guard.
pub(crate) fn merge_member(
    context: &Arc<DaemonContext>,
    group_id: &str,
    members: &[MemberInfo],
    roster_epoch: i64,
) -> Result<()> {
    if roster_epoch <= context.store.roster_epoch(group_id)? {
        return Ok(());
    }
    upsert_roster(context, group_id, members)?;
    context.store.adopt_roster_epoch(group_id, roster_epoch)?;
    Ok(())
}

/// Pull the authoritative roster from the owner and reconcile membership,
/// contributions, and grants. If the owner says we are no longer a member,
/// drop the group.
pub(crate) async fn sync_group(context: &Arc<DaemonContext>, group_name_or_id: &str) -> Result<()> {
    let group = context.store.group(group_name_or_id)?;
    let owner = context.store.owner(&group.id)?;
    if owner.peer_id == context.identity.id() {
        // The owner is the roster source of truth; nothing to pull.
        context.store.touch_group_sync(&group.id)?;
        return Ok(());
    }
    // Pulling the roster must never hang a local operation on an unreachable
    // owner; callers fall back to the cached roster after this bound.
    // The request carries our current contribution list: the owner repairs
    // its authoritative roster from it, so a share change whose broadcast was
    // missed while the owner was offline still converges on the next pull.
    let shares: Vec<(String, String)> = context
        .store
        .group_shares(&group.id, &context.identity.id())?
        .into_iter()
        .map(|share| (share.share_id, share.share_name))
        .collect();
    let response = tokio::time::timeout(
        SYNC_PULL_TIMEOUT,
        net::exchange_with_peer(
            &context.store,
            &context.endpoint,
            &owner.peer_id,
            RemoteRequest::GroupSync {
                group_id: group.id.clone(),
                shares,
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
            roster_epoch,
        } => {
            if !member {
                drop_group(context, &group.id).await?;
                return Ok(());
            }
            // Discard a stale pull response: a member-added broadcast may
            // have advanced our watermark past this snapshot, and reconciling
            // it would silently drop that newcomer (and its grants) until the
            // next sync. The owner answers the next pull with the fresh state.
            if roster_epoch <= context.store.roster_epoch(&group.id)? {
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
            context.store.adopt_roster_epoch(&group.id, roster_epoch)?;
            context.store.touch_group_sync(&group.id)?;
            Ok(())
        }
        response => bail!("Unexpected group sync response: {response:?}"),
    }
}

/// Owner side of a `GroupSync`: the member reports its current contribution
/// list, which is authoritative for its own rows. Repair the roster - add
/// missing contributions (granting the other members) and withdraw rows the
/// member no longer lists (revoking the other members' grants) - so later
/// pulls converge even when a share broadcast was missed while the owner was
/// offline. The caller has already verified the sender is a member.
pub(crate) fn sync_member_shares(
    store: &StateStore,
    group_id: &str,
    peer_id: &str,
    shares: &[(String, String)],
) -> Result<()> {
    let current = store.group_shares(group_id, peer_id)?;
    for (share_id, share_name) in shares {
        if !current.iter().any(|share| share.share_id == *share_id) {
            store.add_group_share(group_id, peer_id, share_id, share_name)?;
            for member in store.members(group_id)? {
                if member.peer_id != peer_id {
                    store.group_grant(group_id, share_id, &member.peer_id)?;
                }
            }
        }
    }
    for existing in current {
        if !shares.iter().any(|(id, _)| id == &existing.share_id) {
            store.remove_group_share(group_id, peer_id, &existing.share_id)?;
            store.revoke_group_share(group_id, &existing.share_id, peer_id)?;
        }
    }
    Ok(())
}

/// Best-effort sync when the cached roster is stale; never blocks callers.
/// The owner short-circuits inside [`sync_group`]. Runs in the background so
/// IPC handlers return the cached roster immediately; staleness is bounded by
/// the sync TTL, and a manual `sivtr group sync` forces a pull.
pub(crate) fn maybe_sync_group(context: &Arc<DaemonContext>, group_name_or_id: &str) {
    let stale = match context.store.sync_stale(group_name_or_id, SYNC_TTL_SECS) {
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
    let is_owner = self_member.role.is_owner();
    // Leaving revokes the grants we handed out on our shares and drops the
    // local group row (membership and contributions cascade), so `group list`
    // stops showing it immediately - even while the owner is offline.
    if is_owner {
        // Owner leaving disbands the group: kick every remaining member. The
        // epoch is bumped while the group row still exists so the disband
        // removal carries a fresh watermark (never droppable as stale). Each
        // request names its own target (`peer_id` differs per recipient), so
        // broadcast's shared template does not fit - send them individually.
        let roster_epoch = context.store.bump_roster_epoch(&group.id)?;
        for member in &members {
            if member.peer_id == self_id {
                continue;
            }
            let _ = tokio::time::timeout(
                BROADCAST_TIMEOUT,
                net::exchange_with_peer(
                    &context.store,
                    &context.endpoint,
                    &member.peer_id,
                    RemoteRequest::GroupMemberRemoved {
                        group_id: group.id.clone(),
                        peer_id: member.peer_id.clone(),
                        peer_name: String::new(),
                        roster_epoch,
                    },
                ),
            )
            .await;
        }
        drop_group(context, &group.id).await?;
    } else {
        drop_group(context, &group.id).await?;
        if let Some(owner) = members.iter().find(|member| member.role.is_owner()) {
            let _ = tokio::time::timeout(
                BROADCAST_TIMEOUT,
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
    if !self_member.role.is_owner() {
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
    // Commit the removal and the epoch bump before the broadcast, then notify
    // the pre-removal snapshot (which still lists the target). A background
    // sync from the target in the window now answers `member: false` instead
    // of adopting the new epoch and then discarding this removal as stale -
    // the kicked device clears the group either way.
    let recipients = context.store.members(&group.id)?;
    context.store.remove_member(&group.id, &target.peer_id)?;
    let roster_epoch = context.store.bump_roster_epoch(&group.id)?;
    broadcast(
        context,
        &recipients,
        RemoteRequest::GroupMemberRemoved {
            group_id: group.id.clone(),
            peer_id: target.peer_id.clone(),
            peer_name: target.peer_name.clone(),
            roster_epoch,
        },
        None,
    )
    .await;
    Ok(())
}

/// Best-effort send `request` to every peer in `members` except `skip`. The
/// caller supplies the recipient snapshot so a broadcast can include a member
/// that is being removed from the store at the same time; each member task
/// owns a cloned copy ('static tasks), which is the only clone per member.
async fn broadcast(
    context: &Arc<DaemonContext>,
    members: &[GroupMemberInfo],
    request: RemoteRequest,
    skip: Option<&str>,
) {
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
                BROADCAST_TIMEOUT,
                net::exchange_with_peer(&context.store, &context.endpoint, &peer_id, request),
            )
            .await;
        });
    }
    while tasks.join_next().await.is_some() {}
}

/// Client-side join (first time): redeem the invite with the owner, mirror
/// the roster, register our contributed shares, and grant members. The ticket
/// is already parsed and validated at the daemon boundary ([`InviteTicket`]).
pub(crate) async fn redeem_group_remote(
    context: &Arc<DaemonContext>,
    invite: &InviteTicket,
    shares: &[(String, String)],
) -> Result<(String, usize)> {
    let (response, _observed) = net::exchange(
        &context.endpoint,
        invite.endpoint.clone(),
        RemoteRequest::RedeemGroupInvite {
            invite_id: invite.invite_id.clone(),
            secret: invite.secret.clone(),
            peer_name: context.identity.name.clone(),
            shares: shares.to_vec(),
            endpoint: context.endpoint.addr(),
        },
    )
    .await?;
    // The owner derives the group from the invite row; register under that id.
    let (group_id, group_name, members, roster_epoch) = match response {
        RemoteResponse::GroupJoined {
            group_id,
            group_name,
            members,
            roster_epoch,
        } => (group_id, group_name, members, roster_epoch),
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
    // The join's roster epoch is the local watermark from the start, so
    // broadcasts predating the join are never applied.
    context.store.adopt_roster_epoch(&group_id, roster_epoch)?;
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
    // The advertised endpoint is dialed by every member under this joiner's
    // id; require its id to be the transport-authenticated sender, or queries
    // would route to an unrelated endpoint and the contribution unreachable.
    if endpoint.id.to_string() != sender {
        bail!("Advertised endpoint id does not match the joining device");
    }
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
    // lookup, roster, broadcast). The join bumps the roster epoch so both the
    // response and the newcomer broadcast carry a fresh watermark.
    let group_id = redeemed.group_id;
    let roster_epoch = context.store.bump_roster_epoch(&group_id)?;
    let group_name = context.store.group(&group_id)?.name;
    let members: Vec<MemberInfo> = redeemed
        .roster
        .iter()
        .map(|member| member_info_from_store(&context.store, &group_id, member))
        .collect::<Result<_>>()?;
    // Ensure the owner's own roster entry carries the live endpoint so the
    // joiner can dial back without relying on discovery alone.
    let owner_addr = context.endpoint.addr();
    let members: Vec<MemberInfo> = members
        .into_iter()
        .map(|mut member| {
            if member.peer_id == context.identity.id() {
                member.endpoint = owner_addr.clone();
            }
            member
        })
        .collect();
    // Notify the pre-join members about the newcomer so they can grant it
    // access. The broadcast carries the full post-join roster snapshot - a
    // newer snapshot supersedes an older one, so a member who joined between
    // two broadcasts is never dropped by the receiver's epoch guard. Offline
    // members reconcile on their next sync.
    let recipients = context.store.members(&group_id)?;
    let roster_snapshot = members.clone();
    let context = context.clone();
    let newcomer_id = sender.to_string();
    let broadcast_group_id = group_id.clone();
    tokio::spawn(async move {
        broadcast(
            &context,
            &recipients,
            RemoteRequest::GroupMemberAdded {
                group_id: broadcast_group_id,
                members: roster_snapshot,
                roster_epoch,
            },
            Some(&newcomer_id),
        )
        .await;
    });
    Ok(RemoteResponse::GroupJoined {
        group_id,
        group_name,
        members,
        roster_epoch,
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
                &members,
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
                &members,
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

/// Access rule a remote request must satisfy before dispatch. Evaluated once
/// in the router; the arms then do domain work only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessRule {
    /// No role gate; the request carries its own credential (invite secret,
    /// share grant, or the roster gate inside GroupSync).
    Open,
    /// Only the group owner may call.
    Owner,
    /// The sender must be a group member (any role).
    Member,
}

/// Classify a remote request by its access rule.
pub(crate) fn access_rule(request: &RemoteRequest) -> AccessRule {
    match request {
        RemoteRequest::RedeemInvite { .. }
        | RemoteRequest::RedeemGroupInvite { .. }
        | RemoteRequest::Query { .. }
        | RemoteRequest::Probe { .. }
        | RemoteRequest::GroupSync { .. } => AccessRule::Open,
        RemoteRequest::GroupMemberAdded { .. } | RemoteRequest::GroupMemberRemoved { .. } => {
            AccessRule::Owner
        }
        RemoteRequest::GroupShareAdded { .. }
        | RemoteRequest::GroupShareRemoved { .. }
        | RemoteRequest::GroupLeave { .. } => AccessRule::Member,
    }
}

/// The group a group-scoped request targets.
pub(crate) fn request_group_id(request: &RemoteRequest) -> Option<&str> {
    match request {
        RemoteRequest::GroupMemberAdded { group_id, .. }
        | RemoteRequest::GroupMemberRemoved { group_id, .. }
        | RemoteRequest::GroupShareAdded { group_id, .. }
        | RemoteRequest::GroupShareRemoved { group_id, .. }
        | RemoteRequest::GroupLeave { group_id }
        | RemoteRequest::GroupSync { group_id, .. } => Some(group_id),
        _ => None,
    }
}

/// Require `sender` to be the named contributor before a share add/remove
/// broadcast is applied locally (membership is gated by the access matrix).
fn require_own_share_change(sender: &str, contributor: &str, action: &str) -> Result<()> {
    if contributor != sender {
        bail!("Only the contributor may {action} its own share");
    }
    Ok(())
}

/// Receiver side of a `GroupShareAdded` broadcast: a member registered a new
/// contribution. The contributor granted everyone access locally; members only
/// need to register the contribution so fan-out can reach it.
pub(crate) fn handle_share_added(
    context: &Arc<DaemonContext>,
    group_id: &str,
    sender: &str,
    contributor: &str,
    share_id: &str,
    share_name: &str,
) -> Result<RemoteResponse> {
    // A forged contributor id would let an outsider attach arbitrary shares
    // to other members' rosters.
    require_own_share_change(sender, contributor, "register")?;
    context
        .store
        .add_group_share(group_id, contributor, share_id, share_name)?;
    Ok(RemoteResponse::GroupAck)
}

/// Receiver side of a `GroupShareRemoved` broadcast: a member withdrew a
/// contribution. Drop the local registration so fan-out stops dialing it.
pub(crate) fn handle_share_removed(
    context: &Arc<DaemonContext>,
    group_id: &str,
    sender: &str,
    contributor: &str,
    share_id: &str,
) -> Result<RemoteResponse> {
    require_own_share_change(sender, contributor, "withdraw")?;
    context
        .store
        .remove_group_share(group_id, contributor, share_id)?;
    Ok(RemoteResponse::GroupAck)
}

/// Receiver side of a `GroupMemberRemoved` push: the removed peer drops the
/// group locally; everyone else revokes that peer's grants and removes it.
/// The router has already verified the sender is the group owner.
pub(crate) async fn handle_member_removed(
    context: &Arc<DaemonContext>,
    group_id: &str,
    removed_peer: &str,
    roster_epoch: i64,
) -> Result<RemoteResponse> {
    // Stale removal: a newer roster state already reached us (the peer
    // rejoined after this kick was sent), so the kick must not apply.
    if roster_epoch <= context.store.roster_epoch(group_id)? {
        return Ok(RemoteResponse::GroupAck);
    }
    if removed_peer == context.identity.id() {
        // We were kicked (or the owner disbanded the group). The group row is
        // gone, so there is no local watermark to adopt.
        drop_group(context, group_id).await?;
    } else {
        revoke_member_access(
            &context.store,
            group_id,
            &context.identity.id(),
            removed_peer,
        )?;
        context.store.remove_member(group_id, removed_peer)?;
        context.store.adopt_roster_epoch(group_id, roster_epoch)?;
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
    // Only the owner mutates the authoritative roster and bumps the epoch; a
    // member sending `GroupLeave` at a non-owner would otherwise push that
    // replica's watermark past the owner's and silence later broadcasts.
    require_group_owner(&context.store, group_id, &context.identity.id())?;
    let members_before = context.store.members(group_id)?;
    let leaver = members_before
        .iter()
        .find(|row| row.peer_id == sender)
        .cloned()
        .context("Unknown group member")?;
    context.store.remove_member(group_id, sender)?;
    // Revoke every owner contribution from the leaver.
    if let Some(owner) = members_before.iter().find(|row| row.role.is_owner()) {
        revoke_member_access(&context.store, group_id, &owner.peer_id, sender)?;
    }
    let roster_epoch = context.store.bump_roster_epoch(group_id)?;
    broadcast(
        context,
        &members_before,
        RemoteRequest::GroupMemberRemoved {
            group_id: group_id.to_string(),
            peer_id: leaver.peer_id.clone(),
            peer_name: leaver.peer_name.clone(),
            roster_epoch,
        },
        None,
    )
    .await;
    Ok(())
}

/// Local IPC handlers (delegated from the daemon's router). Each is the
/// daemon-boundary body for one `LocalRequest::Group*` arm: parse, validate,
/// call the domain, and shape the response. The wire-side handlers above
/// mirror these for remote peers.
pub(crate) async fn local_group_create(
    context: &Arc<DaemonContext>,
    name: String,
    share_id: String,
    share_name: String,
) -> Result<LocalResponse> {
    let group = context.store.create_group_with_owner_share(
        &name,
        &context.identity.id(),
        &context.identity.name,
        &share_id,
        &share_name,
    )?;
    Ok(LocalResponse::Group(group))
}

pub(crate) async fn local_group_list(context: &Arc<DaemonContext>) -> Result<LocalResponse> {
    for group in context.store.groups()? {
        // Background pull; the cached roster is returned immediately.
        maybe_sync_group(context, &group.name);
    }
    Ok(LocalResponse::Groups(context.store.groups()?))
}

pub(crate) async fn local_group_members(
    context: &Arc<DaemonContext>,
    group: String,
) -> Result<LocalResponse> {
    // Resolve the stable id first: a background sync may apply an owner
    // rename that makes the caller's name unresolvable.
    let id = context.store.group(&group)?.id;
    maybe_sync_group(context, &id);
    Ok(LocalResponse::Members(context.store.members(&id)?))
}

pub(crate) async fn local_group_shares(
    context: &Arc<DaemonContext>,
    group: String,
) -> Result<LocalResponse> {
    // First-time join runs before any local group row exists; an
    // unknown group simply means nothing is contributed yet.
    let shares = match context.store.group_opt(&group)? {
        Some(group) => context
            .store
            .group_shares(&group.id, &context.identity.id())?,
        None => Vec::new(),
    };
    Ok(LocalResponse::GroupShares(shares))
}

pub(crate) async fn local_group_invite(
    context: &Arc<DaemonContext>,
    group: String,
    valid_for_seconds: i64,
    max_uses: Option<i64>,
) -> Result<LocalResponse> {
    // Only the owner may mint join links: a member-created invite
    // would let anyone join without the owner's consent.
    require_group_owner(&context.store, &group, &context.identity.id())?;
    let invite = context
        .store
        .create_group_invite(&group, valid_for_seconds, max_uses)?;
    let ticket = InviteTicket {
        version: 1,
        endpoint: context.endpoint.addr(),
        share_id: String::new(),
        group_id: Some(invite.share_id.clone()),
        invite_id: invite.id,
        secret: invite.secret,
        expires_at: invite.expires_at,
    }
    .encode()?;
    Ok(LocalResponse::Invitation {
        share_name: invite.share_name,
        ticket,
        expires_at: invite.expires_at,
    })
}

pub(crate) async fn local_group_join(
    context: &Arc<DaemonContext>,
    invite: String,
    shares: Vec<(String, String)>,
) -> Result<LocalResponse> {
    // The ticket is parsed once at the daemon boundary; validation is
    // done here and the parsed ticket is passed down (never re-parsed).
    let ticket = InviteTicket::parse(&invite)?;
    if ticket.expires_at < Utc::now().timestamp() {
        bail!("Invitation is expired");
    }
    let group_id = ticket
        .group_id
        .as_deref()
        .context("Invitation is not a group invite")?;
    // A known member re-running join adjusts contributions (multi-select
    // checkboxes: additions register + grant, withdrawals revoke).
    let member_group = context.store.group(group_id).ok().filter(|group| {
        context.store.members(&group.id).is_ok_and(|members| {
            members
                .iter()
                .any(|member| member.peer_id == context.identity.id())
        })
    });
    if let Some(group) = member_group {
        adjust_group_shares(context, &group.id, &shares).await?;
        Ok(LocalResponse::GroupJoined {
            group_name: group.name,
            member_count: group.member_count as usize,
        })
    } else {
        let (group_name, member_count) = redeem_group_remote(context, &ticket, &shares).await?;
        Ok(LocalResponse::GroupJoined {
            group_name,
            member_count,
        })
    }
}

pub(crate) async fn local_group_leave(
    context: &Arc<DaemonContext>,
    group: String,
) -> Result<LocalResponse> {
    leave_group(context, &group).await?;
    Ok(LocalResponse::Ok)
}

pub(crate) async fn local_group_remove_member(
    context: &Arc<DaemonContext>,
    group: String,
    peer: String,
) -> Result<LocalResponse> {
    remove_group_member(context, &group, &peer).await?;
    Ok(LocalResponse::Ok)
}

pub(crate) async fn local_group_rename(
    context: &Arc<DaemonContext>,
    group: String,
    name: String,
) -> Result<LocalResponse> {
    let info = context.store.group(&group)?;
    require_group_owner(&context.store, &info.id, &context.identity.id())?;
    let renamed = context.store.rename_group(&info.id, &name)?;
    // The rename reaches members on their next roster pull; bump the
    // epoch so that pull carries a fresh watermark.
    context.store.bump_roster_epoch(&info.id)?;
    Ok(LocalResponse::Group(renamed))
}

pub(crate) async fn local_group_sync(
    context: &Arc<DaemonContext>,
    group: String,
) -> Result<LocalResponse> {
    // The sync may apply an owner rename that invalidates the caller's name;
    // the stable id keeps both the pull and the response lookup working.
    let id = context.store.group(&group)?.id;
    sync_group(context, &id).await?;
    Ok(LocalResponse::Group(context.store.group(&id)?))
}

pub(crate) async fn local_group_query(
    context: &Arc<DaemonContext>,
    group: String,
    member: Option<String>,
    share: Option<String>,
    source: String,
    filter: Filter,
) -> Result<LocalResponse> {
    // Unknown group: answer `None` so the caller can continue its
    // scope cascade instead of failing the query.
    let Some(group_info) = context.store.group_opt(&group)? else {
        return Ok(LocalResponse::GroupQuery(None));
    };
    // Background pull; the fan-out below uses the cached roster.
    maybe_sync_group(context, &group);
    let targets = super::fanout::group_targets(
        &context.store,
        &group_info,
        member.as_deref(),
        share.as_deref(),
    )?;
    let response =
        super::fanout::group_fan_out(context, &group_info.name, &targets, &source, filter).await?;
    Ok(LocalResponse::GroupQuery(Some(response)))
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
    fn access_matrix_classifies_every_request() {
        use crate::remote::protocol::RemoteRequest as R;
        let id = iroh::SecretKey::generate().public();
        let member = MemberInfo {
            peer_id: id.to_string(),
            peer_name: "peer".to_string(),
            shares: Vec::new(),
            role: "member".to_string(),
            endpoint: iroh::EndpointAddr::new(id),
        };
        let cases: Vec<(R, AccessRule)> = vec![
            (
                R::RedeemInvite {
                    invite_id: String::new(),
                    secret: String::new(),
                    peer_name: String::new(),
                },
                AccessRule::Open,
            ),
            (
                R::RedeemGroupInvite {
                    invite_id: String::new(),
                    secret: String::new(),
                    peer_name: String::new(),
                    shares: Vec::new(),
                    endpoint: member.endpoint.clone(),
                },
                AccessRule::Open,
            ),
            (
                R::Query {
                    share_id: String::new(),
                    source: String::new(),
                    filter: Default::default(),
                },
                AccessRule::Open,
            ),
            (
                R::Probe {
                    share_id: String::new(),
                },
                AccessRule::Open,
            ),
            (
                R::GroupSync {
                    group_id: String::new(),
                    shares: Vec::new(),
                },
                AccessRule::Open,
            ),
            (
                R::GroupMemberAdded {
                    group_id: String::new(),
                    members: vec![member.clone()],
                    roster_epoch: 0,
                },
                AccessRule::Owner,
            ),
            (
                R::GroupMemberRemoved {
                    group_id: String::new(),
                    peer_id: String::new(),
                    peer_name: String::new(),
                    roster_epoch: 0,
                },
                AccessRule::Owner,
            ),
            (
                R::GroupShareAdded {
                    group_id: String::new(),
                    peer_id: String::new(),
                    peer_name: String::new(),
                    share_id: String::new(),
                    share_name: String::new(),
                },
                AccessRule::Member,
            ),
            (
                R::GroupShareRemoved {
                    group_id: String::new(),
                    peer_id: String::new(),
                    share_id: String::new(),
                },
                AccessRule::Member,
            ),
            (
                R::GroupLeave {
                    group_id: String::new(),
                },
                AccessRule::Member,
            ),
        ];
        for (request, expected) in cases {
            assert_eq!(access_rule(&request), expected, "{request:?}");
        }
        // Group-scoped variants expose their group id; others expose none.
        assert_eq!(
            request_group_id(&R::GroupLeave {
                group_id: "team".to_string(),
            }),
            Some("team")
        );
        assert_eq!(
            request_group_id(&R::Probe {
                share_id: String::new()
            }),
            None
        );
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
        let stale_push = member_info(
            &bob_id,
            "bob",
            "member",
            vec![(s2.id.clone(), s2.name.clone())],
        );
        merge_member(&context, "team", &[stale_push], 1).expect("merge");
        let shares = store.group_shares("team", &bob_id).expect("shares");
        assert!(
            shares.iter().any(|share| share.share_id == s3.id),
            "an add-only push must never prune a share broadcast concurrently"
        );
        assert!(shares.iter().any(|share| share.share_id == s2.id));
    }

    #[tokio::test]
    async fn stale_owner_broadcasts_are_ignored() {
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
        let s = store
            .add_share("workspace-bob", &workspace, "bob-ws", true)
            .expect("share");
        // Fresh join push (epoch 5) applies and registers Bob's share.
        merge_member(
            &context,
            "team",
            &[member_info(
                &bob_id,
                "bob",
                "member",
                vec![(s.id.clone(), s.name.clone())],
            )],
            5,
        )
        .expect("join push");
        // A stale kick (epoch 4, sent before the rejoin) must not remove Bob.
        handle_member_removed(&context, "team", &bob_id, 4)
            .await
            .expect("stale kick ignored");
        assert!(
            store
                .members("team")
                .expect("members")
                .iter()
                .any(|member| member.peer_id == bob_id),
            "a stale kick must not remove a member that rejoined"
        );
        // A fresh kick (epoch 6) does remove Bob and revokes his grants.
        handle_member_removed(&context, "team", &bob_id, 6)
            .await
            .expect("fresh kick");
        assert!(
            !store
                .members("team")
                .expect("members")
                .iter()
                .any(|member| member.peer_id == bob_id),
            "a fresh kick removes the member"
        );
    }

    #[tokio::test]
    async fn owner_sync_repairs_member_contributions() {
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
        // Bob's add/withdraw broadcasts were missed while the owner was
        // offline: the owner still lists `bob-ws-a`, but Bob now contributes
        // `bob-ws-b` instead. His sync report is authoritative for his rows.
        let ws_a = store
            .add_share("workspace-a", &workspace, "bob-ws-a", true)
            .expect("share");
        let ws_b = store
            .add_share("workspace-b", &workspace, "bob-ws-b", true)
            .expect("share");
        store
            .add_group_share("team", &bob_id, &ws_a.id, &ws_a.name)
            .expect("stale contribution");
        sync_member_shares(
            store,
            "team",
            &bob_id,
            &[(ws_b.id.clone(), ws_b.name.clone())],
        )
        .expect("sync repairs the roster");
        let shares = store.group_shares("team", &bob_id).expect("shares");
        assert_eq!(shares.len(), 1, "withdrawn contribution is removed");
        assert_eq!(shares[0].share_id, ws_b.id);
        assert_eq!(shares[0].share_name, ws_b.name);
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
            member_info(
                &bob_id,
                "bob",
                "member",
                vec![(s2.id.clone(), s2.name.clone())],
            ),
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
        store
            .save_remote_peer(&carol_id, "carol", "{}")
            .expect("peer");
        store
            .add_member("team", &carol_id, "member")
            .expect("member");
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
            store
                .group_shares("team", &carol_id)
                .expect("shares")
                .is_empty(),
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
