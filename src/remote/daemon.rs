use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::Engine;
use chrono::Utc;
use fs2::FileExt;
use iroh::endpoint::presets;
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::identity::Identity;
use super::ipc;
use super::protocol::{
    DaemonInfo, DaemonStatus, GroupQueryResponse, InviteTicket, LocalEnvelope, LocalRequest,
    LocalResponse, MemberInfo, QueryResponse, RemoteRequest, RemoteResponse, MAX_MESSAGE_SIZE,
    REMOTE_ALPN,
};
use super::state::{GroupInfo, GroupMemberInfo, MountInfo, StateStore};
use crate::commands::memory::filter::Filter;

pub fn run() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start daemon runtime")?;
    runtime.block_on(run_async())
}

async fn run_async() -> Result<()> {
    let lock_path = ipc::daemon_lock_path();
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    lock.try_lock_exclusive()
        .context("sivtr daemon is already running")?;

    let store = StateStore::open_default()?;
    let identity = Identity::load_or_create()?;
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(identity.secret_key.clone())
        .alpns(vec![REMOTE_ALPN.to_vec()])
        .bind()
        .await
        .context("Failed to bind iroh endpoint")?;
    endpoint.online().await;

    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .context("Failed to bind daemon control listener")?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let started_at = Utc::now().to_rfc3339();
    let info = DaemonInfo {
        pid: std::process::id(),
        port,
        token: token.clone(),
        node_id: identity.id(),
        endpoint: endpoint.addr(),
        started_at: started_at.clone(),
    };
    ipc::write_daemon_info(&info)?;
    let _guard = DaemonInfoGuard;

    let context = Arc::new(DaemonContext {
        store,
        endpoint: endpoint.clone(),
        identity,
        started_at,
        control_token: token,
    });
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("Failed to accept local control connection")?;
                let context = context.clone();
                let shutdown_tx = shutdown_tx.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_local(stream, context, shutdown_tx).await {
                        crate::output::error(format!("local control error: {error:#}"));
                    }
                });
            }
            connecting = endpoint.accept() => {
                let Some(connecting) = connecting else {
                    break;
                };
                let context = context.clone();
                tokio::spawn(async move {
                    // UDP Initial packets from scanners / stale routes fail the QUIC
                    // handshake with PROTOCOL_VIOLATION("authentication failed"). That is
                    // transport noise, not application auth — drop it.
                    let Ok(connection) = connecting.await else {
                        return;
                    };
                    if let Err(error) = handle_remote(connection, context).await {
                        crate::output::error(format!("remote connection error: {error:#}"));
                    }
                });
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }

    endpoint.close().await;
    FileExt::unlock(&lock)?;
    Ok(())
}

struct DaemonInfoGuard;

impl Drop for DaemonInfoGuard {
    fn drop(&mut self) {
        ipc::remove_daemon_info();
    }
}

struct DaemonContext {
    store: StateStore,
    endpoint: Endpoint,
    identity: Identity,
    started_at: String,
    control_token: String,
}

async fn handle_local(
    stream: TcpStream,
    context: Arc<DaemonContext>,
    shutdown_tx: watch::Sender<bool>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut line = String::new();
    BufReader::new(reader)
        .read_line(&mut line)
        .await
        .context("Failed to read local request")?;
    let envelope: LocalEnvelope =
        serde_json::from_str(&line).context("Invalid local control request")?;
    let (response, shutdown) = if envelope.token != context.control_token {
        (
            LocalResponse::Error {
                message: "unauthorized local control request".to_string(),
            },
            false,
        )
    } else {
        match process_local(&context, envelope.request).await {
            Ok(value) => value,
            Err(error) => (
                LocalResponse::Error {
                    message: format!("{error:#}"),
                },
                false,
            ),
        }
    };
    writer.write_all(&serde_json::to_vec(&response)?).await?;
    writer.write_all(b"\n").await?;
    writer.shutdown().await?;
    if shutdown {
        let _ = shutdown_tx.send(true);
    }
    Ok(())
}

