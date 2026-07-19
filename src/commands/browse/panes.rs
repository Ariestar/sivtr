//! Browse product panes implementing [`crate::pane::Pane`].
//!
//! **New pane checklist**
//! 1. `struct MyPane { engine: SlidingPane<K,M,B>, … }`
//! 2. `impl Pane for MyPane` — only map data + call SlidingPane ensure_*
//! 3. Register in picker: `my_pane.poll(); my_pane.ensure(ctx, &input);`
//!
//! Do **not** reimplement viewport growth, keep/evict, or blanking rules.

use crate::pane::{Pane, PaneInput, SlidingPane, WindowRow};
use crate::tui::content_view::{content_view_line_count, ContentViewMode};
use crate::tui::workspace::{
    workspace_content_text, WorkspaceDialogue, WorkspaceSession, WorkspaceSource,
};
use sivtr_core::ai::AgentSelection;
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};

use super::text::record_to_copy_parts;

// ── Source ──────────────────────────────────────────────────────────────

pub type SourceEngine = SlidingPane<String, WorkspaceSource, ()>;

/// Static catalog pane. Ensure is a no-op after construction.
#[derive(Clone, Debug)]
pub struct SourcePane {
    engine: SourceEngine,
}

impl SourcePane {
    pub fn from_catalog(sources: &[WorkspaceSource]) -> Self {
        let rows = sources
            .iter()
            .map(|s| WindowRow::meta_only(s.selector(), s.clone()))
            .collect();
        Self {
            engine: SlidingPane::ready(rows, sources.len().max(1), true),
        }
    }

    #[cfg(test)]
    pub fn exhausted(&self) -> bool {
        self.engine.exhausted()
    }
}

impl Pane for SourcePane {
    type Ctx<'a> = ();

    fn ensure(&mut self, _ctx: (), _input: &PaneInput) -> bool {
        false
    }

    fn len(&self) -> usize {
        self.engine.len()
    }
}

// ── Dialogues ───────────────────────────────────────────────────────────

pub type DialogueKey = String;

#[derive(Clone, Debug)]
pub struct DialogueMeta {
    pub source: WorkspaceSource,
    pub work_ref: Option<WorkRef>,
    pub title: String,
}

pub type DialogueEngine = SlidingPane<DialogueKey, DialogueMeta, WorkspaceDialogue>;

/// Active session set + body readiness. Any change force-rebuilds meta.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DialogueFingerprint {
    sessions: Vec<(String, String, bool, usize)>,
}

#[derive(Default)]
pub struct DialoguePane {
    engine: DialogueEngine,
    fingerprint: DialogueFingerprint,
}

impl DialoguePane {
    pub fn dialogues(&self) -> Vec<WorkspaceDialogue> {
        self.engine
            .rows()
            .iter()
            .map(|row| {
                if let Some(body) = row.body.clone() {
                    body
                } else {
                    WorkspaceDialogue {
                        source: row.meta.source.clone(),
                        work_ref: row.meta.work_ref.clone(),
                        title: row.meta.title.clone(),
                        record: None,
                        copy: crate::tui::workspace::WorkspaceCopyParts::from_block(
                            crate::tui::workspace::TextPair {
                                plain: String::new(),
                                ansi: String::new(),
                            },
                        ),
                    }
                }
            })
            .collect()
    }

    #[cfg(test)]
    pub fn exhausted(&self) -> bool {
        self.engine.exhausted()
    }
}

/// Domain context for dialogue ensure (one frame).
pub struct DialogueCtx<'a> {
    pub sessions: &'a [WorkspaceSession],
    pub session_idx: usize,
    pub selected_sessions: &'a [bool],
}

impl Pane for DialoguePane {
    type Ctx<'a> = DialogueCtx<'a>;

    fn ensure(&mut self, ctx: DialogueCtx<'_>, input: &PaneInput) -> bool {
        let next = fingerprint(ctx.sessions, ctx.session_idx, ctx.selected_sessions);
        let force = if next != self.fingerprint {
            self.engine.clear();
            self.fingerprint = next;
            true
        } else {
            input.force
        };

        let before = self.engine.len();
        let grown = self.engine.ensure_meta_sync(input.viewport, force, |budget| {
            meta_prefix(
                ctx.sessions,
                ctx.session_idx,
                ctx.selected_sessions,
                budget,
            )
        });

        let keep = self
            .engine
            .keep_for_focus(input.focus, &input.selected, input.neighbor_radius);
        self.engine.ensure_bodies_sync(keep, |key| {
            body_for_key(
                ctx.sessions,
                ctx.session_idx,
                ctx.selected_sessions,
                key,
            )
        });
        grown || self.engine.len() != before
    }

