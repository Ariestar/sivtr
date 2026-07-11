use std::collections::HashSet;

use anyhow::{Context, Result};
use sivtr_core::record::{WorkRecord, WorkRef, WorkRefBody};

use crate::cli::ZoomArgs;
use crate::commands::memory::show;
use crate::commands::memory::workset::{self, WorkSet};

pub fn execute(args: &ZoomArgs) -> Result<()> {
    let source = workset::load_source(&args.source, args.cwd.as_deref())?;
    let cwd = source.cwd();
    let (source_records, source_anchors) = source.into_parts();

    let base_context = args.context.unwrap_or(3);
    let before = args.before.unwrap_or(base_context);
    let after = args.after.unwrap_or(base_context);

    let expanded = if source_records.is_empty() || source_anchors.is_empty() {
        Vec::new()
    } else {
        let all_records = workset::load_context_records(&source_records, &source_anchors, &cwd)?;
        expand_around(
            &source_records,
            &source_anchors,
            &all_records,
            before,
            after,
        )?
    };

    let mut workset = WorkSet::new(cwd.display().to_string(), expanded);
    workset.save_last()?;
    if let Some(name) = args.save.as_deref() {
        workset.save_as(name)?;
    }
    show::print_workset(
        &workset,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )?;

    Ok(())
}

fn expand_around(
    source_records: &[WorkRecord],
    source_anchors: &[WorkRef],
    all_records: &[WorkRecord],
    before: usize,
    after: usize,
) -> Result<Vec<WorkRecord>> {
    let mut expanded = Vec::new();
    let mut seen = HashSet::new();

    for anchor in source_anchors {
        let source_ref = anchor.record_ref();
        let source = source_records
            .iter()
            .find(|record| record.work_ref.record_ref() == source_ref)
            .with_context(|| format!("No record found for ref `{source_ref}`"))?;
        let mut session_records = all_records
            .iter()
            .filter(|record| same_stream(source, record))
            .collect::<Vec<_>>();
        session_records.sort_by_key(|record| record.work_ref.record_index());

        let position = session_records
            .iter()
            .position(|record| record.work_ref.record_ref() == source_ref)
            .with_context(|| format!("No record found for ref `{source_ref}`"))?;
        let start = position.saturating_sub(before);
        let end = (position + after).min(session_records.len() - 1);

        for record in &session_records[start..=end] {
            let key = record.work_ref.record_ref().to_string();
            if seen.insert(key) {
                expanded.push((*record).clone());
            }
        }
    }

    Ok(expanded)
}

fn same_stream(left: &WorkRecord, right: &WorkRecord) -> bool {
    match (left.work_ref.body(), right.work_ref.body()) {
        (WorkRefBody::Terminal { .. }, WorkRefBody::Terminal { .. }) => {
            left.work_ref.session() == right.work_ref.session()
        }
        (
            WorkRefBody::Agent {
                provider: left_provider,
                ..
            },
            WorkRefBody::Agent {
                provider: right_provider,
                ..
            },
        ) => left_provider == right_provider && left.work_ref.session() == right.work_ref.session(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{
        WorkChannel, WorkOutcome, WorkRecordKind, WorkSessionRef, WorkSource, WorkStatus, WorkTime,
    };

    #[test]
    fn expand_around_expands_each_record_and_dedups_overlaps() {
        let records = (1..=5).map(test_terminal_record).collect::<Vec<_>>();
        let sources = vec![records[1].clone(), records[2].clone()];
        let anchors = sources
            .iter()
            .map(|record| record.work_ref.record_ref())
            .collect::<Vec<_>>();

        let expanded =
            expand_around(&sources, &anchors, &records, 1, 1).expect("expands around records");

        assert_eq!(
            expanded
                .iter()
                .map(|record| record.work_ref.to_string())
                .collect::<Vec<_>>(),
            vec![
                "terminal/session_1/1",
                "terminal/session_1/2",
                "terminal/session_1/3",
                "terminal/session_1/4",
            ]
        );
    }

    #[test]
    fn expand_around_clamps_session_edges() {
        let records = (1..=3).map(test_terminal_record).collect::<Vec<_>>();
        let sources = vec![records[0].clone()];
        let anchors = vec![sources[0]
            .work_ref
            .with_part(sivtr_core::record::WorkPartIo::Output, 1)];

        let expanded =
            expand_around(&sources, &anchors, &records, 5, 1).expect("expands around edge");

        assert_eq!(
            expanded
                .iter()
                .map(|record| record.work_ref.to_string())
                .collect::<Vec<_>>(),
            vec!["terminal/session_1/1", "terminal/session_1/2"]
        );
    }

    fn test_terminal_record(index: usize) -> WorkRecord {
        WorkRecord {
            schema_version: 1,
            work_ref: WorkRef::terminal_record("session_1", index),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: "session_1".to_string(),
                canonical_id: Some("session_1".to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: Some(WorkStatus {
                outcome: WorkOutcome::Success,
                exit_code: Some(0),
            }),
            title: format!("command {index}"),
            parts: Vec::new(),
        }
    }
}
