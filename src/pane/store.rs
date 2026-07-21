//! Ordered sliding-window store (meta + optional body).

use std::collections::HashSet;
use std::hash::Hash;

/// Prefetch screens beyond the visible rows (1 = one screen past the bottom).
pub const PREFETCH_SCREENS: usize = 1;
/// I/O floor so a 1-row panel still batches.
pub const FETCH_FLOOR: usize = 12;
pub const FETCH_CEILING: usize = 2_000;

/// List viewport geometry (from layout + list offset).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Viewport {
    pub first: usize,
    pub visible: usize,
}

impl Viewport {
    pub fn from_panel(offset: usize, inner_rows: usize) -> Self {
        Self {
            first: offset,
            visible: inner_rows.max(1),
        }
    }

    /// Exclusive end index that should be covered by meta (visible + prefetch).
    pub fn need_end(&self) -> usize {
        let page = self.visible.max(1);
        let prefetch = page.saturating_mul(PREFETCH_SCREENS);
        self.first
            .saturating_add(page)
            .saturating_add(prefetch)
            .max(page.saturating_add(prefetch))
    }

    /// Batch size for a fetch aiming to cover `target_items` items.
    pub fn fetch_budget(target_items: usize) -> usize {
        let raw = target_items.saturating_mul(3).max(FETCH_FLOOR);
        raw.min(FETCH_CEILING)
    }
}

/// Load phase for a pane store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorePhase {
    /// Nothing requested yet.
    Idle,
    /// First fetch; list may be empty.
    Booting,
    /// Have rows; may still be expanding in background.
    Ready,
    /// Hard failure with no rows to show.
    Failed,
}

/// One row: light meta + optional heavy body.
#[derive(Clone, Debug)]
pub struct WindowRow<K, M, B> {
    pub key: K,
    pub meta: M,
    pub body: Option<B>,
    pub body_loaded: bool,
}

impl<K, M, B> WindowRow<K, M, B> {
    pub fn meta_only(key: K, meta: M) -> Self {
        Self {
            key,
            meta,
            body: None,
            body_loaded: false,
        }
    }

    pub fn with_body(key: K, meta: M, body: B) -> Self {
        Self {
            key,
            meta,
            body: Some(body),
            body_loaded: true,
        }
    }

    pub fn clear_body(&mut self) {
        self.body = None;
        self.body_loaded = false;
    }
}

/// Sliding window over an ordered sequence.
#[derive(Clone, Debug)]
pub struct SlidingStore<K, M, B> {
    pub rows: Vec<WindowRow<K, M, B>>,
    pub phase: StorePhase,
    pub exhausted: bool,
    /// Opaque budget last used for meta fetch (loader-defined units).
    pub fetch_budget: usize,
    pub list_inflight: bool,
    pub list_gen: u64,
    pub fail_message: Option<String>,
}

impl<K, M, B> Default for SlidingStore<K, M, B> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            phase: StorePhase::Idle,
            exhausted: false,
            fetch_budget: 0,
            list_inflight: false,
            list_gen: 0,
            fail_message: None,
        }
    }
}

impl<K, M, B> SlidingStore<K, M, B>
where
    K: Clone + Eq + Hash,
    M: Clone,
    B: Clone,
{
    pub fn ready(rows: Vec<WindowRow<K, M, B>>, fetch_budget: usize, exhausted: bool) -> Self {
        Self {
            rows,
            phase: StorePhase::Ready,
            exhausted,
            fetch_budget,
            list_inflight: false,
            list_gen: 0,
            fail_message: None,
        }
    }

    pub fn is_fetching(&self) -> bool {
        self.list_inflight || matches!(self.phase, StorePhase::Booting)
    }

    /// Whether meta must grow to cover `viewport` (true deficit only).
    pub fn needs_meta(&self, viewport: Viewport) -> bool {
        if self.list_inflight || self.exhausted {
            return false;
        }
        if matches!(self.phase, StorePhase::Idle | StorePhase::Booting) {
            return true;
        }
        self.rows.len() < viewport.need_end()
    }

    /// Budget for the next meta fetch given current coverage + viewport.
    pub fn next_meta_budget(&self, viewport: Viewport) -> usize {
        let need = viewport.need_end();
        let target = need.max(self.rows.len().saturating_add(viewport.visible.max(1)));
        let budget = Viewport::fetch_budget(target);
        budget
            .max(self.fetch_budget.saturating_add(FETCH_FLOOR))
            .min(FETCH_CEILING)
    }

    /// Begin a meta job: Booting if empty, else Ready+inflight (never blank).
    pub fn begin_meta(&mut self, gen: u64) {
        self.list_gen = gen;
        self.list_inflight = true;
        self.fail_message = None;
        if self.rows.is_empty() && !matches!(self.phase, StorePhase::Ready) {
            self.phase = StorePhase::Booting;
        }
    }

    /// Apply a successful meta page: merge by key, keep bodies.
    pub fn apply_meta_ok(
        &mut self,
        gen: u64,
        budget: usize,
        exhausted: bool,
        incoming: Vec<WindowRow<K, M, B>>,
    ) -> bool {
        if self.list_gen != gen {
            self.list_inflight = false;
            return false;
        }
        self.rows = merge_by_key(std::mem::take(&mut self.rows), incoming);
        self.fetch_budget = budget;
        self.exhausted = exhausted;
        self.list_inflight = false;
        self.phase = StorePhase::Ready;
        self.fail_message = None;
        true
    }

    pub fn apply_meta_err(&mut self, gen: u64, message: String) -> bool {
        if self.list_gen != gen {
            self.list_inflight = false;
            return false;
        }
        self.list_inflight = false;
        if self.rows.is_empty() {
            self.phase = StorePhase::Failed;
            self.fail_message = Some(message);
        }
        true
    }

    pub fn apply_body(&mut self, key: &K, body: B) -> bool {
        if let Some(row) = self.rows.iter_mut().find(|r| r.key == *key) {
            row.body = Some(body);
            row.body_loaded = true;
            return true;
        }
        false
    }

    /// Keys in `keep` that still need a body fetch.
    pub fn body_missing<'a>(&'a self, keep: &'a HashSet<K>) -> Vec<&'a K> {
        keep.iter()
            .filter(|k| self.get(k).is_none_or(|r| !r.body_loaded))
            .collect()
    }

    pub fn clear_bodies_outside(&mut self, keep: &HashSet<K>) {
        for row in &mut self.rows {
            if row.body_loaded && !keep.contains(&row.key) {
                row.clear_body();
            }
        }
    }

    /// Replace the ordered meta list in one shot (static / in-memory catalogs).
    /// Preserves bodies for keys that still exist.
    pub fn replace_meta(&mut self, incoming: Vec<WindowRow<K, M, B>>, exhausted: bool) {
        self.rows = merge_by_key(std::mem::take(&mut self.rows), incoming);
        self.exhausted = exhausted;
        self.list_inflight = false;
        self.phase = if self.rows.is_empty() {
            StorePhase::Idle
        } else {
            StorePhase::Ready
        };
        self.fail_message = None;
        self.fetch_budget = self.rows.len();
    }

    pub fn get(&self, key: &K) -> Option<&WindowRow<K, M, B>> {
        self.rows.iter().find(|r| r.key == *key)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }
}

