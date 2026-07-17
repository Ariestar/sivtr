//! Real Windows pseudo-console coverage for the terminal lifecycle.
//!
//! This module deliberately stays outside `terminal.rs`: the smoke test needs a sizeable amount
//! of Win32 setup code, but none of it is part of the production implementation. The parent test
//! creates a ConPTY and launches this same test executable inside it. A small child test then
//! exercises the real crossterm/Ratatui path.

use std::{
    ffi::{c_void, OsStr, OsString},
    fs::File,
    io::{self, Read, Write},
    mem::{self, size_of},
    os::windows::{
        ffi::OsStrExt,
        io::{FromRawHandle, RawHandle},
    },
    ptr,
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::widgets::{Block, Borders, Paragraph};
use winapi::{
    shared::{
        minwindef::{DWORD, FALSE},
        ntdef::HRESULT,
        winerror::WAIT_TIMEOUT,
    },
    um::{
        consoleapi::{
            ClosePseudoConsole, CreatePseudoConsole, GetConsoleMode, GetConsoleOutputCP,
            ResizePseudoConsole,
        },
        fileapi::{CreateFileW, OPEN_EXISTING},
        handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
        namedpipeapi::CreatePipe,
        processenv::{GetStdHandle, SetStdHandle},
        processthreadsapi::{
            CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
            InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
            LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
        },
        synchapi::WaitForSingleObject,
        winbase::{
            CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, STARTUPINFOEXW,
            STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
        },
        wincontypes::{COORD, HPCON},
        winnt::{
            FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE,
            HANDLE,
        },
    },
};

use super::{event::read_interaction, terminal};

const CHILD_ENV: &str = "SIVTR_CONPTY_SMOKE_CHILD";
const CHILD_TEST: &str = "tui::conpty_tests::conpty_child_entry";
const STARTED_MARKER: &[u8] = b"SIVTR_CONPTY_CHILD_STARTED";
const READY_MARKER: &[u8] = b"SIVTR_CONPTY_READY";
const RESIZED_MARKER: &[u8] = b"SIVTR_CONPTY_RESIZED";
const RESTORED_MARKER: &[u8] = b"SIVTR_CONPTY_RESTORED";
const INITIAL_SIZE: COORD = COORD { X: 80, Y: 25 };
const RESIZED_SIZE: COORD = COORD { X: 100, Y: 30 };
const START_TIMEOUT: Duration = Duration::from_secs(15);
const RESIZE_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_TIMEOUT: Duration = Duration::from_secs(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

// ProcThreadAttributeValue(ProcThreadAttributePseudoConsole = 22, FALSE, TRUE, FALSE).
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

/// Run explicitly with:
///
/// `cargo test --locked tui::conpty_tests::windows_conpty_tui_smoke -- --ignored --exact --nocapture`
#[test]
#[ignore = "requires a Windows 10+ ConPTY host and launches a real pseudo-terminal"]
fn windows_conpty_tui_smoke() {
    if let Err(error) = run_parent_smoke() {
        panic!("Windows ConPTY TUI smoke failed: {error:#}");
    }
}

/// This is a normal (non-ignored) test so the parent can select it without also selecting every
/// ignored test. It is intentionally a no-op unless the parent supplies a private environment
/// marker in the child's environment block.
#[test]
fn conpty_child_entry() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    if let Err(error) = run_child_smoke() {
        panic!("ConPTY child failed: {error:#}");
    }
}

fn run_parent_smoke() -> Result<()> {
    let (pty_input, parent_input) = create_pipe().context("create ConPTY input pipe")?;
    let (parent_output, pty_output) = create_pipe().context("create ConPTY output pipe")?;
    let mut pseudoconsole = PseudoConsole::create(INITIAL_SIZE, &pty_input, &pty_output)?;

    // Once CreatePseudoConsole succeeds, ConPTY owns duplicates of these two endpoints. Closing
    // our copies is essential: retaining the output endpoint would prevent the reader seeing EOF.
    drop(pty_input);
    drop(pty_output);

    let mut input = unsafe { File::from_raw_handle(parent_input.into_raw_handle()) };
    let output = unsafe { File::from_raw_handle(parent_output.into_raw_handle()) };
    let (reader, receiver) = OutputReader::spawn(output)?;
    let mut captured = Vec::new();

    let executable = std::env::current_exe().context("resolve the current test executable")?;
    let arguments = [
        executable.as_os_str(),
        OsStr::new("--exact"),
        OsStr::new(CHILD_TEST),
        OsStr::new("--nocapture"),
        OsStr::new("--test-threads=1"),
    ];
    let environment = child_environment_block()?;
    let mut process = pseudoconsole.spawn(executable.as_os_str(), &arguments, &environment)?;

    receive_until(&receiver, &mut captured, READY_MARKER, START_TIMEOUT).with_context(|| {
        format!(
            "child did not become ready; output: {}",
            output_excerpt(&captured)
        )
    })?;

    pseudoconsole.resize(RESIZED_SIZE)?;
    receive_until(&receiver, &mut captured, RESIZED_MARKER, RESIZE_TIMEOUT).with_context(|| {
        format!(
            "child did not handle the pseudo-console resize; output: {}",
            output_excerpt(&captured)
        )
    })?;

    input.write_all(b"q").context("send q to ConPTY input")?;
    input.flush().context("flush ConPTY input")?;
    let exit_code = process.wait(EXIT_TIMEOUT).with_context(|| {
        format!(
            "child did not exit after q; output: {}",
            output_excerpt(&captured)
        )
    })?;

    drop(input);
    drop(pseudoconsole);
    receive_to_eof(&receiver, &mut captured, DRAIN_TIMEOUT)?;
    reader.join()?;

    if exit_code != 0 {
        bail!(
            "child exited with code {exit_code}; output: {}",
            output_excerpt(&captured)
        );
    }

    assert_output(&captured, "中文", "UTF-8 CJK frame content")?;
    assert_output(&captured, "😀", "UTF-8 emoji frame content")?;
    assert_bytes(&captured, RESIZED_MARKER, "resize handling marker")?;
    assert_bytes(&captured, RESTORED_MARKER, "terminal restoration marker")?;
    // ConPTY consumes alternate-screen and mouse-mode commands and emits their resulting buffer
    // changes, so their original bytes are not a stable observable contract. Cursor restoration
    // and synchronized-update commands remain visible in the output stream.
    assert_bytes(&captured, b"\x1b[?25h", "cursor-show cleanup sequence")?;
    assert_balanced_sequences(
        &captured,
        b"\x1b[?2026h",
        b"\x1b[?2026l",
        "synchronized terminal updates",
    )?;

    Ok(())
}

fn run_child_smoke() -> Result<()> {
    let bindings = ChildConsoleBindings::install()?;
    child_marker(STARTED_MARKER)?;
    let before = ConsoleSnapshot::capture(bindings.pipe_input_handle())?;
    let mut tui = terminal::init().context("initialize TUI inside ConPTY")?;

    let operation = (|| -> Result<()> {
        terminal::draw(&mut tui, |frame| {
            frame.render_widget(
                Paragraph::new("Windows ConPTY smoke 中文 😀")
                    .block(Block::default().borders(Borders::ALL).title("sivtr")),
                frame.area(),
            );
        })?;
        child_marker(READY_MARKER)?;

        loop {
            match read_interaction()? {
                Event::Resize(width, height) => {
                    terminal::draw(&mut tui, |frame| {
                        frame.render_widget(
                            Paragraph::new(format!(
                                "Windows ConPTY resized to {width}x{height} 中文 😀"
                            ))
                            .block(Block::default().borders(Borders::ALL).title("sivtr")),
                            frame.area(),
                        );
                    })?;
                    child_marker(RESIZED_MARKER)?;
                }
                Event::Key(key)
                    if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') =>
                {
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    })();

    terminal::finish(&mut tui, operation)?;
    let after = ConsoleSnapshot::capture(bindings.pipe_input_handle())?;
    if before != after {
        bail!("console state was not restored exactly: before={before:?}, after={after:?}");
    }
    child_marker(RESTORED_MARKER)?;
    Ok(())
}

fn child_marker(marker: &[u8]) -> Result<()> {
    let mut stderr = io::stderr().lock();
    stderr.write_all(b"\r\n")?;
    stderr.write_all(marker)?;
    stderr.write_all(b"\r\n")?;
    stderr.flush()?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ConsoleSnapshot {
    input_mode: DWORD,
    output_mode: DWORD,
    output_code_page: DWORD,
}

impl ConsoleSnapshot {
    fn capture(expected_standard_input: HANDLE) -> Result<Self> {
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if input != expected_standard_input {
            bail!(
                "terminal did not preserve the test's piped stdin handle: expected {expected_standard_input:p}, got {input:p}"
            );
        }
        if !valid_handle(output) {
            bail!("ConPTY child has an invalid standard output handle");
        }

        let console_input = open_console_handle("CONIN$", GENERIC_READ | GENERIC_WRITE)?;
        let mut input_mode = 0;
        let mut output_mode = 0;
        if unsafe { GetConsoleMode(console_input.as_raw(), &mut input_mode) } == 0 {
            return Err(io::Error::last_os_error()).context("read initial console input mode");
        }
        if unsafe { GetConsoleMode(output, &mut output_mode) } == 0 {
            return Err(io::Error::last_os_error()).context("read initial console output mode");
        }

        Ok(Self {
            input_mode,
            output_mode,
            output_code_page: unsafe { GetConsoleOutputCP() },
        })
    }
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE) -> Result<Self> {
        if !valid_handle(handle) {
            Err(io::Error::last_os_error()).context("Win32 returned an invalid handle")
        } else {
            Ok(Self(handle))
        }
    }

    fn as_raw(&self) -> HANDLE {
        self.0
    }

    fn into_raw_handle(mut self) -> RawHandle {
        let handle = self.0;
        self.0 = ptr::null_mut();
        handle.cast()
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if valid_handle(self.0) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn valid_handle(handle: HANDLE) -> bool {
    !handle.is_null() && handle != INVALID_HANDLE_VALUE
}

fn create_pipe() -> Result<(OwnedHandle, OwnedHandle)> {
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, ptr::null_mut(), 0) } == 0 {
        return Err(io::Error::last_os_error()).context("CreatePipe failed");
    }
    // Both handles are valid together after a successful CreatePipe call.
    Ok((OwnedHandle::new(read)?, OwnedHandle::new(write)?))
}

fn open_console_handle(name: &str, access: DWORD) -> Result<OwnedHandle> {
    let name = OsStr::new(name)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    OwnedHandle::new(handle).with_context(|| format!("open {name:?}"))
}

/// Test-only standard handle bindings. The child test runner may itself have inherited pipe
/// handles from `cargo test`, even though it is attached to the new pseudo-console. Bind output to
/// `CONOUT$` and deliberately bind input to a pipe so production `terminal::init` must exercise
/// its `CONIN$` fallback. Drop restores the test runner's original handles before closing ours.
struct ChildConsoleBindings {
    original_input: HANDLE,
    original_output: HANDLE,
    original_error: HANDLE,
    pipe_input: OwnedHandle,
    _pipe_writer: OwnedHandle,
    _console_output: OwnedHandle,
    input_bound: bool,
    output_bound: bool,
    error_bound: bool,
}

impl ChildConsoleBindings {
    fn install() -> Result<Self> {
        let (pipe_input, pipe_writer) = create_pipe().context("create child stdin pipe")?;
        let console_output = open_console_handle("CONOUT$", GENERIC_READ | GENERIC_WRITE)
            .context("bind child output to ConPTY")?;
        let mut bindings = Self {
            original_input: unsafe { GetStdHandle(STD_INPUT_HANDLE) },
            original_output: unsafe { GetStdHandle(STD_OUTPUT_HANDLE) },
            original_error: unsafe { GetStdHandle(STD_ERROR_HANDLE) },
            pipe_input,
            _pipe_writer: pipe_writer,
            _console_output: console_output,
            input_bound: false,
            output_bound: false,
            error_bound: false,
        };

        bindings.set(
            STD_INPUT_HANDLE,
            bindings.pipe_input.as_raw(),
            "piped stdin",
        )?;
        bindings.input_bound = true;
        bindings.set(
            STD_OUTPUT_HANDLE,
            bindings._console_output.as_raw(),
            "ConPTY stdout",
        )?;
        bindings.output_bound = true;
        bindings.set(
            STD_ERROR_HANDLE,
            bindings._console_output.as_raw(),
            "ConPTY stderr",
        )?;
        bindings.error_bound = true;
        Ok(bindings)
    }

    fn set(&self, kind: DWORD, handle: HANDLE, description: &str) -> Result<()> {
        if unsafe { SetStdHandle(kind, handle) } == 0 {
            Err(io::Error::last_os_error()).with_context(|| format!("set test child {description}"))
        } else {
            Ok(())
        }
    }

    fn pipe_input_handle(&self) -> HANDLE {
        self.pipe_input.as_raw()
    }
}

impl Drop for ChildConsoleBindings {
    fn drop(&mut self) {
        unsafe {
            if self.error_bound {
                SetStdHandle(STD_ERROR_HANDLE, self.original_error);
                self.error_bound = false;
            }
            if self.output_bound {
                SetStdHandle(STD_OUTPUT_HANDLE, self.original_output);
                self.output_bound = false;
            }
            if self.input_bound {
                SetStdHandle(STD_INPUT_HANDLE, self.original_input);
                self.input_bound = false;
            }
        }
    }
}

struct PseudoConsole(HPCON);

impl PseudoConsole {
    fn create(size: COORD, input: &OwnedHandle, output: &OwnedHandle) -> Result<Self> {
        let mut handle = ptr::null_mut();
        let result =
            unsafe { CreatePseudoConsole(size, input.as_raw(), output.as_raw(), 0, &mut handle) };
        hresult(result, "CreatePseudoConsole")?;
        if handle.is_null() {
            bail!("CreatePseudoConsole succeeded without returning a handle");
        }
        Ok(Self(handle))
    }

    fn resize(&mut self, size: COORD) -> Result<()> {
        hresult(
            unsafe { ResizePseudoConsole(self.0, size) },
            "ResizePseudoConsole",
        )
    }

    fn spawn(
        &self,
        executable: &OsStr,
        arguments: &[&OsStr],
        environment: &[u16],
    ) -> Result<ChildProcess> {
        let mut attributes = AttributeList::new()?;
        attributes.set_pseudoconsole(self.0)?;

        let mut application = executable.encode_wide().chain(Some(0)).collect::<Vec<_>>();
        let mut command_line = windows_command_line(arguments);
        let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as DWORD;
        startup.lpAttributeList = attributes.as_mut_ptr();
        let mut information: PROCESS_INFORMATION = unsafe { mem::zeroed() };

        let created = unsafe {
            CreateProcessW(
                application.as_mut_ptr(),
                command_line.as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                FALSE,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast::<c_void>() as *mut c_void,
                ptr::null(),
                &mut startup.StartupInfo,
                &mut information,
            )
        };
        if created == 0 {
            return Err(io::Error::last_os_error()).context("CreateProcessW inside ConPTY failed");
        }

        let process = OwnedHandle::new(information.hProcess)?;
        let thread = OwnedHandle::new(information.hThread)?;
        drop(thread);
        Ok(ChildProcess {
            process,
            exited: false,
        })
    }
}

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                ClosePseudoConsole(self.0);
            }
            self.0 = ptr::null_mut();
        }
    }
}

