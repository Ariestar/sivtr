//! PTY proxy: owns the terminal, runs the shell inside a pty, and records one
//! session entry per command.
//!
//! The child gets a real pty, so `isatty` is true and interactive programs
//! behave exactly as they do without the proxy. The proxy only forwards bytes
//! and acts on the OSC 133 markers the shell integration emits.

pub mod stream;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sivtr_core::session::{self, SessionEntry, SessionState};
use sivtr_core::workspace;

use stream::{Event, Stream};

/// How often the outer terminal size is compared against the pty's.
///
/// Polling instead of SIGWINCH keeps one code path for Unix and Windows and
/// leaves the shell's terminal state alone.
/// ponytail: resize lags by up to 100 ms; wire up SIGWINCH if anyone notices.
const RESIZE_POLL: Duration = Duration::from_millis(100);

/// Set on the child whenever it runs under the proxy, so shell rc guards stop
/// re-execing. Capture itself is gated on [`PROXY_ENV`].
pub const PROXIED_ENV: &str = "SIVTR_PTY_PROXIED";

/// Set on the child only when capture is on; the shell integration keys off it,
/// so a shell without a proxy pays nothing per prompt.
pub const PROXY_ENV: &str = "SIVTR_PTY_PROXY";

/// Metadata the shell hands over immediately before the block's `OSC 133;D`.
///
/// Written by `sivtr pty-proxy report`, read by the proxy. `report` finishes the
/// write before it prints `D`, and `D` only reaches the proxy after that, so the
/// proxy never reads a partial file.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Pending {
    pub command_id: Option<String>,
    pub prompt: String,
    pub command: String,
    pub cwd: Option<String>,
}

/// Where `report` leaves metadata for `terminal_id`.
pub fn pending_path(terminal_id: &str) -> PathBuf {
    workspace::home_dir()
        .join("pty")
        .join(format!("{terminal_id}.pending"))
}

/// Run `program` inside a pty until it exits, returning its exit code.
pub fn run(program: &str, args: &[String]) -> Result<i32> {
    // Inherited by the child, so everything in the session agrees on one log.
    let id = format!("pty_{}", std::process::id());
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    // Take the terminal before spawning anything: failing here must not leave a
    // shell running in a pty that nobody drains.
    let restore = RawMode::enable().context("failed to put the terminal in raw mode")?;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("failed to open a pty")?;

    let mut command = CommandBuilder::new(program);
    command.args(args);
    // portable-pty falls back to `$HOME` when no cwd is given, which would drop
    // the shell into the wrong directory and resolve the wrong workspace.
    command.cwd(std::env::current_dir().context("failed to resolve the current directory")?);
    command.env(PROXIED_ENV, "1");
    command.env(PROXY_ENV, "1");
    command.env("SIVTR_TERMINAL_ID", &id);
    let mut child = pair
        .slave
        .spawn_command(command)
        .with_context(|| format!("failed to start {program} in a pty"))?;

    let mut reader = pair.master.try_clone_reader().context("pty reader")?;
    let writer = pair.master.take_writer().context("pty writer")?;
    // Closing our copy of the slave is what lets the reader see EOF once the
    // child exits; holding it open would hang the reader thread forever.
    drop(pair.slave);

    let stop = Arc::new(AtomicBool::new(false));
    let resizer = spawn_resizer(Arc::clone(&stop), pair.master);

    // Input and resize stay detached: both can block forever, and the process
    // exits once the child is reaped and the reader has drained.
    spawn_input(writer);
    let output = std::thread::spawn(move || pump(&mut reader, id));

    let status = child.wait().context("failed to wait for the shell")?;
    let exit_code = status.exit_code() as i32;

    let outcome = output.join().unwrap_or_else(|_| Ok(()));
    stop.store(true, Ordering::Relaxed);
    let _ = resizer.join();
    drop(restore);

    if let Err(error) = outcome {
        eprintln!("sivtr: pty-proxy capture stopped: {error:#}");
    }
    Ok(exit_code)
}

