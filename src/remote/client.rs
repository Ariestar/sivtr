//! Synchronous client for a remote sivtr device.
//!
//! Branches on the remote's transport: `Tcp` calls the HTTP JSON API via ureq;
//! `Iroh` runs the iroh future on a one-shot runtime (the read path stays sync,
//! so callers like `sivtr show` don't need to be async). Both produce the same
//! owned `WorkRecord` that `load_source` feeds downstream.

use anyhow::{Context, Result};
use serde::Serialize;
use sivtr_core::record::{WorkRecord, WorkRef};

use super::config::Remote;
use super::protocol::{AgentCard, SourceRequest, SourceResponse};

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

    /// Reachability check with a real transport and protocol round trip.
    pub fn ping(&self) -> Result<String> {
        match &self.remote {
            Remote::Tcp { .. } => self.ping_tcp(),
            Remote::Iroh { ticket } => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("Failed to start the iroh client runtime")?;
                runtime
                    .block_on(tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        crate::serve::iroh::probe_via_iroh(ticket),
                    ))
                    .context("iroh probe timed out")??;
                Ok("iroh".into())
            }
        }
    }

    /// Load one concrete ref or selector from the remote workspace.
    pub fn load_source(&self, source: &str) -> Result<SourceResponse> {
        let response = if let Ok(reference) = source.parse::<WorkRef>() {
            SourceResponse {
                records: vec![self.resolve(source)?],
                anchors: vec![reference],
            }
        } else {
            match &self.remote {
                Remote::Tcp { .. } => self.load_source_tcp(source)?,
                Remote::Iroh { ticket } => {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .context("Failed to start the iroh client runtime")?;
                    runtime.block_on(crate::serve::iroh::source_via_iroh(ticket, source))?
                }
            }
        };
        Ok(self.qualify_source(response))
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
        let resp: AgentCard = self
            .agent()
            .get(&format!("http://{host}:{port}/agent-card"))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .context(format!("failed to reach remote `{}`", self.alias))?
            .into_json()
            .context("invalid agent-card response")?;
        Ok(resp.name)
    }

    fn load_source_tcp(&self, source: &str) -> Result<SourceResponse> {
        let (host, port, token) = self.tcp_parts();
        let url = format!("http://{host}:{port}/source");
        let response = self
            .agent()
            .post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(serde_json::to_value(&SourceRequest {
                source: source.to_string(),
            })?)
            .map_err(|error| map_ureq_error(&self.alias, "/source", error))?;
        response.into_json().context(format!(
            "invalid response from remote `{}` at /source",
            self.alias
        ))
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

    fn qualify_source(&self, mut response: SourceResponse) -> SourceResponse {
        for record in &mut response.records {
            record.work_ref = WorkRef::Remote {
                name: self.alias.clone(),
                body: record.work_ref.body().clone(),
            };
        }
        for anchor in &mut response.anchors {
            *anchor = WorkRef::Remote {
                name: self.alias.clone(),
                body: anchor.body().clone(),
            };
        }
        response
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

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{WorkChannel, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime};

    #[test]
    fn qualifying_source_rewrites_records_and_anchors() {
        let client = RemoteClient::new(
            "desk",
            Remote::Iroh {
                ticket: "unused".to_string(),
            },
        );
        let record = WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::terminal_record("shell", 1),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: "shell".to_string(),
                canonical_id: Some("shell".to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "command".to_string(),
            parts: Vec::new(),
        };

        let response = client.qualify_source(SourceResponse {
            records: vec![record],
            anchors: vec![WorkRef::terminal_record("shell", 1)],
        });

        assert_eq!(
            response.records[0].work_ref.to_string(),
            "desk://terminal/shell/1"
        );
        assert_eq!(response.anchors[0].to_string(), "desk://terminal/shell/1");
    }
}
