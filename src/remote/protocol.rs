use anyhow::{bail, Context, Result};
use base64::Engine;
use iroh::EndpointAddr;
use serde::{Deserialize, Serialize};
use sivtr_core::record::{WorkRecord, WorkRef};

use crate::commands::memory::filter::Filter;

pub use super::state::ShareInfo;
pub use super::state::{
    GrantInfo, GroupInfo, GroupMemberInfo, GroupShareInfo, MountInfo, PeerInfo,
};

pub const REMOTE_ALPN: &[u8] = b"sivtr/memory/1";
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;
/// Wire protocol version. Every request and response travels inside a
/// [`RemoteEnvelope`] carrying this version; a daemon rejects any envelope
/// whose version it does not speak instead of failing on an unknown variant.
pub const PROTOCOL_VERSION: u32 = 2;

/// Wire envelope for remote requests and responses: the payload plus the
/// protocol version, so a mixed fleet fails loudly and explicitly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEnvelope<T> {
    pub protocol_version: u32,
    pub kind: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteTicket {
    pub version: u16,
    pub endpoint: EndpointAddr,
    pub share_id: String,
    /// Present for group invites; share invites leave this empty.
    #[serde(default)]
    pub group_id: Option<String>,
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
    /// Joiner → group owner: redeem a multi-use group invite and register the
    /// joiner's contributed workspaces so the owner can grant the joiner access.
    /// The group is not part of the request: the invite row is the authority,
    /// and the owner returns the authoritative id in [`RemoteResponse::GroupJoined`].
    RedeemGroupInvite {
        invite_id: String,
        secret: String,
        peer_name: String,
        shares: Vec<(String, String)>,
        endpoint: EndpointAddr,
    },
    /// Group owner → existing members: a new member joined; grant them access
    /// to your group shares. Members also treat this for their own peer_id as a
    /// revocation signal (kicked).
    GroupMemberAdded {
        group_id: String,
        member: MemberInfo,
    },
    GroupMemberRemoved {
        group_id: String,
        peer_id: String,
        peer_name: String,
    },
    /// An existing member added another contributed workspace.
    GroupShareAdded {
        group_id: String,
        peer_id: String,
        peer_name: String,
        share_id: String,
        share_name: String,
    },
    /// An existing member withdrew one contributed workspace.
    GroupShareRemoved {
        group_id: String,
        peer_id: String,
        share_id: String,
    },
    /// Member → group owner: announce leaving so the roster drops the member.
    GroupLeave {
        group_id: String,
    },
    /// Member → group owner: pull the authoritative roster for reconciliation.
    GroupSync {
        group_id: String,
    },
    /// Run the same local query the peer would run: load `source` then apply `filter`.
    Query {
        share_id: String,
        /// Local-shaped body only (`agent`, `terminal`, `codex/…`), never `remote:path`.
        source: String,
        filter: Filter,
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
    GroupJoined {
        /// Authoritative group id, read from the invite row by the owner.
        group_id: String,
        group_name: String,
        members: Vec<MemberInfo>,
    },
    GroupSynced {
        group_name: String,
        member: bool,
        members: Vec<MemberInfo>,
    },
    GroupAck,
    Query(QueryResponse),
    Probe {
        server_name: String,
        share_name: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    pub peer_id: String,
    pub peer_name: String,
    /// Every contributed workspace as `(share_id, share_name)`.
    pub shares: Vec<(String, String)>,
    pub role: String,
    pub endpoint: EndpointAddr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub records: Vec<WorkRecord>,
    pub anchors: Vec<WorkRef>,
}

/// Prefix every ref in `response` with `scope` so results from different
/// sources stay apart and round-trip through show/zoom/nav. Shared by the
/// mount query path (alias scope) and group fan-out (member/share scope).
pub fn qualify_query_scope(scope: &str, response: &mut QueryResponse) {
    let scope = scope.to_ascii_lowercase();
    for record in &mut response.records {
        record.work_ref = record.work_ref.with_named_scope(scope.clone());
    }
    for anchor in &mut response.anchors {
        *anchor = anchor.with_named_scope(scope.clone());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupQueryResponse {
    pub query: QueryResponse,
    /// Peer names that did not answer within the fan-out budget.
    pub skipped: Vec<String>,
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
    /// Same [`PROTOCOL_VERSION`] as the remote wire; old clients (version 1)
    /// are rejected loudly instead of failing on an unknown request variant.
    #[serde(default)]
    pub protocol_version: u32,
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
    /// Client → local daemon → peer: full query (source + filter) on a remote share.
    RemoteQuery {
        workspace_key: String,
        alias: String,
        source: String,
        filter: Filter,
    },
    /// Create a group owned by this device, bound to one of our shares.
    GroupCreate {
        name: String,
        share_id: String,
        share_name: String,
    },
    GroupList,
    GroupMembers {
        group: String,
    },
    /// This device's contributed workspaces for a group (for join checkboxes).
    GroupShares {
        group: String,
    },
    /// Cheap existence probe for the query scope cascade; resolves the group
    /// from the store without syncing, so a query is never held up by an
    /// unreachable owner.
    GroupResolve {
        group: String,
    },
    GroupInvite {
        group: String,
        valid_for_seconds: i64,
        max_uses: Option<i64>,
    },
    /// Join a group (or adjust contributions): redeem the invite and register
    /// the final contributed-workspace list. The daemon diffs against current
    /// contributions — additions broadcast, withdrawals revoke grants.
    GroupJoin {
        invite: String,
        shares: Vec<(String, String)>,
    },
    GroupLeave {
        group: String,
    },
    /// Owner-only: remove a member (kicks them out of the group).
    GroupRemoveMember {
        group: String,
        peer: String,
    },
    /// Owner-only: rename the group. The name is the ref segment, stored once
    /// in `groups.name`; members mirror it on their next roster sync.
    GroupRename {
        group: String,
        name: String,
    },
    /// Force a roster pull from the group owner.
    GroupSync {
        group: String,
    },
    /// Fan out a query to every group member (or one, when `member` is set;
    /// and one contributed share when `share` is set).
    GroupQuery {
        group: String,
        member: Option<String>,
        share: Option<String>,
        source: String,
        filter: Filter,
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
    Group(GroupInfo),
    Groups(Vec<GroupInfo>),
    Members(Vec<GroupMemberInfo>),
    GroupShares(Vec<GroupShareInfo>),
    GroupResolved {
        exists: bool,
    },
    GroupJoined {
        group_name: String,
        member_count: usize,
    },
    GroupQuery(GroupQueryResponse),
    Query(QueryResponse),
    Error {
        message: String,
    },
}
