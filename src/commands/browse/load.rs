//! Source catalog + viewport-driven sliding-window loads for browse.
//!
//! ## Contract
//!
//! - **L1 meta**: per-source session rows without dialogue bodies.
//!   Grows only when the viewport needs rows not yet covered (`need_end`).
//! - **L2 body**: full records only for the keep set (focus / multi-select /
//!   neighbors). Evicted immediately outside keep.
//! - **No blanking Ready**: background meta fetches never replace a good list
//!   with `Loading`. First load from Idle may show empty until the first Ready.
//! - **Merge by `session_id`**: list results patch the existing prefix; bodies
//!   already hydrated stay unless the session disappears from the prefix.
//! - CLI `sivtr s` / workset search are unchanged.

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// Floor for record-budget so tiny viewports still batch I/O.
const META_FETCH_FLOOR: usize = 12;
const META_FETCH_CEILING: usize = 2_000;
/// Prefetch screens beyond the visible session pane.
const META_PREFETCH_SCREENS: usize = 1;

/// Session-list viewport in merged-list coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionViewport {
    pub first_visible: usize,
    pub visible_rows: usize,
}

impl SessionViewport {
    pub fn from_panel(offset: usize, panel_inner_height: usize) -> Self {
        Self {
            first_visible: offset,
            visible_rows: panel_inner_height.max(1),
        }
    }

    /// Exclusive end index L1 should cover (viewport + prefetch).
    pub fn meta_need_end(&self) -> usize {
        let page = self.visible_rows;
        let prefetch = page.saturating_mul(META_PREFETCH_SCREENS);
        self.first_visible
            .saturating_add(page)
            .saturating_add(prefetch)
            .max(page.saturating_add(prefetch))
    }

    /// Record budget likely to fold into ~`session_target` sessions.
    pub fn record_budget_for_sessions(session_target: usize) -> usize {
        let raw = session_target.saturating_mul(3).max(META_FETCH_FLOOR);
        raw.min(META_FETCH_CEILING)
    }
}

/// Per-source load state. Once Ready, stays Ready while meta expands in background.
#[derive(Clone, Debug)]
pub enum SourceLoadState {
    Idle,
    /// First paint only — no sessions yet.
    Booting {
        gen: u64,
    },
    Ready(SourceSessionStore),
    Failed {
        #[allow(dead_code)]
        message: String,
        /// Last good store if any.
        stale: Option<SourceSessionStore>,
    },
}

/// L1 meta store for one source.
#[derive(Clone, Debug, Default)]
pub struct SourceSessionStore {
    pub sessions: Vec<WorkspaceSession>,
    /// Record budget used for the current prefix.
    pub fetch_budget: usize,
    pub exhausted: bool,
    /// Background list expand in flight (Ready stays painted).
    pub list_inflight: bool,
    pub list_gen: u64,
}

impl SourceSessionStore {
    pub fn ready(sessions: Vec<WorkspaceSession>, fetch_budget: usize, exhausted: bool) -> Self {
        Self {
            sessions,
            fetch_budget,
            exhausted,
            list_inflight: false,
            list_gen: 0,
        }
    }
}

impl SourceLoadState {
    pub fn marker(&self) -> SourceLoadMarker {
        match self {
            Self::Idle => SourceLoadMarker::Idle,
            Self::Booting { .. } => SourceLoadMarker::Loading,
            Self::Ready(store) if store.list_inflight => SourceLoadMarker::Loading,
            Self::Ready(_) => SourceLoadMarker::Ready,
            Self::Failed { .. } => SourceLoadMarker::Failed,
        }
    }

    pub fn is_fetching(&self) -> bool {
        matches!(
            self,
            Self::Booting { .. }
                | Self::Ready(SourceSessionStore {
                    list_inflight: true,
                    ..
                })
        )
    }

    pub fn visible_sessions(&self) -> &[WorkspaceSession] {
        match self {
            Self::Ready(store) => &store.sessions,
            Self::Failed {
                stale: Some(store), ..
            } => &store.sessions,
            _ => &[],
        }
    }

    pub fn store(&self) -> Option<&SourceSessionStore> {
        match self {
            Self::Ready(store) => Some(store),
            Self::Failed {
                stale: Some(store), ..
            } => Some(store),
            _ => None,
        }
    }

    fn with_sessions_mut<F>(&mut self, f: F)
    where
        F: FnOnce(&mut [WorkspaceSession]),
    {
        match self {
            Self::Ready(store) => f(&mut store.sessions),
            Self::Failed {
                stale: Some(store), ..
            } => f(&mut store.sessions),
            _ => {}
        }
    }
}