/// Merge incoming ordered prefix into previous rows by key; preserve bodies.
pub fn merge_by_key<K, M, B>(
    previous: Vec<WindowRow<K, M, B>>,
    incoming: Vec<WindowRow<K, M, B>>,
) -> Vec<WindowRow<K, M, B>>
where
    K: Clone + Eq + Hash,
    M: Clone,
    B: Clone,
{
    let mut bodies: std::collections::HashMap<K, B> = std::collections::HashMap::new();
    for row in previous {
        if row.body_loaded {
            if let Some(body) = row.body {
                bodies.insert(row.key, body);
            }
        }
    }
    let mut out = Vec::with_capacity(incoming.len());
    for mut row in incoming {
        if let Some(body) = bodies.remove(&row.key) {
            row.body = Some(body);
            row.body_loaded = true;
        } else {
            row.clear_body();
        }
        out.push(row);
    }
    out
}

/// Keep set from focus index + multi-select + neighbors on an ordered key list.
pub fn keep_keys<K: Clone + Eq + Hash>(
    keys: &[K],
    focus_idx: usize,
    selected: &[bool],
    neighbor_radius: usize,
) -> HashSet<K> {
    let mut keep = HashSet::new();
    if keys.is_empty() {
        return keep;
    }
    let focus_idx = focus_idx.min(keys.len() - 1);
    let start = focus_idx.saturating_sub(neighbor_radius);
    let end = (focus_idx + neighbor_radius + 1).min(keys.len());
    for k in &keys[start..end] {
        keep.insert(k.clone());
    }
    for (i, sel) in selected.iter().enumerate() {
        if *sel {
            if let Some(k) = keys.get(i) {
                keep.insert(k.clone());
            }
        }
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn need_end_scales_with_visible_rows() {
        let small = Viewport {
            first: 0,
            visible: 10,
        };
        let large = Viewport {
            first: 0,
            visible: 40,
        };
        assert_eq!(small.need_end(), 20);
        assert!(large.need_end() > small.need_end());
    }

    #[test]
    fn needs_meta_only_on_deficit() {
        let mut store = SlidingStore::<u32, &str, String>::ready(
            (0..30).map(|i| WindowRow::meta_only(i, "x")).collect(),
            90,
            false,
        );
        let covered = Viewport {
            first: 0,
            visible: 10,
        };
        assert!(!store.needs_meta(covered));
        let deep = Viewport {
            first: 25,
            visible: 10,
        };
        assert!(store.needs_meta(deep));
        store.exhausted = true;
        assert!(!store.needs_meta(deep));
    }

    #[test]
    fn merge_preserves_bodies() {
        let prev = vec![WindowRow::with_body(1u32, "a", "BODY".to_string())];
        let incoming = vec![
            WindowRow::meta_only(1u32, "a2"),
            WindowRow::meta_only(2u32, "b"),
        ];
        let merged = merge_by_key(prev, incoming);
        assert!(merged[0].body_loaded);
        assert_eq!(merged[0].body.as_deref(), Some("BODY"));
        assert!(!merged[1].body_loaded);
    }

    #[test]
    fn begin_meta_does_not_clear_ready_rows() {
        let mut store: SlidingStore<u32, &str, String> =
            SlidingStore::ready(vec![WindowRow::meta_only(1u32, "a")], 10, false);
        store.begin_meta(1);
        assert_eq!(store.phase, StorePhase::Ready);
        assert!(store.list_inflight);
        assert_eq!(store.rows.len(), 1);
    }

    #[test]
    fn keep_keys_includes_focus_neighbors_and_selection() {
        let keys = vec![10, 20, 30, 40, 50];
        let selected = [false, true, false, false, false];
        let keep = keep_keys(&keys, 3, &selected, 1);
        assert!(keep.contains(&20));
        assert!(keep.contains(&30));
        assert!(keep.contains(&40));
        assert!(keep.contains(&50));
        assert!(!keep.contains(&10));
    }
}
