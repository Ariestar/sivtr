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
    /// Create a single-use invitation peers redeem with `remote add`
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
    /// List remotes in the current workspace (like `git remote -v`)
    List,
    /// Redeem an invitation and name a remote share in this workspace (like `git remote add`)
    Add {
        /// Local remote name used in refs, e.g. `desk:terminal/...`
        alias: String,
        /// Single-use invite key from `sivtr share invite` (bare key only)
        invite: String,
    },
    /// Remove a remote name from the current workspace
    Remove { alias: String },
    /// Rename a remote in the current workspace
    Rename { alias: String, new_alias: String },
    /// Reachability + authorization probe for a remote
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

#[derive(Parser, Debug)]
pub struct GroupCommand {
    #[command(subcommand)]
    pub action: GroupAction,
}

#[derive(Subcommand, Debug)]
pub enum GroupAction {
    /// Create a group and share the current workspace with it
    Create {
        /// Group name used in refs, e.g. `team:terminal/...`
        name: String,
        /// Workspace path to contribute; defaults to the current directory
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Share name for the contributed workspace; defaults to the directory name
        #[arg(long)]
        share_name: Option<String>,
    },
    /// Print a reusable join link (expires, optionally capped at max_uses)
    Invite {
        group: String,
        /// Invitation lifetime, such as 10m, 2h, or 1d
        #[arg(long, default_value = "1d")]
        expires: String,
        /// Cap on how many peers may join with this link
        #[arg(long)]
        max_uses: Option<u32>,
    },
    /// Join a group with a join link and contribute the current workspace
    Join {
        /// Join link from `sivtr group invite` (bare key only)
        invite: String,
        /// Workspace path to contribute; defaults to the current directory
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Share name for the contributed workspace; defaults to the directory name
        #[arg(long)]
        share_name: Option<String>,
        /// Disable secret redaction for the contributed workspace
        #[arg(long)]
        no_redact: bool,
    },
    /// List groups this device has joined
    List,
    /// List group members with their last-seen state
    Members { group: String },
    /// Owner only: remove a member from the group
    Remove { group: String, peer: String },
    /// Owner only: rename the group (the new name is the ref segment)
    Rename { group: String, name: String },
    /// Leave a group (owner leaving disbands the group)
    Leave { group: String },
    /// Force a roster refresh from the group owner
    Sync { group: String },
}
