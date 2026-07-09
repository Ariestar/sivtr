//! Synchronous HTTP client for a remote sivtr `serve` endpoint.
//!
//! Wraps the same JSON API `sivtr serve` exposes, so a remote WorkRef
//! (`desk://terminal/...`) resolves to a `WorkRecord`/`WorkPart` exactly like a
//! local ref — the client turns a network round-trip into the same owned types
//! `load_source` already feeds downstream.

use anyhow::{Context, Result};
use serde::Serialize;
use sivtr_core::record::WorkRecord;

use super::config::Remote;

/// One connection to a remote device's serve endpoint.
pub struct RemoteClient {
    agent: ureq::Agent,
    base_url: String,
    token: String,
    alias: String,
}

impl RemoteClient {
    pub fn new(alias: &str, remote: Remote) -> Self {
        let base_url = remote.base_url();
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(15))
                .build(),
            base_url,
            token: remote.token,
            alias: alias.to_string(),
        }
    }

    /// Ping `/agent-card` to confirm reachability and the token.
    pub fn ping(&self) -> Result<String> {
        let resp: serde_json::Value = self
            .agent
            .get(&format!("{}/agent-card", self.base_url))
            .set("Authorization", &format!("Bearer {}", self.token))
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

    /// Resolve a local-shape ref (the body, no origin prefix) to a record.
    pub fn resolve(&self, body_ref: &str) -> Result<WorkRecord> {
        #[derive(Serialize)]
        struct Req<'a> {
            #[serde(rename = "ref")]
            reference: &'a str,
        }
        let resp: ResolveResponse = self.post_json(
            "/resolve",
            &Req {
                reference: body_ref,
            },
        )?;
        Ok(resp.record)
    }

    fn post_json<Req: Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<Resp> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .agent
            .post(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .send_json(serde_json::to_value(body)?)
            .map_err(|e| map_ureq_error(&self.alias, path, e))?;
        response.into_json().context(format!(
            "invalid response from remote `{}` at {path}",
            self.alias
        ))
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
