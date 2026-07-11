use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{TimeZone, Utc};
use sivtr_core::workspace;

use crate::cli::{ShareAction, ShareCommand};
use crate::output;
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse, ShareInfo};

use super::serve;

pub fn execute(command: ShareCommand) -> Result<()> {
    match command.action {
        ShareAction::Open {
            path,
            name,
            expires,
            no_redact,
        } => open(path, name, !no_redact, &expires),
        ShareAction::Add {
            path,
            name,
            no_redact,
        } => {
            serve::ensure_running()?;
            add(path, name, !no_redact).map(|_| ())
        }
        ShareAction::List => {
            serve::ensure_running()?;
            list()
        }
        ShareAction::Remove { share } => {
            serve::ensure_running()?;
            remove(&share)
        }
        ShareAction::Enable { share } => {
            serve::ensure_running()?;
            set_enabled(&share, true)
        }
        ShareAction::Disable { share } => {
            serve::ensure_running()?;
            set_enabled(&share, false)
        }
        ShareAction::Invite { share, expires } => {
            serve::ensure_running()?;
            invite(&share, &expires)
        }
        ShareAction::Grants { share } => {
            serve::ensure_running()?;
            grants(&share)
        }
        ShareAction::Revoke { share, peer } => {
            serve::ensure_running()?;
            revoke(&share, &peer)
        }
    }
}

/// One-shot share flow:
/// 1. ensure daemon
/// 2. interactively confirm current workspace (Enter = yes)
/// 3. share if needed
/// 4. print invite
fn open(path: Option<PathBuf>, name: Option<String>, redact: bool, expires: &str) -> Result<()> {
    serve::ensure_running()?;

    let path =
        path.unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let paths = workspace::ensure_workspace_for_dir(&path)?
        .with_context(|| format!("{} is not inside a git workspace", path.display()))?;
    let share_name = name.unwrap_or_else(|| default_share_name(&paths.root));

    confirm_share_current(&paths.root, &share_name)?;

    let share = match find_share_for_workspace(&paths.key) {
        Ok(existing) => {
            output::info(format!("workspace already shared as `{}`", existing.name));
            existing
        }
        Err(_) => add(Some(path), Some(share_name), redact)?,
    };
    if !share.enabled {
        set_enabled(&share.name, true)?;
    }
    invite(&share.name, expires)
}

fn confirm_share_current(root: &Path, share_name: &str) -> Result<()> {
    if !atty::is(atty::Stream::Stdin) || !atty::is(atty::Stream::Stderr) {
        bail!(
            "refusing to share non-interactively; re-run in a terminal, or use `sivtr share add` explicitly"
        );
    }

    output::info(format!(
        "share workspace `{}` as `{share_name}`?",
        root.display()
    ));
    output::hint("Press Enter to confirm, or type n/no to cancel");
    eprint!("> ");
    let _ = io::stderr().flush();

    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("Failed to read confirmation")?;
    let answer = line.trim();
    if answer.is_empty() || answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
        return Ok(());
    }
    if answer.eq_ignore_ascii_case("n") || answer.eq_ignore_ascii_case("no") {
        bail!("share cancelled");
    }
    bail!("share cancelled; expected Enter / y / n");
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
            output::info(format!(
                "single-use invitation for `{share_name}`; expires {}",
                expires_at.to_rfc3339()
            ));
            output::hint("on the other machine:");
            output::plain(format!("  sivtr remote add <alias> {ticket}"));
            output::plain(ticket);
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
