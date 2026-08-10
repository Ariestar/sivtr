//! `sivtr origin rename` — one rename path for every addressable source.

use anyhow::{Context, Result};

use crate::cli::{OriginAction, OriginCommand};
use crate::output;

pub fn execute(command: OriginCommand) -> Result<()> {
    match command.action {
        OriginAction::Rename { name, new_name } => rename(&name, &new_name),
    }
}

fn rename(name: &str, new_name: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let updated = crate::origins::rename(&cwd, name, new_name)?;
    output::success(format!("renamed origin `{name}` to `{updated}`"));
    Ok(())
}
