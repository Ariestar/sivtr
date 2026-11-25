//! Windows pseudoconsole opened with no extra flags.
//!
//! `CreatePseudoConsole` flags stay 0, so the pipes carry the same VT bytes a
//! Unix pty does. `INHERIT_CURSOR` would block until we answer a cursor query,
//! and `WIN32_INPUT_MODE` would demand a different key encoding.

use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::ptr;

use winapi::shared::minwindef::{DWORD, FALSE};
use winapi::shared::winerror::S_OK;
use winapi::um::consoleapi::{ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::namedpipeapi::CreatePipe;
use winapi::um::processthreadsapi::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, PROCESS_INFORMATION,
    STARTUPINFOW,
};
use winapi::um::synchapi::WaitForSingleObject;
use winapi::um::winbase::{
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE, STARTF_USESTDHANDLES,
    STARTUPINFOEXW,
};
use winapi::um::wincontypes::{COORD, HPCON};
use winapi::um::winnt::HANDLE;

/// `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`.
const PSEUDOCONSOLE: usize = 0x0002_0016;

pub struct ConPty {
    pub output: File,
    pub input: File,
    pub session: std::sync::Arc<Session>,
}

pub struct Session {
    hpcon: HPCON,
    process: HANDLE,
}

unsafe impl Send for Session {}
unsafe impl Sync for Session {}

impl Session {
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = unsafe {
            ResizePseudoConsole(
                self.hpcon,
                COORD {
                    X: cols as i16,
                    Y: rows as i16,
                },
            )
        };
    }

    pub fn wait(&self) -> Result<i32> {
        let waited = unsafe { WaitForSingleObject(self.process, INFINITE) };
        if waited != 0 {
            anyhow::bail!("waiting for the shell failed: {waited}");
        }
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(self.process, &mut code) } == 0 {
            anyhow::bail!("failed to read the shell exit code");
        }
        Ok(code as i32)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            ClosePseudoConsole(self.hpcon);
            CloseHandle(self.process);
        }
    }
}

impl ConPty {
    pub fn spawn(
        program: &str,
        args: &[String],
        cols: u16,
        rows: u16,
        terminal_id: &str,
    ) -> Result<Self> {
        let (input_read, input_write) = pipe().context("conpty input pipe")?;
        let (output_read, output_write) = pipe().context("conpty output pipe")?;
        let mut hpcon: HPCON = ptr::null_mut();
        let hr = unsafe {
            CreatePseudoConsole(
                COORD {
                    X: cols as i16,
                    Y: rows as i16,
                },
                input_read,
                output_write,
                0,
                &mut hpcon,
            )
        };
        unsafe {
            CloseHandle(input_read);
            CloseHandle(output_write);
        }
        if hr != S_OK {
            unsafe {
                CloseHandle(input_write);
                CloseHandle(output_read);
            }
            anyhow::bail!("CreatePseudoConsole failed: {hr:#x}");
        }

        let spawned = spawn_process(program, args, hpcon, terminal_id);
        if let Err(error) = spawned {
            unsafe { ClosePseudoConsole(hpcon) };
            unsafe {
                CloseHandle(input_write);
                CloseHandle(output_read);
            }
            return Err(error);
        }
        let process = spawned?;
        Ok(Self {
            output: unsafe { File::from_raw_handle(output_read as RawHandle) },
            input: unsafe { File::from_raw_handle(input_write as RawHandle) },
            session: std::sync::Arc::new(Session { hpcon, process }),
        })
    }
}

fn pipe() -> Result<(HANDLE, HANDLE)> {
    let mut read = ptr::null_mut();
    let mut write = ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, ptr::null_mut(), 0) } == 0 {
        anyhow::bail!("CreatePipe failed: {}", std::io::Error::last_os_error());
    }
    Ok((read, write))
}

fn spawn_process(
    program: &str,
    args: &[String],
    hpcon: HPCON,
    terminal_id: &str,
) -> Result<HANDLE> {
    let mut size = 0usize;
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
    let mut attributes = vec![0u8; size];
    if unsafe { InitializeProcThreadAttributeList(attributes.as_mut_ptr().cast(), 1, 0, &mut size) }
        == 0
    {
        anyhow::bail!(
            "InitializeProcThreadAttributeList failed: {}",
            std::io::Error::last_os_error()
        );
    }
    if unsafe {
        UpdateProcThreadAttribute(
            attributes.as_mut_ptr().cast(),
            0,
            PSEUDOCONSOLE,
            hpcon as *mut _,
            std::mem::size_of::<HPCON>(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    } == 0
    {
        unsafe { DeleteProcThreadAttributeList(attributes.as_mut_ptr().cast()) };
        anyhow::bail!(
            "UpdateProcThreadAttribute failed: {}",
            std::io::Error::last_os_error()
        );
    }

    let mut command = wide(&command_line(program, args));
    let mut env = environment_block(terminal_id);
    let cwd =
        wide_path(&std::env::current_dir().context("failed to resolve the current directory")?);
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as DWORD;
    // Invalid stdio handles stop the child inheriting this process's console.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attributes.as_mut_ptr().cast();
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            ptr::null(),
            command.as_mut_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            FALSE,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env.as_mut_ptr().cast(),
            cwd.as_ptr(),
            &mut startup.StartupInfo as *mut STARTUPINFOW,
            &mut info,
        )
    };
    unsafe { DeleteProcThreadAttributeList(attributes.as_mut_ptr().cast()) };
    if created == 0 {
        anyhow::bail!(
            "failed to start {program}: {}",
            std::io::Error::last_os_error()
        );
    }
    unsafe { CloseHandle(info.hThread) };
    Ok(info.hProcess)
}

fn environment_block(terminal_id: &str) -> Vec<u16> {
    let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os().collect();
    for (key, value) in [
        ("SIVTR_PTY_PROXIED", "1"),
        ("SIVTR_PTY_PROXY", "1"),
        ("SIVTR_TERMINAL_ID", terminal_id),
    ] {
        vars.retain(|(name, _)| name != key);
        vars.push((key.into(), value.into()));
    }
    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn command_line(program: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(1 + args.len());
    parts.push(quote(program));
    parts.extend(args.iter().map(|arg| quote(arg)));
    parts.join(" ")
}

fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', slashes));
                slashes = 0;
                out.push(ch);
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

fn wide(text: &str) -> Vec<u16> {
    let mut encoded: Vec<u16> = OsStr::new(text).encode_wide().collect();
    encoded.push(0);
    encoded
}

fn wide_path(path: &std::path::Path) -> Vec<u16> {
    let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    encoded.push(0);
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn quotes_windows_command_lines() {
        assert_eq!(quote("pwsh"), "pwsh");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn plain_conpty_runs_a_command() {
        let pty = ConPty::spawn(
            "cmd.exe",
            &["/c".into(), "echo sivtr-conpty".into()],
            80,
            24,
            "conpty-test",
        )
        .expect("spawn");
        let ConPty {
            mut output,
            input,
            session,
        } = pty;
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                match output.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            buf
        });
        let code = session.wait().expect("wait");
        std::thread::sleep(std::time::Duration::from_millis(300));
        drop(session);
        drop(input);
        let collected = reader.join().expect("reader");
        let text = String::from_utf8_lossy(&collected);
        assert!(text.contains("sivtr-conpty"), "exit {code} output {text:?}");
        assert_eq!(code, 0);
    }
}