fn hresult(result: HRESULT, operation: &str) -> Result<()> {
    if result >= 0 {
        Ok(())
    } else {
        bail!("{operation} failed with HRESULT 0x{:08X}", result as u32)
    }
}

struct AttributeList {
    // usize gives the opaque Win32 structure pointer alignment without requiring another heap API.
    storage: Vec<usize>,
    initialized: bool,
}

impl AttributeList {
    fn new() -> Result<Self> {
        let mut bytes = 0;
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(io::Error::last_os_error())
                .context("query process thread attribute list size");
        }

        let words = bytes.div_ceil(size_of::<usize>());
        let mut list = Self {
            storage: vec![0; words],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(list.as_mut_ptr(), 1, 0, &mut bytes) } == 0 {
            return Err(io::Error::last_os_error())
                .context("initialize process thread attribute list");
        }
        list.initialized = true;
        Ok(list)
    }

    fn as_mut_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }

    fn set_pseudoconsole(&mut self, pseudoconsole: HPCON) -> Result<()> {
        let result = unsafe {
            UpdateProcThreadAttribute(
                self.as_mut_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                pseudoconsole.cast(),
                size_of::<HPCON>(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if result == 0 {
            Err(io::Error::last_os_error()).context("attach ConPTY process attribute")
        } else {
            Ok(())
        }
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                DeleteProcThreadAttributeList(self.as_mut_ptr());
            }
            self.initialized = false;
        }
    }
}

