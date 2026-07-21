//! Browse product panes implementing [`crate::pane::Pane`].
//!
//! **New pane checklist**
//! 1. `struct MyPane { engine: SlidingPane<K,M,B>, … }`
//! 2. `impl Pane for MyPane` — only map data + call SlidingPane ensure_*
//! 3. Register in picker: `my_pane.poll(); my_pane.ensure(ctx, &input);`
//!
//! Do **not** reimplement viewport growth, keep/evict, or blanking rules.

use crate::pane::{Pane, PaneInput, SlidingPane, WindowRow};
use crate::tui::content::view::ContentViewMode;
use crate::tui::workspace::{
    workspace_content_io_texts, ContentIoFocus, ContentIoFrame, ContentIoTexts, WorkspaceDialogue,
    WorkspaceSession, WorkspaceSource,
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

    fn ensure(&mut self, _ctx: (), _input: &PaneInput<'_>) -> bool {
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
    /// List paint: titles only (no body clone).
    pub fn titles(&self) -> impl Iterator<Item = &str> + '_ {
        self.engine.rows().iter().map(|r| r.meta.title.as_str())
    }

    /// Index-stable rows for content/copy/vim.
    /// Clones **body only** for focus ∪ multi-select; other rows are title shells.
    pub fn materialize(&self, selected: &[bool], focus: usize) -> Vec<WorkspaceDialogue> {
        let rows = self.engine.rows();
        if rows.is_empty() {
            return Vec::new();
        }
        let any = selected.iter().any(|s| *s);
        let focus = focus.min(rows.len() - 1);
        rows.iter()
            .enumerate()
            .map(|(i, row)| {
                let need_body = if any {
                    selected.get(i).copied().unwrap_or(false)
                } else {
                    i == focus
                };
                if need_body {
                    if let Some(body) = row.body.clone() {
                        body
                    } else {
                        shell_from_row(row)
                    }
                } else {
                    shell_from_row(row)
                }
            })
            .collect()
    }

    #[cfg(test)]
    pub fn exhausted(&self) -> bool {
        self.engine.exhausted()
    }

    #[cfg(test)]
    pub fn dialogues(&self) -> Vec<WorkspaceDialogue> {
        // Tests want full bodies when present.
        let n = self.engine.len();
        let selected = vec![true; n];
        self.materialize(&selected, 0)
    }

    /// Bench-only: inspect engine rows without cloning the pane.
    #[cfg(feature = "perf-benches")]
    pub(crate) fn engine_rows_for_perf(
        &self,
    ) -> &[crate::pane::WindowRow<DialogueKey, DialogueMeta, WorkspaceDialogue>] {
        self.engine.rows()
    }
}

fn shell_from_row(
    row: &crate::pane::WindowRow<DialogueKey, DialogueMeta, WorkspaceDialogue>,
) -> WorkspaceDialogue {
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

/// Domain context for dialogue ensure (one frame).
///
/// `sessions` is the **meta** list (titles/ids/body_loaded). Turn bodies are
/// read through `records` (product: `SessionColumn::body_for`).
pub struct DialogueCtx<'a> {
    pub sessions: &'a [WorkspaceSession],
    pub session_idx: usize,
    pub selected_sessions: &'a [bool],
    /// Body lookup; returned slice lives as long as the storage behind the
    /// callback (`SessionColumn` / fixture table), not the `&session` arg.
    pub records: &'a dyn Fn(&WorkspaceSession) -> Option<&'a [WorkRecord]>,
}

impl Pane for DialoguePane {
    type Ctx<'a> = DialogueCtx<'a>;

