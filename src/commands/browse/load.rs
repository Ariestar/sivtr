//! Source catalog + viewport-driven sliding-window loads for browse.
//!
//! ## Memory model (final)
//!
//! - **L1 meta prefix** per source: `WorkspaceSession` rows with empty `records`
//!   until hydrated. Grows only as the session-list viewport demands.
//! - **L2 body**: full `WorkRecord`s live only on sessions in the **keep set**
//!   (focus + multi-select + small optional neighbor). Everything else is
//!   `records.clear()` + `body_loaded = false`.
//! - **No product page size.** Batch sizes are derived from
//!   `visible_rows` (layout) with a small I/O floor so we never under-fetch one
//!   screen. CLI `sivtr s` / workset search are untouched.
//!
//! Multi-source schedule still uses `workset::query_many`.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::commands::memory::filter::Filter;
use crate::commands::memory::workset::{self, QuerySource, QuerySourceResult, REMOTE_QUERY_TIMEOUT};
use crate::remote::ipc;
use crate::remote::protocol::{LocalRequest, LocalResponse};
use crate::tui::workspace::{
    SourceLoadMarker, WorkspaceSession, WorkspaceSource, WorkspaceSourceKind,
};
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::WorkRecord;
use sivtr_core::workspace;

/// I/O floor so a 1-row glitch never fetches a single record forever.
const META_FETCH_FLOOR: usize = 12;
/// Absolute ceiling per source (pathological workspaces).
const META_FETCH_CEILING: usize = 2_000;
/// Prefetch screens beyond the visible session pane (1 = one screen above/below).
const META_PREFETCH_SCREENS: usize = 1;

/// Session-list viewport in **merged list** coordinates (after mtime sort).
#[derive(Clone, Copy, Debug)]
pub struct SessionViewport {
    /// First visible row in the merged session list (`ListState::offset`).
    pub first_visible: usize,
    /// Rows that fit in the sessions panel (inner height).
    pub visible_rows: usize,
}

impl SessionViewport {
    pub fn from_panel(offset: usize, panel_inner_height: usize) -> Self {
        Self {
            first_visible: offset,
            visible_rows: panel_inner_height.max(1),
        }
    }

    /// Exclusive end index we want covered by L1 meta (viewport + prefetch).
    pub fn meta_need_end(&self, merged_len: usize) -> usize {
        let page = self.visible_rows;
        let prefetch = page.saturating_mul(META_PREFETCH_SCREENS);
        let end = self
            .first_visible
            .saturating_add(page)
            .saturating_add(prefetch);
        if merged_len == 0 {
            end.max(page.saturating_add(prefetch))
        } else {
            end
        }
    }

    /// How many **records** to request so folding is likely to yield ~`session_target` sessions.
    ///
    /// workset `latest` is record-based; we over-fetch slightly then fold to sessions.
    pub fn record_budget_for_sessions(session_target: usize) -> usize {
        // ~3 records/session heuristic, clamped.
        let raw = session_target.saturating_mul(3).max(META_FETCH_FLOOR);
        raw.min(META_FETCH_CEILING)
    }
}

/// Per-source load state.
#[derive(Clone, Debug)]
pub enum SourceLoadState {
    Idle,
    Loading {
        stale: Option<SourceSessionStore>,
        gen: u64,
    },
    Ready(SourceSessionStore),
    Failed {
        #[allow(dead_code)]
        message: String,
        stale: Option<SourceSessionStore>,
    },
}

/// L1 meta prefix for one source (newest-first after workset sort/fold).
#[derive(Clone, Debug, Default)]
pub struct SourceSessionStore {
    pub sessions: Vec<WorkspaceSession>,
    /// Record budget last successfully fetched for this prefix.
    pub fetch_budget: usize,
    /// True when last fetch returned fewer records than requested.
    pub exhausted: bool,
}

impl SourceSessionStore {
    pub fn ready(sessions: Vec<WorkspaceSession>, fetch_budget: usize, exhausted: bool) -> Self {
        Self {
            sessions,
            fetch_budget,
            exhausted,
        }
    }
}

