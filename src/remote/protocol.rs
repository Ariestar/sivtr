use anyhow::{bail, Context, Result};
use base64::Engine;
use iroh::EndpointAddr;
use serde::{Deserialize, Serialize};
use sivtr_core::record::{WorkRecord, WorkRef};

pub use super::state::ShareInfo;
use super::state::{GrantInfo, MountInfo, PeerInfo};

pub const REMOTE_ALPN: &[u8] = b"sivtr/memory/1";
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteTicket {
    pub version: u16,
    pub endpoint: EndpointAddr,
    pub share_id: String,
    pub invite_id: String,
    pub secret: String,
    pub expires_at: i64,
}

impl InviteTicket {
    pub fn encode(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self).context("Failed to encode invitation")?;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn parse(value: &str) -> Result<Self> {
        let encoded = value.trim();
        if encoded.is_empty() {
            bail!("Expected an invitation key");
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .context("Invalid invitation key")?;
        let ticket: Self = serde_json::from_slice(&bytes).context("Invalid invitation key")?;
        if ticket.version != 1 {
            bail!("Unsupported invitation version {}", ticket.version);
        }
        Ok(ticket)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoteRequest {
    RedeemInvite {
        invite_id: String,
        secret: String,
        peer_name: String,
    },
    Source {
        share_id: String,
        source: String,
    },
    Probe {
        share_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoteResponse {
    Redeemed {
        server_name: String,
        share_id: String,
        share_name: String,
    },
    Source(SourceResponse),
    Probe {
        server_name: String,
        share_name: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceResponse {
    pub records: Vec<WorkRecord>,
    pub anchors: Vec<WorkRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub node_id: String,
    pub endpoint: EndpointAddr,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub node_id: String,
    pub device_name: String,
    pub endpoint: EndpointAddr,
    pub started_at: String,
    pub shares: usize,
    pub peers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalEnvelope {
    pub token: String,
    pub request: LocalRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LocalRequest {
    Status,
    Shutdown,
    ShareAdd {
        workspace_key: String,
        root: String,
        name: String,
        redact: bool,
    },
    ShareList,
    ShareRemove {
        share: String,
    },
    ShareSetEnabled {
        share: String,
        enabled: bool,
    },
    ShareInvite {
        share: String,
        valid_for_seconds: i64,
    },
    ShareGrants {
        share: String,
    },
    ShareRevoke {
        share: String,
        peer: String,
    },
    PeerList,
    PeerForget {
        peer: String,
    },
    RemoteAdd {
        workspace_key: String,
        alias: String,
        invite: String,
    },
    RemoteList {
        workspace_key: String,
    },
    RemoteRemove {
        workspace_key: String,
        alias: String,
    },
    RemoteRename {
        workspace_key: String,
        alias: String,
        new_alias: String,
    },
    RemoteTest {
        workspace_key: String,
        alias: String,
    },
    RemoteSource {
        workspace_key: String,
        alias: String,
        source: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LocalResponse {
    Ok,
    Status(DaemonStatus),
    Share(ShareInfo),
    Shares(Vec<ShareInfo>),
    Invitation {
        share_name: String,
        ticket: String,
        expires_at: i64,
    },
    Grants(Vec<GrantInfo>),
    Grant(GrantInfo),
    Peers(Vec<PeerInfo>),
    Peer(PeerInfo),
    Mount(MountInfo),
    Mounts(Vec<MountInfo>),
    RemoteAdded {
        mount: MountInfo,
    },
    RemoteTested {
        peer_name: String,
        share_name: String,
    },
    Source(SourceResponse),
    Error {
        message: String,
    },
}
