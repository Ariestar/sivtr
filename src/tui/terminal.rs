use anyhow::{anyhow, bail, Context, Result};
#[cfg(not(windows))]
use crossterm::terminal::disable_raw_mode;
#[cfg(windows)]
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use crossterm::{
    cursor::Show,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{buffer::CellDiffOption, prelude::*};
use std::{
    fmt::Display,
    io::{self, Stdout},
    mem,
    ops::{Deref, DerefMut},
};
use unicode_width::UnicodeWidthStr;

type InnerTui = Terminal<CrosstermBackend<Stdout>>;

/// An active terminal session that restores terminal state when it is dropped.
///
/// Explicit restoration is used when errors can be reported. `Drop` is the final safety net for
/// early returns, draw/event failures, and panics.
pub struct Tui {
    terminal: InnerTui,
    drawing_active: bool,
    state: TerminalState,
    #[cfg(windows)]
    previous_frame_had_wide_cells: bool,
    #[cfg(windows)]
    synchronized_updates_supported: bool,
}

impl Deref for Tui {
    type Target = InnerTui;

    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        if self.state.has_pending_cleanup() {
            let _ = restore_terminal_state(&mut self.terminal, &mut self.state);
        }
        self.drawing_active = false;
    }
}

/// Initialize the terminal for TUI rendering.
pub fn init() -> Result<Tui> {
    ensure_tui_stdout()?;

    let mut setup = TerminalSetup::default();
    match ConsoleInputHandle::ensure_console() {
        Ok(input) => setup.state.console_input = input,
        Err(error) => return setup.fail(error.context("Failed to acquire console input")),
    }

    match ConsoleCodePages::capture() {
        Ok(code_pages) => setup.state.code_pages = code_pages,
        Err(error) => return setup.fail(error),
    }
    if let Some(code_pages) = setup.state.code_pages.as_ref() {
        if let Err(error) = code_pages.enable_utf8() {
            return setup.fail(error);
        }
    }

    match TerminalModes::capture() {
        Ok(modes) => setup.state.modes = Some(modes),
        Err(error) => return setup.fail(error),
    }
    #[cfg(windows)]
    let synchronized_updates_supported = setup
        .state
        .modes
        .as_ref()
        .is_some_and(TerminalModes::enable_virtual_terminal_processing);
    if let Err(error) = enable_raw_mode().context("Failed to enable terminal raw mode") {
        return setup.fail(error);
    }

    let mut stdout = io::stdout();
    // Arm before issuing each command: a failed write may still have partially reached the
    // terminal, so rollback must conservatively send the inverse command.
    setup.state.alternate_screen = true;
    if let Err(error) = execute!(stdout, EnterAlternateScreen)
        .context("Failed to enter the terminal alternate screen")
    {
        return setup.fail(error);
    }

    setup.state.mouse_capture = true;
    if let Err(error) =
        execute!(stdout, EnableMouseCapture).context("Failed to enable terminal mouse capture")
    {
        return setup.fail(error);
    }

    setup.state.cursor_restore_pending = true;
    let backend = CrosstermBackend::new(stdout);
    let terminal = match Terminal::new(backend).context("Failed to initialize terminal buffers") {
        Ok(terminal) => terminal,
        Err(error) => return setup.fail(error),
    };

    Ok(Tui {
        terminal,
        drawing_active: true,
        state: setup.commit(),
        #[cfg(windows)]
        previous_frame_had_wide_cells: false,
        #[cfg(windows)]
        synchronized_updates_supported,
    })
}