impl SourceLoadState {
    pub fn marker(&self) -> SourceLoadMarker {
        match self {
            Self::Idle => SourceLoadMarker::Idle,
            Self::Loading { .. } => SourceLoadMarker::Loading,
            Self::Ready(_) => SourceLoadMarker::Ready,
            Self::Failed { .. } => SourceLoadMarker::Failed,
        }
    }

    pub fn visible_sessions(&self) -> &[WorkspaceSession] {
        match self {
            Self::Ready(store) => &store.sessions,
            Self::Loading {
                stale: Some(store), ..
            }
            | Self::Failed {
                stale: Some(store), ..
            } => &store.sessions,
            _ => &[],
        }
    }

    pub fn store(&self) -> Option<&SourceSessionStore> {
        match self {
            Self::Ready(store) => Some(store),
            Self::Loading {
                stale: Some(store), ..
            }
            | Self::Failed {
                stale: Some(store), ..
            } => Some(store),
            _ => None,
        }
    }

    fn take_stale(&self) -> Option<SourceSessionStore> {
        match self {
            Self::Ready(store) => Some(store.clone()),
            Self::Loading { stale, .. } | Self::Failed { stale, .. } => stale.clone(),
            Self::Idle => None,
        }
    }

    fn with_sessions_mut<F>(&mut self, f: F)
    where
        F: FnOnce(&mut [WorkspaceSession]),
    {
        match self {
            Self::Ready(store) => f(&mut store.sessions),
            Self::Loading {
                stale: Some(store), ..
            }
            | Self::Failed {
                stale: Some(store), ..
            } => f(&mut store.sessions),
            _ => {}
        }
    }
}

#[derive(Debug)]
pub struct SourceLoadEvent {
    pub index: usize,
    pub gen: u64,
    /// `fetch_budget == 0` marks a body-hydrate completion (patch only).
    pub result: std::result::Result<SourceSessionStore, String>,
}

pub struct SourceLoadPump {
    tx: Sender<SourceLoadEvent>,
    rx: Receiver<SourceLoadEvent>,
    cwd: PathBuf,
    gens: Vec<u64>,
    last_kick: Vec<Option<Instant>>,
    /// In-flight body keys `"source_idx\\0session_id"`.
    body_inflight: HashSet<String>,
}

