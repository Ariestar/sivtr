use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;

use regex::{Regex, RegexBuilder};
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};
use sivtr_core::search::{content_line_matches, Bm25Index, Field, Filter, Searcher, Sort};

use crate::tui::workspace::WorkspaceSession;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceSearchScope {
    Content,
    Session,
    Dialogue,
}

impl WorkspaceSearchScope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Content => "",
            Self::Session => "session",
            Self::Dialogue => "dialogue",
        }
    }
}

pub(crate) fn workspace_search_query(query: &str) -> (WorkspaceSearchScope, &str) {
    let query = query.trim_start();
    if let Some(term) = query.strip_prefix('>') {
        (WorkspaceSearchScope::Session, term.trim_start())
    } else if let Some(term) = query.strip_prefix('#') {
        (WorkspaceSearchScope::Dialogue, term.trim_start())
    } else {
        (WorkspaceSearchScope::Content, query)
    }
}

pub(crate) fn workspace_search_scope(query: &str) -> WorkspaceSearchScope {
    workspace_search_query(query).0
}

pub(crate) fn workspace_search_has_query(query: &str) -> bool {
    !workspace_search_query(query).1.is_empty()
}

pub(crate) fn workspace_search_regex(term: &str) -> Option<Regex> {
    let term = term.trim();
    if term.is_empty() {
        return None;
    }
    RegexBuilder::new(term).case_insensitive(true).build().ok()
}

pub(crate) fn workspace_search_regex_for_query(query: &str) -> Option<Regex> {
    let (_, term) = workspace_search_query(query);
    workspace_search_regex(term)
}

#[derive(Clone)]
struct WorkspaceSearchSessionEntry {
    session_index: usize,
    session_title: String,
}

#[derive(Clone)]
struct WorkspaceSearchDialogueEntry {
    session_index: usize,
    dialogue_index: usize,
    dialogue_title: String,
}

pub(crate) struct WorkspaceSearchIndex {
    sessions: Vec<WorkspaceSearchSessionEntry>,
    dialogues: Vec<WorkspaceSearchDialogueEntry>,
    /// Loaded dialogue records, aligned one-to-one with `dialogues`.
    records: Vec<WorkRecord>,
    /// BM25 index over `records`, built on first content search and reused
    /// across searches while the corpus stays unchanged.
    bm25: RefCell<Option<Bm25Index>>,
    /// Content fingerprint so callers can detect staleness cheaply.
    fingerprint: u64,
}

