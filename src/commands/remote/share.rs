use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{TimeZone, Utc};
use sivtr_core::workspace;

use crate::cli::{ShareAction, ShareCommand};
use crate::output;
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse};

pub fn execute(command: ShareCommand) -> Result<()> {
    match command.action {
        ShareAction::Add {
            path,
            name,
            no_redact,
        } => add(path, name, !no_redact),
        ShareAction::List => list(),
        ShareAction::Remove { share } => remove(&share),
        ShareAction::Enable { share } => set_enabled(&share, true),
        ShareAction::Disable { share } => set_enabled(&share, false),
        ShareAction::Invite { share, expires } => invite(&share, &expires),
        ShareAction::Grants { share } => grants(&share),
        ShareAction::Revoke { share, peer } => revoke(&share, &peer),
    }
}

fn add(path: Option<PathBuf>, name: Option<String>, redact: bool) -> Result<()> {
    let path =
        path.unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let paths = workspace::ensure_workspace_for_dir(&path)?
        .with_context(|| format!("{} is not inside a git workspace", path.display()))?;
    let name = name.unwrap_or_else(|| {
        paths
            .root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("workspace")
            .to_string()
    });
    match ipc::call(LocalRequest::ShareAdd {
        workspace_key: paths.key,
        root: paths.root.display().to_string(),
        name,
        redact,
    })? {
        LocalResponse::Share(share) => {
            output::success(format!("shared workspace `{}`", share.name));
            output::detail("id", share.id);
            output::detail("root", share.root);
            output::detail(
                "redaction",
                if share.redact { "enabled" } else { "disabled" },
            );
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
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
