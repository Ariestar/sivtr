//! `sivtr serve` — expose a workspace's sessions read-only so another device
//! can read it via a `desk://...` ref.
//!
//! Picks the workspace to expose (`-w <key>`, or an interactive numbered picker,
//! or the current workspace), resolves the bind address (localhost default;
//! `--lan` for the network), then blocks on the async server via a one-shot
//! tokio runtime confined to this command.

use std::io::{self, BufRead, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::cli::ServeArgs;
use crate::output;
use crate::serve::{self as serve_backend, ServeConfig};
use sivtr_core::workspace::{self, WorkspaceMetadata};

pub fn execute(args: &ServeArgs) -> Result<()> {
    let workspace = resolve_workspace(args.workspace.as_deref())?;

    // A dedicated runtime for the server path (both iroh and axum are async);
    // every other CLI command stays fully synchronous.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to start the server runtime")?;

    // iroh is the default: zero-config, encrypted, cross-network. `--tcp` opts
    // into the plain HTTP server (localhost, or --lan for all interfaces).
    if !args.tcp {
        runtime.block_on(crate::serve::iroh::serve_iroh(workspace, !args.no_redact))?;
        return Ok(());
    }

    let token = match &args.token {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            let generated = generate_token();
            output::info("generated bearer token (share it with the remote client)");
            output::plain(format!("  {generated}"));
            generated
        }
    };

    let ip = if args.lan {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED) // 0.0.0.0
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    let addr = SocketAddr::new(ip, args.port);

    if args.lan {
        output::warning(
            "binding to all interfaces (LAN); ensure the network is trusted and the token is kept secret",
        );
    }
    output::info(format!(
        "serving {} on http://{}",
        workspace.display(),
        addr
    ));
    output::plain("press Ctrl+C to stop");

    let cfg = ServeConfig {
        addr,
        token,
        workspace,
        redact: !args.no_redact,
    };

    runtime.block_on(serve_backend::serve(cfg))?;
    Ok(())
}

/// Decide which workspace root to expose.
///
/// Order: `-w <key>` selects from the registry; otherwise the interactive
/// picker lists known workspaces (when stdin is a tty); otherwise the current
/// directory's workspace. Every path here is a registered or git-rooted
/// workspace, so we never accidentally expose an unrelated tree.
fn resolve_workspace(flag: Option<&str>) -> Result<PathBuf> {
    let all = workspace::list_workspaces()?;

    if let Some(key) = flag {
        let meta = all
            .iter()
            .find(|m| m.key == key)
            .with_context(|| format!("no workspace with key `{key}`"))?;
        return Ok(PathBuf::from(&meta.root));
    }

    if all.is_empty() {
        return current_workspace_root();
    }

    if atty::is(atty::Stream::Stdin) {
        if let Some(root) = pick_interactive(&all)? {
            return Ok(root);
        }
    }

    current_workspace_root()
}

/// The current directory's git workspace root, or an error if not in one.
fn current_workspace_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    match workspace::resolve_workspace_for_dir(&cwd)? {
        Some(paths) => Ok(paths.root),
        None => bail!(
            "{} is not inside a git workspace; run `sivtr serve` from a repo or pass -w <key>",
            cwd.display()
        ),
    }
}

/// Numbered picker over known workspaces. Returns the chosen root, or None if
/// the user just hits enter (caller falls back to the current workspace).
fn pick_interactive(all: &[WorkspaceMetadata]) -> Result<Option<PathBuf>> {
    let cwd = std::env::current_dir().ok();
    let cwd_root = cwd
        .as_ref()
        .and_then(|c| workspace::resolve_workspace_for_dir(c).ok().flatten())
        .map(|p| p.root);

    output::plain("Select a workspace to serve:");
    for (i, meta) in all.iter().enumerate() {
        let marker = match &cwd_root {
            Some(root) if root == &PathBuf::from(&meta.root) => " (current)",
            _ => "",
        };
        output::plain(format!("  {:>2}. {}{}", i + 1, meta.root, marker));
    }
    eprint!("Enter number (blank for current): ");
    io::stderr().flush().ok();

    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .context("failed to read selection")?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let index: usize = trimmed
        .parse()
        .with_context(|| format!("enter a number 1-{}", all.len()))?;
    if index == 0 || index > all.len() {
        bail!("number out of range (1-{})", all.len());
    }
    Ok(Some(PathBuf::from(&all[index - 1].root)))
}

/// Generate a short, URL-safe bearer token via the OS RNG. Prefixed with `s-`
/// (the sivtr token namespace) so the built-in redactor masks it — a generated
/// token that leaks into captured output is itself redacted.
fn generate_token() -> String {
    let mut bytes = [0u8; 20];
    getrandom::getrandom(&mut bytes).expect("OS RNG unavailable; pass --token explicitly");
    format!("s-{}", hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