/// Draw one TUI frame.
///
/// Windows terminals can leave stale trailing cells when an incremental diff replaces CJK or
/// other wide glyphs. A frame containing wide cells, the frame after one, and a resized frame are
/// fully refreshed. ASCII-only steady-state frames keep Ratatui's incremental rendering policy.
pub fn draw<F>(terminal: &mut Tui, render: F) -> Result<()>
where
    F: FnOnce(&mut Frame),
{
    if !terminal.drawing_active {
        bail!("Cannot draw while the terminal session is suspended");
    }

    #[cfg(windows)]
    {
        let resized = synchronize_terminal_size(&mut terminal.terminal, || {
            Ok(CrosstermBackend::new(io::stdout()))
        })
        .context("Failed to synchronize the Windows terminal size")?;
        let force_full_redraw = resized || terminal.previous_frame_had_wide_cells;

        let draw_result = if terminal.synchronized_updates_supported {
            let update = SynchronizedUpdateGuard::begin()?;
            let result = draw_terminal_frame(&mut terminal.terminal, force_full_redraw, render)
                .context("Failed to draw a Windows terminal frame");
            combine_results(
                result,
                update.finish(),
                "additionally failed to end the synchronized terminal update",
            )
        } else {
            draw_terminal_frame(&mut terminal.terminal, force_full_redraw, render)
                .context("Failed to draw a Windows terminal frame")
        };

        match draw_result {
            Ok(has_wide_cells) => {
                terminal.previous_frame_had_wide_cells = has_wide_cells;
                Ok(())
            }
            Err(error) => {
                // If a partial frame reached the terminal, make the next successful draw a full
                // refresh rather than trusting Ratatui's previous-buffer assumptions.
                terminal.previous_frame_had_wide_cells = true;
                Err(error)
            }
        }
    }

    #[cfg(not(windows))]
    {
        draw_terminal_frame(&mut terminal.terminal, false, render)
            .context("Failed to draw terminal frame")?;
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn synchronize_terminal_size<B, F>(
    terminal: &mut Terminal<B>,
    create_backend: F,
) -> std::result::Result<bool, B::Error>
where
    B: Backend,
    F: FnOnce() -> std::result::Result<B, B::Error>,
{
    let backend_size = terminal.backend().size()?;
    let viewport_size = terminal.get_frame().area().as_size();
    if backend_size == viewport_size {
        return Ok(false);
    }

    // Recreate Ratatui's buffers without calling Terminal::resize, whose physical clear can leave
    // stale wide-character continuation cells in Windows consoles.
    *terminal = Terminal::new(create_backend()?)?;
    Ok(true)
}

fn draw_terminal_frame<B, F>(
    terminal: &mut Terminal<B>,
    force_full_redraw: bool,
    render: F,
) -> std::result::Result<bool, B::Error>
where
    B: Backend,
    F: FnOnce(&mut Frame),
{
    let mut has_wide_cells = false;
    terminal.draw(|frame| {
        render(frame);
        has_wide_cells = frame
            .buffer_mut()
            .content
            .iter()
            .any(|cell| UnicodeWidthStr::width(cell.symbol()) > 1);

        if force_full_redraw || has_wide_cells {
            apply_full_redraw_policy(frame.buffer_mut());
        }
    })?;
    Ok(has_wide_cells)
}

fn apply_full_redraw_policy(buffer: &mut Buffer) {
    for cell in &mut buffer.content {
        if cell.diff_option == CellDiffOption::None {
            cell.set_diff_option(CellDiffOption::AlwaysUpdate);
        }
    }
}

/// Restore the terminal to its original state.
///
/// Each resource is tracked separately. Successful cleanup steps are immediately disarmed, so a
/// later retry only touches operations that previously failed.
pub fn restore(terminal: &mut Tui) -> Result<()> {
    terminal.drawing_active = false;
    restore_terminal_state(&mut terminal.terminal, &mut terminal.state)
}

/// Restore a terminal and preserve both the operation error and a cleanup error, if both occur.
pub fn finish<T>(terminal: &mut Tui, operation: Result<T>) -> Result<T> {
    combine_results(
        operation,
        restore(terminal),
        "additionally failed to restore terminal state",
    )
}

/// Temporarily restore the terminal while an external program runs, then resume the TUI.
///
/// The outer result reports suspend/resume failures. The inner result is the external operation's
/// result, allowing callers such as the main browser to display editor errors and continue.
pub fn with_suspended<T, F>(terminal: &mut Tui, operation: F) -> Result<Result<T>>
where
    F: FnOnce() -> Result<T>,
{
    restore(terminal)?;
    let operation_result = operation();

    match init() {
        Ok(resumed) => {
            *terminal = resumed;
            Ok(operation_result)
        }
        Err(resume_error) => match operation_result {
            Ok(_) => Err(resume_error.context("Failed to resume the terminal interface")),
            Err(operation_error) => Err(anyhow!(
                "{operation_error:#}; additionally failed to resume the terminal interface: {resume_error:#}"
            )),
        },
    }
}

fn restore_terminal_state(terminal: &mut InnerTui, state: &mut TerminalState) -> Result<()> {
    let mut failures = CleanupFailures::default();

    if state.mouse_capture {
        failures.record_flag(
            "disable mouse capture",
            &mut state.mouse_capture,
            execute!(terminal.backend_mut(), DisableMouseCapture),
        );
    }
    if state.alternate_screen {
        failures.record_flag(
            "leave alternate screen",
            &mut state.alternate_screen,
            execute!(terminal.backend_mut(), LeaveAlternateScreen),
        );
    }
    if state.cursor_restore_pending {
        failures.record_flag(
            "show cursor",
            &mut state.cursor_restore_pending,
            terminal.show_cursor(),
        );
    }
    restore_owned_state(&mut failures, state);

    failures.finish("Failed to restore terminal state")
}

fn rollback_setup(state: &mut TerminalState) -> Result<()> {
    let mut failures = CleanupFailures::default();
    let mut stdout = io::stdout();

    if state.mouse_capture {
        failures.record_flag(
            "disable mouse capture",
            &mut state.mouse_capture,
            execute!(stdout, DisableMouseCapture),
        );
    }
    if state.alternate_screen {
        failures.record_flag(
            "leave alternate screen",
            &mut state.alternate_screen,
            execute!(stdout, LeaveAlternateScreen),
        );
    }
    if state.cursor_restore_pending {
        failures.record_flag(
            "show cursor",
            &mut state.cursor_restore_pending,
            execute!(stdout, Show),
        );
    }
    restore_owned_state(&mut failures, state);

    failures.finish("Failed to roll back partial terminal initialization")
}

fn restore_owned_state(failures: &mut CleanupFailures, state: &mut TerminalState) {
    let display_commands_restored =
        !state.mouse_capture && !state.alternate_screen && !state.cursor_restore_pending;
    let mut input_mode_restored = true;

    if let Some(modes) = state.modes.as_mut() {
        let input_result = modes.restore_input();
        failures.record("restore console input mode", input_result);
        input_mode_restored = modes.is_input_restored();

        // Mouse capture, alternate-screen, and cursor cleanup are emitted as ANSI sequences on
        // Windows. Keep virtual-terminal output enabled until every such sequence succeeds, so a
        // later Drop retry can still take effect instead of merely writing inert escape bytes.
        if display_commands_restored {
            let output_result = modes.restore_output();
            failures.record("restore console output mode", output_result);
        }

        if modes.is_restored() {
            state.modes = None;
        }
    }

    if display_commands_restored {
        if let Some(code_pages) = state.code_pages.as_mut() {
            let result = code_pages.restore();
            failures.record("restore Windows console code page", result);
            if code_pages.is_restored() {
                state.code_pages = None;
            }
        }
    }

    // TerminalModes holds the current CONIN$ handle. Do not restore STDIN or close that handle
    // until its exact input mode has been restored; otherwise a cleanup retry would target a
    // closed handle and could leave the console in raw mode permanently.
    if input_mode_restored {
        if let Some(console_input) = state.console_input.as_mut() {
            let result = console_input.restore();
            failures.record("restore standard console input", result);
            if console_input.is_restored() {
                state.console_input = None;
            }
        }
    }
}

#[derive(Default)]
struct TerminalState {
    mouse_capture: bool,
    alternate_screen: bool,
    cursor_restore_pending: bool,
    modes: Option<TerminalModes>,
    code_pages: Option<ConsoleCodePages>,
    console_input: Option<ConsoleInputHandle>,
}

impl TerminalState {
    fn has_pending_cleanup(&self) -> bool {
        self.mouse_capture
            || self.alternate_screen
            || self.cursor_restore_pending
            || self.modes.is_some()
            || self.code_pages.is_some()
            || self.console_input.is_some()
    }
}

struct TerminalSetup {
    state: TerminalState,
    armed: bool,
}

impl Default for TerminalSetup {
    fn default() -> Self {
        Self {
            state: TerminalState::default(),
            armed: true,
        }
    }
}

impl TerminalSetup {
    fn fail<T>(&mut self, error: anyhow::Error) -> Result<T> {
        let cleanup = rollback_setup(&mut self.state);
        if !self.state.has_pending_cleanup() {
            self.armed = false;
        }
        setup_failure(error, cleanup)
    }

    fn commit(mut self) -> TerminalState {
        self.armed = false;
        mem::take(&mut self.state)
    }
}

impl Drop for TerminalSetup {
    fn drop(&mut self) {
        if self.armed && self.state.has_pending_cleanup() {
            let _ = rollback_setup(&mut self.state);
        }
    }
}

fn setup_failure<T>(error: anyhow::Error, cleanup: Result<()>) -> Result<T> {
    match cleanup {
        Ok(()) => Err(error),
        Err(cleanup_error) => Err(anyhow!(
            "{error:#}; additionally failed to roll back terminal state: {cleanup_error:#}"
        )),
    }
}

fn combine_results<T>(
    primary: Result<T>,
    secondary: Result<()>,
    secondary_context: &str,
) -> Result<T> {
    match (primary, secondary) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(secondary_error)) => Err(anyhow!(
            "{error:#}; {secondary_context}: {secondary_error:#}"
        )),
    }
}

