use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct SyncCommand {
    #[command(subcommand)]
    pub action: SyncAction,
}

#[derive(Subcommand, Debug)]
pub enum SyncAction {
    /// Configure the cloud remote and library key
    Init(SyncInitArgs),
    /// Synchronize local history with the configured remote
    Now(SyncRemoteArgs),
    /// Show cloud sync state
    Status(SyncRemoteArgs),
}

#[derive(Parser, Debug)]
pub struct ShareCommand {
    #[command(subcommand)]
    pub action: ShareAction,
}

#[derive(Subcommand, Debug)]
pub enum ShareAction {
    /// Register this device's public key on the remote
    Register(ShareRegisterArgs),
    /// Create a scoped capability for a device
    Create(ShareCreateArgs),
    /// Attach a capability and hydrate its scoped sessions
    Attach(ShareAttachArgs),
    /// Revoke a capability on the remote
    Revoke(ShareRevokeArgs),
}

#[derive(Args, Debug)]
pub struct SyncInitArgs {
    /// Remote URL: file://path or s3://bucket/prefix
    #[arg(long)]
    pub remote: String,

    /// Library key path; defaults to <SIVTR_HOME>/cloud.key
    #[arg(long)]
    pub key_file: Option<PathBuf>,

    /// Emit the complete machine-readable report
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SyncRemoteArgs {
    /// Override the saved remote URL
    #[arg(long)]
    pub remote: Option<String>,

    /// Override the saved library key path
    #[arg(long)]
    pub key_file: Option<PathBuf>,

    /// Remove cached sessions absent from the remote catalog after refresh
    #[arg(long)]
    pub prune: bool,

    /// Emit the complete machine-readable report
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ShareRegisterArgs {
    /// Device name published on the remote
    #[arg(long)]
    pub device: String,

    /// Remote URL when this home has no sync settings
    #[arg(long)]
    pub remote: Option<String>,

    /// Recipient private key path; defaults to <SIVTR_HOME>/recipient.key
    #[arg(long)]
    pub key_file: Option<PathBuf>,

    /// Emit the complete machine-readable report
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ShareCreateArgs {
    /// Scope: all, session:<id>, or provider:<name>
    #[arg(long)]
    pub scope: String,

    /// Recipient device name registered on the remote
    #[arg(long)]
    pub to: String,

    /// Capability id; generated when omitted
    #[arg(long)]
    pub id: Option<String>,

    /// Emit the complete machine-readable report
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ShareAttachArgs {
    /// Capability id published on the remote
    pub capability_id: String,

    /// Remote URL when this home has no sync settings
    #[arg(long)]
    pub remote: Option<String>,

    /// Recipient private key path; defaults to <SIVTR_HOME>/recipient.key
    #[arg(long)]
    pub key_file: Option<PathBuf>,

    /// Emit the complete machine-readable report
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ShareRevokeArgs {
    /// Capability id to remove from the remote
    pub capability_id: String,

    /// Emit the complete machine-readable report
    #[arg(long)]
    pub json: bool,
}
