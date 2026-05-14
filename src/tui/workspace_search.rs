use regex::{Regex, RegexBuilder};

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