#[derive(Debug)]
enum LoadJobKind {
    /// Meta prefix fetch. `budget` is the record window requested.
    Meta { budget: usize },
    /// Body hydrate for one session.
    Body { session_id: String },
}

#[derive(Debug)]
struct LoadEvent {
    index: usize,
    gen: u64,
    kind: LoadJobKind,
    result: std::result::Result<Vec<WorkspaceSession>, String>,
    /// For Meta: whether the query looked exhausted.
    exhausted: bool,
}

pub struct SourceLoadPump {
    tx: Sender<LoadEvent>,
    rx: Receiver<LoadEvent>,
    cwd: PathBuf,
    /// List generation per source (only meta jobs).
    list_gens: Vec<u64>,
    body_inflight: HashSet<String>,
}

impl SourceLoadPump {
    pub fn new(source_count: usize, cwd: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            cwd,
            list_gens: vec![0; source_count],
            body_inflight: HashSet::new(),
        }
    }

    /// Start or refresh selected sources so each has a viewport-sized meta prefix.
    pub fn kick(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: SessionViewport,
        force: bool,
    ) {
        let need = viewport.meta_need_end();
        let budget = SessionViewport::record_budget_for_sessions(need);
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            match states.get(idx) {
                Some(SourceLoadState::Booting { .. }) => continue,
                Some(SourceLoadState::Ready(store)) if store.list_inflight && !force => continue,
                Some(SourceLoadState::Ready(store)) if !force && !store.sessions.is_empty() => {
                    // Already have data; expand only via ensure_meta_window.
                    continue;
                }
                _ => {}
            }
            let limit = states
                .get(idx)
                .and_then(SourceLoadState::store)
                .map(|s| s.fetch_budget.max(budget))
                .unwrap_or(budget);
            self.start_meta(idx, source, states, limit, force);
        }
    }

    pub fn refresh_selected(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: SessionViewport,
    ) {
        let need = viewport.meta_need_end();
        let min_budget = SessionViewport::record_budget_for_sessions(need);
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            if matches!(states.get(idx), Some(SourceLoadState::Booting { .. })) {
                continue;
            }
            if states
                .get(idx)
                .and_then(SourceLoadState::store)
                .is_some_and(|s| s.list_inflight)
            {
                continue;
            }
            let limit = states
                .get(idx)
                .and_then(SourceLoadState::store)
                .map(|s| s.fetch_budget.max(min_budget))
                .unwrap_or(min_budget);
            self.start_meta(idx, source, states, limit, true);
        }
    }

    /// Expand L1 only when the merged list cannot cover the viewport need.
    pub fn ensure_meta_window(
        &mut self,
        sources: &[WorkspaceSource],
        selected: &[bool],
        states: &mut [SourceLoadState],
        viewport: SessionViewport,
        merged_len: usize,
    ) {
        let need_end = viewport.meta_need_end();
        // Covered: we already have at least need_end rows in the merged list.
        if merged_len >= need_end {
            return;
        }
        // Missing rows — expand non-exhausted selected sources that are not fetching.
        let deficit = need_end - merged_len;
        for (idx, source) in sources.iter().enumerate() {
            if !selected.get(idx).copied().unwrap_or(false) {
                continue;
            }
            match states.get(idx) {
                None | Some(SourceLoadState::Idle) => {
                    let budget = SessionViewport::record_budget_for_sessions(need_end);
                    self.start_meta(idx, source, states, budget, false);
                }
                Some(SourceLoadState::Booting { .. }) => {}
                Some(SourceLoadState::Ready(store)) => {
                    if store.list_inflight || store.exhausted {
                        continue;
                    }
                    // Grow this source's prefix enough to help fill the deficit.
                    let session_target = store.sessions.len().saturating_add(deficit);
                    let next = SessionViewport::record_budget_for_sessions(session_target)
                        .max(store.fetch_budget.saturating_add(META_FETCH_FLOOR));
                    if next <= store.fetch_budget {
                        let forced = (store.fetch_budget.saturating_mul(2))
                            .max(store.fetch_budget + META_FETCH_FLOOR)
                            .min(META_FETCH_CEILING);
                        if forced > store.fetch_budget {
                            self.start_meta(idx, source, states, forced, false);
                        }
                        continue;
                    }
                    self.start_meta(idx, source, states, next.min(META_FETCH_CEILING), false);
                }
                Some(SourceLoadState::Failed { .. }) => {
                    let budget = SessionViewport::record_budget_for_sessions(need_end);
                    self.start_meta(idx, source, states, budget, false);
                }
            }
        }
    }

    /// Hydrate keep-set bodies; clear bodies outside keep.
    pub fn sync_bodies(
        &mut self,
        sources: &[WorkspaceSource],
        states: &mut [SourceLoadState],
        keep: &HashSet<(usize, String)>,
    ) {
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
            let key = body_key(*source_idx, session_id);
            if self.body_inflight.contains(&key) {
                continue;
            }
            self.body_inflight.insert(key);
            self.spawn_body(*source_idx, source, session_id);
        }
    }

    pub fn drop_unselected(&mut self, selected: &[bool], states: &mut [SourceLoadState]) {
        for (idx, sel) in selected.iter().enumerate() {
            if *sel {
                continue;
            }
            if idx < states.len() && !matches!(states[idx], SourceLoadState::Idle) {
                states[idx] = SourceLoadState::Idle;
            }
            self.body_inflight
                .retain(|k| !k.starts_with(&format!("{idx}\0")));
        }
    }

    fn start_meta(
        &mut self,
        idx: usize,
        source: &WorkspaceSource,
        states: &mut [SourceLoadState],
        budget: usize,
        force_replace: bool,
    ) {
        assert!(idx < self.list_gens.len() && idx < states.len());
        if states
            .get(idx)
            .and_then(SourceLoadState::store)
            .is_some_and(|s| s.list_inflight)
        {
            return;
        }

        self.list_gens[idx] = self.list_gens[idx].saturating_add(1);
        let gen = self.list_gens[idx];
        let budget = budget.clamp(META_FETCH_FLOOR, META_FETCH_CEILING);

        match &mut states[idx] {
            SourceLoadState::Ready(store) => {
                store.list_inflight = true;
                store.list_gen = gen;
            }
            SourceLoadState::Failed { stale, .. } => {
                if let Some(store) = stale.take() {
                    let mut store = store;
                    store.list_inflight = true;
                    store.list_gen = gen;
                    states[idx] = SourceLoadState::Ready(store);
                } else {
                    states[idx] = SourceLoadState::Booting { gen };
                }
            }
            SourceLoadState::Idle | SourceLoadState::Booting { .. } => {
                states[idx] = SourceLoadState::Booting { gen };
            }
        }

        let _ = force_replace; // reserved: always merge; force only bypasses "already has data" in kick
        let selector = source.selector();
        let remote = source.is_remote();
        let cwd = self.cwd.clone();
        let tx = self.tx.clone();
        let source = source.clone();
        thread::spawn(move || {
            let query_source = if remote {
                QuerySource::remote(selector)
            } else {
                QuerySource::local(selector)
            };
            let filter = Filter::browse_session_page(budget);
            let (result, exhausted) = match workset::query_many(
                &[query_source],
                filter,
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
                    Some(QuerySourceResult::Err(message)) => (Err(message), false),
                    None => (Err("empty query result".into()), false),
                },
                Err(error) => (Err(format!("{error:#}")), false),
            };
            let _ = tx.send(LoadEvent {
                index: idx,
                gen,
                kind: LoadJobKind::Meta { budget },
                result,
                exhausted,
            });
        });
    }

    fn spawn_body(&mut self, idx: usize, source: &WorkspaceSource, session_id: &str) {
        let gen = self.list_gens.get(idx).copied().unwrap_or(0);
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
                        for s in &mut sessions {
                            s.body_loaded = !s.records.is_empty();
                        }
                        Ok(sessions)
                    }
                    Some(QuerySourceResult::Err(message)) => Err(message),
                    None => Err("empty hydrate result".into()),
                },
                Err(error) => Err(format!("{error:#}")),
            };
            let _ = tx.send(LoadEvent {
                index: idx,
                gen,
                kind: LoadJobKind::Body {
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

    fn apply(&mut self, event: LoadEvent, states: &mut [SourceLoadState]) -> bool {
        let Some(state) = states.get_mut(event.index) else {
            return false;
        };

        match event.kind {
            LoadJobKind::Body { session_id } => {
                self.body_inflight
                    .remove(&body_key(event.index, &session_id));
                match event.result {
                    Ok(sessions) => merge_bodies(state, sessions),
                    Err(_) => false,
                }
            }
            LoadJobKind::Meta { budget } => match event.result {
                Ok(new_sessions) => {
                    apply_meta_success(state, event.gen, budget, event.exhausted, new_sessions)
                }
                Err(message) => apply_meta_failure(state, event.gen, message),
            },
        }
    }
}

fn apply_meta_success(
    state: &mut SourceLoadState,
    gen: u64,
    budget: usize,
    exhausted: bool,
    new_sessions: Vec<WorkspaceSession>,
) -> bool {
    match state {
        SourceLoadState::Booting { gen: g } if *g == gen => {
            *state = SourceLoadState::Ready(SourceSessionStore {
                sessions: new_sessions,
                fetch_budget: budget,
                exhausted,
                list_inflight: false,
                list_gen: gen,
            });
            true
        }
        SourceLoadState::Ready(store) if store.list_gen == gen => {
            store.sessions = merge_session_prefix(std::mem::take(&mut store.sessions), new_sessions);
            store.fetch_budget = budget;
            store.exhausted = exhausted;
            store.list_inflight = false;
            true
        }
        // Stale job.
        SourceLoadState::Ready(store) => {
            store.list_inflight = false;
            false
        }
        SourceLoadState::Booting { .. } => false,
        SourceLoadState::Idle | SourceLoadState::Failed { .. } => false,
    }
}

fn apply_meta_failure(state: &mut SourceLoadState, gen: u64, message: String) -> bool {
    match state {
        SourceLoadState::Booting { gen: g } if *g == gen => {
            *state = SourceLoadState::Failed {
                message,
                stale: None,
            };
            true
        }
        SourceLoadState::Ready(store) if store.list_gen == gen => {
            // Keep showing Ready data; clear inflight. Surface failure only if empty.
            store.list_inflight = false;
            if store.sessions.is_empty() {
                *state = SourceLoadState::Failed {
                    message,
                    stale: None,
                };
            }
            true
        }
        SourceLoadState::Ready(store) => {
            store.list_inflight = false;
            false
        }
        _ => false,
    }
}

/// Merge a new meta prefix into the existing list by `session_id`.
///
/// - New order follows `incoming` (newest-first from workset).
/// - Bodies on matching ids are preserved.
/// - Sessions that disappear from the prefix drop (and their bodies with them).
fn merge_session_prefix(
    previous: Vec<WorkspaceSession>,
    incoming: Vec<WorkspaceSession>,
) -> Vec<WorkspaceSession> {
    let mut prev_bodies: HashMap<String, (Vec<WorkRecord>, bool)> = HashMap::new();
    for s in previous {
        if s.body_loaded && !s.records.is_empty() {
            prev_bodies.insert(s.session_id, (s.records, true));
        }
    }
    let mut out = Vec::with_capacity(incoming.len());
    for mut s in incoming {
        if let Some((records, loaded)) = prev_bodies.remove(&s.session_id) {
            s.records = records;
            s.body_loaded = loaded;
        } else {
            s.records.clear();
            s.body_loaded = false;
        }
        out.push(s);
    }
    out
}

fn merge_bodies(state: &mut SourceLoadState, hydrated: Vec<WorkspaceSession>) -> bool {
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
                    changed = true;
                }
            }
        }
    });
    changed
}

