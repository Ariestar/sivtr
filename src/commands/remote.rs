//! `sivtr remote` — manage the `remotes.toml` registry of remote devices.

use anyhow::Result;

use crate::cli::{RemoteAction, RemoteCommand};
use crate::output;
use crate::remote::{RemoteClient, Remotes};

pub fn execute(cmd: RemoteCommand) -> Result<()> {
    match cmd.action {
        RemoteAction::List => list(),
        RemoteAction::Add {
            name,
            host,
            port,
            token,
        } => add(&name, host, port, token),
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

fn add(name: &str, host: String, port: u16, token: String) -> Result<()> {
    let mut remotes = Remotes::load()?;
    let remote = crate::remote::Remote {
        host,
        port,
        token,
        workspace: None,
    };
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
