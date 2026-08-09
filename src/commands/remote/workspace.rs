use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use sivtr_core::workspace;

use crate::cli::{WorkspaceAction, WorkspaceCommand};
use crate::output;

pub fn execute(command: WorkspaceCommand) -> Result<()> {
    match command.action.unwrap_or(WorkspaceAction::List) {
        WorkspaceAction::List => list(),
    }
}

/// List every addressable origin (local workspaces + remote mounts + cloud)
/// through the unified [`OriginRegistry`] — rendering never branches on kind.
fn list() -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let registry = crate::origins::collect(&cwd)?;
    if registry.all().is_empty() {
        output::plain("no origins recorded yet");
        output::hint("run a command inside a git repo after `sivtr init`");
        return Ok(());
    }

    for origin in registry.all() {
        let label = if origin.current {
            format!("{}:current", origin.kind.label())
        } else {
            origin.kind.label().to_string()
        };
        output::detail(origin.name.clone(), format!("[{label}] {}", origin.detail));
    }
    Ok(())
}

/// Resolve a local workspace by origin label (`docs`, `sivtr`, …).
/// Prefers exact basename match; ambiguous names error.
pub fn resolve_local_workspace_by_name(name: &str) -> Result<Option<PathBuf>> {
    let needle = name.to_ascii_lowercase();
    let matches: Vec<_> = workspace::list_workspaces()?
        .into_iter()
        .filter(|meta| workspace::workspace_display_name(meta) == needle)
        .collect();
    match matches.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(PathBuf::from(&only.root))),
        many => {
            let roots = many
                .iter()
                .map(|meta| meta.root.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("ambiguous local workspace `{name}`; matches: {roots}")
        }
    }
}