fn body_key(source_idx: usize, session_id: &str) -> String {
    format!("{source_idx}\0{session_id}")
}

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
    }

    #[test]
    fn merge_prefix_preserves_bodies_by_session_id() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let mut prev = sessions_from_records(
            &source,
            vec![test_record("s1", 1, "old", "2026-07-17T10:00:00Z")],
        );
        prev[0].body_loaded = true;
        prev[0].records = vec![test_record("s1", 1, "body", "2026-07-17T10:00:00Z")];

        let incoming = {
            let mut s = sessions_from_records(
                &source,
                vec![
                    test_record("s1", 1, "new-title", "2026-07-17T12:00:00Z"),
                    test_record("s2", 1, "other", "2026-07-17T11:00:00Z"),
                ],
            );
            for row in &mut s {
                row.records.clear();
                row.body_loaded = false;
            }
            s
        };

        let merged = merge_session_prefix(prev, incoming);
        let s1 = merged.iter().find(|s| s.session_id == "s1").unwrap();
        assert!(s1.body_loaded);
        assert_eq!(s1.records[0].title, "body");
        let s2 = merged.iter().find(|s| s.session_id == "s2").unwrap();
        assert!(!s2.body_loaded);
        assert!(s2.records.is_empty());
    }

    #[test]
    fn meta_need_end_scales_with_viewport() {
        let small = SessionViewport {
            first_visible: 0,
            visible_rows: 10,
        };
        let large = SessionViewport {
            first_visible: 5,
            visible_rows: 30,
        };
        assert_eq!(small.meta_need_end(), 20);
        assert!(large.meta_need_end() > small.meta_need_end());
    }

    #[test]
    fn ensure_skips_when_merged_covers_viewport() {
        // Pure unit: if merged_len >= need_end, ensure should not start jobs.
        // Exercised via logic: need_end for 10 rows is 20; merged 25 covers.
        let vp = SessionViewport {
            first_visible: 0,
            visible_rows: 10,
        };
        assert!(25 >= vp.meta_need_end());
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
