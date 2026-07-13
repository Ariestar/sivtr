use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{TimeZone, Utc};
use sivtr_core::workspace::{self, WorkspaceMetadata};

use crate::cli::{ShareAction, ShareCommand};
use crate::commands::interactive;
use crate::commands::remote::workspace::workspace_display_name;
use crate::output;
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse, ShareInfo};

use super::serve;

pub fn execute(command: ShareCommand) -> Result<()> {
    match command.action {
        None => interactive_share(
            command.path,
            command.name,
            !command.no_redact,
            &command.expires,
        ),
        Some(ShareAction::Add {
            path,
            name,
            no_redact,
        }) => {
            serve::ensure_running()?;
            add(path, name, !no_redact).map(|_| ())
        }
        Some(ShareAction::List) => {
            serve::ensure_running()?;
            list()
        }
        Some(ShareAction::Remove { share }) => {
            serve::ensure_running()?;
            remove(&share)
        }
        Some(ShareAction::Enable { share }) => {
            serve::ensure_running()?;
            set_enabled(&share, true)
        }
        Some(ShareAction::Disable { share }) => {
            serve::ensure_running()?;
            set_enabled(&share, false)
        }
        Some(ShareAction::Invite { share, expires }) => {
            serve::ensure_running()?;
            invite(&share, &expires)
        }
        Some(ShareAction::Grants { share }) => {
            serve::ensure_running()?;
            grants(&share)
        }
        Some(ShareAction::Revoke { share, peer }) => {
            serve::ensure_running()?;
            revoke(&share, &peer)
        }
    }
}

/// Default `sivtr share` flow:
/// 1. ensure daemon
/// 2. interactively pick a workspace (Enter = current)
/// 3. share if needed
/// 4. print invite
fn interactive_share(
    path: Option<PathBuf>,
    name: Option<String>,
    redact: bool,
    expires: &str,
) -> Result<()> {
    serve::ensure_running()?;

    // Explicit --path/--name skip the picker and share that target after confirm.
    if path.is_some() || name.is_some() {
        let path =
            path.unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
        let paths = workspace::ensure_workspace_for_dir(&path)?
            .with_context(|| format!("{} is not inside a git workspace", path.display()))?;
        let share_name = name.unwrap_or_else(|| default_share_name(&paths.root));
        confirm_single(&paths.root, &share_name)?;
        return finish_share(paths.root, paths.key, share_name, redact, expires);
    }

    let selected = pick_workspace()?;
    let share_name = default_share_name(Path::new(&selected.root));
    finish_share(
        PathBuf::from(&selected.root),
        selected.key,
        share_name,
        redact,
        expires,
    )
}

fn finish_share(
    root: PathBuf,
    workspace_key: String,
    share_name: String,
    redact: bool,
    expires: &str,
) -> Result<()> {
    let share = match find_share_for_workspace(&workspace_key) {
        Ok(existing) => {
            output::info(format!("workspace already shared as `{}`", existing.name));
            if !existing.enabled {
                set_enabled(&existing.name, true)?;
            }
            // Re-fetch after enable so we invite with consistent state.
            find_share_for_workspace(&workspace_key).unwrap_or(existing)
        }
        Err(_) => add(Some(root), Some(share_name), redact)?,
    };
    invite(&share.name, expires)
}

#[derive(Clone)]
struct WorkspaceChoice {
    key: String,
    root: String,
    name: String,
    current: bool,
}

fn pick_workspace() -> Result<WorkspaceChoice> {
    interactive::require_interactive("share")?;

    let current = workspace::resolve_current_workspace()?;
    // Ensure the current repo is registered so it appears in the list.
    if let Some(ref paths) = current {
        let _ = workspace::ensure_workspace_for_dir(&paths.root)?;
    }

    let mut metas = workspace::list_workspaces()?;
    if let Some(ref paths) = current {
        if !metas.iter().any(|meta| meta.key == paths.key) {
            metas.insert(
                0,
                WorkspaceMetadata {
                    key: paths.key.clone(),
                    root: paths.root.display().to_string(),
                    created_at: String::new(),
                    last_seen_at: String::new(),
                },
            );
        }
    }
    if metas.is_empty() {
        bail!("no workspaces recorded yet; run inside a git repo first");
    }

    // Current first, then recent.
    let current_key = current.as_ref().map(|paths| paths.key.as_str());
    metas.sort_by(|a, b| {
        let a_cur = Some(a.key.as_str()) == current_key;
        let b_cur = Some(b.key.as_str()) == current_key;
        b_cur
            .cmp(&a_cur)
            .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
    });

    let choices: Vec<WorkspaceChoice> = metas
        .into_iter()
        .map(|meta| {
            let current = Some(meta.key.as_str()) == current_key;
            WorkspaceChoice {
                name: workspace_display_name(&meta),
                key: meta.key,
                root: meta.root,
                current,
            }
        })
        .collect();

    let default_index = choices
        .iter()
        .position(|choice| choice.current)
        .unwrap_or(0);
    let labels: Vec<String> = choices
        .iter()
        .map(|choice| {
            let marker = if choice.current { " [current]" } else { "" };
            format!("{}  {}{marker}", choice.name, choice.root)
        })
        .collect();

    let index = interactive::select("Share which workspace?", &labels, default_index)?;
    Ok(choices[index].clone())
}

