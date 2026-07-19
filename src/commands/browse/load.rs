//! Session pane loaders: catalog + workset I/O on [`crate::pane::SlidingPane`].
//!
//! Per selected source owns one [`SessionPane`]. Meta/body growth is driven by
//! [`SlidingPane::ensure_meta`] / [`SlidingPane::ensure_bodies`]; this module
//! only fulfills those requests over workset.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::commands::memory::filter::Filter;
use crate::commands::memory::workset::{self, QuerySource, QuerySourceResult, REMOTE_QUERY_TIMEOUT};
use crate::pane::{
    keep_keys, MetaNeed, Pane, PaneInput, SlidingPane, StorePhase, Viewport, WindowRow,
    FETCH_CEILING, FETCH_FLOOR,
};
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse};
use crate::tui::workspace::{
    SourceLoadMarker, WorkspaceSession, WorkspaceSource, WorkspaceSourceKind,
};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::WorkRecord;
use sivtr_core::workspace;

/// Session meta without dialogue bodies.
#[derive(Clone, Debug)]
pub struct SessionMeta {
    pub source: WorkspaceSource,
    pub session_id: String,
    pub modified: SystemTime,
    pub title: String,
    pub search_title: String,
}

pub type SessionKey = String;
pub type SessionBody = Vec<WorkRecord>;
pub type SessionPane = SlidingPane<SessionKey, SessionMeta, SessionBody>;

/// UI-facing per-source session pane.
#[derive(Clone, Debug, Default)]
pub struct SourceLoadState {
    pub pane: SessionPane,
}

impl SourceLoadState {
    pub fn idle() -> Self {
        Self {
            pane: SessionPane::default(),
        }
    }

    pub fn ready_from_sessions(sessions: Vec<WorkspaceSession>, budget: usize) -> Self {
        let rows = sessions
            .into_iter()
            .map(|s| {
                let key = s.session_id.clone();
                let meta = SessionMeta {
                    source: s.source,
                    session_id: s.session_id,
                    modified: s.modified,
                    title: s.title,
                    search_title: s.search_title,
                };
                if s.body_loaded && !s.records.is_empty() {
                    WindowRow::with_body(key, meta, s.records)
                } else {
                    WindowRow::meta_only(key, meta)
                }
            })
            .collect();
        Self {
            pane: SessionPane::ready(rows, budget, true),
        }
    }

    pub fn marker(&self) -> SourceLoadMarker {
        let store = self.pane.store();
        match store.phase {
            StorePhase::Idle => SourceLoadMarker::Idle,
            StorePhase::Booting => SourceLoadMarker::Loading,
            StorePhase::Ready if store.list_inflight => SourceLoadMarker::Loading,
            StorePhase::Ready => SourceLoadMarker::Ready,
            StorePhase::Failed => SourceLoadMarker::Failed,
        }
    }

    pub fn is_fetching(&self) -> bool {
        self.pane.is_fetching()
    }

    pub fn visible_sessions(&self) -> Vec<WorkspaceSession> {
        self.pane
            .rows()
            .iter()
            .map(row_to_workspace_session)
            .collect()
    }
}

fn row_to_workspace_session(
    row: &WindowRow<SessionKey, SessionMeta, SessionBody>,
) -> WorkspaceSession {
    WorkspaceSession {
        source: row.meta.source.clone(),
        session_id: row.meta.session_id.clone(),
        modified: row.meta.modified,
        title: row.meta.title.clone(),
        search_title: row.meta.search_title.clone(),
        records: row.body.clone().unwrap_or_default(),
        body_loaded: row.body_loaded,
    }
}

#[derive(Debug)]
enum JobKind {
    Meta { budget: usize },
    Body { session_id: String },
}

#[derive(Debug)]
struct JobEvent {
    index: usize,
    gen: u64,
    kind: JobKind,
    result: std::result::Result<Vec<WorkspaceSession>, String>,
    exhausted: bool,
}

/// Background workset pump that fulfills session [`SlidingPane`] needs.
pub struct SourceLoadPump {
    tx: Sender<JobEvent>,
    rx: Receiver<JobEvent>,
    cwd: PathBuf,
    body_inflight: HashSet<String>,
}

