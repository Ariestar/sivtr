use anyhow::{bail, Result};

use super::local;
use super::protocol::{LocalRequest, LocalResponse, SourceResponse};

pub struct RemoteClient {
    workspace_key: String,
    alias: String,
}

impl RemoteClient {
    pub fn new(workspace_key: &str, alias: &str) -> Self {
        Self {
            workspace_key: workspace_key.to_string(),
            alias: alias.to_ascii_lowercase(),
        }
    }

    pub fn load_source(&self, source: &str) -> Result<SourceResponse> {
        match local::call(LocalRequest::RemoteSource {
            workspace_key: self.workspace_key.clone(),
            alias: self.alias.clone(),
            source: source.to_string(),
        })? {
            LocalResponse::Source(response) => Ok(response),
            response => bail!("Unexpected daemon response: {response:?}"),
        }
    }

    pub fn load_canonical(
        peer_id: &str,
        share_id: &str,
        alias: &str,
        source: &str,
    ) -> Result<SourceResponse> {
        match local::call(LocalRequest::RemoteSourceCanonical {
            peer_id: peer_id.to_string(),
            share_id: share_id.to_string(),
            alias: alias.to_string(),
            source: source.to_string(),
        })? {
            LocalResponse::Source(response) => Ok(response),
            response => bail!("Unexpected daemon response: {response:?}"),
        }
    }
}
