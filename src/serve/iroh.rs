//! iroh transport — zero-config, encrypted, cross-network connection.
//!
//! Replaces host/port/NAT with "connect by EndpointAddr". `sivtr serve`
//! prints a ticket (base64 of the endpoint address); `sivtr remote add <ticket>`
//! stores it; `RemoteClient` connects over iroh. n0's default relays handle
//! rendezvous + hole-punching, with a relay fallback — so it works across NATs
//! without any network config. Self-hostable relays later for full local-first.
//!
//! The wire protocol is the same resolve request/response the HTTP server uses,
//! framed over a single QUIC bi-stream (request bytes → finish → response bytes).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::prelude::*;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr};
use serde::{Deserialize, Serialize};
use sivtr_core::ai::AgentProvider;
use sivtr_core::query::load_workspace_records;
use sivtr_core::record::{WorkRecord, WorkRef};

use crate::output;

/// Application protocol identifier for the resolve service.
const ALPN: &[u8] = b"sivtr/resolve/1";
/// Cap a single request/response at 64 MiB so a runaway read can't OOM.
const MAX_MSG: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct ResolveRequest {
    reference: String,
}

#[derive(Serialize, Deserialize)]
struct ResolveResponse {
    record: Option<WorkRecord>,
    error: Option<String>,
}

/// Encode an [`EndpointAddr`] as a pasteable ticket: base64(json).
pub fn ticket_from_addr(addr: &EndpointAddr) -> Result<String> {
    let bytes = serde_json::to_vec(addr).context("serialize endpoint addr")?;
    Ok(BASE64_STANDARD.encode(bytes))
}

/// Decode a ticket back into an [`EndpointAddr`].
pub fn addr_from_ticket(ticket: &str) -> Result<EndpointAddr> {
    let bytes = BASE64_STANDARD
        .decode(ticket)
        .with_context(|| format!("invalid iroh ticket `{ticket}`"))?;
    serde_json::from_slice(&bytes).context("malformed iroh ticket")
}

/// Run the iroh resolve server until interrupted. Blocks the (tokio) caller.
pub async fn serve_iroh(workspace: PathBuf, redact: bool) -> Result<()> {
    let endpoint = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("Failed to bind iroh endpoint")?;
    endpoint.online().await;

    let ticket = ticket_from_addr(&endpoint.addr())?;
    output::info("iroh serve endpoint ready; on the other device run:");
    output::plain(format!("  sivtr remote add {ticket}"));
    output::plain("press Ctrl+C to stop");

    while let Some(connecting) = endpoint.accept().await {
        let conn = match connecting.await {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        // Handle one connection inline (prototype; a spawn pool can come later).
        if let Err(err) = handle_connection(conn, &workspace, redact).await {
            output::error(format!("iroh connection error: {err:#}"));
        }
    }
    Ok(())
}

async fn handle_connection(
    conn: iroh::endpoint::Connection,
    workspace: &Path,
    redact: bool,
) -> Result<()> {
    let (mut send, mut recv) = conn.accept_bi().await.context("accept stream")?;
    let req_bytes = recv.read_to_end(MAX_MSG).await.context("read request")?;
    let req: ResolveRequest = serde_json::from_slice(&req_bytes).context("parse request")?;

    let resp = match resolve_record(workspace, &req.reference) {
        Ok(mut record) => {
            if redact {
                record = crate::serve::redact::redact_record(&record);
            }
            ResolveResponse {
                record: Some(record),
                error: None,
            }
        }
        Err(err) => ResolveResponse {
            record: None,
            error: Some(format!("{err:#}")),
        },
    };

    let resp_bytes = serde_json::to_vec(&resp).context("serialize response")?;
    send.write_all(&resp_bytes)
        .await
        .context("write response")?;
    send.finish().context("finish response")?;
    conn.closed().await;
    Ok(())
}

/// Resolve a local-shape ref against a workspace (the server-side logic).
fn resolve_record(workspace: &Path, reference: &str) -> Result<WorkRecord> {
    let work_ref: WorkRef = reference.parse().context("parse ref")?;
    let providers: Vec<AgentProvider> = AgentProvider::all()
        .iter()
        .map(|spec| spec.provider)
        .collect();
    let index = load_workspace_records(&providers, workspace, None)?.into_index();
    index
        .resolve(&work_ref)
        .cloned()
        .with_context(|| format!("no record for `{reference}`"))
}

/// Client side: connect via ticket and resolve a ref. Returns the record.
pub async fn resolve_via_iroh(ticket: &str, body_ref: &str) -> Result<WorkRecord> {
    let addr = addr_from_ticket(ticket)?;
    let endpoint = Endpoint::bind(presets::N0)
        .await
        .context("Failed to bind iroh client endpoint")?;
    let conn = endpoint
        .connect(addr, ALPN)
        .await
        .context("Failed to connect over iroh")?;

    let (mut send, mut recv) = conn.open_bi().await.context("open stream")?;
    let req = serde_json::to_vec(&ResolveRequest {
        reference: body_ref.to_string(),
    })?;
    send.write_all(&req).await.context("write request")?;
    send.finish().context("finish request")?;

    let resp_bytes = recv.read_to_end(MAX_MSG).await.context("read response")?;
    let resp: ResolveResponse = serde_json::from_slice(&resp_bytes).context("parse response")?;

    conn.close(0u32.into(), b"done");
    endpoint.close().await;

    match resp.record {
        Some(record) => Ok(record),
        None => anyhow::bail!(resp.error.unwrap_or_else(|| "unknown remote error".into())),
    }
}