/// Forward everything the shell prints to the real terminal, recording blocks.
fn pump(reader: &mut Box<dyn Read + Send>, terminal_id: String) -> Result<()> {
    let mut stdout = std::io::stdout();
    let mut stream = Stream::default();
    let mut recorder = Recorder {
        terminal_id,
        ..Recorder::default()
    };
    let mut chunk = [0u8; 8192];

    loop {
        let read = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if is_hangup(&error) => break,
            Err(error) => return Err(error).context("failed to read from the pty"),
        };

        stdout.write_all(&chunk[..read])?;
        stdout.flush()?;

        for event in stream.push(&chunk[..read]) {
            match event {
                Event::CommandStart => recorder.started = Some(Instant::now()),
                Event::CommandEnd(exit_code) => {
                    let output = stream.take_output();
                    recorder.record(&output, exit_code);
                }
            }
        }
    }
    Ok(())
}

/// Feed the real terminal's input to the pty.
///
/// Ctrl-C and Ctrl-Z travel as ordinary bytes, and the pty's line discipline
/// turns them into signals for the child — no signal forwarding needed.
fn spawn_input(mut writer: Box<dyn Write + Send>) {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut chunk = [0u8; 8192];
        while let Ok(read) = stdin.read(&mut chunk) {
            if read == 0 || writer.write_all(&chunk[..read]).is_err() {
                break;
            }
            let _ = writer.flush();
        }
        let _ = writer.flush();
    });
}

/// Keep the pty sized to the real terminal.
fn spawn_resizer(
    stop: Arc<AtomicBool>,
    master: Box<dyn MasterPty + Send>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut current = crossterm::terminal::size().unwrap_or((80, 24));
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(RESIZE_POLL);
            let Ok(size) = crossterm::terminal::size() else {
                continue;
            };
            if size == current {
                continue;
            }
            current = size;
            let _ = master.resize(PtySize {
                rows: size.1,
                cols: size.0,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    })
}

/// Turns finished blocks into session entries.
#[derive(Default)]
struct Recorder {
    terminal_id: String,
    state: SessionState,
    started: Option<Instant>,
    /// Capture failures must not take the terminal down, but they must not be
    /// swallowed either — surface the first one and point at `sivtr doctor`.
    reported: bool,
}

impl Recorder {
    fn record(&mut self, output: &[u8], exit_code: Option<i32>) {
        let duration_ms = self
            .started
            .take()
            .map(|started| started.elapsed().as_millis() as u64);

        let path = pending_path(&self.terminal_id);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let _ = std::fs::remove_file(&path);

        let metadata: Pending = match serde_json::from_str(&text) {
            Ok(metadata) => metadata,
            Err(error) => return self.report(error),
        };
        if !should_record(
            &self.state,
            metadata.command_id.as_deref(),
            &metadata.command,
        ) {
            return;
        }

        let output = strip_command_echo(output, &metadata.command);
        let output = trim_trailing_prompt_artifact(
            String::from_utf8_lossy(output).into_owned(),
            &metadata.prompt,
        );
        match self.append(&output, exit_code, duration_ms, &metadata) {
            Ok(()) => {
                self.state.last_command_id = metadata.command_id;
                self.state.last_command = Some(metadata.command);
            }
            Err(error) => self.report(error),
        }
    }

    fn append(
        &self,
        output: &str,
        exit_code: Option<i32>,
        duration_ms: Option<u64>,
        metadata: &Pending,
    ) -> Result<()> {
        let cwd = metadata.cwd.as_deref().map(Path::new);
        let Some(path) = workspace::terminal_log_path_for(cwd, &self.terminal_id)? else {
            return Ok(());
        };
        let entry = SessionEntry::new(metadata.prompt.clone(), metadata.command.clone(), output)
            .with_metadata(
                metadata.cwd.clone(),
                Some(now_timestamp()),
                duration_ms,
                exit_code,
            );
        session::append_entry(&path, &entry)?;
        session::save_state(&path.with_extension("state"), &self.state)
    }

