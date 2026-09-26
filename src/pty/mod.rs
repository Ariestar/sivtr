//! PTY proxy: owns the terminal, runs the shell inside a pty, and records one
//! session entry per command.
//!
//! Both sides speak VT. Unix uses a kernel pty. Windows opens a pseudoconsole
//! with no extra flags, so it is the same kind of byte pipe. The proxy copies
//! those bytes and slices commands on the OSC 133 markers the shell emits.

pub mod stream;

#[cfg(windows)]
mod conpty;

use anyhow::{Context, Result};
#[cfg(unix)]
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
/// proxy never reads a partial file — and a fresh session id per run means it
/// never reads a stale one either.
#[derive(Debug, Serialize, Deserialize)]
pub struct Pending {
    pub command_id: Option<String>,
    pub prompt: String,
    pub command: String,
    pub cwd: Option<String>,
    /// The shell's block opens with the echoed input, because it can only emit
    /// `C` at the end of its prompt. Declared by PowerShell; the proxy then drops
    /// exactly that echo — continuation lines included — instead of guessing
    /// where the output starts.
    pub echoed_input: bool,
}

/// Where `report` leaves metadata for `terminal_id`.
pub fn pending_path(terminal_id: &str) -> PathBuf {
    workspace::home_dir()
        .join("pty")
        .join(format!("{terminal_id}.pending"))
}

struct Opened {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    resize: Box<dyn FnMut(u16, u16) + Send>,
    wait: Box<dyn FnOnce() -> Result<i32> + Send>,
    /// Last owner of the Windows pseudoconsole. Dropped after the child has
    /// exited so the output pipe reaches EOF. Unix closes the pty by dropping
    /// the master inside `resize`.
    close: Option<Box<dyn Send>>,
}

/// Run `program` inside a pty until it exits, returning its exit code.
pub fn run(program: &str, args: &[String]) -> Result<i32> {
    // Inherited by the child, so everything in the session agrees on one log.
    let id = workspace::new_terminal_id();
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    // Take the terminal before spawning anything: failing here must not leave a
    // shell running in a pty that nobody drains.
    let restore = RawMode::enable().context("failed to put the terminal in raw mode")?;
    let opened = open_pty(program, args, cols, rows, &id)?;
    let writer = Arc::new(Mutex::new(opened.writer));
    let stop = Arc::new(AtomicBool::new(false));
    let resizer = spawn_resizer(Arc::clone(&stop), opened.resize);
    let output = std::thread::spawn(move || pump(opened.reader, id));
    spawn_input(writer);

    let exit_code = (opened.wait)();
    // Stop resizing, then close the pty, then drain the last output. Closing
    // is what makes the reader see EOF.
    stop.store(true, Ordering::Relaxed);
    let _ = resizer.join();
    drop(opened.close);
    let outcome = output.join().unwrap_or_else(|_| Ok(()));
    let exit_code = exit_code.context("failed to wait for the shell")?;
    drop(restore);

    if let Err(error) = outcome {
        eprintln!("sivtr: pty-proxy capture stopped: {error:#}");
    }
    Ok(exit_code)
}

#[cfg(unix)]
fn open_pty(program: &str, args: &[String], cols: u16, rows: u16, id: &str) -> Result<Opened> {
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
    command.env("SIVTR_TERMINAL_ID", id);
    let mut child = pair
        .slave
        .spawn_command(command)
        .with_context(|| format!("failed to start {program} in a pty"))?;
    let reader = pair.master.try_clone_reader().context("pty reader")?;
    let writer = pair.master.take_writer().context("pty writer")?;
    // Closing our copy of the slave is what lets the reader see EOF once the
    // child exits; holding it open would hang the reader thread forever.
    drop(pair.slave);
    let mut master = pair.master;
    Ok(Opened {
        reader,
        writer,
        resize: Box::new(move |cols, rows| {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }),
        wait: Box::new(move || Ok(child.wait()?.exit_code() as i32)),
        close: None,
    })
}

