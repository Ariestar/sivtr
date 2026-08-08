use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use base64::Engine;
use chrono::Utc;
use fs2::FileExt;
use iroh::endpoint::presets;
use iroh::endpoint::Connection;
use iroh::Endpoint;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::context::DaemonContext;
use super::group;
use super::identity::Identity;
use super::ipc;
use super::net;
use super::protocol::{
    qualify_query_scope, DaemonInfo, DaemonStatus, InviteTicket, LocalEnvelope, LocalRequest,
    LocalResponse, QueryResponse, RemoteRequest, RemoteResponse, MAX_MESSAGE_SIZE, REMOTE_ALPN,
};
use super::state::{MountInfo, StateStore};

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
            let response = net::exchange_with_peer(
                &context.store,
                &context.endpoint,
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
            let response = net::exchange_with_peer(
                &context.store,
                &context.endpoint,
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
                group::maybe_sync_group(context, &group.name).await;
            }
            LocalResponse::Groups(context.store.groups()?)
        }
        LocalRequest::GroupMembers { group } => {
            group::maybe_sync_group(context, &group).await;
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
            // Only the owner may mint join links: a member-created invite
            // would let anyone join without the owner's consent.
            group::require_group_owner(&context.store, &group, &context.identity.id())?;
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
            let member_group = context.store.group(group_id).ok().filter(|group| {
                context.store.members(&group.id).is_ok_and(|members| {
                    members
                        .iter()
                        .any(|member| member.peer_id == context.identity.id())
                })
            });
            if let Some(group) = member_group {
                group::adjust_group_shares(context, &group.id, &shares).await?;
                LocalResponse::GroupJoined {
                    group_name: group.name,
                    member_count: group.member_count as usize,
                }
            } else {
                let (group_name, member_count) =
                    group::redeem_group_remote(context, &invite, &shares).await?;
                LocalResponse::GroupJoined {
                    group_name,
                    member_count,
                }
            }
        }
        LocalRequest::GroupLeave { group } => {
            group::leave_group(context, &group).await?;
            LocalResponse::Ok
        }
        LocalRequest::GroupRemoveMember { group, peer } => {
            group::remove_group_member(context, &group, &peer).await?;
            LocalResponse::Ok
        }
        LocalRequest::GroupRename { group, name } => {
            let info = context.store.group(&group)?;
            group::require_group_owner(&context.store, &info.id, &context.identity.id())?;
            LocalResponse::Group(context.store.rename_group(&info.id, &name)?)
        }
        LocalRequest::GroupSync { group } => {
            group::sync_group(context, &group).await?;
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
            group::maybe_sync_group(context, &group).await;
            let targets = group::group_targets(
                &context.store,
                &group_info,
                member.as_deref(),
                share.as_deref(),
            )?;
            let response =
                group::group_fan_out(context, &group_info.name, &targets, &source, filter).await?;
            LocalResponse::GroupQuery(response)
        }
    };
    Ok((response, false))
}

/// Mount-side join: redeem a share invite with its owner and register the
/// resulting peer + mount locally.
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
    let (response, observed) = net::exchange(
        &context.endpoint,
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
            let response = group::handle_redeem_group_invite(
                context, peer_id, &invite_id, &secret, peer_name, shares, endpoint,
            )
            .await?;
            Ok(response)
        }
        RemoteRequest::GroupMemberAdded { group_id, member } => {
            group::require_group_owner(&context.store, &group_id, peer_id)?;
            group::merge_member(context, &group_id, &member)?;
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
            group::require_group_owner(&context.store, &group_id, peer_id)?;
            group::handle_member_removed(context, &group_id, &removed_peer).await
        }
        RemoteRequest::GroupLeave { group_id } => {
            group::handle_leave(context, &group_id, peer_id).await?;
            Ok(RemoteResponse::GroupAck)
        }
        RemoteRequest::GroupSync { group_id } => {
            let Some((group_name, members)) = group::roster_for(context, &group_id, peer_id)?
            else {
                return Ok(RemoteResponse::GroupSynced {
                    group_name: group_id.clone(),
                    member: false,
                    members: Vec::new(),
                });
            };
            if members.is_empty() {
                return Ok(RemoteResponse::GroupSynced {
                    group_name,
                    member: false,
                    members: Vec::new(),
                });
            }
            Ok(RemoteResponse::GroupSynced {
                group_name,
                member: true,
                members,
            })
        }
    }
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
