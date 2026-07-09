//! `sivtr remote` — manage the `remotes.toml` registry of remote devices.

use std::io::{self, BufRead, Write};

use anyhow::{bail, Context, Result};

use crate::cli::{RemoteAction, RemoteCommand};
use crate::output;
use crate::remote::{RemoteClient, Remotes};

pub fn execute(cmd: RemoteCommand) -> Result<()> {
    match cmd.action {
        RemoteAction::List => list(),
        RemoteAction::Add { target, token } => add(&target, token),
        RemoteAction::Remove { name } => remove(&name),
        RemoteAction::Test { name } => test(&name),
    }
}

fn list() -> Result<()> {
    let remotes = Remotes::load()?;
    if remotes.remotes.is_empty() {
        output::plain("no remotes configured; add one with `sivtr remote add`");
        return Ok(());
    }
    for (alias, remote) in &remotes.remotes {
        output::detail(alias, format!("{} (port {})", remote.host, remote.port));
    }
    Ok(())
}

/// Parse an SSH-style target `<alias>@<host>[:<port>]`.
fn parse_target(target: &str) -> Result<(String, String, u16)> {
    let (alias, host_port) = target
        .split_once('@')
        .with_context(|| format!("expected `<alias>@<host>[:<port>]`, got `{target}`"))?;
    let alias = alias.trim().to_ascii_lowercase();
    if alias.is_empty() {
        bail!("empty alias in `{target}`");
    }
    if !alias
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("alias `{alias}` must be [a-z0-9_-]+");
    }

    // Split host[:port] on the last ':' (so IPv6 hosts in brackets still work).
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().context("invalid port")?),
        None => (host_port.to_string(), 7421),
    };
    if host.is_empty() {
        bail!("empty host in `{target}`");
    }
    Ok((alias, host, port))
}

/// Resolve the bearer token: use `--token` if given, otherwise prompt on a tty.
/// Prompting keeps the token out of shell history and `ps` output.
fn resolve_token(flag: Option<String>) -> Result<String> {
    if let Some(token) = flag {
        let token = token.trim().to_string();
        if token.is_empty() {
            bail!("--token must not be empty");
        }
        return Ok(token);
    }

    if !atty::is(atty::Stream::Stdin) {
        bail!(
            "no token provided and stdin is not interactive; pass --token for non-interactive use"
        );
    }
    eprint!("token: ");
    io::stderr().flush().ok();
    let stdin = io::stdin();
    let mut line = String::new();
    stdin
        .lock()
        .read_line(&mut line)
        .context("failed to read token")?;
    let token = line.trim().to_string();
    if token.is_empty() {
        bail!("no token entered");
    }
    Ok(token)
}

fn add(target: &str, token: Option<String>) -> Result<()> {
    let (name, host, port) = parse_target(target)?;
    let token = resolve_token(token)?;
    let mut remotes = Remotes::load()?;
    let remote = crate::remote::Remote {
        host,
        port,
        token,
        workspace: None,
    };
    let existed = remotes.remotes.insert(name.clone(), remote).is_some();
    remotes.save()?;
    output::success(if existed {
        format!("updated remote `{name}`")
    } else {
        format!("added remote `{name}`")
    });
    Ok(())
}

fn remove(name: &str) -> Result<()> {
    let mut remotes = Remotes::load()?;
    if remotes.remotes.remove(name).is_none() {
        output::warning(format!("no remote named `{name}`"));
        return Ok(());
    }
    remotes.save()?;
    output::success(format!("removed remote `{name}`"));
    Ok(())
}

fn test(name: &str) -> Result<()> {
    let remote = crate::remote::lookup(name)?;
    let client = RemoteClient::new(name, remote);
    match client.ping() {
        Ok(agent_name) => output::success(format!("remote `{name}` reachable ({agent_name})")),
        Err(e) => output::error(format!("remote `{name}` unreachable: {e:#}")),
    }
    Ok(())
}