impl SourceLoadPump {
    pub fn new(_source_count: usize, cwd: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            cwd,
            body_inflight: HashSet::new(),
        }
    }

    pub fn kick(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: Viewport,
        force: bool,
    ) {
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            let need: Option<MetaNeed> = if force {
                states[idx].pane.force_meta(viewport)
            } else {
                states[idx].pane.ensure_meta(viewport)
            };
            if let Some(MetaNeed { gen, budget }) = need {
                self.spawn_meta(idx, source, gen, budget);
            }
        }
    }

    pub fn refresh_selected(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: Viewport,
    ) {
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            if let Some(need) = states[idx].pane.force_meta(viewport) {
                self.spawn_meta(idx, source, need.gen, need.budget);
            }
        }
    }

    /// Grow per-source panes until the merged session list covers `viewport`.
    pub fn ensure_meta_window(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: Viewport,
        merged_len: usize,
    ) {
        let need_end = viewport.need_end();
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            // Per-pane native ensure first.
            if let Some(need) = states[idx].pane.ensure_meta(viewport) {
                self.spawn_meta(idx, source, need.gen, need.budget);
                continue;
            }
            // Multi-source: this pane alone may be "covered" while the merged
            // list is still short — ask for a larger budget explicitly.
            if merged_len >= need_end {
                continue;
            }
            let store = states[idx].pane.store();
            if store.list_inflight || states[idx].pane.exhausted() || store.rows.is_empty() {
                continue;
            }
            let deficit = need_end - merged_len;
            let target = store.rows.len().saturating_add(deficit);
            let next = Viewport::fetch_budget(target)
                .max(store.fetch_budget + FETCH_FLOOR)
                .min(FETCH_CEILING);
            if next <= store.fetch_budget {
                let forced = (store.fetch_budget * 2)
                    .max(store.fetch_budget + FETCH_FLOOR)
                    .min(FETCH_CEILING);
                if forced > store.fetch_budget {
                    if let Some(need) = states[idx].pane.begin_meta_budget(forced) {
                        self.spawn_meta(idx, source, need.gen, need.budget);
                    }
                }
                continue;
            }
            if let Some(need) = states[idx].pane.begin_meta_budget(next) {
                self.spawn_meta(idx, source, need.gen, need.budget);
            }
        }
    }

    pub fn sync_bodies(
        &mut self,
        sources: &[WorkspaceSource],
        states: &mut [SourceLoadState],
        keep: &HashSet<(usize, String)>,
    ) {
        for (source_idx, state) in states.iter_mut().enumerate() {
            let keep_local: HashSet<String> = keep
                .iter()
                .filter(|(si, _)| *si == source_idx)
                .map(|(_, id)| id.clone())
                .collect();
            let missing = state.pane.ensure_bodies(keep_local);
            let Some(source) = sources.get(source_idx) else {
                continue;
            };
            for session_id in missing {
                let ik = format!("{source_idx}\0{session_id}");
                if self.body_inflight.contains(&ik) {
                    continue;
                }
                self.body_inflight.insert(ik);
                self.spawn_body(source_idx, source, &session_id);
            }
        }
    }

    pub fn drop_unselected(&mut self, selected: &[bool], states: &mut [SourceLoadState]) {
        for (idx, sel) in selected.iter().enumerate() {
            if *sel {
                continue;
            }
            if let Some(state) = states.get_mut(idx) {
                state.pane.clear();
            }
            self.body_inflight
                .retain(|k| !k.starts_with(&format!("{idx}\0")));
        }
    }

    fn spawn_meta(&mut self, idx: usize, source: &WorkspaceSource, gen: u64, budget: usize) {
        let budget = budget.clamp(FETCH_FLOOR, FETCH_CEILING);
        let selector = source.selector();
        let remote = source.is_remote();
        let cwd = self.cwd.clone();
        let tx = self.tx.clone();
        let source = source.clone();
        thread::spawn(move || {
            let qs = if remote {
                QuerySource::remote(selector)
            } else {
                QuerySource::local(selector)
            };
            let (result, exhausted) = match workset::query_many(
                &[qs],
                Filter::browse_session_page(budget),
                Some(&cwd),
                REMOTE_QUERY_TIMEOUT,
            ) {
                Ok(mut results) => match results.pop() {
                    Some(QuerySourceResult::Ok(set)) => {
                        let n = set.records.len();
                        let mut sessions = sessions_from_records(&source, set.records);
                        for s in &mut sessions {
                            s.records.clear();
                            s.body_loaded = false;
                        }
                        sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
                        (Ok(sessions), n < budget)
                    }
                    Some(QuerySourceResult::Err(m)) => (Err(m), false),
                    None => (Err("empty query".into()), false),
                },
                Err(e) => (Err(format!("{e:#}")), false),
            };
            let _ = tx.send(JobEvent {
                index: idx,
                gen,
                kind: JobKind::Meta { budget },
                result,
                exhausted,
            });
        });
    }

    fn spawn_body(&mut self, idx: usize, source: &WorkspaceSource, session_id: &str) {
        let gen = sources_list_gen_placeholder();
        let selector = source.selector();
        let remote = source.is_remote();
        let cwd = self.cwd.clone();
        let tx = self.tx.clone();
        let source = source.clone();
        let session_id = session_id.to_string();
        thread::spawn(move || {
            let sel = format!("{selector}/{session_id}");
            let qs = if remote {
                QuerySource::remote(sel)
            } else {
                QuerySource::local(sel)
            };
            let result = match workset::query_many(
                &[qs],
                Filter::none(),
                Some(&cwd),
                REMOTE_QUERY_TIMEOUT,
            ) {
                Ok(mut results) => match results.pop() {
                    Some(QuerySourceResult::Ok(set)) => {
                        let mut sessions = sessions_from_records(&source, set.records);
                        for s in &mut sessions {
                            s.body_loaded = !s.records.is_empty();
                        }
                        Ok(sessions)
                    }
                    Some(QuerySourceResult::Err(m)) => Err(m),
                    None => Err("empty body".into()),
                },
                Err(e) => Err(format!("{e:#}")),
            };
            let _ = tx.send(JobEvent {
                index: idx,
                gen,
                kind: JobKind::Body {
                    session_id: session_id.clone(),
                },
                result,
                exhausted: true,
            });
        });
    }

    pub fn drain(&mut self, states: &mut [SourceLoadState]) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(ev) => {
                    if self.apply(ev, states) {
                        changed = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        changed
    }

    fn apply(&mut self, ev: JobEvent, states: &mut [SourceLoadState]) -> bool {
        let Some(state) = states.get_mut(ev.index) else {
            return false;
        };
        match ev.kind {
            JobKind::Body { session_id } => {
                self.body_inflight
                    .remove(&format!("{}\0{session_id}", ev.index));
                let Ok(sessions) = ev.result else {
                    return false;
                };
                let mut changed = false;
                for s in sessions {
                    if state.pane.apply_body(&s.session_id, s.records) {
                        changed = true;
                    }
                }
                changed
            }
            JobKind::Meta { budget } => match ev.result {
                Ok(sessions) => {
                    let rows = sessions
                        .into_iter()
                        .map(|s| {
                            WindowRow::meta_only(
                                s.session_id.clone(),
                                SessionMeta {
                                    source: s.source,
                                    session_id: s.session_id,
                                    modified: s.modified,
                                    title: s.title,
                                    search_title: s.search_title,
                                },
                            )
                        })
                        .collect();
                    state
                        .pane
                        .apply_meta_ok(ev.gen, budget, ev.exhausted, rows)
                }
                Err(message) => state.pane.apply_meta_err(ev.gen, message),
            },
        }
    }
}

