//! Unified origin composition.
//!
//! The single place that assembles every addressable memory source (local
//! workspaces, remote device mounts, cloud accounts) into one
//! [`OriginRegistry`]. Each entry pairs the display [`Origin`] with its
//! [`Reach`] payload, so resolution never re-looks-up what composition
//! already knew. Upper layers consume the registry — new sources add a
//! constructor block here, and [`Origin`] itself never changes.

use anyhow::Result;
use std::path::Path;

use sivtr_core::origin::{Entry, Origin, OriginKind, OriginRegistry, Reach};
use sivtr_core::workspace;

use crate::commands::remote::serve;
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse};

/// All origins addressable from `cwd`: every local workspace (the current one
/// flagged), the current workspace's remote mounts, and cloud sources
/// (reserved — none yet).
pub fn collect(cwd: &Path) -> Result<OriginRegistry> {
    let mut entries = Vec::new();

    // Register `cwd` when it is a git repo, so the current workspace is part
    // of the registry even before its first capture.
    let _ = workspace::ensure_workspace_for_dir(cwd);

    let current_key = workspace::resolve_workspace_for_dir(cwd)?.map(|paths| paths.key);
    for meta in workspace::list_workspaces()? {
        let current = current_key.as_deref() == Some(meta.key.as_str());
        entries.push(Entry {
            origin: Origin {
                name: workspace::workspace_display_name(&meta),
                kind: OriginKind::Local,
                current,
                detail: format!("{} ({})", meta.root, meta.key),
            },
            reach: Reach::Local { root: meta.root },
        });
    }

    if let Some(workspace_key) = current_key.as_deref() {
        serve::ensure_running()?;
        match ipc::call(LocalRequest::RemoteList {
            workspace_key: workspace_key.to_string(),
        })? {
            LocalResponse::Mounts(mounts) => {
                for mount in mounts {
                    entries.push(Entry {
                        origin: Origin {
                            name: mount.alias.clone(),
                            kind: OriginKind::Remote,
                            current: false,
                            detail: format!("{}/{}", mount.peer_name, mount.share_name),
                        },
                        reach: Reach::Remote {
                            workspace_key: workspace_key.to_string(),
                            alias: mount.alias,
                        },
                    });
                }
            }
            response => anyhow::bail!("Unexpected daemon response: {response:?}"),
        }
    }

    // Cloud origins: reserved — cloud sources will construct here.

    Ok(OriginRegistry::new(entries))
}
