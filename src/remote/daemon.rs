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
use iroh::{Endpoint, EndpointAddr};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::identity::Identity;
use super::ipc;
use super::protocol::{
    DaemonInfo, DaemonStatus, InviteTicket, LocalEnvelope, LocalRequest, LocalResponse,
    QueryResponse, RemoteRequest, RemoteResponse, MAX_MESSAGE_SIZE, REMOTE_ALPN,
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
    context: &DaemonContext,
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
            LocalResponse::Grant(context.store.revoke(&share, &peer)?)
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
        let observed = EndpointAddr::from_parts(
            info.id(),
            info.into_addrs().map(|addr| addr.into_addr()),
        );
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

async fn process_remote(
    context: &DaemonContext,
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