async fn process_local(
    context: &Arc<DaemonContext>,
    request: LocalRequest,
) -> Result<(LocalResponse, bool)> {
    let response = match request {
        LocalRequest::Status => LocalResponse::Status(DaemonStatus {
            node_id: context.identity.id(),
            device_name: context.identity.name.clone(),
            endpoint: context.endpoint.addr(),
            started_at: context.started_at.clone(),
            shares: context.store.shares()?.len(),
            peers: context.store.peers()?.len(),
        }),
        LocalRequest::Shutdown => return Ok((LocalResponse::Ok, true)),
        LocalRequest::ShareAdd {
            workspace_key,
            root,
            name,
            redact,
        } => LocalResponse::Share(context.store.add_share(
            &workspace_key,
            &PathBuf::from(root),
            &name,
            redact,
        )?),
        LocalRequest::ShareList => LocalResponse::Shares(context.store.shares()?),
        LocalRequest::ShareRemove { share } => {
            LocalResponse::Share(context.store.remove_share(&share)?)
        }
        LocalRequest::ShareSetEnabled { share, enabled } => {
            LocalResponse::Share(context.store.set_share_enabled(&share, enabled)?)
        }
        LocalRequest::ShareInvite {
            share,
            valid_for_seconds,
        } => {
            // Use live addr after online(); N0 may still refine paths after this snapshot.
            let invite = context.store.create_invite(&share, valid_for_seconds)?;
            let ticket = InviteTicket {
                version: 1,
                endpoint: context.endpoint.addr(),
                share_id: invite.share_id,
                group_id: None,
                invite_id: invite.id,
                secret: invite.secret,
                expires_at: invite.expires_at,
            }
            .encode()?;
            LocalResponse::Invitation {
                share_name: invite.share_name,
                ticket,
                expires_at: invite.expires_at,
            }
        }
        LocalRequest::ShareGrants { share } => LocalResponse::Grants(context.store.grants(&share)?),
        LocalRequest::ShareRevoke { share, peer } => {
            match context.store.revoke(&share, &peer)? {
                Some(grant) => LocalResponse::Grant(grant),
                // An explicit revoke of a grant that never existed is an error
                // on the CLI path, even though the group paths treat it as an
                // idempotent no-op.
                None => bail!("Peer `{peer}` has no active grant for `{share}`"),
            }
        }
        LocalRequest::PeerList => LocalResponse::Peers(context.store.peers()?),
        LocalRequest::PeerForget { peer } => LocalResponse::Peer(context.store.forget_peer(&peer)?),
        LocalRequest::RemoteAdd {
            workspace_key,
            alias,
            invite,
        } => {
            let mount = redeem_remote(context, &workspace_key, &alias, &invite).await?;
            LocalResponse::RemoteAdded { mount }
        }
        LocalRequest::RemoteList { workspace_key } => {
            LocalResponse::Mounts(context.store.mounts(&workspace_key)?)
        }
        LocalRequest::RemoteRemove {
            workspace_key,
            alias,
        } => LocalResponse::Mount(context.store.remove_mount(&workspace_key, &alias)?),
        LocalRequest::RemoteRename {
            workspace_key,
            alias,
            new_alias,
        } => LocalResponse::Mount(context.store.rename_mount(
            &workspace_key,
            &alias,
            &new_alias,
        )?),
        LocalRequest::RemoteTest {
            workspace_key,
            alias,
        } => {
            let mount = context.store.mount(&workspace_key, &alias)?;
            let response = exchange_with_peer(
                context,
                &mount.peer_id,
                RemoteRequest::Probe {
                    share_id: mount.share_id.clone(),
                },
            )
            .await?;
            match response {
                RemoteResponse::Probe {
                    server_name,
                    share_name,
                } => LocalResponse::RemoteTested {
                    peer_name: server_name,
                    share_name,
                },
                response => bail!("Unexpected remote response: {response:?}"),
            }
        }
        LocalRequest::RemoteQuery {
            workspace_key,
            alias,
            source,
            filter,
        } => {
            let mount = context.store.mount(&workspace_key, &alias)?;
            let response = exchange_with_peer(
                context,
                &mount.peer_id,
                RemoteRequest::Query {
                    share_id: mount.share_id.clone(),
                    source,
                    filter,
                },
            )
            .await?;
            match response {
                RemoteResponse::Query(mut query) => {
                    qualify_query_scope(&mount.alias, &mut query);
                    LocalResponse::Query(query)
                }
                response => bail!("Unexpected remote response: {response:?}"),
            }
        }
        LocalRequest::GroupCreate {
            name,
            share_id,
            share_name,
        } => {
            let group =
                context
                    .store
                    .add_group(&name, &context.identity.id(), &context.identity.name)?;
            // The owner's first contribution is the workspace they created from.
            context.store.add_group_share(
                &group.id,
                &context.identity.id(),
                &share_id,
                &share_name,
            )?;
            LocalResponse::Group(group)
        }
        LocalRequest::GroupList => {
            for group in context.store.groups()? {
                maybe_sync_group(context, &group.name).await;
            }
            LocalResponse::Groups(context.store.groups()?)
        }
        LocalRequest::GroupMembers { group } => {
            maybe_sync_group(context, &group).await;
            LocalResponse::Members(context.store.members(&group)?)
        }
        LocalRequest::GroupShares { group } => {
            // First-time join runs before any local group row exists; an
            // unknown group simply means nothing is contributed yet.
            let shares = match context.store.group_opt(&group)? {
                Some(group) => context
                    .store
                    .group_shares(&group.id, &context.identity.id())?,
                None => Vec::new(),
            };
            LocalResponse::GroupShares(shares)
        }
        LocalRequest::GroupInvite {
            group,
            valid_for_seconds,
            max_uses,
        } => {
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
            LocalResponse::Invitation {
                share_name: invite.share_name,
                ticket,
                expires_at: invite.expires_at,
            }
        }
        LocalRequest::GroupJoin { invite, shares } => {
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
            let known = context.store.group(group_id).ok();
            let is_member = known.as_ref().is_some_and(|group| {
                context
                    .store
                    .members(&group.id)
                    .map(|members| {
                        members
                            .iter()
                            .any(|member| member.peer_id == context.identity.id())
                    })
                    .unwrap_or(false)
            });
            if is_member {
                let group = known.expect("group exists");
                adjust_group_shares(context, &group.id, &shares).await?;
                LocalResponse::GroupJoined {
                    group_name: group.name,
                    member_count: group.member_count as usize,
                }
            } else {
                let (group_name, member_count) =
                    redeem_group_remote(context, &invite, &shares).await?;
                LocalResponse::GroupJoined {
                    group_name,
                    member_count,
                }
            }
        }
        LocalRequest::GroupLeave { group } => {
            leave_group(context, &group).await?;
            LocalResponse::Ok
        }
        LocalRequest::GroupRemoveMember { group, peer } => {
            remove_group_member(context, &group, &peer).await?;
            LocalResponse::Ok
        }
        LocalRequest::GroupSync { group } => {
            sync_group(context, &group).await?;
            LocalResponse::Group(context.store.group(&group)?)
        }
        LocalRequest::GroupQuery {
            group,
            member,
            share,
            source,
            filter,
        } => {
            let group_info = context.store.group(&group)?;
            maybe_sync_group(context, &group).await;
            let targets = group_targets(
                &context.store,
                &group_info,
                member.as_deref(),
                share.as_deref(),
            )?;
            if targets.is_empty() {
                return Ok((
                    LocalResponse::GroupQuery(GroupQueryResponse {
                        query: QueryResponse {
                            records: Vec::new(),
                            anchors: Vec::new(),
                        },
                        skipped: Vec::new(),
                    }),
                    false,
                ));
            }
            let response =
                group_fan_out(context, &group_info.name, &targets, &source, filter).await?;
            LocalResponse::GroupQuery(response)
        }
    };
    Ok((response, false))
}

