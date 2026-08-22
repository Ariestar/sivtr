use crate::cli::{WorkspaceAction, WorkspaceCommand};
use crate::output;
use anyhow::{Context, Result};

pub fn execute(command: WorkspaceCommand) -> Result<()> {
    match command.action.unwrap_or(WorkspaceAction::List) {
        WorkspaceAction::List => list(),
    }
}

/// List every addressable origin (local workspaces + remote mounts) through
/// the unified [`OriginRegistry`] — rendering never branches on kind.
fn list() -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let registry = crate::origins::collect(&cwd)?;
    if registry.is_empty() {
        output::plain("no origins recorded yet");
        output::hint("run a command inside a git repo after `sivtr init`");
        return Ok(());
    }

    for entry in registry.entries() {
        let origin = &entry.origin;
        let label = if origin.current {
            format!("{}:current", entry.reach.label())
        } else {
            entry.reach.label().to_string()
        };
        output::detail(origin.name.clone(), format!("[{label}] {}", origin.detail));
    }
    Ok(())
}
