use regex::{Regex, RegexBuilder};
use sivtr_core::record::{work_record_content_matches, WorkAt};

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
            }
        }

        Self {
            sessions: session_entries,
            dialogues: dialogue_entries,
        }
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
            WorkspaceSearchScope::Content => self.search_dialogue_content(all_sessions, &regex),
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
    ) -> WorkspaceSearchOutput {
        let mut sessions = Vec::new();
        let mut matches = Vec::new();
        let mut session_map: Vec<(usize, usize)> = Vec::new();
        for entry in &self.dialogues {
            let Some(record) = all_sessions
                .get(entry.session_index)
                .and_then(|session| session.records.get(entry.dialogue_index))
            else {
                continue;
            };
            let line_matches = work_record_content_matches(record, regex);
            if line_matches.is_empty() {
                continue;
            }
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
            for matched in line_matches {
                matches.push(WorkspaceSearchMatch {
                    session_index: hit_idx,
                    dialogue_index: entry.dialogue_index,
                    at: matched.at,
                    matched_line: matched.matched_line,
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
