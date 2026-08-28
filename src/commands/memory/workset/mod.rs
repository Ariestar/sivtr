mod source;
pub mod store;

pub(crate) use source::{
    load_context_records, query, query_sources, run_on_share, QuerySource, QuerySourceResult,
};
pub(crate) use store::{cleanup_saved, delete_saved, list_saved, load_saved, save_named};

use anyhow::{bail, Context, Result};
use sivtr_core::query::{load_session_records, LoadMode};
use sivtr_core::record::{WorkPath, WorkRecord, WorkRef};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub use crate::workset::{WorkSet, WORKSET_SCHEMA_VERSION};

fn apply_selection(mut set: WorkSet, selection: WorkSetSelection) -> WorkSet {
    let WorkSetSelection::Indices(indices) = selection else {
        return set;
    };

    let anchors = indices
        .into_iter()
        .map(|index| set.anchors()[index - 1].clone())
        .collect::<Vec<_>>();
    let records = records_for_anchors(set.records(), &anchors);
    set.replace_selection(records, anchors);
    set
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkSetSelection {
    All,
    Indices(Vec<usize>),
}

/// Cache namespace for a record's session file, used by [`materialize_parts`]
/// to pick the right cache view when re-loading full records.
fn session_namespace(path: &WorkPath) -> Option<&'static str> {
    match path {
        WorkPath::Agent { provider, .. } => Some(provider.command_name()),
        WorkPath::Terminal { .. } => Some("terminal"),
    }
}

impl WorkSet {
    /// Fill in `parts` for any light-loaded record (empty `parts`) whose
    /// session file path is known.  Each session file is loaded once (full
    /// view), then matching records are patched in place.  Records without a
    /// session path (stdin sets) are already complete and stay untouched.
    pub fn materialize_parts(&mut self) -> Result<()> {
        // Group light records by their session file path, then load each
        // session's full records once and patch matching parts back.
        let mut needed: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, record) in self.records().iter().enumerate() {
            if !record.parts.is_empty() {
                continue;
            }
            let Some(path) = record.session.path.as_deref() else {
                continue;
            };
            needed.entry(path.to_string()).or_default().push(index);
        }

        for (path, indices) in &needed {
            // Any record in the group gives us the namespace; pick the first.
            let namespace = session_namespace(&self.records()[indices[0]].work_ref.path);
            let Some(namespace) = namespace else {
                continue;
            };
            let full = load_session_records(namespace, Path::new(path), LoadMode::Full)
                .with_context(|| format!("Failed to load full session {path} for {namespace}"))?;
            for index in indices {
                if let Some(record) = self.records_mut().get_mut(*index) {
                    if let Some(full_record) = full
                        .iter()
                        .find(|r| r.work_ref.path.index() == record.work_ref.path.index())
                    {
                        record.parts = full_record.parts.clone();
                    }
                }
            }
        }
        Ok(())
    }

    pub fn save_as(&mut self, name: &str) -> Result<()> {
        store::validate_name(name)?;
        self.materialize_parts()?;
        self.name = Some(name.to_string());
        save_named(name, self)
    }

    pub fn save_last(&self) -> Result<()> {
        let mut set = self.clone();
        set.materialize_parts()?;
        save_named("last", &set)
    }
}

pub fn records_for_anchors(records: &[WorkRecord], anchors: &[WorkRef]) -> Vec<WorkRecord> {
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for anchor in anchors {
        let record_ref = anchor.whole();
        if !seen.insert(record_ref.to_string()) {
            continue;
        }
        if let Some(record) = records
            .iter()
            .find(|record| record.work_ref.whole() == record_ref)
        {
            selected.push(record.clone());
        }
    }
    selected
}

pub fn record_for_anchor<'a>(
    records: &'a [WorkRecord],
    anchor: &WorkRef,
) -> Option<&'a WorkRecord> {
    find_record([records], anchor)
}

pub fn find_record<'a, I>(record_sets: I, anchor: &WorkRef) -> Option<&'a WorkRecord>
where
    I: IntoIterator<Item = &'a [WorkRecord]>,
{
    let record_ref = anchor.whole();
    record_sets
        .into_iter()
        .flat_map(|records| records.iter())
        .find(|record| record.work_ref.whole() == record_ref)
}

pub fn require_record<'a, I>(record_sets: I, anchor: &WorkRef) -> Result<&'a WorkRecord>
where
    I: IntoIterator<Item = &'a [WorkRecord]>,
{
    find_record(record_sets, anchor)
        .with_context(|| format!("No record found for ref `{}`", anchor.whole()))
}

pub fn load_reference(reference: &str) -> Result<WorkSet> {
    let parsed = parse_reference(reference)?;
    let set = load_saved(parsed.name)?;
    validate_selection(reference, &set, &parsed.selection)?;
    Ok(apply_selection(set, parsed.selection))
}

struct ParsedWorkSetReference<'a> {
    name: &'a str,
    selection: WorkSetSelection,
}

fn parse_reference(reference: &str) -> Result<ParsedWorkSetReference<'_>> {
    let body = reference
        .strip_prefix('@')
        .ok_or_else(|| anyhow::anyhow!("WorkSet reference must start with @"))?;
    if let Some(open) = body.find('[') {
        if !body.ends_with(']') {
            bail!("Invalid WorkSet reference `{reference}`; missing closing ]");
        }
        let name = &body[..open];
        store::validate_name(name)?;
        let selector = &body[open + 1..body.len() - 1];
        if selector.is_empty() {
            bail!("Invalid WorkSet reference `{reference}`");
        }
        let selection = parse_selector(selector, reference)?;
        Ok(ParsedWorkSetReference { name, selection })
    } else {
        store::validate_name(body)?;
        Ok(ParsedWorkSetReference {
            name: body,
            selection: WorkSetSelection::All,
        })
    }
}

