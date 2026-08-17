//! [`SlidingPane`]: native dynamic-loading capability for content panes.
//!
//! This type never performs I/O. Callers (browse loaders) fulfill [`MetaNeed`]
//! and body-key requests, then call `apply_*`.

use std::collections::HashSet;
use std::hash::Hash;

use super::store::{SlidingStore, Viewport, WindowRow, FETCH_CEILING, FETCH_FLOOR};

/// Request returned by [`SlidingPane::ensure_meta`] / [`SlidingPane::force_meta`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetaNeed {
    pub gen: u64,
    pub budget: usize,
}

/// Data-side pane: ordered window + ensure protocol.
///
/// ```text
/// on_layout(viewport)  → ensure_meta → Option<MetaNeed>  → loader fetch → apply_meta_*
/// on_focus(keep)       → ensure_bodies → Vec<Key>        → loader fetch → apply_body
/// ```
#[derive(Clone, Debug)]
pub struct SlidingPane<K, M, B> {
    store: SlidingStore<K, M, B>,
}

impl<K, M, B> Default for SlidingPane<K, M, B> {
    fn default() -> Self {
        Self {
            store: SlidingStore::default(),
        }
    }
}

impl<K, M, B> SlidingPane<K, M, B>
where
    K: Clone + Eq + Hash,
    M: Clone,
    B: Clone,
{
    pub fn ready(rows: Vec<WindowRow<K, M, B>>, fetch_budget: usize, exhausted: bool) -> Self {
        Self {
            store: SlidingStore::ready(rows, fetch_budget, exhausted),
        }
    }

    pub fn store(&self) -> &SlidingStore<K, M, B> {
        &self.store
    }

    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.len() == 0
    }

    pub fn is_fetching(&self) -> bool {
        self.store.is_fetching()
    }

    pub fn exhausted(&self) -> bool {
        self.store.exhausted
    }

    pub fn rows(&self) -> &[WindowRow<K, M, B>] {
        &self.store.rows
    }

    /// Viewport-driven meta ensure. Starts inflight and returns a fetch request
    /// only when there is a true deficit. Never blanks Ready rows.
    pub fn ensure_meta(&mut self, viewport: Viewport) -> Option<MetaNeed> {
        if !self.store.needs_meta(viewport) {
            return None;
        }
        let budget = self.store.next_meta_budget(viewport);
        Some(self.begin_meta_fetch(budget))
    }

    /// Force a meta refresh (e.g. `R`), even when the viewport is covered.
    pub fn force_meta(&mut self, viewport: Viewport) -> Option<MetaNeed> {
        if self.store.list_inflight {
            return None;
        }
        let budget = self
            .store
            .fetch_budget
            .max(Viewport::fetch_budget(viewport.need_end()))
            .clamp(FETCH_FLOOR, FETCH_CEILING);
        Some(self.begin_meta_fetch(budget))
    }

    /// Explicit budget meta fetch (multi-source pumps that coordinate budgets).
    pub fn begin_meta_budget(&mut self, budget: usize) -> Option<MetaNeed> {
        if self.store.list_inflight {
            return None;
        }
        let budget = budget.clamp(FETCH_FLOOR, FETCH_CEILING);
        Some(self.begin_meta_fetch(budget))
    }

    fn begin_meta_fetch(&mut self, budget: usize) -> MetaNeed {
        let gen = self.store.list_gen.saturating_add(1);
        self.store.begin_meta(gen);
        MetaNeed { gen, budget }
    }

    pub fn apply_meta_ok(
        &mut self,
        gen: u64,
        budget: usize,
        exhausted: bool,
        incoming: Vec<WindowRow<K, M, B>>,
    ) -> bool {
        self.store.apply_meta_ok(gen, budget, exhausted, incoming)
    }

    pub fn apply_meta_err(&mut self, gen: u64, message: String) -> bool {
        self.store.apply_meta_err(gen, message)
    }

    /// Evict bodies outside `keep`; return keys that still need a body.
    pub fn ensure_bodies(&mut self, keep: HashSet<K>) -> Vec<K> {
        self.store.clear_bodies_outside(&keep);
        self.store
            .body_missing(&keep)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Same body protocol for sync loaders: evict, then fill missing via `fetch`.
    ///
    /// `fetch` returns `Some(body)` when the key can be resolved now.
    pub fn ensure_bodies_sync<F>(&mut self, keep: HashSet<K>, mut fetch: F)
    where
        F: FnMut(&K) -> Option<B>,
    {
        for key in self.ensure_bodies(keep) {
            if let Some(body) = fetch(&key) {
                self.store.apply_body(&key, body);
            }
        }
    }

    /// Focus + multi-select + neighbor keep set over current meta keys.
    pub fn keep_for_focus(
        &self,
        focus_idx: usize,
        selected: &[bool],
        neighbor_radius: usize,
    ) -> HashSet<K> {
        let keys: Vec<K> = self.store.rows.iter().map(|r| r.key.clone()).collect();
        super::store::keep_keys(&keys, focus_idx, selected, neighbor_radius)
    }

    /// Sync meta growth: same deficit rules as [`ensure_meta`], but the caller
    /// fills the page inline (in-memory sources). No per-pane reimplementation.
    ///
    /// `fetch(budget) -> (ordered_prefix, exhausted)` must return a mergeable
    /// ordered prefix of length ≤ budget (typically first `budget` source rows).
    ///
    /// When `force` is true, refreshes even if the viewport is already covered
    /// (context rebuild / explicit reload).
    pub fn ensure_meta_sync<F>(&mut self, viewport: Viewport, force: bool, mut fetch: F) -> bool
    where
        F: FnMut(usize) -> (Vec<WindowRow<K, M, B>>, bool),
    {
        let need = if force {
            self.force_meta(viewport)
        } else {
            self.ensure_meta(viewport)
        };
        let Some(need) = need else {
            return false;
        };
        let (incoming, exhausted) = fetch(need.budget);
        self.store
            .apply_meta_ok(need.gen, need.budget, exhausted, incoming)
    }

    pub fn apply_body(&mut self, key: &K, body: B) -> bool {
        self.store.apply_body(key, body)
    }

    /// Static catalog (Source pane, Content line map). Exhausted so ensure stays quiet.
    pub fn set_catalog(&mut self, rows: Vec<WindowRow<K, M, B>>, exhausted: bool) {
        self.store.replace_meta(rows, exhausted);
    }

    pub fn clear(&mut self) {
        self.store = SlidingStore::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::store::WindowRow;

    #[test]
    fn ensure_meta_requests_only_on_deficit() {
        let mut pane: SlidingPane<u32, &str, String> = SlidingPane::ready(
            (0..30).map(|i| WindowRow::meta_only(i, "x")).collect(),
            90,
            false,
        );
        assert!(pane
            .ensure_meta(Viewport {
                first: 0,
                visible: 10
            })
            .is_none());
        let need = pane
            .ensure_meta(Viewport {
                first: 25,
                visible: 10,
            })
            .expect("deficit");
        assert!(need.budget >= 12);
        assert!(pane.is_fetching());
        assert!(pane
            .ensure_meta(Viewport {
                first: 25,
                visible: 10
            })
            .is_none());
    }

    #[test]
    fn ensure_bodies_evicts_and_reports_missing() {
        let mut pane: SlidingPane<u32, &str, String> = SlidingPane::ready(
            vec![
                WindowRow::with_body(1, "a", "A".into()),
                WindowRow::meta_only(2, "b"),
                WindowRow::with_body(3, "c", "C".into()),
            ],
            3,
            true,
        );
        let mut keep = HashSet::new();
        keep.insert(1);
        keep.insert(2);
        let missing = pane.ensure_bodies(keep);
        assert_eq!(missing, vec![2]);
        assert!(pane.rows().iter().find(|r| r.key == 1).unwrap().body_loaded);
        assert!(!pane.rows().iter().find(|r| r.key == 3).unwrap().body_loaded);
    }

    #[test]
    fn set_catalog_marks_exhausted_static_pane() {
        let mut pane: SlidingPane<String, i32, ()> = SlidingPane::default();
        pane.set_catalog(
            vec![
                WindowRow::meta_only("a".into(), 1),
                WindowRow::meta_only("b".into(), 2),
            ],
            true,
        );
        assert!(pane.exhausted());
        assert_eq!(pane.len(), 2);
        assert!(pane
            .ensure_meta(Viewport {
                first: 0,
                visible: 40
            })
            .is_none());
    }

    #[test]
    fn ensure_meta_sync_grows_only_to_budget() {
        let source: Vec<_> = (0..100u32).map(|i| WindowRow::meta_only(i, "m")).collect();
        let mut pane: SlidingPane<u32, &str, String> = SlidingPane::default();
        let grown = pane.ensure_meta_sync(
            Viewport {
                first: 0,
                visible: 10,
            },
            false,
            |budget| {
                let end = budget.min(source.len());
                (source[..end].to_vec(), end >= source.len())
            },
        );
        assert!(grown);
        // need_end=20 → budget ≥ 12 and typically covers ~60; not the full 100.
        assert!(pane.len() < 100);
        assert!(pane.len() >= 20);
        assert!(!pane.exhausted());
        // covered → no further growth
        assert!(!pane.ensure_meta_sync(
            Viewport {
                first: 0,
                visible: 10,
            },
            false,
            |_| panic!("must not fetch when covered"),
        ));
        // force still refetches
        assert!(pane.ensure_meta_sync(
            Viewport {
                first: 0,
                visible: 10,
            },
            true,
            |budget| {
                let end = budget.min(source.len());
                (source[..end].to_vec(), end >= source.len())
            },
        ));
    }

    #[test]
    fn ensure_bodies_sync_fills_missing() {
        let mut pane: SlidingPane<u32, &str, String> = SlidingPane::ready(
            vec![WindowRow::meta_only(1, "a"), WindowRow::meta_only(2, "b")],
            2,
            true,
        );
        let mut keep = HashSet::new();
        keep.insert(1);
        pane.ensure_bodies_sync(keep, |k| Some(format!("B{k}")));
        assert_eq!(
            pane.rows()
                .iter()
                .find(|r| r.key == 1)
                .unwrap()
                .body
                .as_deref(),
            Some("B1")
        );
        assert!(!pane.rows().iter().find(|r| r.key == 2).unwrap().body_loaded);
    }
}
