use anyhow::{Context, Result};
use sivtr_core::config::SivtrConfig;
use std::io::Write;

use crate::cli::PtyProxyAction;
use crate::commands::terminal::init;
use crate::output;
use crate::pty::{Pending, PROXIED_ENV, PROXY_ENV};

pub fn execute(action: &PtyProxyAction) -> Result<()> {
    match action {
        PtyProxyAction::Run { command, args } => run(command, args),
        PtyProxyAction::Report {
            command_id,
            command,
            prompt,
            cwd,
            exit,
            echoed_input,
        } => report(command_id, command, prompt, cwd, *exit, *echoed_input),
        PtyProxyAction::Enable { shell } => enable(shell),
        PtyProxyAction::Disable => disable(),
    }
}

/// Run the shell inside the capture pty.
///
/// Any failure falls back to an ordinary shell, so a broken proxy can never
/// cost the user their terminal — just their capture.
fn run(program: &str, args: &[String]) -> Result<()> {
    if !SivtrConfig::load().unwrap_or_default().pty_proxy.enabled {
        // Capture is off: the rc block is either stale or deliberately kept.
        // Hand the terminal straight to the shell.
        return spawn_shell(program, args);
    }

    match crate::pty::run(program, args) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            output::warning(format!("pty-proxy unavailable: {error:#}"));
            output::hint("starting the shell without capture");
            spawn_shell(program, args)
        }
    }
}

/// Start the shell as a child and exit with its status.
///
/// Marks the child as running *inside* a proxy; without that the rc guard would
/// exec the proxy again and loop forever.
fn spawn_shell(program: &str, args: &[String]) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .env(PROXIED_ENV, "1")
        .env_remove(PROXY_ENV)
        .status()
        .with_context(|| format!("failed to start {program}"))?;
    std::process::exit(status.code().unwrap_or(1));
}

/// Hand one finished command to the proxy, then close its block.
///
/// The metadata must be on disk *before* the `D` marker reaches the pty: the
/// marker is the proxy's cue to read it, so writing first removes any race
/// between the two processes.
fn report(
    command_id: &str,
    command: &str,
    prompt: &str,
    cwd: &str,
    exit: i32,
    echoed_input: bool,
) -> Result<()> {
    let Some(terminal_id) = std::env::var("SIVTR_TERMINAL_ID")
        .ok()
        .filter(|id| !id.trim().is_empty())
    else {
        return Ok(());
    };

    let pending = Pending {
        command_id: non_empty(command_id),
        prompt: prompt.to_string(),
        command: command.to_string(),
        cwd: non_empty(cwd),
        echoed_input,
    };
    let path = crate::pty::pending_path(&terminal_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&path, serde_json::to_string(&pending)?)
        .with_context(|| format!("failed to write {}", path.display()))?;

    let mut stdout = std::io::stdout();
    write!(stdout, "\x1b]133;D;{exit}\x1b\\")?;
    stdout.flush()?;
    Ok(())
}

pub(crate) fn enable(shell: &str) -> Result<()> {
    let mut config = SivtrConfig::load().unwrap_or_default();
    config.pty_proxy.enabled = true;
    config.save()?;

    // The capture block lives in the same marked section as the rest of the
    // shell integration, so reusing `init` keeps one installer and one
    // upgrade path for both.
    init::execute(shell)
}

fn disable() -> Result<()> {
    let mut config = SivtrConfig::load().unwrap_or_default();
    config.pty_proxy.enabled = false;
    config.save()?;

    output::success("terminal capture disabled");
    output::hint("shell blocks left in place are inert; `sivtr init uninstall` removes them");
    Ok(())
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
