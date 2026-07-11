use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use sivtr_core::workspace::{self, WorkspaceMetadata};

use crate::cli::{WorkspaceAction, WorkspaceCommand};
use crate::output;

pub fn execute(command: WorkspaceCommand) -> Result<()> {
    match command.action.unwrap_or(WorkspaceAction::List) {
        WorkspaceAction::List => list(),
    }
}

fn list() -> Result<()> {
    let current = workspace::resolve_current_workspace()?.map(|paths| paths.key);
    let mut workspaces = workspace::list_workspaces()?;
    if workspaces.is_empty() {
        output::plain("no workspaces recorded yet");
        output::hint("run a command inside a git repo after `sivtr init`");
        return Ok(());
    }

    // Prefer current workspace first, then keep most-recently-seen order.
    if let Some(current_key) = current.as_deref() {
        workspaces.sort_by(|a, b| {
            let a_cur = a.key == current_key;
            let b_cur = b.key == current_key;
            b_cur
                .cmp(&a_cur)
                .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
        });
    }

    for meta in workspaces {
        let name = workspace_display_name(&meta);
        let marker = if current.as_deref() == Some(meta.key.as_str()) {
            "current"
        } else {
            "local"
        };
        output::detail(name, format!("[{marker}] {} ({})", meta.root, meta.key));
    }
    Ok(())
}

/// Human origin label for a local workspace: directory basename, lowercased.
pub fn workspace_display_name(meta: &WorkspaceMetadata) -> String {
    Path::new(&meta.root)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(meta.key.as_str())
        .to_ascii_lowercase()
}

/// Resolve a local workspace by origin label (`docs`, `sivtr`, …).
/// Prefers exact basename match; ambiguous names error.
pub fn resolve_local_workspace_by_name(name: &str) -> Result<Option<PathBuf>> {
    let needle = name.to_ascii_lowercase();
    let matches: Vec<_> = workspace::list_workspaces()?
        .into_iter()
        .filter(|meta| workspace_display_name(meta) == needle)
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

#[cfg(test)]
mod tests {
    use super::workspace_display_name;
    use sivtr_core::workspace::WorkspaceMetadata;

    #[test]
    fn display_name_uses_basename() {
        let meta = WorkspaceMetadata {
            key: "abc".into(),
            root: r"D:\Coding\sivtr".into(),
            created_at: "t".into(),
            last_seen_at: "t".into(),
        };
        assert_eq!(workspace_display_name(&meta), "sivtr");
    }
}
