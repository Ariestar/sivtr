use regex::Regex;

use super::model::{WorkChannel, WorkRecord};
use super::refs::WorkRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkRecordSearchScope {
    Content,
    Title,
    Session,
}

#[derive(Debug, Clone, Copy)]
pub struct WorkRecordMatch<'a> {
    pub record: &'a WorkRecord,
    pub line_index: usize,
    pub content: &'a str,
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
        let record_ref = reference.record_ref();
        self.records
            .iter()
            .find(|record| record.session.work_ref == record_ref)
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
                WorkRecordSearchScope::Content => matching_line(record, regex),
                WorkRecordSearchScope::Title => {
                    regex.is_match(&record.title).then_some(WorkRecordMatch {
                        record,
                        line_index: 0,
                        content: record.title.as_str(),
                    })
                }
                WorkRecordSearchScope::Session => {
                    regex
                        .is_match(&record.session.id)
                        .then_some(WorkRecordMatch {
                            record,
                            line_index: 0,
                            content: record.session.id.as_str(),
                        })
                }
            })
            .take(limit)
            .collect()
    }
}

impl WorkRecord {
    pub fn kind_label(&self) -> &'static str {
        match self.source.channel {
            WorkChannel::Terminal => "shell",
            WorkChannel::Chat => "ai",
        }
    }
}

fn matching_line<'a>(record: &'a WorkRecord, regex: &Regex) -> Option<WorkRecordMatch<'a>> {
    record
        .text
        .combined
        .lines()
        .enumerate()
        .find(|(_, line)| regex.is_match(line))
        .map(|(line_index, line)| WorkRecordMatch {
            record,
            line_index,
            content: line,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AgentProvider;
    use crate::record::model::{
        WorkOutcome, WorkPayload, WorkRecordKind, WorkSessionRef, WorkSource, WorkStatus, WorkText,
        WorkTime,
    };

    #[test]
    fn resolves_records_by_typed_ref() {
        let records = vec![test_record("pi/abcdef12/2", "abcdef12", 2, "hello\nneedle")];
        let index = WorkRecordIndex::new(records);
        let reference = WorkRef::agent_record(AgentProvider::Pi, "abcdef12", 2);

        assert_eq!(index.resolve(&reference).unwrap().title, "title");
        assert!(index
            .resolve(&WorkRef::agent_record(AgentProvider::Pi, "abcdef12", 3))
            .is_none());
    }

    #[test]
    fn searches_record_content() {
        let records = vec![test_record("pi/abcdef12/1", "abcdef12", 1, "hello\nneedle")];
        let index = WorkRecordIndex::new(records);
        let regex = Regex::new("needle").unwrap();

        let hits = index.search(&regex, WorkRecordSearchScope::Content, 10, |_| true);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_index, 1);
        assert_eq!(hits[0].content, "needle");
    }

    fn test_record(
        ref_id: &str,
        session_id: &str,
        turn_index: usize,
        combined: &str,
    ) -> WorkRecord {
        WorkRecord {
            schema_version: 1,
            id: ref_id.to_string(),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("pi".to_string()),
            },
            session: WorkSessionRef {
                id: session_id.to_string(),
                path: None,
                index: 1,
                work_ref: WorkRef::agent_record(AgentProvider::Pi, session_id, turn_index),
            },
            cwd: None,
            time: WorkTime::default(),
            status: WorkStatus {
                outcome: WorkOutcome::Unknown,
                exit_code: None,
            },
            title: "title".to_string(),
            text: WorkText {
                input: None,
                output: None,
                combined: combined.to_string(),
            },
            parts: Vec::new(),
            payload: WorkPayload::ChatTurn {
                user: String::new(),
                assistant: combined.to_string(),
                messages: Vec::new(),
            },
        }
    }
}
