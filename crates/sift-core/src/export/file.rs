use anyhow::Result;
use std::path::Path;

/// Save text content to a file.
pub fn save_to_file(text: &str, path: &Path) -> Result<()> {
    std::fs::write(path, text)?;
    Ok(())
}
