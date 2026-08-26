//! Build a publication WorkSet from the current picker selection and drive
//! the lifetime overlay keys.

use anyhow::Result;
use crossterm::event::KeyCode;
use sivtr_core::publication::{create_publication_draft, PublicationExpiry, PublicationPolicy};
use sivtr_core::record::{WorkRecord, WorkRecordKind};
use std::collections::HashSet;

use crate::commands::memory::workset::WorkSet;
use crate::commands::publish::{expand_picker_anchors, publication_workset};
use crate::tui::workspace::{selected_count, WorkspaceDialogue, WorkspacePickedContent};

use super::content::{workspace_picked_content, workspace_picked_content_for_marked_blocks};
use super::panes::ContentPane;

pub struct PublishOverlay {
    pub selected: usize,
    pub set: WorkSet,
    pub redaction_count: usize,
    pub warning_count: usize,
    pub item_count: usize,
    pub schema_version: u32,
}

pub enum OverlayKey {
    Continue,
    Cancel,
    Confirm,
}

pub fn default_selected() -> usize {
    PublicationExpiry::picker_default_index()
}

pub fn selected_expiry(selected: usize) -> &'static str {
    let choices = PublicationExpiry::PICKER_CHOICES;
    choices[selected.min(choices.len() - 1)].as_str()
}

pub fn handle_key(code: KeyCode, selected: &mut usize) -> OverlayKey {
    let last = PublicationExpiry::PICKER_CHOICES.len().saturating_sub(1);
    match code {
        KeyCode::Esc => OverlayKey::Cancel,
        KeyCode::Enter => OverlayKey::Confirm,
        KeyCode::Up | KeyCode::Char('k') => {
            *selected = selected.saturating_sub(1);
            OverlayKey::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            *selected = (*selected + 1).min(last);
            OverlayKey::Continue
        }
        _ => OverlayKey::Continue,
    }
}

pub fn try_open(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    content_pane: &ContentPane,
    line_filter: Option<&str>,
    cwd: String,
) -> Result<PublishOverlay, String> {
    if line_filter.is_some() {
        return Err("publication cannot use a line filter".into());
    }
    let picked = selection(dialogues, selected_dialogues, dialogue_idx, content_pane)?;
    if picked.anchors.is_empty() {
        return Err("publication selection is empty".into());
    }
    let records = records_for_pick(dialogues, &picked)?;
    validate_local_agent_session(&records)?;
    let anchors =
        expand_picker_anchors(&records, &picked.anchors).map_err(|error| error.to_string())?;
    if anchors.is_empty() {
        return Err("publication selection is empty".into());
    }
    let set = publication_workset(WorkSet::with_anchors(cwd, records, Vec::new()), anchors);
    let draft =
        create_publication_draft(&set.records, &set.anchors(), &PublicationPolicy::default())
            .map_err(|error| error.to_string())?;
    Ok(PublishOverlay {
        selected: default_selected(),
        set,
        redaction_count: draft.redaction_count,
        warning_count: draft
            .risks
            .iter()
            .filter(|risk| {
                matches!(
                    risk.kind.as_str(),
                    "absolute_path" | "email" | "internal_url"
                )
            })
            .map(|risk| risk.count)
            .sum(),
        item_count: draft.item_count(),
        schema_version: draft.snapshot.schema_version(),
    })
}

fn selection(
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[bool],
    dialogue_idx: usize,
    content_pane: &ContentPane,
) -> Result<WorkspacePickedContent, String> {
    if content_pane.marked_count() > 0 {
        return workspace_picked_content_for_marked_blocks(
            dialogues,
            selected_dialogues,
            dialogue_idx,
            content_pane,
            None,
        )
        .ok_or_else(|| "publication selection is empty".to_string());
    }
    if selected_count(selected_dialogues) == 0 {
        return Err("mark blocks or select dialogues, then press p".into());
    }
    Ok(workspace_picked_content(
        dialogues,
        selected_dialogues,
        dialogue_idx,
        None,
    ))
}

