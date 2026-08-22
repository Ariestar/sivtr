//! Unified origin composition.
//!
//! The single place that assembles every addressable memory source (local
//! workspaces, remote device mounts) into one [`OriginRegistry`]. Each entry
//! pairs the display [`Origin`] with its [`Reach`] payload, so resolution
//! never re-looks-up what composition already knew. Upper layers consume the
//! registry — new sources add a constructor block here, and [`Origin`] itself
//! never changes.

use anyhow::{bail, Context, Result};
use std::path::Path;

use sivtr_core::origin::{Entry, OriginRegistry, Reach};
use sivtr_core::workspace;

use crate::commands::remote::serve;
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse};

/// All origins addressable from `cwd`: every local workspace (the current one
/// flagged) plus the current workspace's remote mounts.
pub fn collect(cwd: &Path) -> Result<OriginRegistry> {
    let mut entries = Vec::new();

    // Register `cwd` when it is a git repo, so the current workspace is part
    // of the registry even before its first capture.
    workspace::ensure_workspace_for_dir(cwd)?;

    let current_key = workspace::resolve_workspace_for_dir(cwd)?.map(|paths| paths.key);
    for meta in workspace::list_workspaces()? {
        let current = current_key.as_deref() == Some(meta.key.as_str());
        entries.push(Entry::new(
            workspace::workspace_alias(&meta),
            current,
            format!("{} ({})", meta.root, meta.key),
            Reach::Local {
                root: meta.root.clone(),
            },
        ));
    }

    if let Some(workspace_key) = current_key.as_deref() {
        // Passive enumeration: mounts are listed only while the daemon is
        // already running. Read-only callers (`ws list`, `sivtr_status`) must
        // not start the daemon; query paths start it explicitly before
        // collecting, so a scoped query still sees its mounts.
        if ipc::running() {
            match ipc::call(LocalRequest::RemoteList {
                workspace_key: workspace_key.to_string(),
            })? {
                LocalResponse::Mounts(mounts) => {
                    for mount in mounts {
                        entries.push(Entry::new(
                            mount.alias.clone(),
                            false,
                            format!("{}/{}", mount.peer_name, mount.share_name),
                            Reach::Remote {
                                workspace_key: workspace_key.to_string(),
                            },
                        ));
                    }
                }
                response => anyhow::bail!("Unexpected daemon response: {response:?}"),
            }
        }
    }

    Ok(OriginRegistry::new(entries))
}

/// Rename any origin by its current name, resolving through the registry so
/// local workspace aliases and remote mount aliases share one path. Fails when
/// the new name is empty or already belongs to another origin.
pub fn rename(cwd: &Path, name: &str, new_name: &str) -> Result<String> {
    // A rename may address a remote mount, which only enters the registry
    // while the daemon is running.
    serve::ensure_running()?;
    let registry = collect(cwd)?;
    let entry = registry.resolve(name)?.with_context(|| {
        format!("no origin named `{name}`; use `sivtr ws list` / `sivtr remote list`")
    })?;
    let new_name = new_name.trim().to_ascii_lowercase();
    if new_name.is_empty() {
        bail!("new name must not be empty");
    }
    if new_name == entry.origin.name.to_ascii_lowercase() {
        return Ok(entry.origin.name.clone());
    }
    // The new name must not belong to any other origin (local or remote).
    if let Some(other) = registry.resolve(&new_name)? {
        bail!(
            "origin `{new_name}` already exists ({})",
            other.origin.detail
        );
    }

    match &entry.reach {
        Reach::Local { root } => {
            let updated = workspace::rename_workspace(root, &new_name)?;
            Ok(workspace::workspace_alias(&updated))
        }
        Reach::Remote { workspace_key } => match ipc::call(LocalRequest::RemoteRename {
            workspace_key: workspace_key.clone(),
            alias: entry.origin.name.clone(),
            new_alias: new_name.clone(),
        })? {
            LocalResponse::Mount(mount) => Ok(mount.alias),
            response => bail!("Unexpected daemon response: {response:?}"),
        },
    }
}