    fn len(&self) -> usize {
        self.engine.len()
    }

    fn is_fetching(&self) -> bool {
        self.engine.is_fetching()
    }
}

fn dialogue_key(source: &WorkspaceSource, session_id: &str, record: &WorkRecord) -> DialogueKey {
    format!(
        "{}/{}/{}",
        source.selector(),
        session_id,
        record.work_ref.path.index()
    )
}

fn dialogue_from_record(session: &WorkspaceSession, record: &WorkRecord) -> WorkspaceDialogue {
    WorkspaceDialogue {
        source: session.source.clone(),
        work_ref: Some(record.work_ref.clone()),
        title: record.title.clone(),
        record: Some(record.clone()),
        copy: record_to_copy_parts(record, AgentSelection::LastTurn),
    }
}

fn active_session_indices(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
) -> Vec<usize> {
    let selected: Vec<usize> = selected_sessions
        .iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect();
    if !selected.is_empty() {
        return selected;
    }
    if sessions.is_empty() {
        Vec::new()
    } else {
        vec![session_idx.min(sessions.len() - 1)]
    }
}

fn fingerprint(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
) -> DialogueFingerprint {
    DialogueFingerprint {
        sessions: active_session_indices(sessions, session_idx, selected_sessions)
            .into_iter()
            .filter_map(|i| {
                let s = sessions.get(i)?;
                Some((
                    s.source.selector(),
                    s.session_id.clone(),
                    s.body_loaded,
                    s.records.len(),
                ))
            })
            .collect(),
    }
}

fn meta_prefix(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
    budget: usize,
) -> (
    Vec<WindowRow<DialogueKey, DialogueMeta, WorkspaceDialogue>>,
    bool,
) {
    let indices = active_session_indices(sessions, session_idx, selected_sessions);
    if indices.is_empty() {
        return (Vec::new(), true);
    }
    let mut all_ready = true;
    let mut total = 0usize;
    for &i in &indices {
        let Some(session) = sessions.get(i) else {
            all_ready = false;
            continue;
        };
        if session.body_loaded {
            total += session.records.len();
        } else {
            all_ready = false;
        }
    }

    let end = budget.min(total);
    let mut rows = Vec::with_capacity(end);
    let mut taken = 0usize;
    'outer: for &i in &indices {
        let Some(session) = sessions.get(i) else {
            continue;
        };
        if !session.body_loaded {
            continue;
        }
        for record in &session.records {
            if taken >= end {
                break 'outer;
            }
            rows.push(WindowRow::meta_only(
                dialogue_key(&session.source, &session.session_id, record),
                DialogueMeta {
                    source: session.source.clone(),
                    work_ref: Some(record.work_ref.clone()),
                    title: record.title.clone(),
                },
            ));
            taken += 1;
        }
    }
    (rows, all_ready && end >= total)
}

fn body_for_key(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
    key: &DialogueKey,
) -> Option<WorkspaceDialogue> {
    for i in active_session_indices(sessions, session_idx, selected_sessions) {
        let session = sessions.get(i)?;
        if !session.body_loaded {
            continue;
        }
        for record in &session.records {
            if dialogue_key(&session.source, &session.session_id, record) == *key {
                return Some(dialogue_from_record(session, record));
            }
        }
    }
    None
}

// ── Content ─────────────────────────────────────────────────────────────

pub type ContentEngine = SlidingPane<usize, (), ()>;

#[derive(Default)]
pub struct ContentPane {
    engine: ContentEngine,
}

/// Domain context for content line-count catalog.
pub struct ContentCtx<'a> {
    pub dialogues: &'a [WorkspaceDialogue],
    pub selected_dialogues: &'a [bool],
    pub highlighted_idx: usize,
    pub mode: ContentViewMode,
    pub target: Option<WorkAt>,
    pub area: ratatui::layout::Rect,
}

impl ContentPane {
    /// Layout line count last ensured (at least 1).
    pub fn line_count(&self) -> usize {
        self.engine.len().max(1)
    }
}

impl Pane for ContentPane {
    type Ctx<'a> = ContentCtx<'a>;