struct ChildProcess {
    process: OwnedHandle,
    exited: bool,
}

impl ChildProcess {
    fn wait(&mut self, timeout: Duration) -> Result<DWORD> {
        let timeout_ms = timeout.as_millis().min((u32::MAX - 1) as u128) as DWORD;
        match unsafe { WaitForSingleObject(self.process.as_raw(), timeout_ms) } {
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.process.as_raw(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error()).context("read ConPTY child exit code");
                }
                self.exited = true;
                Ok(code)
            }
            WAIT_TIMEOUT => bail!("timed out waiting for ConPTY child after {timeout:?}"),
            WAIT_FAILED => Err(io::Error::last_os_error()).context("wait for ConPTY child"),
            status => bail!("unexpected WaitForSingleObject status {status}"),
        }
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        if !self.exited {
            unsafe {
                TerminateProcess(self.process.as_raw(), 1);
                WaitForSingleObject(self.process.as_raw(), 2_000);
            }
        }
    }
}

enum ReaderMessage {
    Bytes(Vec<u8>),
    Eof,
    Error(io::Error),
}

struct OutputReader(Option<JoinHandle<()>>);

impl OutputReader {
    fn spawn(mut output: File) -> Result<(Self, Receiver<ReaderMessage>)> {
        let (sender, receiver) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("sivtr-conpty-output".into())
            .spawn(move || {
                let mut buffer = [0; 4096];
                loop {
                    match output.read(&mut buffer) {
                        Ok(0) => {
                            let _ = sender.send(ReaderMessage::Eof);
                            break;
                        }
                        Ok(count) => {
                            if sender
                                .send(ReaderMessage::Bytes(buffer[..count].to_vec()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(ReaderMessage::Error(error));
                            break;
                        }
                    }
                }
            })
            .context("spawn ConPTY output reader")?;
        Ok((Self(Some(handle)), receiver))
    }

    fn join(mut self) -> Result<()> {
        if let Some(handle) = self.0.take() {
            handle
                .join()
                .map_err(|_| anyhow!("ConPTY output reader panicked"))?;
        }
        Ok(())
    }
}

fn receive_until(
    receiver: &Receiver<ReaderMessage>,
    captured: &mut Vec<u8>,
    marker: &[u8],
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while !contains_bytes(captured, marker) {
        if !receive_one(receiver, captured, deadline)? {
            bail!(
                "ConPTY output reached EOF before marker {}",
                String::from_utf8_lossy(marker)
            );
        }
    }
    Ok(())
}

fn receive_to_eof(
    receiver: &Receiver<ReaderMessage>,
    captured: &mut Vec<u8>,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        match receive_one(receiver, captured, deadline) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => return Err(error).context("drain ConPTY output"),
        }
    }
}

