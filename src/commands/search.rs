use anyhow::{Context, Result};
use sivtr_core::record::{semantic_search, WorkRef};

use crate::cli::{SearchArgs, SearchFieldArg, SearchSortArg};
use crate::commands::workset::WorkSetSource;
use crate::commands::{filter, show, workset};

pub fn execute(args: &SearchArgs) -> Result<()> {
    let source = workset::load_source(&args.source, args.cwd.as_deref())?;
    if args.semantic {
        return execute_semantic_search(args, source);
    }

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

fn execute_semantic_search(args: &SearchArgs, source: WorkSetSource) -> Result<()> {
    let query = args
        .match_
        .as_deref()
        .context("--match is required with --semantic")?;

    let mut filter_args = args.clone();
    filter_args.match_ = None;
    filter_args.exclude = None;
    filter_args.kind = None;
    filter_args.in_field = SearchFieldArg::All;
    filter_args.latest = None;
    filter_args.limit = None;
    filter_args.sort = SearchSortArg::Newest;

    let filtered =
        filter::apply_source(source, filter::FilterSpec::from_search_args(&filter_args)?)?;
    let limit = args.limit.or(args.latest).unwrap_or(20);
    let results = semantic_search(&filtered.records, query, limit, |_| true);
    if results.is_empty() {
        println!("No semantic matches for `{query}`");
        return Ok(());
    }

    let anchors: Vec<WorkRef> = results
        .iter()
        .map(|result| result.record_ref.clone())
        .collect();
    let records = workset::records_for_anchors(&filtered.records, &anchors);
    let mut semantic = workset::WorkSet::with_anchors(filtered.cwd, records, anchors);
    semantic.save_last()?;
    if let Some(name) = args.save.as_deref() {
        semantic.save_as(name)?;
    }

    for result in &results {
        eprintln!(
            "{}  score:{}  [{}]",
            result.record_ref,
            result.score,
            result.matched_terms.join(", ")
        );
    }

    show::print_workset(
        &semantic,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}
