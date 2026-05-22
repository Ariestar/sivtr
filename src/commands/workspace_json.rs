use serde::Serialize;

use crate::tui::workspace::{WorkspaceSession, WorkspaceSource};

#[derive(Serialize)]
pub(crate) struct WorkspaceJsonItem {
    #[serde(rename = "ref")]
    pub(crate) ref_: String,
    pub(crate) kind: String,
    pub(crate) timestamp: Option<String>,
    pub(crate) title: WorkspaceJsonTitle,
    pub(crate) content: String,
}

#[derive(Serialize)]
pub(crate) struct WorkspaceJsonTitle {
    pub(crate) session: String,
    pub(crate) dialogue: Option<String>,
}

pub(crate) fn workspace_item(
    session: &WorkspaceSession,
    ref_: String,
    dialogue_title: Option<String>,
    timestamp: Option<String>,
    content: String,
) -> WorkspaceJsonItem {
    WorkspaceJsonItem {
        ref_,
        kind: workspace_kind(session.source).to_string(),
        timestamp,
        title: WorkspaceJsonTitle {
            session: clean_session_title(&session.title, &session.ref_id),
            dialogue: dialogue_title,
        },
        content,
    }
}

pub(crate) fn workspace_ref(session: &WorkspaceSession) -> String {
    format!("{}/{}", workspace_source(session.source), session.ref_id)
}

pub(crate) fn dialogue_ref(session: &WorkspaceSession, dialogue_index: usize) -> String {
    format!("{}/{}", workspace_ref(session), dialogue_index + 1)
}

pub(crate) fn line_ref(
    session: &WorkspaceSession,
    dialogue_index: usize,
    line_index: usize,
) -> String {
    format!(
        "{}/{}",
        dialogue_ref(session, dialogue_index),
        line_index + 1
    )
}

pub(crate) fn workspace_source(source: WorkspaceSource) -> &'static str {
    match source {
        WorkspaceSource::Terminal => "terminal",
        WorkspaceSource::Agent(provider) => provider.command_name(),
    }
}

fn workspace_kind(source: WorkspaceSource) -> &'static str {
    match source {
        WorkspaceSource::Terminal => "shell",
        WorkspaceSource::Agent(_) => "ai",
    }
}

fn clean_session_title(title: &str, ref_id: &str) -> String {
    let suffix = format!("  [{ref_id}]");
    title.strip_suffix(&suffix).unwrap_or(title).to_string()
}