impl SourceLoadPump {
    pub fn new(source_count: usize, cwd: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            cwd,
            gens: vec![0; source_count],
            last_kick: vec![None; source_count],
            body_inflight: HashSet::new(),
        }
    }

    /// Initial / selection kick: ensure each selected source has at least one viewport of meta.
    pub fn kick(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: SessionViewport,
        refresh_ready: bool,
    ) {
        const READY_DEBOUNCE: Duration = Duration::from_millis(800);
        let now = Instant::now();
        let initial_sessions = viewport
            .visible_rows
            .saturating_add(viewport.visible_rows.saturating_mul(META_PREFETCH_SCREENS))
            .max(viewport.visible_rows);
        let budget = SessionViewport::record_budget_for_sessions(initial_sessions);

        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            let needs = match &states[idx] {
                SourceLoadState::Loading { .. } => false,
                SourceLoadState::Idle | SourceLoadState::Failed { .. } => true,
                SourceLoadState::Ready(store) => {
                    refresh_ready
                        && self
                            .last_kick
                            .get(idx)
                            .and_then(|t| *t)
                            .is_none_or(|t| now.duration_since(t) >= READY_DEBOUNCE)
                        || store.sessions.is_empty()
                }
            };
            if !needs {
                continue;
            }
            let limit = states
                .get(idx)
                .and_then(SourceLoadState::store)
                .map(|s| s.fetch_budget.max(budget))
                .unwrap_or(budget);
            self.spawn_list(idx, source, states, limit);
        }
    }

    pub fn refresh_selected(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: SessionViewport,
    ) {
        let min_sessions = viewport
            .visible_rows
            .saturating_mul(1 + META_PREFETCH_SCREENS)
            .max(viewport.visible_rows);
        let min_budget = SessionViewport::record_budget_for_sessions(min_sessions);
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            if matches!(states.get(idx), Some(SourceLoadState::Loading { .. })) {
                continue;
            }
            let limit = states
                .get(idx)
                .and_then(SourceLoadState::store)
                .map(|s| s.fetch_budget.max(min_budget))
                .unwrap_or(min_budget);
            self.spawn_list(idx, source, states, limit);
        }
    }

    /// Grow L1 prefixes so the merged list can cover `viewport` (sliding ensure).
    ///
    /// Call each frame with the **merged** session list length and viewport.
    pub fn ensure_meta_window(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: SessionViewport,
        merged_len: usize,
    ) {
        let need_end = viewport.meta_need_end(merged_len);
        // If merged list already covers need_end and no source is empty-loading, still
        // expand sources that are not exhausted when the user is near the end.
        let near_end = merged_len == 0 || need_end + viewport.visible_rows >= merged_len;

        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            if matches!(states.get(idx), Some(SourceLoadState::Loading { .. })) {
                continue;
            }
            let Some(store) = states.get(idx).and_then(SourceLoadState::store) else {
                // Idle selected source — kick with viewport budget.
                let budget = SessionViewport::record_budget_for_sessions(
                    viewport
                        .visible_rows
                        .saturating_mul(1 + META_PREFETCH_SCREENS),
                );
                self.spawn_list(idx, source, states, budget);
                continue;
            };
            if store.exhausted {
                continue;
            }
            // Expand when this source has fewer sessions than a viewport, or merged list
            // needs more rows near the end.
            let want_more = store.sessions.len() < need_end || near_end;
            if !want_more {
                continue;
            }
            let session_target = store
                .sessions
                .len()
                .saturating_add(viewport.visible_rows.max(META_FETCH_FLOOR));
            let next_budget = SessionViewport::record_budget_for_sessions(session_target)
                .max(store.fetch_budget.saturating_add(META_FETCH_FLOOR));
            if next_budget <= store.fetch_budget && !store.sessions.is_empty() {
                // Density was lower than expected — force a larger record window.
                let forced = (store.fetch_budget.saturating_mul(2)).max(store.fetch_budget + 32);
                if forced > store.fetch_budget && forced <= META_FETCH_CEILING {
                    self.spawn_list(idx, source, states, forced.min(META_FETCH_CEILING));
                }
                continue;
            }
            self.spawn_list(idx, source, states, next_budget.min(META_FETCH_CEILING));
        }
    }

    /// Hydrate bodies for `keep` keys and **evict** every other loaded body.
    ///
    /// `keep` entries are `(source_index, session_id)`.
    pub fn sync_bodies(
        &mut self,
        sources: &[WorkspaceSource],
        states: &mut [SourceLoadState],
        keep: &HashSet<(usize, String)>,
    ) {
        // Evict first so memory drops even if hydrate is busy.
        for (source_idx, state) in states.iter_mut().enumerate() {
            state.with_sessions_mut(|sessions| {
                for session in sessions.iter_mut() {
                    let key = (source_idx, session.session_id.clone());
                    if session.body_loaded && !keep.contains(&key) {
                        session.records.clear();
                        session.body_loaded = false;
                    }
                }
            });
        }

        for (source_idx, session_id) in keep {
            let Some(source) = sources.get(*source_idx) else {
                continue;
            };
            let Some(state) = states.get(*source_idx) else {
                continue;
            };
            if state.store().is_none() {
                continue;
            }
            let already = state
                .visible_sessions()
                .iter()
                .find(|s| s.session_id == *session_id)
                .is_some_and(|s| s.body_loaded);
            if already {
                continue;
            }
            let inflight_key = body_inflight_key(*source_idx, session_id);
            if self.body_inflight.contains(&inflight_key) {
                continue;
            }
            self.body_inflight.insert(inflight_key);
            self.spawn_hydrate(*source_idx, source, session_id);
        }
    }

    /// Drop all state for unselected sources (free meta + body).
    pub fn drop_unselected(&mut self, selected: &[bool], states: &mut [SourceLoadState]) {
        for (idx, sel) in selected.iter().enumerate() {
            if *sel {
                continue;
            }
            if idx < states.len() && !matches!(states[idx], SourceLoadState::Idle) {
                states[idx] = SourceLoadState::Idle;
            }
            // Clear inflight markers for this source.
            self.body_inflight
                .retain(|k| !k.starts_with(&format!("{idx}\0")));
        }
    }

    fn spawn_list(
        &mut self,
        idx: usize,
        source: &WorkspaceSource,
        states: &mut [SourceLoadState],
        budget: usize,
    ) {
        assert!(
            idx < self.gens.len() && idx < states.len(),
            "source index out of range"
        );
        self.gens[idx] = self.gens[idx].saturating_add(1);
        let gen = self.gens[idx];
        self.last_kick[idx] = Some(Instant::now());

        let stale = states[idx].take_stale();
        // Preserve bodies that are still wanted — caller should have evicted first;
        // reattach any remaining body_loaded rows by id.
        let hydrated: HashMap<String, Vec<WorkRecord>> = stale
            .as_ref()
            .map(|store| {
                store
                    .sessions
                    .iter()
                    .filter(|s| s.body_loaded && !s.records.is_empty())
                    .map(|s| (s.session_id.clone(), s.records.clone()))
                    .collect()
            })
            .unwrap_or_default();

        states[idx] = SourceLoadState::Loading { stale, gen };

        let selector = source.selector();
        let remote = source.is_remote();
        let cwd = self.cwd.clone();
        let tx = self.tx.clone();
        let source = source.clone();
        let budget = budget.clamp(META_FETCH_FLOOR, META_FETCH_CEILING);
        thread::spawn(move || {
            let query_source = if remote {
                QuerySource::remote(selector)
            } else {
                QuerySource::local(selector)
            };
            let filter = Filter::browse_session_page(budget);
            let result = match workset::query_many(
                &[query_source],
                filter,
                Some(&cwd),
                REMOTE_QUERY_TIMEOUT,
            ) {
                Ok(mut results) => match results.pop() {
                    Some(QuerySourceResult::Ok(set)) => {
                        let record_count = set.records.len();
                        let mut sessions = sessions_from_records(&source, set.records);
                        for session in &mut sessions {
                            if let Some(records) = hydrated.get(&session.session_id) {
                                session.records = records.clone();
                                session.body_loaded = true;
                            } else {
                                session.records.clear();
                                session.body_loaded = false;
                            }
                        }
                        // Newest-first list for the source (mtime).
                        sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
                        Ok(SourceSessionStore {
                            sessions,
                            fetch_budget: budget,
                            exhausted: record_count < budget,
                        })
                    }
                    Some(QuerySourceResult::Err(message)) => Err(message),
                    None => Err("empty query result".to_string()),
                },
                Err(error) => Err(format!("{error:#}")),
            };
            let _ = tx.send(SourceLoadEvent {
                index: idx,
                gen,
                result,
            });
        });
    }

    fn spawn_hydrate(&mut self, idx: usize, source: &WorkspaceSource, session_id: &str) {
        let gen = self.gens.get(idx).copied().unwrap_or(0);
        let selector = source.selector();
        let remote = source.is_remote();
        let cwd = self.cwd.clone();
        let tx = self.tx.clone();
        let source = source.clone();
        let session_id = session_id.to_string();
        thread::spawn(move || {
            let session_selector = format!("{selector}/{session_id}");
            let query_source = if remote {
                QuerySource::remote(session_selector)
            } else {
                QuerySource::local(session_selector)
            };
            let result = match workset::query_many(
                &[query_source],
                Filter::none(),
                Some(&cwd),
                REMOTE_QUERY_TIMEOUT,
            ) {
                Ok(mut results) => match results.pop() {
                    Some(QuerySourceResult::Ok(set)) => {
                        let mut sessions = sessions_from_records(&source, set.records);
                        for session in &mut sessions {
                            session.body_loaded = !session.records.is_empty();
                        }
                        Ok(SourceSessionStore {
                            sessions,
                            fetch_budget: 0, // hydrate marker
                            exhausted: true,
                        })
                    }
                    Some(QuerySourceResult::Err(message)) => Err(message),
                    None => Err("empty hydrate result".to_string()),
                },
                Err(error) => Err(format!("{error:#}")),
            };
            let _ = tx.send(SourceLoadEvent {
                index: idx,
                gen,
                result,
            });
        });
    }

    pub fn drain(&mut self, states: &mut [SourceLoadState]) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(event) => {
                    if self.apply(event, states) {
                        changed = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        changed
    }

    fn apply(&mut self, event: SourceLoadEvent, states: &mut [SourceLoadState]) -> bool {
        let Some(state) = states.get_mut(event.index) else {
            return false;
        };

        match event.result {
            Ok(store) if store.fetch_budget == 0 => {
                // Hydrate patch.
                for s in &store.sessions {
                    self.body_inflight
                        .remove(&body_inflight_key(event.index, &s.session_id));
                }
                merge_hydrated_bodies(state, store.sessions)
            }
            Ok(store) => {
                let SourceLoadState::Loading { gen, .. } = state else {
                    return false;
                };
                if *gen != event.gen {
                    return false;
                }
                *state = SourceLoadState::Ready(store);
                true
            }
            Err(message) => {
                // Clear inflight for this source on hard list failure only.
                if let SourceLoadState::Loading { gen, stale, .. } = state {
                    if *gen != event.gen {
                        return false;
                    }
                    let stale = stale.clone();
                    *state = SourceLoadState::Failed { message, stale };
                    return true;
                }
                // Hydrate failure: drop inflight markers we can infer? leave them —
                // next sync will retry after timeout if we clear by gen mismatch.
                false
            }
        }
    }
}

