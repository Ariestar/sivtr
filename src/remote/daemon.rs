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
use super::groups;
use super::identity::Identity;
use super::ipc;
use super::net;
use super::protocol::{
    qualify_query_scope, DaemonInfo, DaemonStatus, InviteTicket, LocalEnvelope, LocalRequest,
    LocalResponse, QueryResponse, RemoteEnvelope, RemoteRequest, RemoteResponse, MAX_MESSAGE_SIZE,
    PROTOCOL_VERSION, REMOTE_ALPN,
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
    if envelope.protocol_version != PROTOCOL_VERSION {
        anyhow::bail!(
            "Unsupported control protocol version {} (this build speaks {PROTOCOL_VERSION})",
            envelope.protocol_version
        );
    }
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
            protocol_version: PROTOCOL_VERSION,
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
        } => groups::local_group_create(context, name, share_id, share_name).await?,
        LocalRequest::GroupList => groups::local_group_list(context).await?,
        LocalRequest::GroupMembers { group } => groups::local_group_members(context, group).await?,
        LocalRequest::GroupShares { group } => groups::local_group_shares(context, group).await?,
        LocalRequest::GroupInvite {
            group,
            valid_for_seconds,
            max_uses,
        } => groups::local_group_invite(context, group, valid_for_seconds, max_uses).await?,
        LocalRequest::GroupJoin { invite, shares } => {
            groups::local_group_join(context, invite, shares).await?
        }
        LocalRequest::GroupLeave { group } => groups::local_group_leave(context, group).await?,
        LocalRequest::GroupRemoveMember { group, peer } => {
            groups::local_group_remove_member(context, group, peer).await?
        }
        LocalRequest::GroupRename { group, name } => {
            groups::local_group_rename(context, group, name).await?
        }
        LocalRequest::GroupSync { group } => groups::local_group_sync(context, group).await?,
        LocalRequest::GroupQuery {
            group,
            member,
            share,
            source,
            filter,
        } => groups::local_group_query(context, group, member, share, source, filter).await?,
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
    let envelope: RemoteEnvelope<RemoteRequest> =
        serde_json::from_slice(&bytes).context("Invalid remote request")?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        anyhow::bail!(
            "Unsupported peer protocol version {} (this build speaks {PROTOCOL_VERSION})",
            envelope.protocol_version
        );
    }
    let response = match process_remote(&context, &peer_id, envelope.kind).await {
        Ok(response) => response,
        Err(error) => RemoteResponse::Error {
            message: format!("{error:#}"),
        },
    };
    let envelope = RemoteEnvelope {
        protocol_version: PROTOCOL_VERSION,
        kind: response,
    };
    send.write_all(&serde_json::to_vec(&envelope)?).await?;
    send.finish()?;
    connection.closed().await;
    Ok(())
}

async fn process_remote(
    context: &Arc<DaemonContext>,
    peer_id: &str,
    request: RemoteRequest,
) -> Result<RemoteResponse> {
    // The access matrix is the single gate: every request is classified once
    // against its role rule, then the arms do domain work only.
    match groups::access_rule(&request) {
        groups::AccessRule::Open => {}
        groups::AccessRule::Owner => {
            let group_id =
                groups::request_group_id(&request).context("Owner rule needs a group")?;
            groups::require_group_owner(&context.store, group_id, peer_id)?;
        }
        groups::AccessRule::Member => {
            let group_id =
                groups::request_group_id(&request).context("Member rule needs a group")?;
            if !context.store.is_member(group_id, peer_id)? {
                bail!("Only group members may send this request");
            }
        }
    }
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
                let (records, anchors) = crate::commands::memory::workset::run_on_share(
                    std::path::Path::new(&share.root),
                    &source,
                    filter,
                    share.redact,
                )?;
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
            let response = groups::handle_redeem_group_invite(
                context, peer_id, &invite_id, &secret, peer_name, shares, endpoint,
            )
            .await?;
            Ok(response)
        }
        RemoteRequest::GroupMemberAdded {
            group_id,
            members,
            roster_epoch,
        } => {
            groups::merge_member(context, &group_id, &members, roster_epoch)?;
            Ok(RemoteResponse::GroupAck)
        }
        RemoteRequest::GroupShareAdded {
            group_id,
            peer_id: contributor,
            peer_name: _,
            share_id,
            share_name,
        } => groups::handle_share_added(
            context,
            &group_id,
            peer_id,
            &contributor,
            &share_id,
            &share_name,
        ),
        RemoteRequest::GroupShareRemoved {
            group_id,
            peer_id: contributor,
            share_id,
        } => groups::handle_share_removed(context, &group_id, peer_id, &contributor, &share_id),
        RemoteRequest::GroupMemberRemoved {
            group_id,
            peer_id: removed_peer,
            peer_name: _,
            roster_epoch,
        } => groups::handle_member_removed(context, &group_id, &removed_peer, roster_epoch).await,
        RemoteRequest::GroupLeave { group_id } => {
            groups::handle_leave(context, &group_id, peer_id).await?;
            Ok(RemoteResponse::GroupAck)
        }
        RemoteRequest::GroupSync { group_id, shares } => {
            let Some((group_name, members)) = groups::roster_for(context, &group_id, peer_id)?
            else {
                return Ok(RemoteResponse::GroupSynced {
                    group_name: group_id.clone(),
                    member: false,
                    members: Vec::new(),
                    roster_epoch: 0,
                });
            };
            let roster_epoch = context.store.roster_epoch(&group_id)?;
            if members.is_empty() {
                return Ok(RemoteResponse::GroupSynced {
                    group_name,
                    member: false,
                    members: Vec::new(),
                    roster_epoch,
                });
            }
            // The member is the authority on its own contributions: repair the
            // roster from the reported list (the add/withdraw broadcast may
            // have been missed while we were offline), then answer with the
            // fresh roster so the member converges immediately.
            groups::sync_member_shares(&context.store, &group_id, peer_id, &shares)?;
            let roster_epoch = context.store.roster_epoch(&group_id)?;
            let (group_name, members) = groups::roster_for(context, &group_id, peer_id)?
                .context("Group disappeared during roster sync")?;
            Ok(RemoteResponse::GroupSynced {
                group_name,
                member: true,
                members,
                roster_epoch,
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