#[derive(Default)]
struct CleanupFailures(Vec<String>);

impl CleanupFailures {
    fn record<E>(&mut self, operation: &str, result: std::result::Result<(), E>)
    where
        E: Display,
    {
        if let Err(error) = result {
            self.0.push(format!("{operation}: {error}"));
        }
    }

    fn record_flag<E>(
        &mut self,
        operation: &str,
        pending: &mut bool,
        result: std::result::Result<(), E>,
    ) where
        E: Display,
    {
        match result {
            Ok(()) => *pending = false,
            Err(error) => self.0.push(format!("{operation}: {error}")),
        }
    }

    fn finish(self, context: &str) -> Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            bail!("{context}: {}", self.0.join("; "))
        }
    }
}

fn ensure_tui_stdout() -> Result<()> {
    ensure_tui_stdout_value(atty::is(atty::Stream::Stdout))
}

fn ensure_tui_stdout_value(is_terminal: bool) -> Result<()> {
    if is_terminal {
        return Ok(());
    }

    bail!(
        "sivtr: TUI mode requires an interactive terminal\n  hint: run inside a terminal or set `general.open_mode = \"editor\"` in config"
    )
}

#[cfg(windows)]
struct ConsoleInputHandle {
    original: winapi::um::winnt::HANDLE,
    replacement: winapi::um::winnt::HANDLE,
    standard_handle_pending: bool,
    close_pending: bool,
}

