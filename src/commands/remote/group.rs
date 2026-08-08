use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{TimeZone, Utc};
use sivtr_core::workspace;

use crate::cli::{GroupAction, GroupCommand};
use crate::commands::interactive;
use crate::output;
use crate::remote::ipc;
use crate::remote::protocol::{InviteTicket, LocalRequest, LocalResponse, ShareInfo};

use super::serve;

pub fn execute(command: GroupCommand) -> Result<()> {
    serve::ensure_running()?;
    match command.action {
        GroupAction::Create {
            name,
            workspace,
            share_name,
        } => create(&name, workspace, share_name),
        GroupAction::Invite {
            group,
            expires,
            max_uses,
        } => invite(&group, &expires, max_uses),
        GroupAction::Join {
            invite,
            workspace,
            share_name,
            no_redact,
        } => join(&invite, workspace, share_name, !no_redact),
        GroupAction::List => list(),
        GroupAction::Members { group } => members(&group),
        GroupAction::Remove { group, peer } => remove(&group, &peer),
        GroupAction::Leave { group } => leave(&group),
        GroupAction::Sync { group } => sync(&group),
    }
}

fn create(name: &str, path: Option<PathBuf>, share_name: Option<String>) -> Result<()> {
    let (share, root) = ensure_share(path, share_name, true)?;
    match ipc::call(LocalRequest::GroupCreate {
        name: name.to_string(),
        share_id: share.id.clone(),
        share_name: share.name.clone(),
    })? {
        LocalResponse::Group(group) => {
            output::success(format!("created group `{}`", group.name));
            output::detail("id", group.id);
            output::detail("share", format!("{} ({})", share.name, root.display()));
            output::hint(format!(
                "invite others with: sivtr group invite {}",
                group.name
            ));
            output::hint(format!(
                "query the group with: sivtr s {}:terminal --latest 5",
                group.name
            ));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn invite(group: &str, expires: &str, max_uses: Option<u32>) -> Result<()> {
    let valid_for_seconds = super::share::parse_duration(expires)?;
    match ipc::call(LocalRequest::GroupInvite {
        group: group.to_string(),
        valid_for_seconds,
        max_uses: max_uses.map(i64::from),
    })? {
        LocalResponse::Invitation {
            share_name,
            ticket,
            expires_at,
        } => {
            let expires_at = Utc
                .timestamp_opt(expires_at, 0)
                .single()
                .context("Invalid invitation expiration")?;
            // Keep stdout clean for copy: one status line on stderr, key alone on stdout.
            output::info(format!(
                "join link for group `{share_name}` (expires {}). Peer: sivtr group join <link>",
                expires_at.to_rfc3339()
            ));
            println!("{ticket}");
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn join(
    encoded_invite: &str,
    path: Option<PathBuf>,
    share_name: Option<String>,
    redact: bool,
) -> Result<()> {
    let shares = select_contributions(encoded_invite, path, share_name, redact)?;
    match ipc::call(LocalRequest::GroupJoin {
        invite: encoded_invite.to_string(),
        shares,
    })? {
        LocalResponse::GroupJoined {
            group_name,
            member_count,
        } => {
            output::success(format!(
                "joined group `{group_name}` ({member_count} members)"
            ));
            output::hint(format!(
                "query the group with: sivtr s {group_name}:terminal --latest 5"
            ));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

/// Decide which workspaces to contribute: an explicit `--workspace` (append),
/// the current workspace when non-interactive (append), or an interactive
/// multi-select whose checkboxes default to already-contributed workspaces —
/// unchecking one withdraws that contribution. In every case the returned
/// list is the *final* contribution set; the daemon diffs it against current
/// contributions (additions register, withdrawals revoke).
fn select_contributions(
    encoded_invite: &str,
    path: Option<PathBuf>,
    share_name: Option<String>,
    redact: bool,
) -> Result<Vec<(String, String)>> {
    // Explicit workspace or non-interactive: append that workspace to the
    // current contributions (re-running join in another workspace adds it).
    let picker = interactive::is_interactive() && path.is_none();
    if !picker {
        let (share, _) = ensure_share(path, share_name, redact)?;
        let group_id = ticket_group_id(encoded_invite)?;
        return append_contribution(&group_id, &share);
    }

    let group_id = ticket_group_id(encoded_invite)?;
    let contributed: HashSet<String> = match ipc::call(LocalRequest::GroupShares {
        group: group_id.clone(),
    })? {
        LocalResponse::GroupShares(shares) => {
            shares.into_iter().map(|share| share.share_id).collect()
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    };
    let local_shares = match ipc::call(LocalRequest::ShareList)? {
        LocalResponse::Shares(shares) => shares,
        response => bail!("Unexpected daemon response: {response:?}"),
    };
    let choices = super::share::list_workspace_choices()?;
    if choices.is_empty() {
        bail!("no workspaces to contribute; run inside a git repo first");
    }
    let labels: Vec<String> = choices.iter().map(|choice| choice.label()).collect();
    let is_contributed = |choice: &super::share::WorkspaceChoice| {
        local_shares
            .iter()
            .find(|share| share.workspace_key == choice.key)
            .map(|share| contributed.contains(&share.id))
            .unwrap_or(false)
    };
    // Already-contributed workspaces default to checked; first join defaults
    // the current workspace so the picker is useful.
    let mut defaults: Vec<usize> = choices
        .iter()
        .enumerate()
        .filter(|(_, choice)| is_contributed(choice))
        .map(|(index, _)| index)
        .collect();
    if defaults.is_empty() {
        if let Some(index) = choices.iter().position(|choice| choice.current) {
            defaults.push(index);
        }
    }
    let selected = interactive::multi_select(
        "Contribute which workspaces to the group?",
        &labels,
        &defaults,
    )?;
    let mut shares = Vec::with_capacity(selected.len());
    for index in selected {
        let choice = &choices[index];
        let (share, _) = ensure_share(Some(PathBuf::from(&choice.root)), None, redact)?;
        shares.push((share.id, share.name));
    }
    Ok(shares)
}

/// Extract the group id from a join link.
fn ticket_group_id(encoded_invite: &str) -> Result<String> {
    let ticket = InviteTicket::parse(encoded_invite)?;
    ticket.group_id.context("Invitation is not a group invite")
}

/// Append one contribution to the current set (idempotent).
fn append_contribution(group_id: &str, share: &ShareInfo) -> Result<Vec<(String, String)>> {
    let mut shares: Vec<(String, String)> = match ipc::call(LocalRequest::GroupShares {
        group: group_id.to_string(),
    })? {
        LocalResponse::GroupShares(shares) => shares
            .into_iter()
            .map(|share| (share.share_id, share.share_name))
            .collect(),
        response => bail!("Unexpected daemon response: {response:?}"),
    };
    if !shares.iter().any(|(id, _)| *id == share.id) {
        shares.push((share.id.clone(), share.name.clone()));
    }
    Ok(shares)
}

fn list() -> Result<()> {
    match ipc::call(LocalRequest::GroupList)? {
        LocalResponse::Groups(groups) => {
            if groups.is_empty() {
                output::plain("no groups on this device");
            }
            for group in groups {
                output::detail(
                    group.name,
                    format!("{} members ({})", group.member_count, group.id),
                );
            }
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn members(group: &str) -> Result<()> {
    match ipc::call(LocalRequest::GroupMembers {
        group: group.to_string(),
    })? {
        LocalResponse::Members(members) => {
            if members.is_empty() {
                output::plain("no members");
            }
            for member in members {
                let marker = if member.role == "owner" {
                    " [owner]"
                } else {
                    ""
                };
                let last_seen = member
                    .last_seen_at
                    .map(|timestamp| format!(", last seen {timestamp}"))
                    .unwrap_or_default();
                output::detail(
                    format!("{}{marker}", member.peer_name),
                    format!(
                        "{} workspaces ({}){}",
                        member.share_count, member.peer_id, last_seen
                    ),
                );
            }
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn remove(group: &str, peer: &str) -> Result<()> {
    match ipc::call(LocalRequest::GroupRemoveMember {
        group: group.to_string(),
        peer: peer.to_string(),
    })? {
        LocalResponse::Ok => {
            output::success(format!("removed `{peer}` from `{group}`"));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn leave(group: &str) -> Result<()> {
    match ipc::call(LocalRequest::GroupLeave {
        group: group.to_string(),
    })? {
        LocalResponse::Ok => {
            output::success(format!("left group `{group}`"));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn sync(group: &str) -> Result<()> {
    match ipc::call(LocalRequest::GroupSync {
        group: group.to_string(),
    })? {
        LocalResponse::Group(info) => {
            output::success(format!(
                "synced group `{}` ({} members)",
                info.name, info.member_count
            ));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

/// Contribute a workspace to the group: reuse the existing share for this
/// workspace when one exists, otherwise create it.
fn ensure_share(
    path: Option<PathBuf>,
    share_name: Option<String>,
    redact: bool,
) -> Result<(ShareInfo, PathBuf)> {
    let path =
        path.unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let paths = workspace::ensure_workspace_for_dir(&path)?
        .with_context(|| format!("{} is not inside a git workspace", path.display()))?;
    let name = share_name.unwrap_or_else(|| super::share::default_share_name(&paths.root));
    let share = match super::share::find_share_for_workspace(&paths.key) {
        Ok(existing) if existing.enabled => existing,
        Ok(existing) => {
            // Re-enable a disabled share, mirroring `sivtr share add`: a
            // contributed workspace must be queryable, or members' `authorize`
            // would deny every read.
            match ipc::call(LocalRequest::ShareSetEnabled {
                share: existing.id,
                enabled: true,
            })? {
                LocalResponse::Share(share) => share,
                response => bail!("Unexpected daemon response: {response:?}"),
            }
        }
        Err(_) => match ipc::call(LocalRequest::ShareAdd {
            workspace_key: paths.key,
            root: paths.root.display().to_string(),
            name,
            redact,
        })? {
            LocalResponse::Share(share) => share,
            response => bail!("Unexpected daemon response: {response:?}"),
        },
    };
    Ok((share, paths.root))
}
