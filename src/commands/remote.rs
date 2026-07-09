//! `sivtr remote` — manage the `remotes.toml` registry of remote devices.

use std::io::{self, BufRead, Write};

use anyhow::{bail, Context, Result};

use crate::cli::{RemoteAction, RemoteCommand};
use crate::output;
use crate::remote::{RemoteClient, Remotes};

pub fn execute(cmd: RemoteCommand) -> Result<()> {
    match cmd.action {
        RemoteAction::List => list(),
        RemoteAction::Add { name, addr, token } => add(&name, &addr, token),
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
        output::detail(alias, format!("[{}] {}", remote.kind(), remote.describe()));
    }
    Ok(())
}

/// Parse `host[:port]` (no alias). Port defaults to 7421.
fn parse_host_port(addr: &str) -> Result<(String, u16)> {
    let (host, port) = match addr.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().context("invalid port")?),
        None => (addr.to_string(), 7421),
    };
    if host.is_empty() {
        bail!("empty host in `{addr}`");
    }
    Ok((host, port))
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

fn add(name: &str, addr: &str, token: Option<String>) -> Result<()> {
    let remote = if crate::serve::iroh::addr_from_ticket(addr).is_ok() {
        // iroh ticket — no token needed.
        crate::remote::Remote::Iroh {
            ticket: addr.to_string(),
        }
    } else {
        let (host, port) = parse_host_port(addr)?;
        let token = resolve_token(token)?;
        crate::remote::Remote::Tcp { host, port, token }
    };
    let mut remotes = Remotes::load()?;
    let existed = remotes.remotes.insert(name.to_string(), remote).is_some();
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
