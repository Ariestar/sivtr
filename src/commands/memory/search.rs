use anyhow::Result;

use crate::cli::SearchArgs;
use crate::commands::memory::workset::WorkSet;
use crate::commands::memory::{filter, show, workset};

pub fn execute(args: &SearchArgs) -> Result<()> {
    let workset = run(args)?;
    show::print_workset(
        &workset,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

/// Unified query for search: local and remote both run load+filter at the data owner.
pub fn run(args: &SearchArgs) -> Result<WorkSet> {
    let mut workset = workset::query(
        &args.source,
        filter::Filter::from_search_args(args)?,
        args.cwd.as_deref(),
    )?;
    workset.save_last()?;
    if let Some(name) = args.save.as_deref() {
        workset.save_as(name)?;
    }
    Ok(workset)
}
