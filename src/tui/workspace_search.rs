use regex::{Regex, RegexBuilder};
use sivtr_core::record::{work_record_content_matches, WorkRefTarget};

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
    pub(crate) target: WorkRefTarget,
    pub(crate) matched_line: usize,
}

impl WorkspaceSearchMatch {
    pub(crate) fn content_scroll_index(&self) -> usize {
        self.matched_line.saturating_sub(1)
    }
}

impl WorkspaceSearchIndex {
    pub(crate) fn new(sessions: &[WorkspaceSession]) -> Self {
        let loaded_sessions = sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| session.load.is_none())
            .collect::<Vec<_>>();
        let mut session_entries = Vec::with_capacity(loaded_sessions.len());
        let dialogue_count = loaded_sessions
            .iter()
            .map(|(_, session)| session.records.len())
            .sum();
        let mut dialogue_entries = Vec::with_capacity(dialogue_count);

        for (session_index, session) in loaded_sessions {
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
                    if let Some(session) = all_sessions.get(entry.session_index).cloned() {
                        sessions.push(session);
                        matches.push(WorkspaceSearchMatch {
                            session_index: filtered_session_index,
                            dialogue_index: 0,
                            target: WorkRefTarget::Record,
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
        let mut grouped: Vec<(usize, Vec<usize>)> = Vec::new();
        for entry in self
            .dialogues
            .iter()
            .filter(|entry| regex.is_match(&entry.dialogue_title))
        {
            if let Some((_, dialogue_indices)) = grouped
                .iter_mut()
                .find(|(session_index, _)| *session_index == entry.session_index)
            {
                dialogue_indices.push(entry.dialogue_index);
            } else {
                grouped.push((entry.session_index, vec![entry.dialogue_index]));
            }
        }

        let sessions = grouped
            .into_iter()
            .filter_map(|(session_index, dialogue_indices)| {
                let session = all_sessions.get(session_index)?;
                Some(filter_workspace_session_dialogues(
                    session,
                    &dialogue_indices,
                ))
            })
            .collect::<Vec<_>>();
        let matches = sessions
            .iter()
            .enumerate()
            .flat_map(|(session_index, session)| {
                session
                    .records
                    .iter()
                    .enumerate()
                    .filter(|(_, record)| regex.is_match(&record.title))
                    .map(move |(dialogue_index, _)| WorkspaceSearchMatch {
                        session_index,
                        dialogue_index,
                        target: WorkRefTarget::Record,
                        matched_line: 1,
                    })
            })
            .collect();
        WorkspaceSearchOutput { sessions, matches }
    }

    fn search_dialogue_content(
        &self,
        all_sessions: &[WorkspaceSession],
        regex: &Regex,
    ) -> WorkspaceSearchOutput {
        let mut grouped: Vec<(usize, Vec<usize>)> = Vec::new();
        for entry in &self.dialogues {
            let Some(record) = all_sessions
                .get(entry.session_index)
                .and_then(|session| session.records.get(entry.dialogue_index))
            else {
                continue;
            };
            if work_record_content_matches(record, regex).is_empty() {
                continue;
            }
            if let Some((_, dialogue_indices)) = grouped
                .iter_mut()
                .find(|(session_index, _)| *session_index == entry.session_index)
            {
                dialogue_indices.push(entry.dialogue_index);
            } else {
                grouped.push((entry.session_index, vec![entry.dialogue_index]));
            }
        }

        let sessions = grouped
            .into_iter()
            .filter_map(|(session_index, dialogue_indices)| {
                let session = all_sessions.get(session_index)?;
                Some(filter_workspace_session_dialogues(
                    session,
                    &dialogue_indices,
                ))
            })
            .collect::<Vec<_>>();
        let matches = sessions
            .iter()
            .enumerate()
            .flat_map(|(session_index, session)| {
                session
                    .records
                    .iter()
                    .enumerate()
                    .flat_map(move |(dialogue_index, record)| {
                        work_record_content_matches(record, regex)
                            .into_iter()
                            .map(move |matched| WorkspaceSearchMatch {
                                session_index,
                                dialogue_index,
                                target: matched.target,
                                matched_line: matched.matched_line,
                            })
                            .collect::<Vec<_>>()
                    })
            })
            .collect();
        WorkspaceSearchOutput { sessions, matches }
    }
}

fn filter_workspace_session_dialogues(
    session: &WorkspaceSession,
    dialogue_indices: &[usize],
) -> WorkspaceSession {
    let mut filtered = session.clone();
    filtered.records = dialogue_indices
        .iter()
        .filter_map(|idx| session.records.get(*idx).cloned())
        .collect();
    filtered
}
