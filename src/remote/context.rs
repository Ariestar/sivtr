//! Shared daemon state that every remote subsystem routes through.

use iroh::Endpoint;

use super::identity::Identity;
use super::state::StateStore;

/// Shared daemon state: storage, identity, and the iroh endpoint.
pub(crate) struct DaemonContext {
    pub(crate) store: StateStore,
    pub(crate) endpoint: Endpoint,
    pub(crate) identity: Identity,
    pub(crate) started_at: String,
    pub(crate) control_token: String,
}
