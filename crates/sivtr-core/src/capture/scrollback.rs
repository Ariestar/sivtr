use anyhow::{Context, Result};
use std::env;

/// Get the session log path used by the shell hook.
pub fn session_log_path() -> std::path::PathBuf {
    if let Ok(path) = env::var("SIVTR_SESSION_LOG") {
        return std::path::PathBuf::from(path);
    }

    if let Ok(path) = env::var("SIFT_SESSION_LOG") {
        return std::path::PathBuf::from(path);
    }

    let base_dir = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let current = base_dir.join("sivtr").join("session.log");
    if current.exists() {
        return current;
    }

    let legacy = base_dir.join("sift").join("session.log");
    if legacy.exists() {
        return legacy;
    }

    current
}

/// Get the flush state file path (derived from session log path).
pub fn flush_state_path() -> std::path::PathBuf {
    session_log_path().with_extension("state")
}

/// Read the current session log (populated by `sivtr flush` shell hook).
pub fn capture_scrollback() -> Result<Option<String>> {
    let log = session_log_path();
    if log.exists() {
        let content = std::fs::read_to_string(&log).context("Failed to read session log")?;
        if !content.trim().is_empty() {
            return Ok(Some(content));
        }
    }
    Ok(None)
}

/// Read the current console buffer with ANSI color codes.
/// Used by `sivtr flush` to incrementally capture output.
#[cfg(windows)]
pub fn capture_console_buffer() -> Result<String> {
    capture_windows_console()
}

/// Read the Windows console screen buffer via Win32 API,
/// including color attributes converted to ANSI escape codes.
#[cfg(windows)]
fn capture_windows_console() -> Result<String> {
    use std::ptr;

    use winapi::um::fileapi::{CreateFileW, OPEN_EXISTING};
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::processenv::GetStdHandle;
    use winapi::um::winbase::STD_OUTPUT_HANDLE;
    use winapi::um::wincon::{
        GetConsoleScreenBufferInfo, ReadConsoleOutputAttribute, ReadConsoleOutputCharacterW,
        CONSOLE_SCREEN_BUFFER_INFO, COORD,
    };
    use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE};

    unsafe {
        // In PowerShell and ConPTY-based terminals, a child process's STDOUT is often a pipe.
        // Opening CONOUT$ gives us the active console screen buffer instead of the redirected handle.
        let conout: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let conout_handle = CreateFileW(
            conout.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null_mut(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        );
        let (handle, should_close) =
            if !conout_handle.is_null() && conout_handle != INVALID_HANDLE_VALUE {
                (conout_handle, true)
            } else {
                let std_handle = GetStdHandle(STD_OUTPUT_HANDLE);
                (std_handle, false)
            };

        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            anyhow::bail!("Failed to get console handle");
        }

        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(handle, &mut info) == 0 {
            if should_close {
                CloseHandle(handle);
            }
            anyhow::bail!("Failed to get console buffer info");
        }

        let width = info.dwSize.X as usize;
        if width == 0 {
            return Ok(String::new());
        }
        let cursor_row = info.dwCursorPosition.Y as usize;
        let content_rows = (cursor_row + 1).min(info.dwSize.Y as usize);
        let default_attr = info.wAttributes;

        let mut result = String::with_capacity(content_rows * (width + 20));
        let mut char_buf = vec![0u16; width];
        let mut attr_buf = vec![0u16; width];

        for row in 0..content_rows {
            let mut chars_read: u32 = 0;
            let mut attrs_read: u32 = 0;
            let coord = COORD {
                X: 0,
                Y: row as i16,
            };

            ReadConsoleOutputCharacterW(
                handle,
                char_buf.as_mut_ptr(),
                width as u32,
                coord,
                &mut chars_read,
            );
            ReadConsoleOutputAttribute(
                handle,
                attr_buf.as_mut_ptr(),
                width as u32,
                coord,
                &mut attrs_read,
            );

            let line_str = String::from_utf16_lossy(&char_buf[..chars_read as usize]);
            let trimmed = line_str.trim_end();

            if trimmed.is_empty() {
                result.push('\n');
                continue;
            }

            let trimmed_len = trimmed.len();
            let mut prev_attr = default_attr;
            let mut byte_idx: usize = 0;
            let mut cell_idx: usize = 0;

            for ch in line_str.chars() {
                if byte_idx >= trimmed_len {
                    break;
                }

                let attr = if cell_idx < attrs_read as usize {
                    attr_buf[cell_idx]
                } else {
                    default_attr
                };

                // Emit ANSI code when attribute changes
                if attr != prev_attr || cell_idx == 0 {
                    if attr != default_attr {
                        result.push_str(&win_attr_to_ansi(attr));
                    } else if prev_attr != default_attr {
                        result.push_str("\x1b[0m");
                    }
                    prev_attr = attr;
                }

                result.push(ch);
                byte_idx += ch.len_utf8();

                // Wide chars occupy 2 console cells
                if unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) > 1 {
                    cell_idx += 2;
                } else {
                    cell_idx += 1;
                }
            }

            // Reset at end of line if we had non-default color
            if prev_attr != default_attr {
                result.push_str("\x1b[0m");
            }

            result.push('\n');
        }

        if should_close {
            CloseHandle(handle);
        }

        let trimmed = result.trim_end_matches('\n');
        Ok(trimmed.to_string())
    }
}

/// Convert a Windows console attribute word to an ANSI SGR escape sequence.
#[cfg(windows)]
fn win_attr_to_ansi(attr: u16) -> String {
    // Windows console color bits (BGR order):
    //   Foreground: bits 0-3 (B=0x01, G=0x02, R=0x04, Intensity=0x08)
    //   Background: bits 4-7 (B=0x10, G=0x20, R=0x40, Intensity=0x80)
    //
    // ANSI standard color order (index 0-7):
    //   0=black 1=red 2=green 3=yellow 4=blue 5=magenta 6=cyan 7=white

    // Map Windows BGR bits (0-7) to ANSI color index
    const WIN_TO_ANSI: [u8; 8] = [0, 4, 2, 6, 1, 5, 3, 7];

    let fg_bits = (attr & 0x07) as usize;
    let fg_intense = (attr & 0x08) != 0;
    let bg_bits = ((attr >> 4) & 0x07) as usize;
    let bg_intense = (attr & 0x80) != 0;

    let fg_ansi = WIN_TO_ANSI[fg_bits];
    let bg_ansi = WIN_TO_ANSI[bg_bits];

    let mut codes = Vec::new();

    // Foreground
    if fg_intense {
        codes.push(format!("{}", 90 + fg_ansi)); // bright fg
    } else {
        codes.push(format!("{}", 30 + fg_ansi)); // normal fg
    }

    // Background (only if non-black)
    if bg_bits != 0 || bg_intense {
        if bg_intense {
            codes.push(format!("{}", 100 + bg_ansi));
        } else {
            codes.push(format!("{}", 40 + bg_ansi));
        }
    }

    format!("\x1b[{}m", codes.join(";"))
}
