//! Peer networking over iroh: dial a peer and exchange one request/response.

use anyhow::{Context, Result};
use iroh::endpoint::{Connection, Endpoint};
use iroh::EndpointAddr;

use super::protocol::{RemoteRequest, RemoteResponse, MAX_MESSAGE_SIZE, REMOTE_ALPN};
use super::state::StateStore;

/// Dial `peer_id` using its stored endpoint and exchange one request, then
/// refresh the peer's endpoint with the observed addresses.
pub(crate) async fn exchange_with_peer(
    store: &StateStore,
    endpoint: &Endpoint,
    peer_id: &str,
    request: RemoteRequest,
) -> Result<RemoteResponse> {
    let endpoint_json = store.peer_endpoint(peer_id)?;
    let address: EndpointAddr =
        serde_json::from_str(&endpoint_json).context("Invalid stored peer endpoint")?;
    let (response, observed) = exchange(endpoint, address, request).await?;
    let endpoint_json =
        serde_json::to_string(&observed).context("Failed to encode peer endpoint")?;
    store
        .refresh_peer_endpoint(peer_id, &endpoint_json)
        .context("Failed to refresh peer endpoint after successful dial")?;
    Ok(response)
}

/// Dial the peer and exchange one request/response.
///
/// Default mode (`presets::N0`) includes address lookup. We dial the stored/bootstrap
/// address first; if that fails, dial by `EndpointId` alone so N0 discovery can resolve
/// current direct/relay paths. That is how default mode works - not a path rewrite.
///
/// After a successful dial, return iroh's observed addresses so callers can refresh storage.
pub(crate) async fn exchange(
    endpoint: &Endpoint,
    address: EndpointAddr,
    request: RemoteRequest,
) -> Result<(RemoteResponse, EndpointAddr)> {
    let connection = connect_default(endpoint, &address).await?;
    let observed = observed_endpoint(endpoint, &connection, &address).await;
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
