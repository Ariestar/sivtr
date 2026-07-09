//! Synchronous client for a remote sivtr device.
//!
//! Branches on the remote's transport: `Tcp` calls the HTTP JSON API via ureq;
//! `Iroh` runs the iroh future on a one-shot runtime (the read path stays sync,
//! so callers like `sivtr show` don't need to be async). Both produce the same
//! owned `WorkRecord` that `load_source` feeds downstream.

use anyhow::{Context, Result};
use serde::Serialize;
use sivtr_core::record::WorkRecord;

use super::config::Remote;

/// One connection to a remote device.
pub struct RemoteClient {
    alias: String,
    remote: Remote,
}

impl RemoteClient {
    pub fn new(alias: &str, remote: Remote) -> Self {
        Self {
            alias: alias.to_string(),
            remote,
        }
    }

    /// Reachability check. For TCP, pings `/agent-card`; for iroh, resolves the
    /// ticket shape (a live probe happens on first resolve).
    pub fn ping(&self) -> Result<String> {
        match &self.remote {
            Remote::Tcp { .. } => self.ping_tcp(),
            Remote::Iroh { ticket } => {
                crate::serve::iroh::addr_from_ticket(ticket)?;
                Ok("iroh".into())
            }
        }
    }

    /// Resolve a local-shape ref (the body, no origin prefix) to a record.
    pub fn resolve(&self, body_ref: &str) -> Result<WorkRecord> {
        match &self.remote {
            Remote::Tcp { .. } => self.resolve_tcp(body_ref),
            Remote::Iroh { ticket } => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("Failed to start the iroh client runtime")?;
                runtime.block_on(crate::serve::iroh::resolve_via_iroh(ticket, body_ref))
            }
        }
    }

    fn tcp_parts(&self) -> (&str, u16, &str) {
        match &self.remote {
            Remote::Tcp { host, port, token } => (host, *port, token),
            Remote::Iroh { .. } => unreachable!("called tcp_parts on an iroh remote"),
        }
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(15))
            .build()
    }

    fn ping_tcp(&self) -> Result<String> {
        let (host, port, token) = self.tcp_parts();
        let resp: serde_json::Value = self
            .agent()
            .get(&format!("http://{host}:{port}/agent-card"))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .context(format!("failed to reach remote `{}`", self.alias))?
            .into_json()
            .context("invalid agent-card response")?;
        Ok(resp
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string())
    }

    fn resolve_tcp(&self, body_ref: &str) -> Result<WorkRecord> {
        #[derive(Serialize)]
        struct Req<'a> {
            #[serde(rename = "ref")]
            reference: &'a str,
        }
        let (host, port, token) = self.tcp_parts();
        let url = format!("http://{host}:{port}/resolve");
        let response = self
            .agent()
            .post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(serde_json::to_value(&Req {
                reference: body_ref,
            })?)
            .map_err(|e| map_ureq_error(&self.alias, "/resolve", e))?;
        let resp: ResolveResponse = response.into_json().context(format!(
            "invalid response from remote `{}` at /resolve",
            self.alias
        ))?;
        Ok(resp.record)
    }
}

fn map_ureq_error(alias: &str, path: &str, e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let trimmed = body.trim().trim_matches('"');
            anyhow::anyhow!("remote `{alias}` returned HTTP {code} at {path}: {trimmed}")
        }
        other => anyhow::anyhow!("request to remote `{alias}` at {path} failed: {other}"),
    }
}

#[derive(serde::Deserialize)]
struct ResolveResponse {
    record: WorkRecord,
}