/// Returns false after EOF and true after receiving bytes.
fn receive_one(
    receiver: &Receiver<ReaderMessage>,
    captured: &mut Vec<u8>,
    deadline: Instant,
) -> Result<bool> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        bail!("timed out waiting for ConPTY output");
    }
    match receiver.recv_timeout(remaining) {
        Ok(ReaderMessage::Bytes(bytes)) => {
            captured.extend_from_slice(&bytes);
            Ok(true)
        }
        Ok(ReaderMessage::Eof) => Ok(false),
        Ok(ReaderMessage::Error(error)) => Err(error).context("read ConPTY output"),
        Err(RecvTimeoutError::Timeout) => bail!("timed out waiting for ConPTY output"),
        Err(RecvTimeoutError::Disconnected) => bail!("ConPTY output reader disconnected"),
    }
}

fn assert_output(captured: &[u8], expected: &str, description: &str) -> Result<()> {
    let output =
        String::from_utf8(captured.to_vec()).context("ConPTY output was not valid UTF-8")?;
    if output.contains(expected) {
        Ok(())
    } else {
        bail!(
            "missing {description}; output: {}",
            output_excerpt(captured)
        )
    }
}

fn assert_bytes(captured: &[u8], expected: &[u8], description: &str) -> Result<()> {
    if contains_bytes(captured, expected) {
        Ok(())
    } else {
        bail!(
            "missing {description}; output: {}",
            output_excerpt(captured)
        )
    }
}

