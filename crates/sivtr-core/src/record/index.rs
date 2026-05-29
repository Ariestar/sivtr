use regex::Regex;

use super::model::{WorkOutcome, WorkPart, WorkPartIo, WorkRecord, WorkRecordKind};
use super::refs::{WorkLink, WorkLinkKind, WorkRef, WorkRefTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkRecordSearchScope {
    Content,
    Title,
    Session,
}

#[derive(Debug, Clone)]
pub struct WorkRecordMatch<'a> {
    pub record: &'a WorkRecord,
    pub target: WorkRefTarget,
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
        let record_ref = reference.record_ref();
        self.records
            .iter()
            .find(|record| record.work_ref == record_ref)
    }

    pub fn resolve_part(&self, reference: &WorkRef) -> Option<&WorkPart> {
        let (io, index) = reference.part()?;
        self.resolve(reference)
            .and_then(|record| find_part(record, io, index))
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
                        target: WorkRefTarget::Record,
                        content: record.title.clone(),
                        matched_line: 1,
                    })
                }
                WorkRecordSearchScope::Session => matching_session(record, regex),
            })
            .take(limit)
            .collect()
    }

    pub fn infer_links(&self) -> Vec<WorkLink> {
        let mut links = Vec::new();
        let records = &self.records;
        for (i, record) in records.iter().enumerate() {
            if let Some(link) = infer_terminal_failure(record, records, i) {
                links.push(link);
            }
            if let Some(link) = infer_chat_follows_up(record, records, i) {
                links.push(link);
            }
            links.extend(infer_text_references(record, records, i));
        }
        links
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
    let part_matches = matching_parts(record, regex);
    if part_matches.is_empty() {
        matching_lines(record, regex)
    } else {
        part_matches
    }
}

fn matching_parts<'a>(record: &'a WorkRecord, regex: &Regex) -> Vec<WorkRecordMatch<'a>> {
    record
        .parts
        .iter()
        .flat_map(|part| {
            part.text
                .lines()
                .enumerate()
                .filter(|(_, line)| regex.is_match(line))
                .map(|(line_index, line)| WorkRecordMatch {
                    record,
                    target: WorkRefTarget::Part {
                        io: part.io,
                        index: part.index,
                    },
                    content: line.to_string(),
                    matched_line: line_index + 1,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn matching_lines<'a>(record: &'a WorkRecord, regex: &Regex) -> Vec<WorkRecordMatch<'a>> {
    record
        .combined_text()
        .lines()
        .enumerate()
        .filter(|(_, line)| regex.is_match(line))
        .map(|(line_index, line)| WorkRecordMatch {
            record,
            target: WorkRefTarget::Line(line_index + 1),
            content: line.to_string(),
            matched_line: line_index + 1,
        })
        .collect()
}

fn matching_session<'a>(record: &'a WorkRecord, regex: &Regex) -> Option<WorkRecordMatch<'a>> {
    let session_id = record.work_ref.session();
    if regex.is_match(session_id) {
        return Some(WorkRecordMatch {
            record,
            target: WorkRefTarget::Record,
            content: session_id.to_string(),
            matched_line: 1,
        });
    }
    None
}

fn find_part(record: &WorkRecord, io: WorkPartIo, index: usize) -> Option<&WorkPart> {
    record
        .parts
        .iter()
        .find(|part| part.io == io && part.index == index)
}

fn infer_terminal_failure(
    record: &WorkRecord,
    records: &[WorkRecord],
    index: usize,
) -> Option<WorkLink> {
    if record.kind != WorkRecordKind::TerminalCommand {
        return None;
    }
    let status = record.status.as_ref()?;
    if status.outcome != WorkOutcome::Failure {
        return None;
    }
    if index + 1 >= records.len() {
        return None;
    }
    let earlier = &records[index + 1];
    if earlier.kind != WorkRecordKind::TerminalCommand {
        return None;
    }
    if record.work_ref.session() != earlier.work_ref.session() {
        return None;
    }
    Some(WorkLink::new(
        record.work_ref.clone(),
        earlier.work_ref.clone(),
        WorkLinkKind::CausedBy,
    ))
}

fn infer_chat_follows_up(
    record: &WorkRecord,
    records: &[WorkRecord],
    index: usize,
) -> Option<WorkLink> {
    if record.kind != WorkRecordKind::ChatTurn {
        return None;
    }
    if index + 1 >= records.len() {
        return None;
    }
    let earlier = &records[index + 1];
    if earlier.kind != WorkRecordKind::ChatTurn {
        return None;
    }
    if record.work_ref.session() != earlier.work_ref.session() {
        return None;
    }
    Some(WorkLink::new(
        record.work_ref.clone(),
        earlier.work_ref.clone(),
        WorkLinkKind::FollowsUp,
    ))
}

fn infer_text_references(record: &WorkRecord, records: &[WorkRecord], index: usize) -> Vec<WorkLink> {
    let text = record.combined_text().to_lowercase();
    let mut links = Vec::new();
    for (j, earlier) in records.iter().enumerate() {
        if j == index {
            continue;
        }
        if let Some(cmd) = command_from_record(earlier) {
            if text.contains(&cmd.to_lowercase()) {
                links.push(WorkLink::new(
                    record.work_ref.clone(),
                    earlier.work_ref.clone(),
                    WorkLinkKind::References,
                ));
            }
        }
    }
    links
}

fn command_from_record(record: &WorkRecord) -> Option<String> {
    record.input_text().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AgentProvider;
    use crate::record::model::{WorkOutcome, WorkPartKind, WorkRecordKind, WorkStatus, WorkTime};

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
    fn resolves_parts_by_typed_ref() {
        let record = test_record_with_parts("terminal/current/1", "current", 1, "hello");
        let index = WorkRecordIndex::new(vec![record]);
        let reference = WorkRef::terminal_record("current", 1).with_part(WorkPartIo::Output, 1);

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
        let work_ref = WorkRef::agent_record(AgentProvider::Pi, session_id, turn_index);
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
                io: WorkPartIo::Output,
                kind: WorkPartKind::AssistantMessage,
                index: 1,
                occurred_at: None,
                label: None,
                text: combined.to_string(),
                ansi: None,
                tags: Vec::new(),
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
        let work_ref = WorkRef::terminal_record(session_id, turn_index);
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
                io: WorkPartIo::Output,
                kind: WorkPartKind::Text,
                index: 1,
                occurred_at: None,
                label: None,
                text: text.to_string(),
                ansi: None,
                tags: Vec::new(),
            }],
        }
    }
}
