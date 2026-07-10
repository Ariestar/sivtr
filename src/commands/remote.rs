use anyhow::{bail, Context, Result};
use sivtr_core::workspace;

use crate::cli::{RemoteAction, RemoteCommand};
use crate::output;
use crate::remote::local;
use crate::remote::protocol::{LocalRequest, LocalResponse};

pub fn execute(command: RemoteCommand) -> Result<()> {
    let workspace_key = current_workspace_key()?;
    match command.action {
        RemoteAction::List => list(&workspace_key),
        RemoteAction::Add { alias, invite } => add(&workspace_key, &alias, &invite),
        RemoteAction::Remove { alias } => remove(&workspace_key, &alias),
        RemoteAction::Rename { alias, new_alias } => rename(&workspace_key, &alias, &new_alias),
        RemoteAction::Test { alias } => test(&workspace_key, &alias),
    }
}

fn current_workspace_key() -> Result<String> {
    workspace::resolve_current_workspace()?
        .map(|paths| paths.key)
        .context("Remote mounts require a git workspace")
}

fn list(workspace_key: &str) -> Result<()> {
    match local::call(LocalRequest::RemoteList {
        workspace_key: workspace_key.to_string(),
    })? {
        LocalResponse::Mounts(mounts) => {
            if mounts.is_empty() {
                output::plain("no remote shares mounted in this workspace");
            }
            for mount in mounts {
                output::detail(
                    mount.alias,
                    format!(
                        "{} / {} ({} / {})",
                        mount.peer_name, mount.share_name, mount.peer_id, mount.share_id
                    ),
                );
            }
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn add(workspace_key: &str, alias: &str, invite: &str) -> Result<()> {
    match local::call(LocalRequest::RemoteAdd {
        workspace_key: workspace_key.to_string(),
        alias: alias.to_string(),
        invite: invite.to_string(),
    })? {
        LocalResponse::RemoteAdded { mount } => {
            output::success(format!("mounted remote share as `{}`", mount.alias));
            output::detail("peer", mount.peer_name);
            output::detail("share", mount.share_name);
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn remove(workspace_key: &str, alias: &str) -> Result<()> {
    match local::call(LocalRequest::RemoteRemove {
        workspace_key: workspace_key.to_string(),
        alias: alias.to_string(),
    })? {
        LocalResponse::Mount(mount) => {
            output::success(format!("removed local mount `{}`", mount.alias));
            output::info("the remote grant remains active until the share owner revokes it");
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn rename(workspace_key: &str, alias: &str, new_alias: &str) -> Result<()> {
    match local::call(LocalRequest::RemoteRename {
        workspace_key: workspace_key.to_string(),
        alias: alias.to_string(),
        new_alias: new_alias.to_string(),
    })? {
        LocalResponse::Mount(mount) => {
            output::success(format!("renamed remote mount to `{}`", mount.alias));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn test(workspace_key: &str, alias: &str) -> Result<()> {
    match local::call(LocalRequest::RemoteTest {
        workspace_key: workspace_key.to_string(),
        alias: alias.to_string(),
    })? {
        LocalResponse::RemoteTested {
            peer_name,
            share_name,
        } => {
            output::success(format!(
                "remote `{alias}` reachable ({peer_name} / {share_name})"
            ));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}