#[cfg(windows)]
impl ConsoleInputHandle {
    fn ensure_console() -> Result<Option<Self>> {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
        use winapi::um::{
            consoleapi::GetConsoleMode,
            fileapi::{CreateFileW, OPEN_EXISTING},
            handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
            processenv::{GetStdHandle, SetStdHandle},
            winbase::STD_INPUT_HANDLE,
            winnt::{
                FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ,
                GENERIC_WRITE,
            },
        };

        let original = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut mode = 0;
        if !original.is_null()
            && original != INVALID_HANDLE_VALUE
            && unsafe { GetConsoleMode(original, &mut mode) } != 0
        {
            return Ok(None);
        }

        let name = OsStr::new("CONIN$\0").encode_wide().collect::<Vec<_>>();
        let replacement = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if replacement == INVALID_HANDLE_VALUE {
            bail!("Failed to open CONIN$: {}", io::Error::last_os_error());
        }
        if unsafe { SetStdHandle(STD_INPUT_HANDLE, replacement) } == 0 {
            let error = io::Error::last_os_error();
            unsafe {
                CloseHandle(replacement);
            }
            bail!("Failed to bind standard input to CONIN$: {error}");
        }

        Ok(Some(Self {
            original,
            replacement,
            standard_handle_pending: true,
            close_pending: true,
        }))
    }