/// Body jobs do not use list_gen cancellation; placeholder keeps the event shape.
fn sources_list_gen_placeholder() -> u64 {
    0
}

// ── Session column as unified [`Pane`] ──────────────────────────────────

/// Multi-source session column. Implements [`Pane`]; picker only calls
/// `poll` / `ensure` / `sessions`.
pub struct SessionColumn {
    sources: Vec<WorkspaceSource>,
    states: Vec<SourceLoadState>,
    pump: SourceLoadPump,
    /// Last merged list length (for multi-source budget expansion).
    merged_len: usize,
}

/// One-frame context for session ensure.
pub struct SessionCtx<'a> {
    pub selected_sources: &'a [bool],
    /// Merged sessions currently shown (for body keep mapping).
    pub sessions: &'a [WorkspaceSession],
    pub selected_sessions: &'a [bool],
    /// When true, skip meta growth (search filter owns the list).
    pub search_active: bool,
}

impl SessionColumn {
    pub fn new(sources: Vec<WorkspaceSource>, states: Vec<SourceLoadState>, cwd: PathBuf) -> Self {
        let n = sources.len();
        Self {
            sources,
            states,
            pump: SourceLoadPump::new(n, cwd),
            merged_len: 0,
        }
    }

    pub fn sources(&self) -> &[WorkspaceSource] {
        &self.sources
    }