    fn ensure(&mut self, ctx: ContentCtx<'_>, _input: &PaneInput) -> bool {
        let text = workspace_content_text(
            ctx.dialogues,
            ctx.selected_dialogues,
            ctx.highlighted_idx,
            ctx.mode,
            ctx.target,
        );
        let total = content_view_line_count(ctx.area, &text, ctx.mode).max(1);
        if self.engine.len() == total && self.engine.exhausted() {
            return false;
        }
        let incoming: Vec<WindowRow<usize, (), ()>> =
            (0..total).map(|i| WindowRow::meta_only(i, ())).collect();
        self.engine.set_catalog(incoming, true);
        true
    }

    fn len(&self) -> usize {
        self.engine.len()
    }
}

// ── Compatibility helpers used by tests / selection ─────────────────────

// (none — new panes implement `Pane` only)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::Viewport;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkRecord, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
    };
    use std::time::UNIX_EPOCH;

    fn test_record(session: &str, index: usize, title: &str) -> WorkRecord {
        WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, session, index),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: session.to_string(),
                canonical_id: Some(session.to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::from_components(None, Some("2026-07-17T10:00:00Z".into()), None),
            status: None,
            title: title.to_string(),
            parts: vec![],
        }
    }

    fn session_with_n(n: usize, body_loaded: bool) -> WorkspaceSession {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let records: Vec<_> = (0..n)
            .map(|i| test_record("s", i + 1, &format!("t{i}")))
            .collect();
        WorkspaceSession {
            source,
            session_id: "s".into(),
            modified: UNIX_EPOCH,
            title: "s".into(),
            search_title: "s".into(),
            records: if body_loaded { records } else { Vec::new() },
            body_loaded,
        }
    }

    fn tick(
        pane: &mut DialoguePane,
        sessions: &[WorkspaceSession],
        viewport: Viewport,
        focus: usize,
        selected: &[bool],
    ) {
        pane.ensure(
            DialogueCtx {
                sessions,
                session_idx: 0,
                selected_sessions: &[true],
            },
            &PaneInput::new(viewport, focus).with_selected(selected.to_vec()),
        );
    }

    #[test]
    fn source_pane_is_exhausted_static() {
        let sources = vec![
            WorkspaceSource::terminal(),
            WorkspaceSource::agent(AgentProvider::Codex),
        ];
        let mut pane = SourcePane::from_catalog(&sources);
        assert!(pane.exhausted());
        assert_eq!(pane.len(), 2);
        assert!(!pane.ensure((), &PaneInput::new(Viewport { first: 0, visible: 40 }, 0)));
    }

    #[test]
    fn dialogue_waits_for_session_body_then_fills() {
        let pending = vec![session_with_n(0, false)];
        let mut pane = DialoguePane::default();
        tick(
            &mut pane,
            &pending,
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
            &[],
        );
        assert_eq!(pane.len(), 0);
        assert!(!pane.exhausted());

        let ready = vec![session_with_n(30, true)];
        tick(
            &mut pane,
            &ready,
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
            &[],
        );
        assert!(pane.len() >= 20, "len={}", pane.len());
    }

    #[test]
    fn dialogue_meta_grows_with_viewport_not_full_catalog() {
        let sessions = vec![session_with_n(100, true)];
        let mut pane = DialoguePane::default();
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
            &[],
        );
        assert!(pane.len() < 100);
        assert!(pane.len() >= 20);
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 40,
                visible: 10,
            },
            45,
            &[],
        );
        assert!(pane.len() > 40);
        assert!(pane.len() < 100);
    }

    #[test]
    fn dialogue_bodies_only_for_keep_set() {
        let sessions = vec![session_with_n(40, true)];
        let mut pane = DialoguePane::default();
        let empty = vec![false; 40];
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 20,
            },
            5,
            &empty,
        );
        let sel = vec![false; pane.len()];
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 20,
            },
            5,
            &sel,
        );
        let loaded: Vec<_> = pane
            .dialogues()
            .iter()
            .enumerate()
            .filter(|(_, d)| d.record.is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(loaded, vec![4, 5, 6]);
    }

    #[test]
    fn dialogue_context_change_clears_pane() {
        let sessions = vec![session_with_n(30, true)];
        let mut pane = DialoguePane::default();
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
            &[],
        );
        assert!(pane.len() > 0);
        pane.ensure(
            DialogueCtx {
                sessions: &[],
                session_idx: 0,
                selected_sessions: &[],
            },
            &PaneInput::new(
                Viewport {
                    first: 0,
                    visible: 10,
                },
                0,
            ),
        );
        assert_eq!(pane.len(), 0);
    }
}