    fn restore(&mut self) -> Result<()> {
        use winapi::um::{
            handleapi::CloseHandle, processenv::SetStdHandle, winbase::STD_INPUT_HANDLE,
        };

        let mut failures = CleanupFailures::default();
        if self.standard_handle_pending {
            let result = win32_console_call("restore standard input handle", unsafe {
                SetStdHandle(STD_INPUT_HANDLE, self.original)
            });
            if result.is_ok() {
                self.standard_handle_pending = false;
            }
            failures.record("restore standard input handle", result);
        }
        if !self.standard_handle_pending && self.close_pending {
            let result = win32_console_call("close temporary CONIN$ handle", unsafe {
                CloseHandle(self.replacement)
            });
            if result.is_ok() {
                self.close_pending = false;
            }
            failures.record("close temporary CONIN$ handle", result);
        }
        failures.finish("Failed to restore console input handle")
    }

    fn is_restored(&self) -> bool {
        !self.standard_handle_pending && !self.close_pending
    }
}

#[cfg(not(windows))]
struct ConsoleInputHandle;

#[cfg(not(windows))]
impl ConsoleInputHandle {
    fn ensure_console() -> Result<Option<Self>> {
        Ok(None)
    }

    fn restore(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_restored(&self) -> bool {
        true
    }
}

#[cfg(windows)]
struct TerminalModes {
    input_handle: winapi::um::winnt::HANDLE,
    input_mode: u32,
    output_handle: winapi::um::winnt::HANDLE,
    output_mode: u32,
    input_pending: bool,
    output_pending: bool,
}

#[cfg(windows)]
impl TerminalModes {
    fn capture() -> Result<Self> {
        use winapi::um::{
            consoleapi::GetConsoleMode,
            handleapi::INVALID_HANDLE_VALUE,
            processenv::GetStdHandle,
            winbase::{STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
        };

        let input_handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let output_handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if input_handle.is_null()
            || input_handle == INVALID_HANDLE_VALUE
            || output_handle.is_null()
            || output_handle == INVALID_HANDLE_VALUE
        {
            bail!("Failed to resolve Windows console handles");
        }

        let mut input_mode = 0;
        let mut output_mode = 0;
        win32_console_call("read console input mode", unsafe {
            GetConsoleMode(input_handle, &mut input_mode)
        })?;
        win32_console_call("read console output mode", unsafe {
            GetConsoleMode(output_handle, &mut output_mode)
        })?;

        Ok(Self {
            input_handle,
            input_mode,
            output_handle,
            output_mode,
            input_pending: true,
            output_pending: true,
        })
    }

    fn restore_input(&mut self) -> Result<()> {
        use winapi::um::consoleapi::SetConsoleMode;

        if self.input_pending {
            win32_console_call("restore console input mode", unsafe {
                SetConsoleMode(self.input_handle, self.input_mode)
            })?;
            self.input_pending = false;
        }
        Ok(())
    }

    fn restore_output(&mut self) -> Result<()> {
        use winapi::um::consoleapi::SetConsoleMode;

        if self.output_pending {
            win32_console_call("restore console output mode", unsafe {
                SetConsoleMode(self.output_handle, self.output_mode)
            })?;
            self.output_pending = false;
        }
        Ok(())
    }

    fn enable_virtual_terminal_processing(&self) -> bool {
        use winapi::um::{consoleapi::SetConsoleMode, wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING};

        unsafe {
            SetConsoleMode(
                self.output_handle,
                self.output_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            ) != 0
        }
    }

    fn is_restored(&self) -> bool {
        !self.input_pending && !self.output_pending
    }

    fn is_input_restored(&self) -> bool {
        !self.input_pending
    }
}

#[cfg(not(windows))]
struct TerminalModes {
    pending: bool,
}

#[cfg(not(windows))]
impl TerminalModes {
    fn capture() -> Result<Self> {
        Ok(Self { pending: true })
    }

    fn restore_input(&mut self) -> Result<()> {
        if self.pending {
            disable_raw_mode()?;
            self.pending = false;
        }
        Ok(())
    }

    fn restore_output(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_restored(&self) -> bool {
        !self.pending
    }

    fn is_input_restored(&self) -> bool {
        !self.pending
    }
}

#[cfg(windows)]
struct ConsoleCodePages {
    output: u32,
    pending: bool,
}

#[cfg(windows)]
impl ConsoleCodePages {
    fn capture() -> Result<Option<Self>> {
        use winapi::um::consoleapi::GetConsoleOutputCP;

        let output = unsafe { GetConsoleOutputCP() };
        if output == 0 {
            bail!(
                "Failed to read Windows console output code page: {}",
                io::Error::last_os_error()
            );
        }
        Ok(Some(Self {
            output,
            pending: true,
        }))
    }