    pub fn markers(&self) -> Vec<SourceLoadMarker> {
        self.states.iter().map(SourceLoadState::marker).collect()
    }

    pub fn collect(&self, selected: &[bool]) -> Vec<WorkspaceSession> {
        collect_ready_sessions(&self.sources, selected, &self.states)
    }

    /// Bootstrap / force-load selected sources.
    pub fn kick(&mut self, selected: &[bool], viewport: Viewport, force: bool) {
        self.pump
            .kick(&self.sources, selected, &mut self.states, viewport, force);
    }

    /// `R` reload of given source mask.
    pub fn refresh(&mut self, selected: &[bool], viewport: Viewport) {
        self.pump
            .refresh_selected(&self.sources, selected, &mut self.states, viewport);
    }
}

impl Pane for SessionColumn {
    type Ctx<'a> = SessionCtx<'a>;

    fn poll(&mut self) -> bool {
        self.pump.drain(&mut self.states)
    }

    fn ensure(&mut self, ctx: SessionCtx<'_>, input: &PaneInput) -> bool {
        self.pump
            .drop_unselected(ctx.selected_sources, &mut self.states);
        if !ctx.search_active {
            if input.force {
                self.pump.refresh_selected(
                    &self.sources,
                    ctx.selected_sources,
                    &mut self.states,
                    input.viewport,
                );
            } else {
                self.pump.ensure_meta_window(
                    &self.sources,
                    ctx.selected_sources,
                    &mut self.states,
                    input.viewport,
                    self.merged_len,
                );
            }
        }
        let keep = body_keep_set(
            &self.sources,
            ctx.sessions,
            input.focus,
            ctx.selected_sessions,
            input.neighbor_radius,
        );
        self.pump
            .sync_bodies(&self.sources, &mut self.states, &keep);
        // Update merged length for next multi-source budget decision.
        self.merged_len = ctx.sessions.len();
        true
    }

    fn len(&self) -> usize {
        self.merged_len
    }

    fn is_fetching(&self) -> bool {
        self.states.iter().any(SourceLoadState::is_fetching)
    }
}

pub fn body_keep_set(
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    focus_idx: usize,
    selected_sessions: &[bool],
    neighbor_radius: usize,
) -> HashSet<(usize, String)> {
    let index_keys: Vec<usize> = (0..sessions.len()).collect();
    let keep_idx = keep_keys(&index_keys, focus_idx, selected_sessions, neighbor_radius);
    keep_idx
        .into_iter()
        .filter_map(|i| {
            let session = sessions.get(i)?;
            let si = source_index_for_session(sources, session)?;
            Some((si, session.session_id.clone()))
        })
        .collect()
}

pub fn workspace_source_catalog(
    providers: &[AgentProvider],
    cwd: &Path,
) -> Result<Vec<WorkspaceSource>> {
    let mut sources = Vec::new();
    sources.push(WorkspaceSource::terminal());
    for provider in providers {
        sources.push(WorkspaceSource::agent(*provider));
    }
    for alias in list_remote_aliases(cwd)? {
        sources.push(WorkspaceSource::scoped(
            &alias,
            WorkspaceSourceKind::Terminal,
        ));
        for provider in providers {
            sources.push(WorkspaceSource::scoped(
                &alias,
                WorkspaceSourceKind::Agent(*provider),
            ));
        }
    }
    Ok(sources)
}

fn list_remote_aliases(cwd: &Path) -> Result<Vec<String>> {
    let Some(ws) = workspace::resolve_workspace_for_dir(cwd)? else {
        return Ok(Vec::new());
    };
    if !ipc::running() {
        return Ok(Vec::new());
    }
    match ipc::call(LocalRequest::RemoteList {
        workspace_key: ws.key,
    }) {
        Ok(LocalResponse::Mounts(mounts)) => Ok(mounts.into_iter().map(|m| m.alias).collect()),
        Ok(_) => Ok(Vec::new()),
        Err(_) => Ok(Vec::new()),
    }
}

