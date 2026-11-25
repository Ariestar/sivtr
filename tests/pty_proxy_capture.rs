//! End-to-end check that the pty proxy records commands from a real shell.
//!
//! Each test hosts a shell inside its own pty, so the proxy sees a terminal
//! exactly as it would in daily use. Everything is sandboxed — `HOME` and
//! `SIVTR_HOME` both point inside a temp dir — so the developer's dotfiles and
//! real archive are never touched.

#![cfg(unix)]

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use sivtr_core::session::SessionEntry;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Generous: the shell has to start, print a prompt, run a command and report.
const DEADLINE: Duration = Duration::from_secs(30);

fn sivtr() -> &'static str {
    env!("CARGO_BIN_EXE_sivtr")
}

/// The directory holding the test binary, so the shell hook can find `sivtr`.
fn binary_dir() -> PathBuf {
    Path::new(sivtr())
        .parent()
        .expect("binary dir")
        .to_path_buf()
}

/// A path list that resolves `sivtr`, keeping the rest of the environment usable.
fn shell_path() -> String {
    format!(
        "{}:{}",
        binary_dir().display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// Each test needs its own tree: they share a process (and a pid) but run on
    /// separate threads, so a shared root would have them deleting each other's
    /// sandbox and rewriting each other's config.
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sivtr-pty-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["home", "sivtr", "repo/.git"] {
            std::fs::create_dir_all(root.join(dir)).expect("sandbox dir");
        }
        Self { root }
    }

    /// The sandbox `$HOME`, so the shell reads a private profile.
    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// `$SIVTR_HOME`: config, workspaces and the archive all live under it.
    fn sivtr_home(&self) -> PathBuf {
        self.root.join("sivtr")
    }

    /// A git root, so workspace resolution has somewhere to write.
    fn repo(&self) -> PathBuf {
        self.root.join("repo")
    }

    /// Install the shell block with default capture, as `sivtr setup` would.
    fn install_shell_integration(&self) {
        let status = Command::new(sivtr())
            .args(["init", "bash"])
            .env("HOME", self.home())
            .env("SIVTR_HOME", self.sivtr_home())
            .status()
            .expect("run init bash");
        assert!(status.success(), "`init bash` failed");
    }

    /// Environment every shell in the sandbox must run with.
    fn builder(&self, program: &str) -> CommandBuilder {
        let mut command = CommandBuilder::new(program);
        command.cwd(self.repo());
        command.env("HOME", self.home());
        command.env("SIVTR_HOME", self.sivtr_home());
        command.env("TERM", "xterm-256color");
        command.env("PATH", shell_path());
        command
    }

    /// Session logs written so far, across every workspace.
    fn logs(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(workspaces) = std::fs::read_dir(self.sivtr_home().join("workspaces")) else {
            return found;
        };
        for workspace in workspaces.flatten() {
            let Ok(files) = std::fs::read_dir(workspace.path().join("terminals")) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                    found.push(path);
                }
            }
        }
        found
    }

    /// Poll for a recorded command; the proxy writes it from another process.
    /// The budget lets a caller assert the opposite — that nothing arrives.
    fn recorded(&self, command: &str, budget: Duration) -> Option<SessionEntry> {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            if let Some(entry) = find_entry(&self.logs(), command) {
                return Some(entry);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Parse every complete line and return the entry for `command`.
///
/// Lines still being appended by the proxy are skipped rather than failing the
/// read.
fn find_entry(logs: &[PathBuf], command: &str) -> Option<SessionEntry> {
    for path in logs {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let Ok(entry) = serde_json::from_str::<SessionEntry>(line) else {
                continue;
            };
            if entry.command == command {
                return Some(entry);
            }
        }
    }
    None
}

/// Run a shell to completion inside an outer pty and return what it printed.
fn drive(command: CommandBuilder, input: &str) -> String {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open outer pty");

    let mut child = pair.slave.spawn_command(command).expect("spawn shell");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");

    // The pty buffer is small: drain it or the shell blocks on its own output.
    let drain = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => out.extend_from_slice(&buffer[..read]),
            }
        }
        out
    });

    writer.write_all(input.as_bytes()).expect("send input");
    writer.flush().expect("flush");
    drop(writer);

    // Poll rather than hand the only child handle to a wait thread: on a
    // timeout the shell must still be killable, or it keeps the pty open and
    // the drain reader below never returns.
    let deadline = Instant::now() + DEADLINE;
    let mut reaped = false;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => {
                reaped = true;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            // Polling itself failed: the child's state is unknown, so fall
            // through to the kill below rather than assuming it exited.
            Err(_) => break,
        }
    }
    if !reaped {
        let _ = child.kill();
        let _ = child.wait();
    }

    let output = String::from_utf8_lossy(&drain.join().unwrap_or_default()).into_owned();
    assert!(reaped, "the shell did not exit within {DEADLINE:?}");
    output
}

