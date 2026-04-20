use anyhow::Result;
use std::io::{self, Read};

/// Read all content from stdin.
/// Used when sivtr is invoked as `cmd | sivtr`.
pub fn read_stdin() -> Result<String> {
    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer)
}