fn parse_selector(selector: &str, reference: &str) -> Result<WorkSetSelection> {
    let mut indices = Vec::new();
    for segment in selector.split(',') {
        if segment.is_empty() {
            bail!("Invalid WorkSet reference `{reference}`; empty selector segment");
        }
        if let Some((start, end)) = segment.split_once("..") {
            let start = parse_index(start, reference)?;
            let end = parse_index(end, reference)?;
            if start > end {
                bail!("Invalid WorkSet reference `{reference}`; range start must be <= end");
            }
            indices.extend(start..=end);
        } else {
            indices.push(parse_index(segment, reference)?);
        }
    }
    Ok(WorkSetSelection::Indices(indices))
}

fn parse_index(value: &str, reference: &str) -> Result<usize> {
    let index = value.parse::<usize>().with_context(|| {
        format!("Invalid WorkSet reference `{reference}`; index must be a positive integer")
    })?;
    if index == 0 {
        bail!("Invalid WorkSet reference `{reference}`; index must be 1-based");
    }
    Ok(index)
}

fn validate_selection(reference: &str, set: &WorkSet, selection: &WorkSetSelection) -> Result<()> {
    match selection {
        WorkSetSelection::All => Ok(()),
        WorkSetSelection::Indices(indices) => {
            for index in indices {
                if *index > set.anchors().len() {
                    bail!(
                        "Invalid WorkSet reference `{reference}`; index {index} exceeds WorkSet length {}",
                        set.anchors().len()
                    );
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkRecord, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
    };

    fn record(index: usize) -> WorkRecord {
        WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
            work_ref: format!("terminal/session_1/{index}")
                .parse()
                .expect("valid work ref"),
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
            status: None,
            title: format!("record {index}"),
            parts: (1..=2)
                .map(|seq| WorkPart {
                    seq,
                    occurred_at: None,
                    data: sivtr_core::record::WorkPartData::Output {
                        content: format!("record {index} part {seq}"),
                        ansi: None,
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn parses_discrete_and_range_selectors_in_order() {
        let selection = parse_selector("1,3..5,2", "@hits[1,3..5,2]").expect("selector parses");
        assert_eq!(selection, WorkSetSelection::Indices(vec![1, 3, 4, 5, 2]));
    }

    #[test]
    fn selected_keeps_discrete_selector_order() {
        let set = WorkSet::new(".", (1..=5).map(record).collect());
        let selected = apply_selection(set, WorkSetSelection::Indices(vec![3, 1, 5]));

        let refs = selected
            .anchors()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            refs,
            vec![
                "terminal/session_1/3",
                "terminal/session_1/1",
                "terminal/session_1/5"
            ]
        );
    }

    #[test]
    fn selected_keeps_part_anchor_order() {
        let records = vec![record(1), record(2)];
        let anchors = vec![
            records[1].work_ref.with_part(1),
            records[0].work_ref.with_part(1),
        ];
        let set = WorkSet::from_parts(".", records, anchors);
        let selected = apply_selection(set, WorkSetSelection::Indices(vec![2, 1]));

        let refs = selected
            .anchors()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            refs,
            vec!["terminal/session_1/1/p1", "terminal/session_1/2/p1"]
        );
    }

    #[test]
    fn whole_anchor_covers_and_replaces_parts() {
        let first = record(1);
        let mut set = WorkSet::from_parts(".", vec![first.clone()], vec![]);
        set.include_parts(first.clone(), [1]);
        assert!(set.contains(&first.work_ref.with_part(1)));
        set.include_whole(first.clone());
        assert_eq!(set.anchors(), vec![first.work_ref.whole()]);
        assert!(set.contains(&first.work_ref.with_part(1)));
    }

    #[test]
    fn part_anchors_deduplicate_without_collapsing() {
        let first = record(1);
        let mut set = WorkSet::new(".", Vec::new());
        set.include_parts(first.clone(), [1, 1]);
        assert_eq!(set.anchors(), vec![first.work_ref.with_part(1)]);
    }

    #[test]
    fn complete_part_selection_is_canonical_whole() {
        let first = record(1);
        let mut set = WorkSet::new(".", Vec::new());
        set.include_parts(first.clone(), [1, 2, 1]);
        assert_eq!(set.anchors(), vec![first.work_ref.whole()]);
        assert!(set.contains(&first.work_ref.with_part(2)));
    }

    #[test]
    fn toggling_whole_replaces_narrow_selection() {
        let first = record(1);
        let mut set = WorkSet::new(".", Vec::new());
        set.include_parts(first.clone(), [1]);
        set.toggle_whole(first.clone());
        assert_eq!(set.anchors(), vec![first.work_ref.whole()]);
        set.toggle_whole(first.clone());
        assert!(set.anchors().is_empty());
    }

    #[test]
    fn unique_anchors_preserves_first_occurrence() {
        let records = [record(1), record(2)];
        let anchors = vec![
            records[0].work_ref.whole(),
            records[1].work_ref.whole(),
            records[0].work_ref.whole(),
        ];

        let unique = crate::commands::memory::var::unique_anchors(anchors)
            .into_iter()
            .map(|anchor| anchor.to_string())
            .collect::<Vec<_>>();

        assert_eq!(unique, vec!["terminal/session_1/1", "terminal/session_1/2"]);
    }

    #[test]
    fn rejects_empty_discrete_selector_segment() {
        let error = parse_selector("1,,2", "@hits[1,,2]").expect_err("selector rejects empty");
        assert!(error.to_string().contains("empty selector segment"));
    }
}
