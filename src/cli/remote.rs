use clap::{Parser, Subcommand};

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
pub struct OriginCommand {
    #[command(subcommand)]
    pub action: OriginAction,
}

#[derive(Subcommand, Debug)]
pub enum OriginAction {
    /// Rename any origin by name: a local workspace alias or a remote mount
    /// (unified — one command for both, resolved through the origin registry).
    Rename {
        /// Current origin name (from `sivtr ws list` / `sivtr remote list`)
        name: String,
        /// New name used in `name:body` refs
        new_name: String,
    },
}