pub fn collect_ready_sessions(
    sources: &[WorkspaceSource],
    selected: &[bool],
    states: &[SourceLoadState],
) -> Vec<WorkspaceSession> {
    let mut sessions = Vec::new();
    for (idx, _) in sources.iter().enumerate() {
        if !selected.get(idx).copied().unwrap_or(false) {
            continue;
        }
        sessions.extend(states[idx].visible_sessions());
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
    sessions
}

pub fn source_index_for_session(
    sources: &[WorkspaceSource],
    session: &WorkspaceSession,
) -> Option<usize> {
    sources.iter().position(|source| source == &session.source)
}

pub fn sessions_from_records(
    source: &WorkspaceSource,
    records: Vec<WorkRecord>,
) -> Vec<WorkspaceSession> {
    let mut groups: BTreeMap<String, Vec<WorkRecord>> = BTreeMap::new();
    for record in records {
        let key = record.work_ref.session().to_string();
        groups.entry(key).or_default().push(record);
    }
    let mut sessions = Vec::with_capacity(groups.len());
    for (session_id, mut records) in groups {
        records.sort_by_key(|record| record.work_ref.path.index());
        let modified = records
            .iter()
            .filter_map(record_modified)
            .max()
            .unwrap_or(UNIX_EPOCH);
        let search_title = session_search_title(&session_id, &records);
        let title = session_title_with_id(search_title.clone(), Some(session_id.as_str()));
        let body_loaded = !records.is_empty();
        sessions.push(WorkspaceSession {
            source: source.clone(),
            session_id,
            modified,
            title,
            search_title,
            records,
            body_loaded,
        });
    }
    sessions
}

fn session_search_title(session_id: &str, records: &[WorkRecord]) -> String {
    records
        .iter()
        .find_map(|record| {
            let title = record.title.trim();
            if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            }
        })
        .unwrap_or_else(|| session_id.to_string())
}

fn session_title_with_id(title: String, id: Option<&str>) -> String {
    let id = id.map(|value| value.chars().take(8).collect::<String>());
    match id {
        Some(id) if !id.is_empty() => format!("{title}  [{id}]"),
        _ => title,
    }
}

fn record_modified(record: &WorkRecord) -> Option<SystemTime> {
    let stamp = record.time.primary_at()?;
    let dt = DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))?;
    let secs = dt.timestamp().max(0) as u64;
    let nanos = dt.timestamp_subsec_nanos();
    Some(UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_nanos(nanos.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource, WorkTime,
    };

    #[test]
    fn sessions_from_records_groups_by_session() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let records = vec![
            test_record("s1", 1, "first", "2026-07-17T10:00:00Z"),
            test_record("s1", 2, "second", "2026-07-17T11:00:00Z"),
            test_record("s2", 1, "other", "2026-07-17T12:00:00Z"),
        ];
        let sessions = sessions_from_records(&source, records);
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn body_keep_set_uses_shared_keep_keys() {
        let sources = vec![WorkspaceSource::agent(AgentProvider::Codex)];
        let sessions = vec![
            WorkspaceSession {
                source: sources[0].clone(),
                session_id: "a".into(),
                modified: UNIX_EPOCH,
                title: "a".into(),
                search_title: "a".into(),
                records: vec![],
                body_loaded: false,
            },
            WorkspaceSession {
                source: sources[0].clone(),
                session_id: "b".into(),
                modified: UNIX_EPOCH,
                title: "b".into(),
                search_title: "b".into(),
                records: vec![],
                body_loaded: false,
            },
            WorkspaceSession {
                source: sources[0].clone(),
                session_id: "c".into(),
                modified: UNIX_EPOCH,
                title: "c".into(),
                search_title: "c".into(),
                records: vec![],
                body_loaded: false,
            },
        ];
        let selected = [false, false, false];
        let keep = body_keep_set(&sources, &sessions, 1, &selected, 1);
        assert!(keep.contains(&(0, "a".into())));
        assert!(keep.contains(&(0, "b".into())));
        assert!(keep.contains(&(0, "c".into())));
    }

    fn test_record(session: &str, index: usize, title: &str, ended: &str) -> WorkRecord {
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
            time: WorkTime::from_components(None, Some(ended.to_string()), None),
            status: None,
            title: title.to_string(),
            parts: vec![],
        }
    }
}