    fn enable_utf8(&self) -> Result<()> {
        use winapi::um::wincon::SetConsoleOutputCP;

        const CP_UTF8: u32 = 65_001;
        win32_console_call("set console output to UTF-8", unsafe {
            SetConsoleOutputCP(CP_UTF8)
        })
    }

    fn restore(&mut self) -> Result<()> {
        use winapi::um::wincon::SetConsoleOutputCP;

        if self.pending {
            win32_console_call("restore console output code page", unsafe {
                SetConsoleOutputCP(self.output)
            })?;
            self.pending = false;
        }
        Ok(())
    }

    fn is_restored(&self) -> bool {
        !self.pending
    }
}

#[cfg(not(windows))]
struct ConsoleCodePages;

#[cfg(not(windows))]
impl ConsoleCodePages {
    fn capture() -> Result<Option<Self>> {
        Ok(None)
    }

    fn enable_utf8(&self) -> Result<()> {
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_restored(&self) -> bool {
        true
    }
}

#[cfg(windows)]
fn win32_console_call(operation: &str, succeeded: i32) -> Result<()> {
    if succeeded == 0 {
        bail!("{operation}: {}", io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
struct SynchronizedUpdateGuard {
    active: bool,
}

#[cfg(windows)]
impl SynchronizedUpdateGuard {
    fn begin() -> Result<Self> {
        // Arm before writing Begin: an I/O error can occur after bytes have partially reached the
        // terminal, so Drop must still attempt End.
        let guard = Self { active: true };
        execute!(io::stdout(), BeginSynchronizedUpdate)
            .context("Failed to begin synchronized terminal update")?;
        Ok(guard)
    }

    fn finish(mut self) -> Result<()> {
        let result = execute!(io::stdout(), EndSynchronizedUpdate)
            .context("Failed to end synchronized terminal update");
        if result.is_ok() {
            self.active = false;
        }
        result
    }
}

#[cfg(windows)]
impl Drop for SynchronizedUpdateGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(io::stdout(), EndSynchronizedUpdate);
            self.active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, num::NonZeroU16};

    use ratatui::{
        backend::TestBackend, buffer::Buffer, layout::Rect, text::Line, widgets::Paragraph,
        Terminal,
    };

    use super::{
        apply_full_redraw_policy, combine_results, draw_terminal_frame, ensure_tui_stdout_value,
        CleanupFailures,
    };

    #[test]
    fn wide_frame_overwrites_stale_cells_and_forces_output() {
        let backend = TestBackend::with_lines([
            "旧帧中文残留 ABC    ",
            "한글 stale 😀      ",
            "should disappear   ",
        ]);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let has_wide = draw_terminal_frame(&mut terminal, false, |frame| {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from("[x] 新会话 中文 한글 😀"),
                    Line::from("[ ] second row"),
                ]),
                frame.area(),
            );
        })
        .expect("wide redraw");

        assert!(has_wide);
        assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "[");
    }

    #[test]
    fn frame_after_wide_content_clears_stale_tail_cells() {
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).expect("terminal");