fn records_for_pick(
    dialogues: &[WorkspaceDialogue],
    picked: &WorkspacePickedContent,
) -> Result<Vec<WorkRecord>, String> {
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    for anchor in &picked.anchors {
        let whole = anchor.whole();
        if !seen.insert(whole.to_string()) {
            continue;
        }
        let record = dialogues
            .iter()
            .find_map(|dialogue| {
                dialogue
                    .record
                    .as_ref()
                    .filter(|record| record.work_ref.whole() == whole)
            })
            .ok_or_else(|| format!("picker anchor `{anchor}` has no record"))?;
        records.push(record.clone());
    }
    Ok(records)
}

fn validate_local_agent_session(records: &[WorkRecord]) -> Result<(), String> {
    let first = records
        .first()
        .ok_or_else(|| "publication selection is empty".to_string())?;
    if first.kind != WorkRecordKind::ChatTurn || !first.work_ref.is_local() {
        return Err("publication picker only supports one local agent session".into());
    }
    let provider = first
        .work_ref
        .provider()
        .ok_or_else(|| "publication picker only supports agent sessions".to_string())?;
    let session = first.session.id.as_str();
    if records.iter().all(|record| {
        record.kind == WorkRecordKind::ChatTurn
            && record.work_ref.is_local()
            && record.work_ref.provider() == Some(provider)
            && record.session.id == session
    }) {
        Ok(())
    } else {
        Err("publication picker requires exactly one local agent session".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource,
        WorkTime,
    };

    use crate::tui::workspace::{TextPair, WorkspaceCopyParts, WorkspaceSource};

    fn chat_turn(index: usize) -> WorkRecord {
        WorkRecord {
            schema_version: 3,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", index),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".into()),
            },
            session: WorkSessionRef {
                id: "session".into(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "Demo".into(),
            parts: vec![
                WorkPart {
                    seq: 1,
                    occurred_at: None,
                    data: WorkPartData::User {
                        content: "hello".into(),
                    },
                },
                WorkPart {
                    seq: 2,
                    occurred_at: None,
                    data: WorkPartData::Assistant {
                        content: "reply".into(),
                    },
                },
            ],
        }
    }

    fn dialogue(record: WorkRecord) -> WorkspaceDialogue {
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            copy: WorkspaceCopyParts {
                input: TextPair {
                    plain: "hello".into(),
                    ansi: String::new(),
                },
                output: TextPair {
                    plain: "reply".into(),
                    ansi: String::new(),
                },
                command: TextPair::default(),
            },
            record: Some(record),
        }
    }

    #[test]
    fn handle_key_moves_and_confirms_default_seven_days() {
        let mut selected = default_selected();
        assert_eq!(selected_expiry(selected), "7d");
        assert!(matches!(
            handle_key(KeyCode::Char('k'), &mut selected),
            OverlayKey::Continue
        ));
        assert_eq!(selected_expiry(selected), "3d");
        assert!(matches!(
            handle_key(KeyCode::Enter, &mut selected),
            OverlayKey::Confirm
        ));
        assert!(matches!(
            handle_key(KeyCode::Esc, &mut selected),
            OverlayKey::Cancel
        ));
    }

    #[test]
    fn try_open_rejects_empty_and_line_filter() {
        let record = chat_turn(1);
        let dialogues = [dialogue(record)];
        let pane = ContentPane::default();
        assert!(try_open(&dialogues, &[false], 0, &pane, Some("1"), ".".into()).is_err());
        assert!(try_open(&dialogues, &[false], 0, &pane, None, ".".into()).is_err());
    }

    #[test]
    fn try_open_accepts_selected_dialogues_as_v2() {
        let record = chat_turn(1);
        let dialogues = [dialogue(record)];
        let pane = ContentPane::default();
        let overlay = try_open(&dialogues, &[true], 0, &pane, None, ".".into()).unwrap();
        assert_eq!(overlay.schema_version, 2);
        assert_eq!(overlay.item_count, 2);
        assert!(overlay
            .set
            .anchors()
            .iter()
            .all(|anchor| anchor.part().is_some()));
    }
}