    fn report(&mut self, error: impl std::fmt::Display) {
        if self.reported {
            return;
        }
        self.reported = true;
        eprintln!("sivtr: pty-proxy could not record a command: {error}");
    }
}

/// Same rule the old flush path used: a command id already recorded came from
/// the shell re-running its prompt hook, not from new work.
fn should_record(state: &SessionState, command_id: Option<&str>, command: &str) -> bool {
    if command.trim().is_empty() {
        return false;
    }
    match command_id {
        Some(command_id) => state.last_command_id.as_deref() != Some(command_id),
        None => state.last_command.as_deref() != Some(command),
    }
}

/// PowerShell has no pre-exec hook, so it emits `C` at the end of the prompt and
/// its block opens with the echo of the typed line. Drop that first line — but
/// only when it really is the echo, so shells that emit `C` right before running
/// the command (and therefore have clean blocks) never match.
fn strip_command_echo<'a>(output: &'a [u8], command: &str) -> &'a [u8] {
    let command = command.trim();
    if command.is_empty() {
        return output;
    }
    let window = &output[..output.len().min(4096)];
    let Some(newline) = window.iter().position(|byte| *byte == b'\n') else {
        return output;
    };
    let first_line = String::from_utf8_lossy(&window[..newline]);
    if plain_line(&first_line).trim_end().ends_with(command) {
        return &output[newline + 1..];
    }
    output
}

/// `line` with CSI/OSC escape sequences removed, so a syntax-highlighted echo
/// can be compared against the plain command text.
fn plain_line(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            plain.push(ch);
            continue;
        }
        match chars.next() {
            // CSI: parameters and intermediates, then a final byte 0x40..=0x7e.
            Some('[') => {
                chars.by_ref().any(|next| ('\x40'..='\x7e').contains(&next));
            }
            // OSC: everything up to BEL or ST.
            Some(']') => {
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if next == '\x07' || (previous == '\x1b' && next == '\\') {
                        break;
                    }
                    previous = next;
                }
            }
            _ => {}
        }
    }
    plain
}

/// zsh prints `PROMPT_EOL_MARK` *before* it runs its `precmd` hook, so that mark
/// lands inside the block. Drop a trailing line that is really the prompt: an
/// empty line, or one matching the tail of the prompt we recorded.
fn trim_trailing_prompt_artifact(output: String, prompt: &str) -> String {
    let prompt_last_line = SessionEntry::new(prompt, "", "")
        .prompt
        .lines()
        .last()
        .unwrap_or_default()
        .trim_end()
        .to_string();
    if prompt_last_line.is_empty() {
        return output;
    }

    let mut lines: Vec<&str> = output.lines().collect();
    let Some(last_raw_line) = lines.last().copied() else {
        return output;
    };
    let last_plain_line = SessionEntry::new("", "", last_raw_line)
        .output
        .trim()
        .to_string();
    if last_plain_line.is_empty() || prompt_last_line.ends_with(&last_plain_line) {
        lines.pop();
        return lines.join("\n");
    }
    output
}

/// A pty reports "the child hung up" as `EIO` on Unix and a broken pipe on
/// Windows; neither is a capture failure.
fn is_hangup(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
    ) || error.raw_os_error() == Some(5) // EIO
}

fn now_timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Puts the terminal in raw mode and restores it on drop, panics included.
struct RawMode;