async fn redeem_remote(
    context: &DaemonContext,
    workspace_key: &str,
    alias: &str,
    encoded_invite: &str,
) -> Result<MountInfo> {
    let invite = InviteTicket::parse(encoded_invite)?;
    if invite.expires_at < Utc::now().timestamp() {
        bail!("Invitation is expired");
    }
    let peer_id = invite.endpoint.id.to_string();
    let (response, observed) = exchange(
        context,
        invite.endpoint,
        RemoteRequest::RedeemInvite {
            invite_id: invite.invite_id,
            secret: invite.secret,
            peer_name: context.identity.name.clone(),
        },
    )
    .await?;
    let (server_name, share_id, share_name) = match response {
        RemoteResponse::Redeemed {
            server_name,
            share_id,
            share_name,
        } => (server_name, share_id, share_name),
        response => bail!("Unexpected invitation response: {response:?}"),
    };
    let endpoint_json =
        serde_json::to_string(&observed).context("Failed to encode peer endpoint")?;
    context
        .store
        .save_remote_peer(&peer_id, &server_name, &endpoint_json)?;
    context
        .store
        .add_mount(workspace_key, alias, &peer_id, &share_id, &share_name)
}

async fn exchange_with_peer(
    context: &DaemonContext,
    peer_id: &str,
    request: RemoteRequest,
) -> Result<RemoteResponse> {
    let endpoint_json = context.store.peer_endpoint(peer_id)?;
    let address: EndpointAddr =
        serde_json::from_str(&endpoint_json).context("Invalid stored peer endpoint")?;
    let (response, observed) = exchange(context, address, request).await?;
    let endpoint_json =
        serde_json::to_string(&observed).context("Failed to encode peer endpoint")?;
    context
        .store
        .refresh_peer_endpoint(peer_id, &endpoint_json)
        .context("Failed to refresh peer endpoint after successful dial")?;
    Ok(response)
}