#[cfg(windows)]
fn open_pty(program: &str, args: &[String], cols: u16, rows: u16, id: &str) -> Result<Opened> {
    let pty = conpty::ConPty::spawn(program, args, cols, rows, id)?;
    let waited = std::sync::Arc::clone(&pty.session);
    let resized = std::sync::Arc::clone(&pty.session);
    let closed = std::sync::Arc::clone(&pty.session);
    Ok(Opened {
        reader: Box::new(pty.output),
        writer: Box::new(pty.input),
        resize: Box::new(move |cols, rows| resized.resize(cols, rows)),
        wait: Box::new(move || waited.wait()),
        close: Some(Box::new(closed)),
    })
}

/// Forward everything the shell prints to the real terminal, recording blocks.
fn pump(mut reader: Box<dyn Read + Send>, terminal_id: String) -> Result<()> {
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

/// Copy keystrokes to the pty unchanged.
///
/// The outer terminal already encoded the keys. Re-encoding them is what made
/// raw-mode programs (cmdc) see nothing. On Windows the console is put in VT
/// input mode first, so `ReadFile` yields the same byte stream Unix stdin has.
fn spawn_input(writer: Arc<Mutex<Box<dyn Write + Send>>>) {
    std::thread::spawn(move || {
        let _ = forward_input(&writer);
    });
}

fn forward_input(writer: &Mutex<Box<dyn Write + Send>>) -> std::io::Result<()> {
    let mut stdin = std::io::stdin();
    let mut chunk = [0u8; 8192];
    loop {
        let read = stdin.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        let mut input = writer.lock().unwrap_or_else(|poison| poison.into_inner());
        input.write_all(&chunk[..read])?;
        input.flush()?;
    }
}

/// Keep the pty sized to the real terminal.
fn spawn_resizer(
    stop: Arc<AtomicBool>,
    mut resize: Box<dyn FnMut(u16, u16) + Send>,
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
            resize(size.0, size.1);
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

        let output = String::from_utf8_lossy(output);
        let output = if metadata.echoed_input {
            drop_echo(&output, &metadata.command)
        } else {
            output.as_ref()
        };
        let output = trim_trailing_mark(output, &metadata.prompt);
        match self.append(output, exit_code, duration_ms, &metadata) {
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

/// Drop the echoed input that opens a PowerShell block.
///
/// The shell declares this (`echoed_input`), so nothing is guessed: the bytes
/// after `C` are exactly what the user typed, echoed back — as many lines as the
/// command has, continuation lines included — and the output starts after the
/// last of them.
fn drop_echo<'a>(output: &'a str, command: &str) -> &'a str {
    let mut rest = output;
    for _ in 0..command.lines().count().max(1) {
        match rest.find('\n') {
            Some(newline) => rest = &rest[newline + 1..],
            None => return "",
        }
    }
    rest
}

/// zsh draws its partial-line mark *before* it runs `precmd`, so the mark lands
/// inside the block, followed by the spaces and carriage returns zsh clears the
/// line with. Strip that tail only when it has exactly that shape: it carries
/// escape sequences, it ends with the clearing carriage return zsh always emits,
/// and what is left once the escapes are gone is the tail of the prompt.
///
/// The trailing carriage return is what separates the mark from a coloured
/// output line that happens to end in the prompt's last character.
fn trim_trailing_mark<'a>(output: &'a str, prompt: &str) -> &'a str {
    let head = output.rfind('\n').map_or(0, |at| at + 1);
    let fragment = &output[head..];
    if !fragment.contains('\x1b') || !fragment.ends_with('\r') {
        return output;
    }
    let marked = fragment.trim_end_matches([' ', '\r', '\t']);
    let plain = SessionEntry::new("", "", marked).output;
    if plain.is_empty() {
        return output;
    }
    if !SessionEntry::new(prompt, "", "")
        .prompt
        .trim_end()
        .ends_with(&plain)
    {
        return output;
    }
    match head {
        0 => "",
        head => &output[..head - 1],
    }
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
        #[cfg(windows)]
        Self::enable_vt_input()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