fn confirm_single(root: &Path, share_name: &str) -> Result<()> {
    interactive::require_interactive("share")?;
    let prompt = format!("Share workspace `{}` as `{share_name}`?", root.display());
    if interactive::confirm(&prompt, true)? {
        return Ok(());
    }
    bail!("share cancelled");
}

fn add(path: Option<PathBuf>, name: Option<String>, redact: bool) -> Result<ShareInfo> {
    let path =
        path.unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let paths = workspace::ensure_workspace_for_dir(&path)?
        .with_context(|| format!("{} is not inside a git workspace", path.display()))?;
    let name = name.unwrap_or_else(|| default_share_name(&paths.root));
    match ipc::call(LocalRequest::ShareAdd {
        workspace_key: paths.key,
        root: paths.root.display().to_string(),
        name,
        redact,
    })? {
        LocalResponse::Share(share) => {
            output::success(format!("shared workspace `{}`", share.name));
            output::detail("id", share.id.clone());
            output::detail("root", share.root.clone());
            output::detail(
                "redaction",
                if share.redact { "enabled" } else { "disabled" },
            );
            Ok(share)
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn find_share_for_workspace(workspace_key: &str) -> Result<ShareInfo> {
    match ipc::call(LocalRequest::ShareList)? {
        LocalResponse::Shares(shares) => shares
            .into_iter()
            .find(|share| share.workspace_key == workspace_key)
            .context("current workspace is not shared"),
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn default_share_name(root: &Path) -> String {
    root.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("workspace")
        .to_string()
}

fn list() -> Result<()> {
    match ipc::call(LocalRequest::ShareList)? {
        LocalResponse::Shares(shares) => {
            if shares.is_empty() {
                output::plain("no workspaces are shared");
            }
            for share in shares {
                let status = if share.enabled { "enabled" } else { "disabled" };
                output::detail(
                    share.name,
                    format!("[{status}] {} ({})", share.root, share.id),
                );
            }
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn remove(share: &str) -> Result<()> {
    match ipc::call(LocalRequest::ShareRemove {
        share: share.to_string(),
    })? {
        LocalResponse::Share(share) => {
            output::success(format!("removed share `{}`", share.name));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn set_enabled(share: &str, enabled: bool) -> Result<()> {
    match ipc::call(LocalRequest::ShareSetEnabled {
        share: share.to_string(),
        enabled,
    })? {
        LocalResponse::Share(share) => {
            output::success(format!(
                "{} share `{}`",
                if enabled { "enabled" } else { "disabled" },
                share.name
            ));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn invite(share: &str, expires: &str) -> Result<()> {
    let valid_for_seconds = parse_duration(expires)?;
    match ipc::call(LocalRequest::ShareInvite {
        share: share.to_string(),
        valid_for_seconds,
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
                "invite for `{share_name}` (expires {}, single-use). Run on peer: sivtr remote add <alias> <key>",
                expires_at.to_rfc3339()
            ));
            println!("{ticket}");
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn grants(share: &str) -> Result<()> {
    match ipc::call(LocalRequest::ShareGrants {
        share: share.to_string(),
    })? {
        LocalResponse::Grants(grants) => {
            if grants.is_empty() {
                output::plain("no active grants");
            }
            for grant in grants {
                output::detail(
                    grant.peer_name,
                    format!("{} ({})", grant.permission, grant.peer_id),
                );
            }
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn revoke(share: &str, peer: &str) -> Result<()> {
    match ipc::call(LocalRequest::ShareRevoke {
        share: share.to_string(),
        peer: peer.to_string(),
    })? {
        LocalResponse::Grant(grant) => {
            output::success(format!(
                "revoked `{}` from `{}`",
                grant.peer_name, grant.share_name
            ));
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

fn parse_duration(value: &str) -> Result<i64> {
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .context("Duration must include a unit, such as 10m, 2h, or 1d")?;
    let amount: i64 = value[..split].parse().context("Invalid duration amount")?;
    let multiplier = match &value[split..] {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => bail!("Unsupported duration unit; use s, m, h, or d"),
    };
    amount
        .checked_mul(multiplier)
        .filter(|seconds| *seconds > 0)
        .context("Invitation duration must be positive")
}

#[cfg(test)]
mod tests {
    use super::parse_duration;

    #[test]
    fn parses_invite_duration() {
        assert_eq!(parse_duration("10m").unwrap(), 600);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
        assert_eq!(parse_duration("1d").unwrap(), 86400);
        assert!(parse_duration("10").is_err());
    }
}