        draw_terminal_frame(&mut terminal, false, |frame| {
            frame.render_widget(Paragraph::new("中文 wide 😀"), frame.area());
        })
        .expect("wide frame");
        draw_terminal_frame(&mut terminal, true, |frame| {
            frame.render_widget(Paragraph::new("short"), frame.area());
        })
        .expect("short frame");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "s");
        assert!(buffer.content[5..].iter().all(|cell| cell.symbol() == " "));
    }

    #[test]
    fn full_redraw_preserves_special_cell_diff_options() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        let cells = &mut buffer.content;
        cells[0].set_diff_option(ratatui::buffer::CellDiffOption::Skip);
        cells[1].set_diff_option(ratatui::buffer::CellDiffOption::ForcedWidth(
            NonZeroU16::new(2).unwrap(),
        ));

        apply_full_redraw_policy(&mut buffer);

        let cells = &buffer.content;
        assert_eq!(cells[0].diff_option, ratatui::buffer::CellDiffOption::Skip);
        assert_eq!(
            cells[1].diff_option,
            ratatui::buffer::CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap())
        );
        assert_eq!(
            cells[2].diff_option,
            ratatui::buffer::CellDiffOption::AlwaysUpdate
        );
    }

    #[test]
    fn incremental_ascii_draw_keeps_default_diff_policy() {
        let backend = TestBackend::new(12, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let has_wide = draw_terminal_frame(&mut terminal, false, |frame| {
            frame.render_widget(Paragraph::new("plain frame"), frame.area());
        })
        .expect("incremental redraw");

        assert!(!has_wide);
        terminal
            .backend()
            .assert_buffer_lines(["plain frame ", "            "]);
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .all(|cell| cell.diff_option == ratatui::buffer::CellDiffOption::None));
    }

    #[test]
    fn resize_recreates_buffers_without_terminal_clear() {
        let backend = TestBackend::new(8, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.backend_mut().resize(14, 4);

        let recreated =
            super::synchronize_terminal_size(&mut terminal, || Ok(TestBackend::new(14, 4)))
                .expect("resize synchronization");

        assert!(recreated);
        assert_eq!(terminal.get_frame().area().as_size(), (14, 4).into());
    }

    #[test]
    fn successful_cleanup_is_disarmed_while_failed_cleanup_remains_pending() {
        let mut failures = CleanupFailures::default();
        let mut succeeded = true;
        let mut failed = true;

        failures.record_flag("success", &mut succeeded, Ok::<(), io::Error>(()));
        failures.record_flag(
            "failure",
            &mut failed,
            Err(io::Error::other("still pending")),
        );

        assert!(!succeeded);
        assert!(failed);
        assert!(failures
            .finish("cleanup failed")
            .unwrap_err()
            .to_string()
            .contains("failure: still pending"));
    }

    #[cfg(windows)]
    #[test]
    fn pending_display_commands_keep_vt_mode_and_code_page_available_for_retry() {
        use winapi::um::handleapi::INVALID_HANDLE_VALUE;

        let mut state = super::TerminalState {
            mouse_capture: true,
            modes: Some(super::TerminalModes {
                input_handle: INVALID_HANDLE_VALUE,
                input_mode: 0,
                output_handle: INVALID_HANDLE_VALUE,
                output_mode: 0,
                input_pending: false,
                output_pending: true,
            }),
            code_pages: Some(super::ConsoleCodePages {
                output: 65001,
                pending: true,
            }),
            ..super::TerminalState::default()
        };
        let mut failures = CleanupFailures::default();

        super::restore_owned_state(&mut failures, &mut state);

        assert!(state.modes.as_ref().unwrap().output_pending);
        assert!(state.code_pages.as_ref().unwrap().pending);
        assert!(failures.finish("cleanup").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn failed_input_mode_restore_keeps_conin_handle_open_for_retry() {
        use winapi::um::handleapi::INVALID_HANDLE_VALUE;

        let mut state = super::TerminalState {
            modes: Some(super::TerminalModes {
                input_handle: INVALID_HANDLE_VALUE,
                input_mode: 0,
                output_handle: INVALID_HANDLE_VALUE,
                output_mode: 0,
                input_pending: true,
                output_pending: false,
            }),
            console_input: Some(super::ConsoleInputHandle {
                original: INVALID_HANDLE_VALUE,
                replacement: INVALID_HANDLE_VALUE,
                standard_handle_pending: true,
                close_pending: true,
            }),
            ..super::TerminalState::default()
        };
        let mut failures = CleanupFailures::default();

        super::restore_owned_state(&mut failures, &mut state);

        let console_input = state.console_input.as_ref().unwrap();
        assert!(console_input.standard_handle_pending);
        assert!(console_input.close_pending);
        assert!(failures.finish("cleanup").is_err());
    }

    #[test]
    fn primary_and_cleanup_errors_are_both_preserved() {
        let error = combine_results::<()>(
            Err(anyhow::anyhow!("draw failed")),
            Err(anyhow::anyhow!("restore failed")),
            "cleanup also failed",
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("draw failed"));
        assert!(error.contains("cleanup also failed: restore failed"));
    }

    #[test]
    fn rejects_non_interactive_tui_stdout() {
        let error = ensure_tui_stdout_value(false).unwrap_err();

        assert!(error
            .to_string()
            .contains("TUI mode requires an interactive terminal"));
    }
}
