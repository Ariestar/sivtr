use anyhow::{bail, Result};
use dialoguer::{Confirm, MultiSelect, Select};

/// Returns true if stdin is a TTY (interactive terminal).
pub fn is_interactive() -> bool {
    atty::is(atty::Stream::Stdin) && atty::is(atty::Stream::Stderr)
}

/// Fail if stdin/stderr are not attached to a TTY.
pub fn require_interactive(action: &str) -> Result<()> {
    if is_interactive() {
        return Ok(());
    }
    bail!("refusing to {action} non-interactively; re-run in a terminal");
}

/// Confirm a yes/no question. Defaults to `default` when input is empty.
/// Returns `Ok(default)` if stdin is not interactive.
pub fn confirm(prompt: &str, default: bool) -> Result<bool> {
    if !is_interactive() {
        return Ok(default);
    }
    Ok(Confirm::new()
        .with_prompt(prompt)
        .default(default)
        .show_default(true)
        .interact()?)
}

/// Select one item from a list. Returns the selected index.
/// Returns `Ok(default)` if stdin is not interactive.
pub fn select(prompt: &str, items: &[String], default: usize) -> Result<usize> {
    if !is_interactive() {
        return Ok(default.min(items.len().saturating_sub(1)));
    }
    if items.is_empty() {
        bail!("no items to select from");
    }
    Ok(Select::new()
        .with_prompt(prompt)
        .items(items)
        .default(default.min(items.len() - 1))
        .interact()?)
}

/// Select multiple items from a list. Returns the selected indices.
/// Returns `Ok(defaults)` if stdin is not interactive.
pub fn multi_select(prompt: &str, items: &[String], defaults: &[usize]) -> Result<Vec<usize>> {
    if !is_interactive() {
        return Ok(defaults
            .iter()
            .copied()
            .filter(|i| *i < items.len())
            .collect());
    }
    let bool_defaults: Vec<bool> = (0..items.len()).map(|i| defaults.contains(&i)).collect();
    Ok(MultiSelect::new()
        .with_prompt(prompt)
        .items(items)
        .defaults(&bool_defaults)
        .interact()?)
}

