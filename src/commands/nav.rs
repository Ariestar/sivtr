use anyhow::{bail, Context, Result};
use sivtr_core::record::{WorkPartIo, WorkRecord, WorkRef, WorkRefBody, WorkRefTarget};

use crate::cli::NavArgs;
use crate::commands::show;
use crate::commands::var;
use crate::commands::workset::{self, WorkSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Parent,
    Child(usize),
    Sibling(isize),
    Window { start: isize, end: isize },
    Session,
}

pub fn execute(args: &NavArgs) -> Result<()> {
    let source = workset::load_source(&args.source, args.cwd.as_deref())?;
    let cwd = source.cwd();
    let (source_records, source_anchors) = source.into_parts();
    let all_records = workset::load_context_records(&source_records, &source_anchors, &cwd)?;
    let anchors = navigate(&source_records, &source_anchors, &all_records, &args.motion)?;
    let records = workset::records_for_anchors(&all_records, &anchors);
    let set = WorkSet::with_anchors(cwd.display().to_string(), records, anchors);
    set.save_last()?;
    show::print_workset(
        &set,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

fn navigate(
    source_records: &[WorkRecord],
    source_anchors: &[WorkRef],
    all_records: &[WorkRecord],
    motion: &str,
) -> Result<Vec<WorkRef>> {
    let steps = parse_motion(motion)?;
    let mut anchors = source_anchors.to_vec();
    for step in steps {
        anchors = apply_step(source_records, &anchors, all_records, step)?;
        anchors = var::unique_anchors(anchors);
    }
    Ok(anchors)
}

fn apply_step(
    source_records: &[WorkRecord],
    anchors: &[WorkRef],
    all_records: &[WorkRecord],
    step: Step,
) -> Result<Vec<WorkRef>> {
    let mut result = Vec::new();
    for anchor in anchors {
        match step {
            Step::Parent => result.extend(parent(anchor, source_records, all_records)?),
            Step::Child(index) => result.extend(child(anchor, source_records, all_records, index)?),
            Step::Sibling(offset) => {
                result.extend(sibling(anchor, source_records, all_records, offset)?)
            }
            Step::Window { start, end } => {
                result.extend(window(anchor, source_records, all_records, start, end)?)
            }
            Step::Session => result.extend(session(anchor, source_records, all_records)?),
        }
    }
    Ok(result)
}

fn parent(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
) -> Result<Vec<WorkRef>> {
    match anchor.target() {
        WorkRefTarget::Part { .. } | WorkRefTarget::Line(_) => Ok(vec![anchor.record_ref()]),
        WorkRefTarget::Record => session(anchor, source_records, all_records),
    }
}

fn child(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
    index: usize,
) -> Result<Vec<WorkRef>> {
    if index == 0 {
        bail!("child index must be 1-based");
    }
    match anchor.target() {
        WorkRefTarget::Record => {
            let record = record_for_anchor(anchor, source_records, all_records)?;
            let Some(part) = record.parts.get(index - 1) else {
                return Ok(Vec::new());
            };
            Ok(vec![record.work_ref.with_part(part.io, part.index)])
        }
        WorkRefTarget::Line(_) | WorkRefTarget::Part { .. } => Ok(Vec::new()),
    }
}

fn sibling(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
    offset: isize,
) -> Result<Vec<WorkRef>> {
    match anchor.target() {
        WorkRefTarget::Record => {
            let record = record_for_anchor(anchor, source_records, all_records)?;
            let session_records = session_records_for(record, all_records);
            let Some(position) = session_records
                .iter()
                .position(|candidate| candidate.work_ref.record_ref() == anchor.record_ref())
            else {
                return Ok(Vec::new());
            };
            let Some(target) = offset_index(position, offset, session_records.len()) else {
                return Ok(Vec::new());
            };
            Ok(vec![session_records[target].work_ref.record_ref()])
        }
        WorkRefTarget::Part { io, index } => {
            let record = record_for_anchor(anchor, source_records, all_records)?;
            let part_positions = parts_with_io(record, io);
            let Some(position) = part_positions
                .iter()
                .position(|part_index| *part_index == index)
            else {
                return Ok(Vec::new());
            };
            let Some(target) = offset_index(position, offset, part_positions.len()) else {
                return Ok(Vec::new());
            };
            Ok(vec![record.work_ref.with_part(io, part_positions[target])])
        }
        WorkRefTarget::Line(line) => {
            let Some(target) = offset_one_based(line, offset) else {
                return Ok(Vec::new());
            };
            Ok(vec![anchor.record_ref().with_line(target)])
        }
    }
}

fn window(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
    start: isize,
    end: isize,
) -> Result<Vec<WorkRef>> {
    if start > end {
        bail!("window start must be <= end");
    }
    match anchor.target() {
        WorkRefTarget::Record => {
            let record = record_for_anchor(anchor, source_records, all_records)?;
            let session_records = session_records_for(record, all_records);
            let position = session_records
                .iter()
                .position(|candidate| candidate.work_ref.record_ref() == anchor.record_ref())
                .with_context(|| format!("No record found for ref `{}`", anchor.record_ref()))?;
            let start = clamp_offset(position, start, session_records.len());
            let end = clamp_offset(position, end, session_records.len());
            Ok(session_records[start..=end]
                .iter()
                .map(|record| record.work_ref.record_ref())
                .collect())
        }
        WorkRefTarget::Part { io, index } => {
            let record = record_for_anchor(anchor, source_records, all_records)?;
            let part_positions = parts_with_io(record, io);
            let position = part_positions
                .iter()
                .position(|part_index| *part_index == index)
                .with_context(|| format!("No part found for ref `{anchor}`"))?;
            let start = clamp_offset(position, start, part_positions.len());
            let end = clamp_offset(position, end, part_positions.len());
            Ok(part_positions[start..=end]
                .iter()
                .map(|part_index| record.work_ref.with_part(io, *part_index))
                .collect())
        }
        WorkRefTarget::Line(line) => {
            let start = offset_one_based(line, start).unwrap_or(1);
            let end = offset_one_based(line, end).unwrap_or(1).max(start);
            Ok((start..=end)
                .map(|line| anchor.record_ref().with_line(line))
                .collect())
        }
    }
}

fn session(
    anchor: &WorkRef,
    source_records: &[WorkRecord],
    all_records: &[WorkRecord],
) -> Result<Vec<WorkRef>> {
    let record = record_for_anchor(anchor, source_records, all_records)?;
    Ok(session_records_for(record, all_records)
        .into_iter()
        .map(|record| record.work_ref.record_ref())
        .collect())
}

fn record_for_anchor<'a>(
    anchor: &WorkRef,
    source_records: &'a [WorkRecord],
    all_records: &'a [WorkRecord],
) -> Result<&'a WorkRecord> {
    let record_ref = anchor.record_ref();
    source_records
        .iter()
        .chain(all_records.iter())
        .find(|record| record.work_ref.record_ref() == record_ref)
        .with_context(|| format!("No record found for ref `{record_ref}`"))
}

