use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct ServeCommand {
    #[command(subcommand)]
    pub action: ServeAction,
}

#[derive(Subcommand, Debug)]
pub enum ServeAction {
    /// Start the daemon in the background
    Start,
    /// Stop the running daemon cleanly
    Stop,
    /// Restart the daemon
    Restart,
    /// Show daemon identity and runtime state
    Status,
    /// Print the daemon log path
    Logs,
    /// Run the daemon in the foreground
    Foreground,
}

#[derive(Parser, Debug)]
pub struct ShareCommand {
    #[command(subcommand)]
    pub action: Option<ShareAction>,

    /// Workspace path for the default interactive share flow
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Stable share name for the default interactive share flow
    #[arg(long)]
    pub name: Option<String>,

    /// Invitation lifetime for the default interactive share flow
    #[arg(long, default_value = "10m")]
    pub expires: String,

    /// Disable secret redaction for the default interactive share flow
    #[arg(long)]
    pub no_redact: bool,
}

#[derive(Subcommand, Debug)]
pub enum ShareAction {
    /// Explicitly expose a workspace through the daemon
    Add {
        /// Workspace path; defaults to the current directory
        path: Option<PathBuf>,
        /// Stable share name; defaults to the workspace directory name
        #[arg(long)]
        name: Option<String>,
        /// Disable secret redaction for this share
        #[arg(long)]
        no_redact: bool,
    },
    /// List local shares
    List,
    /// Remove a share and all grants and invitations attached to it
    Remove { share: String },
    /// Enable a disabled share
    Enable { share: String },
    /// Disable a share without deleting it
    Disable { share: String },
    /// Create a single-use invitation for a share
    Invite {
        share: String,
        /// Invitation lifetime, such as 10m, 2h, or 1d
        #[arg(long, default_value = "10m")]
        expires: String,
    },
    /// List active peer grants for a share
    Grants { share: String },
    /// Revoke a peer's access to a share
    Revoke { share: String, peer: String },
}

#[derive(Parser, Debug)]
pub struct PeerCommand {
    #[command(subcommand)]
    pub action: PeerAction,
}

#[derive(Subcommand, Debug)]
pub enum PeerAction {
    /// List known peer identities
    List,
    /// Forget a peer and remove all local mounts and grants involving it
    Forget { peer: String },
}

#[derive(Parser, Debug)]
pub struct RemoteCommand {
    #[command(subcommand)]
    pub action: RemoteAction,
}

#[derive(Subcommand, Debug)]
pub enum RemoteAction {
    /// List remote mounts in the current workspace
    List,
    /// Redeem an invitation and mount the remote share in the current workspace
    Add {
        /// Workspace-local alias used in refs, e.g. `desk:terminal/...`
        alias: String,
        /// Invitation key from `sivtr share` (bare key; `sivtr-invite:` prefix optional)
        invite: String,
    },
    /// Remove a mount from the current workspace
    Remove { alias: String },
    /// Rename a workspace-local mount
    Rename { alias: String, new_alias: String },
    /// Perform an authenticated transport and authorization round trip
    Test { alias: String },
}

#[derive(Parser, Debug)]
pub struct WorkspaceCommand {
    #[command(subcommand)]
    pub action: Option<WorkspaceAction>,
}

#[derive(Subcommand, Debug)]
pub enum WorkspaceAction {
    /// List known local workspaces (origin labels for `name:body` refs)
    List,
}
