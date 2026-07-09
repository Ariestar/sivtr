//! `sivtr serve` — run the read-only HTTP server for remote peer access.
//!
//! Generates/loads the bearer token, resolves the workspace, chooses the bind
//! address (localhost by default; `--lan` opts into the network), then blocks
//! on the async server via a one-shot tokio runtime confined to this command.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{bail, Context, Result};

use crate::cli::ServeArgs;
use crate::serve::{self, ServeConfig};
use sivtr_core::workspace;

pub fn execute(args: &ServeArgs) -> Result<()> {
    let workspace = match &args.cwd {
        Some(p) => p.clone(),
        None => std::env::current_dir().context("Failed to resolve current directory")?,
    };

    // Only serve inside a real workspace (git root) so we never accidentally
    // expose records from an unrelated directory tree.
    if workspace::resolve_workspace_for_dir(&workspace)?.is_none() {
        bail!(
            "{} is not inside a git workspace; `sivtr serve` only exposes a workspace's sessions",
            workspace.display()
        );
    }

    let token = match &args.token {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            let generated = generate_token();
            // Printed to stderr so stdout stays clean for any piped consumer.
            eprintln!("sivtr: generated bearer token (save it; clients need it):");
            eprintln!("  {generated}");
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
        eprintln!("sivtr: WARNING — binding to all interfaces (LAN). Ensure the network is trusted and the token is kept secret.");
    }
    eprintln!("sivtr: serving {} on http://{}", workspace.display(), addr);
    eprintln!("sivtr: press Ctrl+C to stop");

    let cfg = ServeConfig {
        addr,
        token,
        workspace,
        redact: !args.no_redact,
    };

    // A dedicated runtime for the serve path only; every other CLI command
    // stays fully synchronous.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to start the server runtime")?;

    runtime.block_on(serve::serve(cfg))?;
    Ok(())
}

/// Generate a short, URL-safe bearer token via the OS RNG. Prefixed with `s-`
/// (the sivtr token namespace) so the built-in redactor masks it — a generated
/// token that leaks into captured output is itself redacted. Not cryptographic
/// strength, but sufficient for a LAN pairing token and readable for sharing.
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
