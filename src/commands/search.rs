use anyhow::Result;

use crate::cli::SearchArgs;
use crate::commands::{filter, show, workset};

pub fn execute(args: &SearchArgs) -> Result<()> {
    let source = workset::load_source(&args.source, args.cwd.as_deref())?;
    let mut workset = filter::apply_source(source, filter::FilterSpec::from_search_args(args)?)?;
    workset.save_last()?;
    if let Some(name) = args.save.as_deref() {
        workset.save_as(name)?;
    }
    show::print_workset(
        &workset,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}