fn session_records_for<'a>(
    record: &WorkRecord,
    all_records: &'a [WorkRecord],
) -> Vec<&'a WorkRecord> {
    let mut records = all_records
        .iter()
        .filter(|candidate| same_stream(record, candidate))
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record.work_ref.record_index());
    records
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

fn parts_with_io(record: &WorkRecord, io: WorkPartIo) -> Vec<usize> {
    record
        .parts
        .iter()
        .filter(|part| part.io == io)
        .map(|part| part.index)
        .collect()
}

fn offset_index(position: usize, offset: isize, len: usize) -> Option<usize> {
    position
        .checked_add_signed(offset)
        .filter(|target| *target < len)
}

fn offset_one_based(position: usize, offset: isize) -> Option<usize> {
    position
        .checked_add_signed(offset)
        .filter(|target| *target > 0)
}

fn clamp_offset(position: usize, offset: isize, len: usize) -> usize {
    position
        .saturating_add_signed(offset)
        .min(len.saturating_sub(1))
}

fn parse_motion(motion: &str) -> Result<Vec<Step>> {
    if motion.is_empty() {
        bail!("motion cannot be empty");
    }

    let chars = motion.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut steps = Vec::new();
    while index < chars.len() {
        match chars[index] {
            '<' => {
                steps.push(Step::Parent);
                index += 1;
            }
            '>' => {
                index += 1;
                let (value, next) = parse_usize(&chars, index, "child")?;
                steps.push(Step::Child(value));
                index = next;
            }
            '+' | '-' => {
                let (value, next) = parse_isize(&chars, index, "sibling")?;
                steps.push(Step::Sibling(value));
                index = next;
            }
            '[' => {
                let close = chars[index..]
                    .iter()
                    .position(|ch| *ch == ']')
                    .map(|offset| index + offset)
                    .ok_or_else(|| anyhow::anyhow!("window motion missing closing ]"))?;
                let body = chars[index + 1..close].iter().collect::<String>();
                let (start, end) = body
                    .split_once("..")
                    .ok_or_else(|| anyhow::anyhow!("window motion must use A..B"))?;
                let start = parse_signed_literal(start, "window start")?;
                let end = parse_signed_literal(end, "window end")?;
                if start > end {
                    bail!("window start must be <= end");
                }
                steps.push(Step::Window { start, end });
                index = close + 1;
            }
            '~' => {
                steps.push(Step::Session);
                index += 1;
            }
            other => bail!("invalid motion token `{other}`"),
        }
    }
    Ok(steps)
}