impl RawMode {
    #[cfg(windows)]
    fn enable_vt_input() -> Result<()> {
        use winapi::um::consoleapi::{GetConsoleMode, SetConsoleMode};
        use winapi::um::processenv::GetStdHandle;
        use winapi::um::winbase::STD_INPUT_HANDLE;
        use winapi::um::wincon::ENABLE_VIRTUAL_TERMINAL_INPUT;

        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut mode = 0u32;
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            anyhow::bail!("failed to read console input mode");
        }
        if unsafe { SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_INPUT) } == 0 {
            anyhow::bail!("failed to enable virtual terminal input");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        drop_echo, now_timestamp, pending_path, should_record, trim_trailing_mark, Pending,
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
            echoed_input: true,
        };
        let text = serde_json::to_string(&pending).expect("encode");
        let decoded: Pending = serde_json::from_str(&text).expect("decode");
        assert_eq!(decoded.command, "echo '中文' && true");
        assert_eq!(decoded.cwd.as_deref(), Some("/home/user/repo"));
        assert!(decoded.echoed_input);
    }

    #[test]
    fn pending_path_is_scoped_per_terminal() {
        let path = pending_path("pty_123");
        assert!(path.to_string_lossy().ends_with("pty_123.pending"));
    }

    #[test]
    fn terminal_ids_do_not_repeat() {
        let first = sivtr_core::workspace::new_terminal_id();
        let second = sivtr_core::workspace::new_terminal_id();
        assert!(first.starts_with("pty_"), "{first}");
        assert_ne!(first, second);
    }

    #[test]
    fn timestamps_are_rfc3339() {
        let stamp = now_timestamp();
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert!(stamp.contains('T'), "{stamp}");
    }

    #[test]
    fn drops_the_declared_echo() {
        assert_eq!(drop_echo("echo hi\r\nhi\r\n", "echo hi"), "hi\r\n");
        // A highlighted echo drops just the same.
        assert_eq!(
            drop_echo("\x1b[32mecho\x1b[0m hi\r\nhi\r\n", "echo hi"),
            "hi\r\n"
        );
        // Nothing follows the echo: no output, not a stray fragment.
        assert_eq!(drop_echo("echo hi\r\n", "echo hi"), "");
        assert_eq!(drop_echo("echo hi", "echo hi"), "");
    }

    #[test]
    fn drops_every_line_of_a_multiline_echo() {
        // PSReadLine echoes continuation lines too, so the whole input goes.
        let command = "foreach ($i in 1..2) {\n  $i\n}";
        let block = "foreach ($i in 1..2) {\r\n  $i\r\n}\r\n1\r\n2\r\n";
        assert_eq!(drop_echo(block, command), "1\r\n2\r\n");
    }

    #[test]
    fn keeps_output_when_the_echo_is_shorter_than_the_command() {
        // A shell that did not echo every line must not lose output.
        assert_eq!(drop_echo("one\r\n", "a\nb\nc"), "");
    }

    #[test]
    fn strips_the_zsh_prompt_end_mark() {
        // zsh draws its mark before precmd, so the block ends with the mark
        // (ANSI-wrapped) plus the padding it clears with.
        let prompt = "MS-Challenger-B760M-F-WIFI% ";
        let output = "from-zsh\n\u{1b}[1m\u{1b}[7m%\u{1b}[27m\u{1b}[1m\u{1b}[0m \r \r";
        assert_eq!(trim_trailing_mark(output, prompt), "from-zsh");
    }

    #[test]
    fn strips_a_block_that_is_only_the_mark() {
        let prompt = "repo% ";
        assert_eq!(trim_trailing_mark("\u{1b}[7m%\u{1b}[0m \r", prompt), "");
    }

    #[test]
    fn keeps_plain_output_that_looks_like_the_prompt() {
        // The reviewer's case: a real `%` line carries no escapes, so it is
        // output, not decoration.
        let prompt = "repo% ";
        assert_eq!(trim_trailing_mark("weird\n%", prompt), "weird\n%");
    }

    #[test]
    fn keeps_coloured_output_that_is_not_the_prompt() {
        let prompt = "repo% ";
        let output = "\u{1b}[31mfailed\u{1b}[0m";
        assert_eq!(trim_trailing_mark(output, prompt), output);
    }

    #[test]
    fn keeps_coloured_output_that_ends_in_the_prompt_character() {
        // No mark to clear the line means no carriage return, which is what
        // separates decoration from a coloured line ending in `%`.
        let prompt = "repo% ";
        let output = "\u{1b}[31m%\u{1b}[0m";
        assert_eq!(trim_trailing_mark(output, prompt), output);
    }

    #[test]
    fn keeps_everything_when_no_prompt_was_recorded() {
        let output = "one\n\u{1b}[7m%\u{1b}[0m";
        assert_eq!(trim_trailing_mark(output, ""), output);
    }
}