fn assert_balanced_sequences(
    captured: &[u8],
    begin: &[u8],
    end: &[u8],
    description: &str,
) -> Result<()> {
    let begin_count = count_bytes(captured, begin);
    let end_count = count_bytes(captured, end);
    if begin_count > 0 && begin_count == end_count {
        Ok(())
    } else {
        bail!(
            "unbalanced {description}: begin={begin_count}, end={end_count}; output: {}",
            output_excerpt(captured)
        )
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn count_bytes(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn output_excerpt(output: &[u8]) -> String {
    const LIMIT: usize = 8 * 1024;
    let start = output.len().saturating_sub(LIMIT);
    String::from_utf8_lossy(&output[start..])
        .escape_debug()
        .to_string()
}

fn child_environment_block() -> Result<Vec<u16>> {
    let mut variables = std::env::vars_os()
        .filter(|(key, _)| !key.to_string_lossy().eq_ignore_ascii_case(CHILD_ENV))
        .collect::<Vec<_>>();
    variables.push((OsString::from(CHILD_ENV), OsString::from("1")));
    variables.sort_by_cached_key(|(key, _)| key.to_string_lossy().to_uppercase());

    let mut block = Vec::new();
    for (key, value) in variables {
        if key.is_empty() || key.to_string_lossy().contains('=') {
            bail!("cannot encode invalid environment variable name {key:?}");
        }
        block.extend(key.encode_wide());
        block.push('=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn windows_command_line(arguments: &[&OsStr]) -> Vec<u16> {
    let mut command = OsString::new();
    for (index, argument) in arguments.iter().enumerate() {
        if index > 0 {
            command.push(" ");
        }
        command.push(quote_windows_argument(argument));
    }
    command.encode_wide().chain(Some(0)).collect()
}

fn quote_windows_argument(argument: &OsStr) -> OsString {
    let text = argument.to_string_lossy();
    if !text.is_empty()
        && !text
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return argument.to_os_string();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in text.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    OsString::from(quoted)
}
