//! Publication lifetime picker for the workspace browser.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sivtr_core::publication::{
    create_publication_draft, expand_publication_anchors, PublicationExpiry, PublicationPolicy,
};
use sivtr_core::workset::WorkSet;

pub(super) struct PublishOverlay {
    pub(super) selected: usize,
    pub(super) name: String,
    pub(super) focus: OverlayFocus,
    pub(super) set: WorkSet,
    pub(super) redaction_count: usize,
    pub(super) warning_count: usize,
    pub(super) item_count: usize,
    pub(super) schema_version: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverlayFocus {
    Name,
    Expiry,
}

pub(super) enum OverlayKey {
    Continue,
    Cancel,
    Confirm,
}

pub(super) fn selected_expiry(selected: usize) -> PublicationExpiry {
    PublicationExpiry::PICKER_CHOICES[selected]
}

pub(super) fn handle_key(
    key: KeyEvent,
    selected: &mut usize,
    name: &mut String,
    focus: &mut OverlayFocus,
) -> OverlayKey {
    let last = PublicationExpiry::PICKER_CHOICES.len().saturating_sub(1);
    match key.code {
        KeyCode::Esc => OverlayKey::Cancel,
        KeyCode::Enter => OverlayKey::Confirm,
        KeyCode::Tab | KeyCode::BackTab => {
            *focus = match focus {
                OverlayFocus::Name => OverlayFocus::Expiry,
                OverlayFocus::Expiry => OverlayFocus::Name,
            };
            OverlayKey::Continue
        }
        KeyCode::Up if *focus == OverlayFocus::Expiry => {
            *selected = selected.saturating_sub(1);
            OverlayKey::Continue
        }
        KeyCode::Char('k') if *focus == OverlayFocus::Expiry => {
            *selected = selected.saturating_sub(1);
            OverlayKey::Continue
        }
        KeyCode::Down if *focus == OverlayFocus::Expiry => {
            *selected = (*selected + 1).min(last);
            OverlayKey::Continue
        }
        KeyCode::Char('j') if *focus == OverlayFocus::Expiry => {
            *selected = (*selected + 1).min(last);
            OverlayKey::Continue
        }
        KeyCode::Backspace if *focus == OverlayFocus::Name => {
            name.pop();
            OverlayKey::Continue
        }
        KeyCode::Char(ch)
            if *focus == OverlayFocus::Name
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            name.push(ch);
            OverlayKey::Continue
        }
        _ => OverlayKey::Continue,
    }
}

pub(super) fn try_open(
    selection: &WorkSet,
    title: Option<&str>,
    expires: PublicationExpiry,
) -> Result<PublishOverlay, String> {
    if selection.anchors().is_empty() {
        return Err("publication selection is empty".into());
    }
    let anchors = expand_publication_anchors(selection.records(), selection.anchors())
        .map_err(|error| error.to_string())?;
    if anchors.is_empty() {
        return Err("publication selection is empty".into());
    }

    let mut set = selection.clone();
    set.select_anchors(anchors);
    let draft = create_publication_draft(
        set.records(),
        set.anchors(),
        &PublicationPolicy {
            title: title.map(str::to_owned),
            expires,
            ..PublicationPolicy::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let selected = PublicationExpiry::PICKER_CHOICES
        .iter()
        .position(|choice| *choice == expires)
        .expect("publish expiry must be offered in the overlay");
    Ok(PublishOverlay {
        selected,
        name: String::new(),
        focus: OverlayFocus::Expiry,
        set,
        redaction_count: draft.redaction_count,
        warning_count: draft.warning_count(),
        item_count: draft.item_count(),
        schema_version: draft.snapshot.schema_version(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef,
        WorkSource, WorkTime,
    };

    fn record() -> WorkRecord {
        WorkRecord {
            schema_version: 3,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
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

    #[test]
    fn handle_key_moves_and_confirms_default_seven_days() {
        let mut selected = PublicationExpiry::picker_default_index();
        assert_eq!(selected_expiry(selected), PublicationExpiry::SevenDays);
        let mut name = String::new();
        let mut focus = OverlayFocus::Expiry;
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(selected_expiry(selected), PublicationExpiry::ThreeDays);
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Confirm
        ));
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Cancel
        ));
    }

    #[test]
    fn name_input_is_optional_and_confirm_keeps_the_entered_name() {
        let mut selected = PublicationExpiry::picker_default_index();
        let mut name = String::new();
        let mut focus = OverlayFocus::Expiry;
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(focus, OverlayFocus::Name);
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(focus, OverlayFocus::Expiry);
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(focus, OverlayFocus::Name);
        for ch in ['r', 'e', 'v', 'i', 'e', 'w', 'k'] {
            assert!(matches!(
                handle_key(
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                    &mut selected,
                    &mut name,
                    &mut focus,
                ),
                OverlayKey::Continue
            ));
        }
        assert_eq!(name, "reviewk");
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Continue
        ));
        assert_eq!(name, "review");
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Confirm
        ));
    }

    #[test]
    fn empty_name_confirms_without_a_save_request() {
        let mut selected = PublicationExpiry::picker_default_index();
        let mut name = String::new();
        let mut focus = OverlayFocus::Expiry;
        assert!(matches!(
            handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut selected,
                &mut name,
                &mut focus,
            ),
            OverlayKey::Confirm
        ));
        assert!(name.is_empty());
    }

    #[test]
    fn try_open_uses_the_canonical_part_selection() {
        let record = record();
        let selection = WorkSet::from_parts(
            ".",
            vec![record.clone()],
            vec![record.work_ref.with_part(1)],
        );
        let overlay =
            try_open(&selection, None, PublicationExpiry::SevenDays).expect("publication overlay");
        assert_eq!(overlay.schema_version, 2);
        assert_eq!(overlay.item_count, 1);
        assert_eq!(overlay.set.anchors(), &[record.work_ref.with_part(1)]);
        assert_eq!(overlay.focus, OverlayFocus::Expiry);
        assert!(overlay.name.is_empty());

        let overlay = try_open(&selection, Some("title"), PublicationExpiry::ThreeDays)
            .expect("publication overlay with configured expiry");
        assert_eq!(
            selected_expiry(overlay.selected),
            PublicationExpiry::ThreeDays
        );
    }

    #[test]
    fn try_open_rejects_empty_selection() {
        let selection = WorkSet::from_parts(".", Vec::new(), Vec::new());
        assert!(try_open(&selection, None, PublicationExpiry::SevenDays).is_err());
    }
}