impl RawMode {
    fn enable() -> Result<Self> {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!("stdin is not a terminal");
        }
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        now_timestamp, pending_path, plain_line, should_record, strip_command_echo,
        trim_trailing_prompt_artifact, Pending,
    };
    use sivtr_core::session::SessionState;

    #[test]
    fn skips_empty_commands_and_repeats() {
        let state = SessionState::default();
        assert!(!should_record(&state, None, ""));
        assert!(!should_record(&state, Some("7"), "   "));
        assert!(should_record(&state, Some("7"), "ls"));

        let repeated = SessionState {
            last_command_id: Some("7".into()),
            ..SessionState::default()
        };
        assert!(!should_record(&repeated, Some("7"), "ls"));
        assert!(should_record(&repeated, Some("8"), "ls"));

        // No command id (shells without a history number): compare the text.
        let text_only = SessionState {
            last_command: Some("ls".into()),
            ..SessionState::default()
        };
        assert!(!should_record(&text_only, None, "ls"));
        assert!(should_record(&text_only, None, "pwd"));
    }

    #[test]
    fn pending_round_trips_through_json() {
        let pending = Pending {
            command_id: Some("12".into()),
            prompt: "repo on main\n❯  ".into(),
            command: "echo '中文' && true".into(),
            cwd: Some("/home/user/repo".into()),
        };
        let text = serde_json::to_string(&pending).expect("encode");
        let decoded: Pending = serde_json::from_str(&text).expect("decode");
        assert_eq!(decoded.command, "echo '中文' && true");
        assert_eq!(decoded.cwd.as_deref(), Some("/home/user/repo"));
    }

    #[test]
    fn pending_path_is_scoped_per_terminal() {
        let path = pending_path("pty_123");
        assert!(path.to_string_lossy().ends_with("pty_123.pending"));
    }

    #[test]
    fn timestamps_are_rfc3339() {
        let stamp = now_timestamp();
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert!(stamp.contains('T'), "{stamp}");
    }

    #[test]
    fn strips_the_echoed_command_line() {
        let block = b"echo hi\r\nhi\r\n";
        assert_eq!(strip_command_echo(block, "echo hi"), b"hi\r\n");
    }

    #[test]
    fn strips_the_zsh_prompt_end_mark() {
        // zsh writes PROMPT_EOL_MARK before precmd, so the block ends with the
        // prompt's last character (ANSI-wrapped) plus the padding it clears.
        let prompt = "MS-Challenger-B760M-F-WIFI% ";
        let output = "from-zsh\n\u{1b}[1m\u{1b}[7m%\u{1b}[27m\u{1b}[1m\u{1b}[0m \r \r";
        assert_eq!(
            trim_trailing_prompt_artifact(output.to_string(), prompt),
            "from-zsh"
        );
    }

    #[test]
    fn keeps_a_trailing_line_that_is_real_output() {
        let prompt = "repo% ";
        let output = "one\ntwo";
        assert_eq!(
            trim_trailing_prompt_artifact(output.to_string(), prompt),
            "one\ntwo"
        );
    }

    #[test]
    fn keeps_everything_when_no_prompt_was_recorded() {
        let output = "one\ntwo\n".to_string();
        assert_eq!(trim_trailing_prompt_artifact(output.clone(), ""), output);
    }

    #[test]
    fn plain_line_drops_escapes_and_keeps_text() {
        assert_eq!(plain_line("\x1b[32mecho\x1b[0m hi"), "echo hi");
        assert_eq!(plain_line("\x1b]0;title\x07text"), "text");
        assert_eq!(plain_line("plain"), "plain");
        // A truncated escape must not swallow the rest of the line.
        assert_eq!(plain_line("kept\x1b[32"), "kept");
    }

    #[test]
    fn strips_echo_with_colour_around_it() {
        let block = b"\x1b[32mecho\x1b[0m hi\r\nhi\r\n";
        assert_eq!(strip_command_echo(block, "echo hi"), b"hi\r\n");
    }

    #[test]
    fn keeps_clean_blocks_that_only_look_similar() {
        // The command appears in the output, but not on the first line: this is
        // a normal block from a shell with a real pre-exec hook.
        let block = b"first line\necho hi\n";
        assert_eq!(strip_command_echo(block, "echo hi"), block);
    }

    #[test]
    fn keeps_output_when_the_command_never_appears() {
        let block = b"just output\n";
        assert_eq!(strip_command_echo(block, "ls -la"), block);
    }

    #[test]
    fn ignores_an_empty_command() {
        let block = b"output\n";
        assert_eq!(strip_command_echo(block, "  "), block);
    }
}
