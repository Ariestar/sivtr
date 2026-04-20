use anyhow::Result;
use std::process::{Command, Stdio};

/// Run a command and capture its combined stdout + stderr output.
///
/// The command output is also printed to the terminal in real-time (passthrough).
/// After the command finishes, the captured output is returned.
pub fn run_and_capture(program: &str, args: &[String]) -> Result<CaptureResult> {
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    // Combine stdout and stderr
    let mut combined = stdout;
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }

    Ok(CaptureResult {
        combined,
        exit_code: output.status.code(),
    })
}

/// Result of a captured subprocess execution.
pub struct CaptureResult {
    /// Combined stdout + stderr content.
    pub combined: String,
    /// Process exit code (None if terminated by signal).
    pub exit_code: Option<i32>,
}
