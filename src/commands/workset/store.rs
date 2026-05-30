use anyhow::{bail, Context, Result};
use std::fs;
use std::path::PathBuf;

use super::WorkSet;

pub fn save_named(name: &str, set: &WorkSet) -> Result<()> {
    let path = set_path(name)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create WorkSet directory {}", parent.display()))?;
    }
    fs::write(&path, serde_json::to_string_pretty(set)?)
        .with_context(|| format!("Failed to write WorkSet @{} to {}", name, path.display()))
}

pub fn load_saved(name: &str) -> Result<WorkSet> {
    validate_name(name)?;
    let path = set_path(name)?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read WorkSet @{name} from {}", path.display()))?;
    let mut set: WorkSet = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse WorkSet @{name} from {}", path.display()))?;
    set.ensure_anchors();
    Ok(set)
}

pub fn delete_saved(name: &str) -> Result<()> {
    validate_name(name)?;
    let path = set_path(name)?;
    fs::remove_file(&path)
        .with_context(|| format!("Failed to delete WorkSet @{name} at {}", path.display()))
}

pub fn cleanup_saved() -> Result<usize> {
    let dir = sets_dir()?;
    if !dir.exists() {
        return Ok(0);
    }

    let mut removed = 0;
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("Failed to read WorkSet directory {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("Failed to read entry in {}", dir.display()))?;
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            fs::remove_file(entry.path()).with_context(|| {
                format!("Failed to delete WorkSet file {}", entry.path().display())
            })?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn set_path(name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(sets_dir()?.join(format!("{name}.json")))
}

fn sets_dir() -> Result<PathBuf> {
    let state_dir = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .or_else(dirs::config_dir)
        .ok_or_else(|| anyhow::anyhow!("Cannot determine state directory"))?;
    Ok(state_dir.join("sivtr").join("sets"))
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("WorkSet name cannot be empty");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        bail!("Invalid WorkSet name `{name}`; use letters, numbers, '-' or '_'");
    }
    Ok(())
}
