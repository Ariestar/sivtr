use anyhow::{bail, Result};

use super::local;
use super::protocol::{LocalRequest, LocalResponse, SourceResponse};

pub struct RemoteClient {
    workspace_key: String,
    origin: String,
}

impl RemoteClient {
    pub fn new(workspace_key: &str, origin: &str) -> Self {
        Self {
            workspace_key: workspace_key.to_string(),
            origin: origin.to_ascii_lowercase(),
        }
    }

    pub fn load_source(&self, source: &str) -> Result<SourceResponse> {
        match local::call(LocalRequest::RemoteSource {
            workspace_key: self.workspace_key.clone(),
            alias: self.origin.clone(),
            source: source.to_string(),
        })? {
            LocalResponse::Source(response) => Ok(response),
            response => bail!("Unexpected daemon response: {response:?}"),
        }
    }
}
