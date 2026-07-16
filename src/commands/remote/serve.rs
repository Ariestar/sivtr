use std::fs::OpenOptions;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::cli::{ServeAction, ServeCommand};
use crate::output;
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse};

pub fn execute(command: &ServeCommand) -> Result<()> {
    match command.action {
        ServeAction::Start => start(true),
        ServeAction::Stop => stop(),
        ServeAction::Restart => {
            if ipc::running() {
                stop()?;
            }
            start(true)
        }
        ServeAction::Status => status(),
        ServeAction::Logs => {
            output::plain(ipc::daemon_log_path().display().to_string());
            Ok(())
        }
        ServeAction::Foreground => crate::remote::daemon::run(),
    }
}

/// Start the daemon if needed. Used by share/remote so users don't run `serve start` first.
pub fn ensure_running() -> Result<()> {
    start(false)
}

fn start(verbose: bool) -> Result<()> {
    if ipc::running() {
        if verbose {
            output::success("sivtr daemon is already running");
            return status();
        }
        return Ok(());
    }
    crate::remote::daemon::remove_stale_daemon_info()?;
    let log_path = ipc::daemon_log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("Failed to open {}", log_path.display()))?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("serve-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    detach(&mut command);
    command.spawn().context("Failed to start sivtr daemon")?;

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if ipc::running() {
            if verbose {
                output::success("sivtr daemon started");
                return status();
            }
            output::info("started sivtr daemon");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!(
        "sivtr daemon did not become ready; inspect {}",
        log_path.display()
    )
}

fn stop() -> Result<()> {
    if !ipc::running() {
        crate::remote::daemon::remove_stale_daemon_info()?;
        output::warning("sivtr daemon is not running");
        return Ok(());
    }
    match ipc::call(LocalRequest::Shutdown)? {
        LocalResponse::Ok => {}
        response => bail!("Unexpected daemon response: {response:?}"),
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !ipc::running() {
            output::success("sivtr daemon stopped");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("sivtr daemon did not stop cleanly")
}

fn status() -> Result<()> {
    if !ipc::running() {
        output::plain("stopped");
        return Ok(());
    }
    match ipc::call(LocalRequest::Status)? {
        LocalResponse::Status(status) => {
            output::plain("running");
            output::detail("device", status.device_name);
            output::detail("node", status.node_id);
            output::detail("started", status.started_at);
            output::detail("mode", "default (n0 discovery + relay)");
            let relays: Vec<String> = status
                .endpoint
                .relay_urls()
                .map(|url| url.to_string())
                .collect();
            let direct = status.endpoint.ip_addrs().count();
            if relays.is_empty() {
                output::detail("relay", "none yet");
            } else {
                output::detail("relay", relays.join(", "));
            }
            output::detail("direct", format!("{direct} address(es)"));
            output::detail("shares", status.shares.to_string());
            output::detail("peers", status.peers.to_string());
            Ok(())
        }
        response => bail!("Unexpected daemon response: {response:?}"),
    }
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}