/// Dial the peer and exchange one request/response.
///
/// Default mode (`presets::N0`) includes address lookup. We dial the stored/bootstrap
/// address first; if that fails, dial by `EndpointId` alone so N0 discovery can resolve
/// current direct/relay paths. That is how default mode works — not a path rewrite.
///
/// After a successful dial, return iroh's observed addresses so callers can refresh storage.
async fn exchange(
    context: &DaemonContext,
    address: EndpointAddr,
    request: RemoteRequest,
) -> Result<(RemoteResponse, EndpointAddr)> {
    let connection = connect_default(&context.endpoint, &address).await?;
    let observed = observed_endpoint(&context.endpoint, &connection, &address).await;
    let (mut send, mut receive) = connection.open_bi().await?;
    send.write_all(&serde_json::to_vec(&request)?).await?;
    send.finish()?;
    let bytes = receive.read_to_end(MAX_MESSAGE_SIZE).await?;
    connection.close(0u32.into(), b"done");
    let response: RemoteResponse =
        serde_json::from_slice(&bytes).context("Invalid remote daemon response")?;
    match response {
        RemoteResponse::Error { message } => Err(anyhow::anyhow!(message)),
        response => Ok((response, observed)),
    }
}

/// Default-mode dial: known address first, then EndpointId discovery via N0.
async fn connect_default(endpoint: &Endpoint, address: &EndpointAddr) -> Result<Connection> {
    match endpoint.connect(address.clone(), REMOTE_ALPN).await {
        Ok(connection) => Ok(connection),
        Err(first) => {
            // Already id-only: discovery was the only path; do not double-dial.
            if address.is_empty() {
                return Err(anyhow::anyhow!(first)).context("Failed to reach remote sivtr daemon");
            }
            match endpoint
                .connect(EndpointAddr::new(address.id), REMOTE_ALPN)
                .await
            {
                Ok(connection) => Ok(connection),
                Err(second) => Err(anyhow::anyhow!(
                    "known address failed ({first:#}); discovery by id failed ({second:#})"
                ))
                .context("Failed to reach remote sivtr daemon"),
            }
        }
    }
}

async fn observed_endpoint(
    endpoint: &Endpoint,
    connection: &Connection,
    dialed: &EndpointAddr,
) -> EndpointAddr {
    let remote_id = connection.remote_id();
    if let Some(info) = endpoint.remote_info(remote_id).await {
        let observed =
            EndpointAddr::from_parts(info.id(), info.into_addrs().map(|addr| addr.into_addr()));
        if !observed.is_empty() {
            return observed;
        }
    }
    dialed.clone()
}

async fn handle_remote(connection: Connection, context: Arc<DaemonContext>) -> Result<()> {
    let peer_id = connection.remote_id().to_string();
    let (mut send, mut receive) = connection.accept_bi().await?;
    let bytes = receive.read_to_end(MAX_MESSAGE_SIZE).await?;
    let request: RemoteRequest =
        serde_json::from_slice(&bytes).context("Invalid remote request")?;
    let response = match process_remote(&context, &peer_id, request).await {
        Ok(response) => response,
        Err(error) => RemoteResponse::Error {
            message: format!("{error:#}"),
        },
    };
    send.write_all(&serde_json::to_vec(&response)?).await?;
    send.finish()?;
    connection.closed().await;
    Ok(())
}

/// Require `sender` to be the group's owner before roster-changing requests.
///
/// Membership changes are always owner-driven: joins, kicks, and leaves are
/// broadcast by the owner after it updates its authoritative roster. Binding
/// them to the transport-authenticated sender prevents a member from forging
/// additions (which would grant an attacker read access to other members'
/// contributions) or removals.
fn require_group_owner(store: &StateStore, group_id: &str, sender: &str) -> Result<()> {
    let owner = store
        .members(group_id)?
        .into_iter()
        .find(|member| member.role == "owner")
        .context("Group has no owner")?;
    if owner.peer_id != sender {
        bail!("Only the group owner may change membership");
    }
    Ok(())
}

