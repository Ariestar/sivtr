use regex::Regex;

use super::model::{WorkPart, WorkRecord, WorkRecordKind};
use super::refs::{WorkAt, WorkRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkRecordSearchScope {
    Content,
    Title,
    Session,
}

#[derive(Debug, Clone)]
pub struct WorkRecordMatch<'a> {
    pub record: &'a WorkRecord,
    pub at: WorkAt,
    pub content: String,
    pub matched_line: usize,
}

#[derive(Debug, Clone)]
pub struct WorkRecordIndex {
    records: Vec<WorkRecord>,
}

impl WorkRecordIndex {
    pub fn new(records: Vec<WorkRecord>) -> Self {
        Self { records }
    }

    pub fn records(&self) -> &[WorkRecord] {
        &self.records
    }

    pub fn resolve(&self, reference: &WorkRef) -> Option<&WorkRecord> {
        let whole = reference.whole();
        self.records.iter().find(|record| record.work_ref == whole)
    }

    pub fn resolve_part(&self, reference: &WorkRef) -> Option<&WorkPart> {
        let seq = reference.part()?;
        self.resolve(reference)
            .and_then(|record| find_part(record, seq))
    }

    pub fn search(
        &self,
        regex: &Regex,
        scope: WorkRecordSearchScope,
        limit: usize,
        include: impl Fn(&WorkRecord) -> bool,
    ) -> Vec<WorkRecordMatch<'_>> {
        self.records
            .iter()
            .filter(|record| include(record))
            .filter_map(|record| match scope {
                WorkRecordSearchScope::Content => matching_content(record, regex),
                WorkRecordSearchScope::Title => {
                    regex.is_match(&record.title).then_some(WorkRecordMatch {
                        record,
                        at: WorkAt::Whole,
                        content: record.title.clone(),
                        matched_line: 1,
                    })
                }
                WorkRecordSearchScope::Session => matching_session(record, regex),
            })
            .take(limit)
            .collect()
    }
}

impl WorkRecord {
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            WorkRecordKind::TerminalCommand => "shell",
            WorkRecordKind::ChatTurn => "ai",
        }
    }
}

fn matching_content<'a>(record: &'a WorkRecord, regex: &Regex) -> Option<WorkRecordMatch<'a>> {
    work_record_content_matches(record, regex)
        .into_iter()
        .next()
}

pub fn work_record_content_matches<'a>(
    record: &'a WorkRecord,
    regex: &Regex,
) -> Vec<WorkRecordMatch<'a>> {
    matching_parts(record, regex)
}

fn matching_parts<'a>(record: &'a WorkRecord, regex: &Regex) -> Vec<WorkRecordMatch<'a>> {
    record
        .parts
        .iter()
        .flat_map(|part| {
            let text = part.text();
            text.lines()
                .enumerate()
                .filter(|(_, line)| regex.is_match(line))
                .map(|(line_index, line)| WorkRecordMatch {
                    record,
                    at: WorkAt::Part(part.seq),
                    content: line.to_string(),
                    matched_line: line_index + 1,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn matching_session<'a>(record: &'a WorkRecord, regex: &Regex) -> Option<WorkRecordMatch<'a>> {
    let session_id = record.work_ref.session();
    if regex.is_match(session_id) {
        return Some(WorkRecordMatch {
            record,
            at: WorkAt::Whole,
            content: session_id.to_string(),
            matched_line: 1,
        });
    }
    None
}

fn find_part(record: &WorkRecord, seq: usize) -> Option<&WorkPart> {
    record.parts.iter().find(|part| part.seq == seq)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AgentProvider;
    use crate::record::model::{WorkOutcome, WorkPartData, WorkRecordKind, WorkStatus, WorkTime};

    #[test]
    fn resolves_records_by_typed_ref() {
        let records = vec![test_record("pi/abcdef12/2", "abcdef12", 2, "hello\nneedle")];
        let index = WorkRecordIndex::new(records);
        let reference = WorkRef::agent(AgentProvider::Pi, "abcdef12", 2);

        assert_eq!(index.resolve(&reference).unwrap().title, "title");
        assert!(index
            .resolve(&WorkRef::agent(AgentProvider::Pi, "abcdef12", 3))
            .is_none());
    }

    #[test]
    fn resolves_parts_by_typed_ref() {
        let record = test_record_with_parts("terminal/current/1", "current", 1, "hello");
        let index = WorkRecordIndex::new(vec![record]);
        let reference = WorkRef::terminal("current", 1).with_part(1);

        assert!(index.resolve_part(&reference).is_some());
    }

    #[test]
    fn search_finds_part_matches() {
        let records = vec![test_record_with_parts(
            "terminal/current/1",
            "current",
            1,
            "hello world",
        )];
        let index = WorkRecordIndex::new(records);
        let regex = Regex::new("hello").unwrap();
        let matches = index.search(&regex, WorkRecordSearchScope::Content, 10, |_| true);

        assert!(!matches.is_empty());
    }

    fn test_record(
        _ref_id: &str,
        session_id: &str,
        turn_index: usize,
        combined: &str,
    ) -> WorkRecord {
        use crate::record::model::{WorkChannel, WorkSessionRef, WorkSource};
        let work_ref = WorkRef::agent(AgentProvider::Pi, session_id, turn_index);
        WorkRecord {
            schema_version: 1,
            work_ref,
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("pi".to_string()),
            },
            session: WorkSessionRef {
                id: session_id.to_string(),
                canonical_id: Some(session_id.to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "title".to_string(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::Assistant {
                    content: combined.to_string(),
                },
            }],
        }
    }

    fn test_record_with_parts(
        _ref_id: &str,
        session_id: &str,
        turn_index: usize,
        text: &str,
    ) -> WorkRecord {
        use crate::record::model::{WorkChannel, WorkSessionRef, WorkSource};
        let work_ref = WorkRef::terminal(session_id, turn_index);
        WorkRecord {
            schema_version: 1,
            work_ref,
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session_id.to_string(),
                canonical_id: Some(session_id.to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: Some(WorkStatus {
                outcome: WorkOutcome::Success,
                exit_code: Some(0),
            }),
            title: "title".to_string(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::Output {
                    content: text.to_string(),
                    ansi: None,
                },
            }],
        }
    }
}