fn parse_usize(chars: &[char], index: usize, label: &str) -> Result<(usize, usize)> {
    let start = index;
    let mut end = index;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    if start == end {
        bail!("{label} motion requires a number");
    }
    let value = chars[start..end]
        .iter()
        .collect::<String>()
        .parse::<usize>()?;
    Ok((value, end))
}

fn parse_isize(chars: &[char], index: usize, label: &str) -> Result<(isize, usize)> {
    let start = index;
    let mut end = index + 1;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    if start + 1 == end {
        bail!("{label} motion requires a number");
    }
    let value = chars[start..end]
        .iter()
        .collect::<String>()
        .parse::<isize>()?;
    Ok((value, end))
}

fn parse_signed_literal(value: &str, label: &str) -> Result<isize> {
    if value.is_empty() {
        bail!("{label} is empty");
    }
    value
        .parse::<isize>()
        .with_context(|| format!("invalid {label} `{value}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartKind, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
    };

    #[test]
    fn parses_motion_steps() {
        assert_eq!(
            parse_motion("<+1>2[-1..+2]~").expect("parse motion"),
            vec![
                Step::Parent,
                Step::Sibling(1),
                Step::Child(2),
                Step::Window { start: -1, end: 2 },
                Step::Session,
            ]
        );
        assert!(parse_motion(">").is_err());
        assert!(parse_motion(">0").is_ok());
        assert!(parse_motion("[+2..-1]").is_err());
    }

    #[test]
    fn navigates_parent_child_sibling_and_window() {
        let records = (1..=4).map(test_record).collect::<Vec<_>>();
        let start = vec![records[1].work_ref.with_part(WorkPartIo::Output, 2)];

        assert_refs(
            navigate(&records, &start, &records, "<").expect("parent"),
            &["terminal/session_1/2"],
        );
        assert_refs(
            navigate(&records, &start, &records, "<+1>1").expect("next record first child"),
            &["terminal/session_1/3/i/1"],
        );
        assert_refs(
            navigate(&records, &start, &records, "<[-1..+1]").expect("record window"),
            &[
                "terminal/session_1/1",
                "terminal/session_1/2",
                "terminal/session_1/3",
            ],
        );
        assert_refs(
            navigate(&records, &start, &records, "~").expect("session"),
            &[
                "terminal/session_1/1",
                "terminal/session_1/2",
                "terminal/session_1/3",
                "terminal/session_1/4",
            ],
        );
    }

    #[test]
    fn child_index_is_deterministic_not_expand() {
        let records = vec![test_record(1)];
        assert_refs(
            navigate(
                &records,
                &[records[0].work_ref.record_ref()],
                &records,
                ">3",
            )
            .expect("child"),
            &["terminal/session_1/1/o/2"],
        );
        assert!(navigate(
            &records,
            &[records[0].work_ref.record_ref()],
            &records,
            ">0"
        )
        .is_err());
    }

    #[test]
    fn navigation_preserves_remote_origin() {
        let mut records = (1..=3).map(test_record).collect::<Vec<_>>();
        for record in &mut records {
            record.work_ref = WorkRef::Remote {
                origin: sivtr_core::record::RemoteRefOrigin {
                    alias: "desk".to_string(),
                    peer_id: Some("peer".to_string()),
                    share_id: Some("share".to_string()),
                },
                body: record.work_ref.body().clone(),
            };
        }
        let start = vec![records[1].work_ref.record_ref()];

        assert_refs(
            navigate(&records, &start, &records, "+1").expect("remote sibling"),
            &["desk://terminal/session_1/3"],
        );
        assert_refs(
            navigate(&records, &start, &records, "~").expect("remote session"),
            &[
                "desk://terminal/session_1/1",
                "desk://terminal/session_1/2",
                "desk://terminal/session_1/3",
            ],
        );
    }

    fn assert_refs(actual: Vec<WorkRef>, expected: &[&str]) {
        let actual = actual
            .into_iter()
            .map(|anchor| anchor.to_string())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    fn test_record(index: usize) -> WorkRecord {
        WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
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
            title: format!("record {index}"),
            time: WorkTime::from_components(None, None, None),
            status: None,
            parts: vec![
                WorkPart {
                    io: WorkPartIo::Input,
                    kind: WorkPartKind::Command,
                    index: 1,
                    occurred_at: None,
                    label: None,
                    text: format!("cmd {index}"),
                    ansi: None,
                },
                WorkPart {
                    io: WorkPartIo::Output,
                    kind: WorkPartKind::Text,
                    index: 1,
                    occurred_at: None,
                    label: None,
                    text: format!("out {index}.1"),
                    ansi: None,
                },
                WorkPart {
                    io: WorkPartIo::Output,
                    kind: WorkPartKind::Text,
                    index: 2,
                    occurred_at: None,
                    label: None,
                    text: format!("out {index}.2"),
                    ansi: None,
                },
            ],
        }
    }
}