async fn process_remote(
    context: &Arc<DaemonContext>,
    peer_id: &str,
    request: RemoteRequest,
) -> Result<RemoteResponse> {
    match request {
        RemoteRequest::RedeemInvite {
            invite_id,
            secret,
            peer_name,
        } => {
            let redeemed = context
                .store
                .redeem_invite(&invite_id, &secret, peer_id, &peer_name)?;
            Ok(RemoteResponse::Redeemed {
                server_name: context.identity.name.clone(),
                share_id: redeemed.share_id,
                share_name: redeemed.share_name,
            })
        }
        RemoteRequest::Query {
            share_id,
            source,
            filter,
        } => {
            let share = context.store.authorize(peer_id, &share_id, "query")?;
            let response = tokio::task::spawn_blocking(move || {
                let result = crate::commands::memory::workset::run_on_share(
                    std::path::Path::new(&share.root),
                    &source,
                    filter,
                    share.redact,
                );
                // An empty workspace has no sessions; report it as an empty
                // result instead of an error (matches the client-side
                // convention in `query_many`).
                let (records, anchors) = match result {
                    Ok(result) => result,
                    Err(error)
                        if error
                            .to_string()
                            .starts_with("No record found for ref selector") =>
                    {
                        (Vec::new(), Vec::new())
                    }
                    Err(error) => return Err(error),
                };
                Ok::<_, anyhow::Error>(QueryResponse { records, anchors })
            })
            .await??;
            Ok(RemoteResponse::Query(response))
        }
        RemoteRequest::Probe { share_id } => {
            let share = context.store.authorize(peer_id, &share_id, "probe")?;
            Ok(RemoteResponse::Probe {
                server_name: context.identity.name.clone(),
                share_name: share.name,
            })
        }
        RemoteRequest::RedeemGroupInvite {
            invite_id,
            secret,
            peer_name,
            shares,
            endpoint,
        } => {
            let endpoint_json = serde_json::to_string(&endpoint)?;
            let joiner = super::state::JoinerInfo {
                peer_id,
                peer_name: &peer_name,
                shares: &shares,
                endpoint_json: &endpoint_json,
            };
            let redeemed = context
                .store
                .redeem_group_invite(&invite_id, &secret, &joiner)?;
            // The invite row is the authority: the group is derived from the
            // ticket, never from the joiner's request, and used for every
            // follow-up (name lookup, roster, broadcast).
            let group_id = redeemed.group_id;
            let group_name = context.store.group(&group_id)?.name;
            let members: Vec<MemberInfo> = redeemed
                .roster
                .iter()
                .map(|member| member_info_from_store(&context.store, &group_id, member))
                .collect::<Result<_>>()?;
            // Ensure the owner's own roster entry carries the live endpoint so
            // the joiner can dial back without relying on discovery alone.
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
                peer_id: peer_id.to_string(),
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
        RemoteRequest::GroupMemberAdded { group_id, member } => {
            require_group_owner(&context.store, &group_id, peer_id)?;
            let endpoint_json = serde_json::to_string(&member.endpoint)?;
            context
                .store
                .save_remote_peer(&member.peer_id, &member.peer_name, &endpoint_json)?;
            context
                .store
                .add_member(&group_id, &member.peer_id, &member.role)?;
            for (share_id, share_name) in &member.shares {
                context
                    .store
                    .add_group_share(&group_id, &member.peer_id, share_id, share_name)?;
            }
            // Grant the newcomer read access to every contribution we make.
            for share in context
                .store
                .group_shares(&group_id, &context.identity.id())?
            {
                context
                    .store
                    .group_grant(&share.share_id, &member.peer_id)?;
            }
            Ok(RemoteResponse::GroupAck)
        }
        RemoteRequest::GroupShareAdded {
            group_id,
            peer_id: contributor,
            peer_name: _,
            share_id,
            share_name,
        } => {
            // Only the contributor may register its own contribution; a forged
            // peer_id would otherwise let a member attach arbitrary shares to
            // other members' rosters.
            if contributor != peer_id {
                bail!("Only the contributor may register its own share");
            }
            // The contributor granted everyone access locally; members only
            // need to register the new contribution so fan-out can reach it.
            context
                .store
                .add_group_share(&group_id, &contributor, &share_id, &share_name)?;
            Ok(RemoteResponse::GroupAck)
        }
        RemoteRequest::GroupShareRemoved {
            group_id,
            peer_id: contributor,
            share_id,
        } => {
            if contributor != peer_id {
                bail!("Only the contributor may withdraw its own share");
            }
            // The share is no longer part of the group; drop the local
            // registration so fan-out stops dialing it.
            context
                .store
                .remove_group_share(&group_id, &contributor, &share_id)?;
            Ok(RemoteResponse::GroupAck)
        }
        RemoteRequest::GroupMemberRemoved {
            group_id,
            peer_id: removed_peer,
            peer_name: _,
        } => {
            require_group_owner(&context.store, &group_id, peer_id)?;
            if removed_peer == context.identity.id() {
                // We were kicked (or the owner disbanded the group).
                drop_group(context, &group_id).await?;
            } else {
                for share in context
                    .store
                    .group_shares(&group_id, &context.identity.id())?
                {
                    revoke_peer_grant(&context.store, &share.share_id, &removed_peer)?;
                }
                context.store.remove_member(&group_id, &removed_peer)?;
            }
            Ok(RemoteResponse::GroupAck)
        }
        RemoteRequest::GroupLeave { group_id } => {
            let members_before = context.store.members(&group_id)?;
            let leaver = members_before
                .iter()
                .find(|row| row.peer_id == peer_id)
                .cloned()
                .context("Unknown group member")?;
            context.store.remove_member(&group_id, peer_id)?;
            // Revoke every owner contribution from the leaver.
            if let Some(owner) = members_before.iter().find(|row| row.role == "owner") {
                for share in context.store.group_shares(&group_id, &owner.peer_id)? {
                    revoke_peer_grant(&context.store, &share.share_id, peer_id)?;
                }
            }
            broadcast(
                context,
                &group_id,
                RemoteRequest::GroupMemberRemoved {
                    group_id: group_id.clone(),
                    peer_id: leaver.peer_id.clone(),
                    peer_name: leaver.peer_name.clone(),
                },
                None,
            )
            .await;
            Ok(RemoteResponse::GroupAck)
        }
        RemoteRequest::GroupSync { group_id } => {
            let Ok(group) = context.store.group(&group_id) else {
                return Ok(RemoteResponse::GroupSynced {
                    group_name: group_id.clone(),
                    member: false,
                    members: Vec::new(),
                });
            };
            let roster: Vec<MemberInfo> = context
                .store
                .members(&group.id)?
                .iter()
                .map(|member| member_info_from_store(&context.store, &group.id, member))
                .collect::<Result<_>>()?;
            // The roster is membership data: only members may read it, so a
            // non-member learns only that it is not a member.
            if !roster.iter().any(|row| row.peer_id == peer_id) {
                return Ok(RemoteResponse::GroupSynced {
                    group_name: group.name,
                    member: false,
                    members: Vec::new(),
                });
            }
            // Live endpoint for the owner's own entry.
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
            Ok(RemoteResponse::GroupSynced {
                group_name: group.name,
                member: true,
                members: roster,
            })
        }
    }
}

fn qualify_query_scope(scope: &str, response: &mut QueryResponse) {
    let scope = scope.to_ascii_lowercase();
    for record in &mut response.records {
        record.work_ref = record.work_ref.with_named_scope(scope.clone());
    }
    for anchor in &mut response.anchors {
        *anchor = anchor.with_named_scope(scope.clone());
    }
}

// ---------------------------------------------------------------------------
// Group mode: a named set of devices that expose their memory to each other.
// Membership is a mesh overlay on the existing share/grant/mount model: join =
// redeem a multi-use invite with the owner, mirror the roster (members and
// their contributed shares) locally, and grant every member read access to
// every share we contribute.
// ---------------------------------------------------------------------------

/// Client-side join (first time): redeem the invite with the owner, mirror the
/// roster, register our contributed shares, and grant members.
async fn redeem_group_remote(
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
    let (response, _observed) = exchange(
        context,
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
    // Mirror the roster and grant every member read access to our shares.
    for member in &members {
        let member_endpoint = serde_json::to_string(&member.endpoint)?;
        context
            .store
            .save_remote_peer(&member.peer_id, &member.peer_name, &member_endpoint)?;
        context
            .store
            .add_member(&group_id, &member.peer_id, &member.role)?;
        for (share_id, share_name) in &member.shares {
            context
                .store
                .add_group_share(&group_id, &member.peer_id, share_id, share_name)?;
        }
        for (share_id, _) in shares {
            context.store.group_grant(share_id, &member.peer_id)?;
        }
    }
    context.store.touch_group_sync(&group_id)?;
    // The returned roster already includes this device; it is the full count.
    Ok((group_name, members.len()))
}

/// An existing member re-runs join with the final checkbox list: register new
/// contributions (granting every member), withdraw unchecked ones (revoking
/// grants), and broadcast both directions so peers stay in sync.
async fn adjust_group_shares(
    context: &Arc<DaemonContext>,
    group_name_or_id: &str,
    shares: &[(String, String)],
) -> Result<()> {
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
                    context.store.group_grant(share_id, &member.peer_id)?;
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

/// Pull the authoritative roster from the owner and reconcile membership,
/// contributions, and grants. If the owner says we are no longer a member,
/// drop the group.
async fn sync_group(context: &Arc<DaemonContext>, group_name_or_id: &str) -> Result<()> {
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
    let response = exchange_with_peer(
        context,
        &owner.peer_id,
        RemoteRequest::GroupSync {
            group_id: group.id.clone(),
        },
    )
    .await?;
    match response {
        RemoteResponse::GroupSynced {
            group_name: _,
            member,
            members,
        } => {
            if !member {
                drop_group(context, &group.id).await?;
                return Ok(());
            }
            let self_id = context.identity.id();
            // Roster is authoritative: upsert peers, membership, contributions.
            for remote in &members {
                context.store.save_remote_peer(
                    &remote.peer_id,
                    &remote.peer_name,
                    &serde_json::to_string(&remote.endpoint)?,
                )?;
                context
                    .store
                    .add_member(&group.id, &remote.peer_id, &remote.role)?;
                // Reconcile this member's full contribution set: a share the
                // owner no longer lists (e.g. a lost GroupShareRemoved push)
                // must not keep fan-out dialing it. Our own contributions are
                // local authority and never pruned here.
                if remote.peer_id != self_id {
                    let local = context.store.group_shares(&group.id, &remote.peer_id)?;
                    for existing in local {
                        if !remote
                            .shares
                            .iter()
                            .any(|(share_id, _)| *share_id == existing.share_id)
                        {
                            context.store.remove_group_share(
                                &group.id,
                                &remote.peer_id,
                                &existing.share_id,
                            )?;
                        }
                    }
                }
                for (share_id, share_name) in &remote.shares {
                    context.store.add_group_share(
                        &group.id,
                        &remote.peer_id,
                        share_id,
                        share_name,
                    )?;
                }
            }
            // Grant every member read access to our contributions.
            let self_shares = context.store.group_shares(&group.id, &self_id)?;
            for remote in &members {
                if remote.peer_id != self_id {
                    for share in &self_shares {
                        context
                            .store
                            .group_grant(&share.share_id, &remote.peer_id)?;
                    }
                }
            }
            // Drop members the owner no longer lists, revoking their grants.
            for local in local_members {
                if !members.iter().any(|remote| remote.peer_id == local.peer_id) {
                    context.store.remove_member(&group.id, &local.peer_id)?;
                    for share in &self_shares {
                        revoke_peer_grant(&context.store, &share.share_id, &local.peer_id)?;
                    }
                }
            }
            context.store.touch_group_sync(&group.id)?;
            Ok(())
        }
        response => bail!("Unexpected group sync response: {response:?}"),
    }
}

/// Best-effort sync when the cached roster is stale; never blocks queries.
/// The owner short-circuits inside [`sync_group`].
async fn maybe_sync_group(context: &Arc<DaemonContext>, group_name_or_id: &str) {
    let stale = match context.store.sync_stale(group_name_or_id, 300) {
        Ok(stale) => stale,
        Err(_) => return,
    };
    if stale {
        if let Err(error) = sync_group(context, group_name_or_id).await {
            // Owner unreachable — fall back to the cached roster.
            crate::output::error(format!(
                "group sync failed for `{group_name_or_id}`: {error:#}"
            ));
        }
    }
}

/// Remove the group locally: revoke the grants we handed out on every
/// contribution, then drop the group row (members + contributions cascade).
async fn drop_group(context: &Arc<DaemonContext>, group_name_or_id: &str) -> Result<()> {
    let group = context.store.group(group_name_or_id)?;
    let members = context.store.members(&group.id)?;
    let self_id = context.identity.id();
    for share in context.store.group_shares(&group.id, &self_id)? {
        for member in &members {
            if member.peer_id != self_id {
                revoke_peer_grant(&context.store, &share.share_id, &member.peer_id)?;
            }
        }
    }
    context.store.remove_group(&group.id)?;
    Ok(())
}

async fn leave_group(context: &Arc<DaemonContext>, group_name_or_id: &str) -> Result<()> {
    let group = context.store.group(group_name_or_id)?;
    let members = context.store.members(&group.id)?;
    let self_id = context.identity.id();
    let self_member = members
        .iter()
        .find(|member| member.peer_id == self_id)
        .context("You are not a member of this group")?;
    let self_shares = context.store.group_shares(&group.id, &self_id)?;
    if self_member.role == "owner" {
        // Owner leaving disbands the group: revoke all grants and kick everyone.
        for member in &members {
            if member.peer_id != self_id {
                for share in &self_shares {
                    revoke_peer_grant(&context.store, &share.share_id, &member.peer_id)?;
                }
            }
        }
        context.store.remove_group(&group.id)?;
        // Kick every remaining member: each request names its own target
        // (`peer_id` differs per recipient), so broadcast's shared template
        // does not fit — send them individually.
        for member in &members {
            if member.peer_id == self_id {
                continue;
            }
            let _ = tokio::time::timeout(
                Duration::from_secs(3),
                exchange_with_peer(
                    context,
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
        return Ok(());
    }
    // Regular member: revoke the grants we gave the group on our shares.
    for member in &members {
        if member.peer_id != self_id {
            for share in &self_shares {
                revoke_peer_grant(&context.store, &share.share_id, &member.peer_id)?;
            }
        }
    }
    context.store.remove_member(&group.id, &self_id)?;
    if let Some(owner) = members.iter().find(|member| member.role == "owner") {
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            exchange_with_peer(
                context,
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

async fn remove_group_member(
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
    for share in context.store.group_shares(&group.id, &self_id)? {
        revoke_peer_grant(&context.store, &share.share_id, &target.peer_id)?;
    }
    context.store.remove_member(&group.id, &target.peer_id)?;
    // Everyone hears about the removal — the removed peer clears the group
    // locally when it sees its own peer_id.
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
                exchange_with_peer(&context, &peer_id, request),
            )
            .await;
        });
    }
    while tasks.join_next().await.is_some() {}
}

/// Fan out a group query: the caller's own contributions run in-process (a
/// failure is a real error), every remote (member, share) is dialed in parallel
/// under a per-peer budget, and results are merged qualified per member and
/// share. Members that did not answer are reported as skipped.
async fn group_fan_out(
    context: &Arc<DaemonContext>,
    group_name: &str,
    members: &[MemberInfo],
    source: &str,
    filter: Filter,
) -> Result<GroupQueryResponse> {
    const PER_PEER_TIMEOUT: Duration = Duration::from_millis(2500);
    let self_id = context.identity.id();
    let mut records = Vec::new();
    let mut anchors = Vec::new();
    // Every result is scoped `team/alice/proj-b` so members stay apart and
    // records round-trip through show/zoom/nav.
    let mut merge = |peer_name: &str, share_name: &str, mut query: QueryResponse| {
        qualify_query_scope(
            &format!("{group_name}/{peer_name}/{share_name}"),
            &mut query,
        );
        records.extend(query.records);
        anchors.extend(query.anchors);
    };

    // The local member's contributions are part of the group, so they are
    // queried like any other share — just in-process instead of over the wire.
    // Local failures propagate; they are not "offline" peers.
    for member in members.iter().filter(|member| member.peer_id == self_id) {
        for (share_id, share_name) in &member.shares {
            let share = context.store.share(share_id)?;
            let query = tokio::task::spawn_blocking({
                let root = share.root.clone();
                let source = source.to_string();
                let filter = filter.clone();
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
            merge(&member.peer_name, share_name, query);
        }
    }

    let mut tasks = JoinSet::new();
    for member in members.iter().filter(|member| member.peer_id != self_id) {
        for (share_id, share_name) in &member.shares {
            let context = context.clone();
            let peer_id = member.peer_id.clone();
            let peer_name = member.peer_name.clone();
            let share_id = share_id.clone();
            let share_name = share_name.clone();
            let source = source.to_string();
            let filter = filter.clone();
            tasks.spawn(async move {
                let result = tokio::time::timeout(
                    PER_PEER_TIMEOUT,
                    exchange_with_peer(
                        &context,
                        &peer_id,
                        RemoteRequest::Query {
                            share_id,
                            source,
                            filter,
                        },
                    ),
                )
                .await;
                (peer_id, peer_name, share_name, result)
            });
        }
    }
    let mut offline: Vec<(String, String)> = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let Ok((peer_id, peer_name, share_name, result)) = joined else {
            continue;
        };
        match result {
            Ok(Ok(RemoteResponse::Query(query))) => {
                merge(&peer_name, &share_name, query);
                // A member is online if any share answered.
                offline.retain(|(id, _)| *id != peer_id);
            }
            _ => {
                if !offline.iter().any(|(id, _)| *id == peer_id) {
                    offline.push((peer_id, peer_name));
                }
            }
        }
    }
    let skipped: Vec<String> = offline
        .into_iter()
        .map(|(_, peer_name)| peer_name)
        .collect();
    Ok(GroupQueryResponse {
        query: QueryResponse { records, anchors },
        skipped,
    })
}

/// Resolve which (member, share) pairs a group query fans out to. The caller's
/// own contribution is a target like any other member's — the local member is
/// queried in-process by [`group_fan_out`]. `member` and `share` pin the
/// three-segment scopes `team/alice` and `team/alice/proj-b`.
fn group_targets(
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

/// Revoke a grant. A missing grant is an idempotent no-op (`revoke` returns
/// `Ok(None)`); every storage failure is propagated, so kick/leave/disband
/// cannot report success while access remains active.
fn revoke_peer_grant(
    store: &StateStore,
    share_name_or_id: &str,
    peer_name_or_id: &str,
) -> Result<()> {
    store.revoke(share_name_or_id, peer_name_or_id).map(|_| ())
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS RNG unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn remove_stale_daemon_info() -> Result<()> {
    match std::fs::remove_file(ipc::daemon_info_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::group_targets;
    use super::require_group_owner;
    use super::StateStore;

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
}
