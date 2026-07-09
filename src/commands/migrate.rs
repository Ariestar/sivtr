//! `sivtr migrate` — re-key legacy workspace dirs to the current scheme.
//!
//! Thin wrapper over [`sivtr_core::workspace::migrate_workspace_keys`]. Also run
//! automatically by `sivtr init`, so the standard setup flow covers upgrades;
//! this command exists for explicit/recovery use.

use anyhow::Result;

use crate::output;
use sivtr_core::workspace;

pub fn execute() -> Result<()> {
    let report = workspace::migrate_workspace_keys()?;
    if report.migrated.is_empty() && report.current == 0 && report.skipped.is_empty() {
        output::plain("no workspaces to migrate");
        return Ok(());
    }
    for (old, new) in &report.migrated {
        output::success(format!("{old} -> {new}"));
    }
    if !report.migrated.is_empty() {
        output::detail("migrated", report.migrated.len().to_string());
    }
    if report.current > 0 {
        output::detail("already current", report.current.to_string());
    }
    for (dir, reason) in &report.skipped {
        output::warning(format!("skipped {}: {reason}", dir.display()));
    }
    Ok(())
}