#[test]
fn records_a_command_run_in_a_proxied_shell() {
    let sandbox = Sandbox::new("direct");
    sandbox.install_shell_integration();

    let mut command = sandbox.builder(sivtr());
    command.args(["pty-proxy", "run", "bash"]);
    drive(command, "echo hello\nexit\n");

    let entry = sandbox
        .recorded("echo hello", DEADLINE)
        .expect("the proxy must record `echo hello`");
    assert_eq!(entry.output, "hello");
    assert_eq!(entry.exit_code, Some(0));
    let cwd = entry.cwd.as_deref().unwrap_or_default();
    assert!(
        cwd.ends_with("repo"),
        "the entry should carry the directory the command ran in, got {cwd:?}"
    );

    // The block must be exactly this command's output: no prompt, no echo, and
    // no leftover marker bytes.
    assert!(!entry.output.contains("echo hello"), "{:?}", entry.output);
    assert!(!entry.output.contains('\x1b'), "{:?}", entry.output);
}

#[test]
fn records_a_failing_command() {
    let sandbox = Sandbox::new("failing");
    sandbox.install_shell_integration();

    let mut command = sandbox.builder(sivtr());
    command.args(["pty-proxy", "run", "bash"]);
    drive(command, "printf 'boom\\n'; false\nexit\n");

    let entry = sandbox
        .recorded("printf 'boom\\n'; false", DEADLINE)
        .expect("the proxy must record the failing command");
    assert_eq!(entry.output, "boom");
    assert_eq!(entry.exit_code, Some(1));
}

/// The user-facing path: nothing runs the proxy explicitly here. A plain
/// interactive shell reads its profile, hits the re-exec guard, and ends up
/// captured — which is how every real session starts.
#[test]
fn the_profile_block_proxies_the_shell_itself() {
    let sandbox = Sandbox::new("block");
    sandbox.install_shell_integration();

    let command = sandbox.builder("bash");
    drive(command, "echo through-the-block\nexit\n");

    let entry = sandbox
        .recorded("echo through-the-block", DEADLINE)
        .expect("the profile block must start the proxy");
    assert_eq!(entry.output, "through-the-block");
}

/// With capture off the block stays inert: the shell still runs, nothing is
/// recorded, and the re-exec must not loop.
#[test]
fn disabling_capture_leaves_the_shell_usable() {
    let sandbox = Sandbox::new("disabled");
    sandbox.install_shell_integration();

    let config = "[pty_proxy]\nenabled = false\n";
    std::fs::write(sandbox.sivtr_home().join("config.toml"), config).unwrap();
    // Reinstalling/upgrading the hook must not reset an explicit opt-out.
    sandbox.install_shell_integration();
    assert_eq!(
        std::fs::read_to_string(sandbox.sivtr_home().join("config.toml")).unwrap(),
        config
    );

    let command = sandbox.builder("bash");
    let output = drive(command, "echo still-alive\nexit\n");

    assert!(output.contains("still-alive"), "the shell must still run");
    // Nothing can ever be written here, so a short sanity wait is enough.
    assert!(
        sandbox
            .recorded("echo still-alive", Duration::from_secs(2))
            .is_none(),
        "capture is off, so nothing may be recorded"
    );
}

/// A pre-exec handler installed before the block must keep running: bash-preexec
/// and the tools built on it install a `DEBUG` trap, so ours has to chain to
/// theirs rather than replace it.
#[test]
fn keeps_an_existing_debug_trap_running() {
    let sandbox = Sandbox::new("debug-trap");
    sandbox.install_shell_integration();

    let profile = sandbox.home().join(".bashrc");
    let installed = std::fs::read_to_string(&profile).expect("read profile");
    let marker = sandbox.root.join("user-debug-ran");
    std::fs::write(
        &profile,
        format!("trap 'printf x >> {}' DEBUG\n{installed}", marker.display()),
    )
    .expect("prepend the user's trap");

    let command = sandbox.builder("bash");
    drive(command, "echo chained\nexit\n");

    let entry = sandbox
        .recorded("echo chained", DEADLINE)
        .expect("capture must work with a user DEBUG trap installed");
    assert_eq!(entry.output, "chained");
    assert!(
        marker.exists(),
        "the user's own DEBUG handler must still run"
    );
}