/// Fingerprint of the loaded dialogue corpus: hash of every record ref.
/// Identical when the corpus has the same records in the same order.
pub(crate) fn workspace_records_fingerprint(sessions: &[WorkspaceSession]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for session in sessions {
        for record in &session.records {
            record.work_ref.whole().hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[derive(Default)]
pub(crate) struct WorkspaceSearchOutput {
    pub(crate) sessions: Vec<WorkspaceSession>,
    pub(crate) matches: Vec<WorkspaceSearchMatch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceSearchMatch {
    pub(crate) session_index: usize,
    pub(crate) dialogue_index: usize,
    pub(crate) at: WorkAt,
    pub(crate) matched_line: usize,
}

impl WorkspaceSearchIndex {
    pub(crate) fn new(sessions: &[WorkspaceSession]) -> Self {
        let mut session_entries = Vec::with_capacity(sessions.len());
        let dialogue_count = sessions.iter().map(|session| session.records.len()).sum();
        let mut dialogue_entries = Vec::with_capacity(dialogue_count);
        let mut records = Vec::with_capacity(dialogue_count);

        for (session_index, session) in sessions.iter().enumerate() {
            session_entries.push(WorkspaceSearchSessionEntry {
                session_index,
                session_title: session.search_title.clone(),
            });

            for (dialogue_index, record) in session.records.iter().enumerate() {
                dialogue_entries.push(WorkspaceSearchDialogueEntry {
                    session_index,
                    dialogue_index,
                    dialogue_title: record.title.clone(),
                });
                records.push(record.clone());
            }
        }

        let fingerprint = workspace_records_fingerprint(sessions);
        Self {
            sessions: session_entries,
            dialogues: dialogue_entries,
            records,
            bm25: RefCell::new(None),
            fingerprint,
        }
    }

    pub(crate) fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    pub(crate) fn search(
        &self,
        all_sessions: &[WorkspaceSession],
        query: &str,
    ) -> WorkspaceSearchOutput {
        let (scope, term) = workspace_search_query(query);
        self.search_with_scope(all_sessions, scope, term)
    }

    pub(crate) fn search_with_scope(
        &self,
        all_sessions: &[WorkspaceSession],
        scope: WorkspaceSearchScope,
        term: &str,
    ) -> WorkspaceSearchOutput {
        let Some(regex) = workspace_search_regex(term) else {
            return WorkspaceSearchOutput::default();
        };
        match scope {
            WorkspaceSearchScope::Session => {
                let mut sessions = Vec::new();
                let mut matches = Vec::new();
                for entry in self
                    .sessions
                    .iter()
                    .filter(|entry| regex.is_match(&entry.session_title))
                {
                    let filtered_session_index = sessions.len();
                    if let Some(session) = all_sessions.get(entry.session_index) {
                        sessions.push(session_meta_shell(session));
                        matches.push(WorkspaceSearchMatch {
                            session_index: filtered_session_index,
                            dialogue_index: 0,
                            at: WorkAt::Whole,
                            matched_line: 1,
                        });
                    }
                }
                WorkspaceSearchOutput { sessions, matches }
            }
            WorkspaceSearchScope::Dialogue => self.search_dialogue_titles(all_sessions, &regex),
            WorkspaceSearchScope::Content => {
                self.search_dialogue_content(all_sessions, &regex, term)
            }
        }
    }

    fn search_dialogue_titles(
        &self,
        all_sessions: &[WorkspaceSession],
        regex: &Regex,
    ) -> WorkspaceSearchOutput {
        // Hit sessions are meta shells; dialogue_index is the original turn
        // index in the session body (read later via SessionColumn::body_for).
        let mut sessions = Vec::new();
        let mut matches = Vec::new();
        let mut session_map: Vec<(usize, usize)> = Vec::new(); // corpus idx -> hit idx
        for entry in self
            .dialogues
            .iter()
            .filter(|entry| regex.is_match(&entry.dialogue_title))
        {
            let hit_idx = if let Some((_, hit)) = session_map
                .iter()
                .find(|(corpus, _)| *corpus == entry.session_index)
            {
                *hit
            } else {
                let session = match all_sessions.get(entry.session_index) {
                    Some(s) => s,
                    None => continue,
                };
                let hit = sessions.len();
                sessions.push(session_meta_shell(session));
                session_map.push((entry.session_index, hit));
                hit
            };
            matches.push(WorkspaceSearchMatch {
                session_index: hit_idx,
                dialogue_index: entry.dialogue_index,
                at: WorkAt::Whole,
                matched_line: 1,
            });
        }
        WorkspaceSearchOutput { sessions, matches }
    }

    fn search_dialogue_content(
        &self,
        all_sessions: &[WorkspaceSession],
        regex: &Regex,
        term: &str,
    ) -> WorkspaceSearchOutput {
        if self.records.is_empty() {
            return WorkspaceSearchOutput::default();
        }
        // Same search path as the CLI: boolean pattern bounds the set, BM25
        // ranks it. The regex is already compiled and valid here, so the
        // pattern inside the filter cannot fail.
        let anchors: Vec<WorkRef> = self
            .records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect();
        let filter = Filter {
            pattern: Some(term.to_string()),
            rank: Some(term.to_string()),
            sort: Sort::Relevance,
            // Browse searches everything, including tool-call payloads, so
            // hidden arguments and tool names stay findable.
            in_field: Field::All,
            ..Filter::default()
        };
        let mut index_slot = self.bm25.borrow_mut();
        let index = index_slot.get_or_insert_with(|| Bm25Index::build(&self.records));
        let searcher = Searcher::with_index(&self.records, index);
        let hits = match searcher.search(&filter, &anchors, Path::new(".")) {
            Ok(hits) => hits,
            Err(_) => return WorkspaceSearchOutput::default(),
        };
        let anchor_position: HashMap<WorkRef, usize> = anchors
            .into_iter()
            .enumerate()
            .map(|(position, anchor)| (anchor, position))
            .collect();

        let mut sessions = Vec::new();
        let mut matches = Vec::new();
        let mut session_map: HashMap<usize, usize> = HashMap::new();
        for hit in hits {
            let Some(&record_index) = anchor_position.get(&hit.anchor) else {
                continue;
            };
            let entry = match self.dialogues.get(record_index) {
                Some(entry) => entry,
                None => continue,
            };
            let record = match self.records.get(record_index) {
                Some(record) => record,
                None => continue,
            };
            let line_matches = content_line_matches(record, regex);
            if line_matches.is_empty() {
                continue;
            }
            let hit_idx = *session_map.entry(entry.session_index).or_insert_with(|| {
                let hit = sessions.len();
                if let Some(session) = all_sessions.get(entry.session_index) {
                    sessions.push(session_meta_shell(session));
                }
                hit
            });
            for matched in line_matches {
                matches.push(WorkspaceSearchMatch {
                    session_index: hit_idx,
                    dialogue_index: entry.dialogue_index,
                    at: WorkAt::Part(matched.part_seq),
                    matched_line: matched.line,
                });
            }
        }
        WorkspaceSearchOutput { sessions, matches }
    }
}

/// Search hit list row: meta only. Bodies stay in SessionColumn.
fn session_meta_shell(session: &WorkspaceSession) -> WorkspaceSession {
    let mut out = session.clone();
    out.records = Vec::new();
    out
}