fn body_inflight_key(source_idx: usize, session_id: &str) -> String {
    format!("{source_idx}\0{session_id}")
}

fn merge_hydrated_bodies(state: &mut SourceLoadState, hydrated: Vec<WorkspaceSession>) -> bool {
    let mut changed = false;
    state.with_sessions_mut(|sessions| {
        for body in hydrated {
            if let Some(slot) = sessions
                .iter_mut()
                .find(|s| s.session_id == body.session_id)
            {
                if !body.records.is_empty() {
                    slot.records = body.records;
                    slot.body_loaded = true;
                    if slot.search_title.is_empty() {
                        slot.search_title = body.search_title;
                        slot.title = body.title;
                    }
                    changed = true;
                }
            }
        }
    });
    changed
}

/// Build keep-set keys for body cache from focus + multi-select + optional neighbors.
pub fn body_keep_set(
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    focus_idx: usize,
    selected_sessions: &[bool],
    neighbor_radius: usize,
) -> HashSet<(usize, String)> {
    let mut keep = HashSet::new();
    let mut push = |session: &WorkspaceSession| {
        if let Some(si) = source_index_for_session(sources, session) {
            keep.insert((si, session.session_id.clone()));
        }
    };
    if let Some(session) = sessions.get(focus_idx) {
        push(session);
        // Neighbors in the **merged** list (small, for smooth j/k).
        let start = focus_idx.saturating_sub(neighbor_radius);
        let end = (focus_idx + neighbor_radius + 1).min(sessions.len());
        for session in &sessions[start..end] {
            push(session);
        }
    }
    for (idx, selected) in selected_sessions.iter().enumerate() {
        if *selected {
            if let Some(session) = sessions.get(idx) {
                push(session);
            }
        }
    }
    keep
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
    // Never auto-start daemon for catalog.
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
    for (idx, _source) in sources.iter().enumerate() {
        if !selected.get(idx).copied().unwrap_or(false) {
            continue;
        }
        sessions.extend(states[idx].visible_sessions().iter().cloned());
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
        let s1 = sessions.iter().find(|s| s.session_id == "s1").unwrap();
        assert_eq!(s1.records.len(), 2);
        assert!(s1.body_loaded);
        assert_eq!(s1.search_title, "first");
        assert!(!s1.source.is_remote());
    }

    #[test]
    fn viewport_meta_need_end_scales_with_visible_rows() {
        let small = SessionViewport {
            first_visible: 0,
            visible_rows: 10,
        };
        let large = SessionViewport {
            first_visible: 0,
            visible_rows: 40,
        };
        assert!(large.meta_need_end(0) > small.meta_need_end(0));
        assert_eq!(small.meta_need_end(0), 10 + 10); // page + 1 screen prefetch
    }

    #[test]
    fn record_budget_is_dynamic_not_fixed_page() {
        let a = SessionViewport::record_budget_for_sessions(10);
        let b = SessionViewport::record_budget_for_sessions(50);
        assert!(b > a);
        assert!(a >= META_FETCH_FLOOR);
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