    fn ensure(&mut self, ctx: DialogueCtx<'_>, input: &PaneInput<'_>) -> bool {
        let next = fingerprint(ctx.sessions, ctx.session_idx, ctx.selected_sessions, ctx.records);
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
                ctx.records,
                budget,
            )
        });

        let keep = self
            .engine
            .keep_for_focus(input.focus, input.selected, input.neighbor_radius);
        self.engine.ensure_bodies_sync(keep, |key| {
            body_for_key(
                ctx.sessions,
                ctx.session_idx,
                ctx.selected_sessions,
                ctx.records,
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

fn fingerprint<'a>(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
    records: &dyn Fn(&WorkspaceSession) -> Option<&'a [WorkRecord]>,
) -> DialogueFingerprint {
    DialogueFingerprint {
        sessions: active_session_indices(sessions, session_idx, selected_sessions)
            .into_iter()
            .filter_map(|i| {
                let s = sessions.get(i)?;
                let n = records(s).map(|r| r.len()).unwrap_or(0);
                Some((
                    s.source.selector(),
                    s.session_id.clone(),
                    s.body_loaded,
                    n,
                ))
            })
            .collect(),
    }
}

fn meta_prefix<'a>(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
    records: &dyn Fn(&WorkspaceSession) -> Option<&'a [WorkRecord]>,
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
    let mut bodies: Vec<Option<&'a [WorkRecord]>> = Vec::with_capacity(indices.len());
    for &i in &indices {
        let Some(session) = sessions.get(i) else {
            all_ready = false;
            bodies.push(None);
            continue;
        };
        if session.body_loaded {
            match records(session) {
                Some(recs) => {
                    total += recs.len();
                    bodies.push(Some(recs));
                }
                None => {
                    // Flagged loaded but body not yet in pane (async gap).
                    all_ready = false;
                    bodies.push(None);
                }
            }
        } else {
            all_ready = false;
            bodies.push(None);
        }
    }

    let end = budget.min(total);
    let mut rows = Vec::with_capacity(end);
    let mut taken = 0usize;
    'outer: for (pos, &i) in indices.iter().enumerate() {
        let Some(session) = sessions.get(i) else {
            continue;
        };
        let Some(recs) = bodies[pos] else {
            continue;
        };
        for record in recs {
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

fn body_for_key<'a>(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    selected_sessions: &[bool],
    records: &dyn Fn(&WorkspaceSession) -> Option<&'a [WorkRecord]>,
    key: &DialogueKey,
) -> Option<WorkspaceDialogue> {
    for i in active_session_indices(sessions, session_idx, selected_sessions) {
        let Some(session) = sessions.get(i) else {
            continue;
        };
        if !session.body_loaded {
            continue;
        }
        let Some(recs) = records(session) else {
            continue;
        };
        for record in recs {
            if dialogue_key(&session.source, &session.session_id, record) == *key {
                return Some(dialogue_from_record(session, record));
            }
        }
    }
    None
}

// ── Content ─────────────────────────────────────────────────────────────

/// Domain context for dual IO content line-count catalogs.
pub struct ContentCtx<'a> {
    pub dialogues: &'a [WorkspaceDialogue],
    pub selected_dialogues: &'a [bool],
    pub highlighted_idx: usize,
    pub mode: ContentViewMode,
    pub target: Option<WorkAt>,
    pub area: ratatui::layout::Rect,
}

/// Tracks layout line counts for Input / Output halves separately.
#[derive(Default)]
pub struct ContentPane {
    input_lines: usize,
    output_lines: usize,
}

impl ContentPane {
    pub fn line_count(&self, half: ContentIoFocus) -> usize {
        match half {
            ContentIoFocus::Input => self.input_lines.max(1),
            ContentIoFocus::Output => self.output_lines.max(1),
        }
    }

    /// Build texts + dynamic layout metrics for this frame.
    pub fn ensure(&mut self, ctx: ContentCtx<'_>) -> ContentIoTexts {
        let texts = workspace_content_io_texts(
            ctx.dialogues,
            ctx.selected_dialogues,
            ctx.highlighted_idx,
            ctx.mode,
            ctx.target,
        );
        let frame = ContentIoFrame::build(ctx.area, &texts, ctx.mode);
        self.input_lines = frame.input_lines;
        self.output_lines = frame.output_lines;
        texts
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
        // Fixture table owns bodies; lookup by key (not reborrow of arg).
        let records = |s: &WorkspaceSession| {
            sessions
                .iter()
                .find(|x| x.session_id == s.session_id && x.source == s.source)
                .filter(|x| x.body_loaded)
                .map(|x| x.records.as_slice())
        };
        pane.ensure(
            DialogueCtx {
                sessions,
                session_idx: 0,
                selected_sessions: &[true],
                records: &records,
            },
            &PaneInput::new(viewport, focus).with_selected(selected),
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
        let empty: &[WorkspaceSession] = &[];
        let records = |s: &WorkspaceSession| {
            empty
                .iter()
                .find(|x| x.session_id == s.session_id)
                .map(|x| x.records.as_slice())
        };
        pane.ensure(
            DialogueCtx {
                sessions: empty,
                session_idx: 0,
                selected_sessions: &[],
                records: &records,
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

    #[test]
    fn materialize_clones_body_only_for_focus() {
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
        assert_eq!(pane.titles().count(), pane.len());
        let rows = pane.materialize(&sel, 5);
        let with_body = rows.iter().filter(|d| d.record.is_some()).count();
        assert_eq!(with_body, 1);
        assert!(rows[5].record.is_some());
    }
}
