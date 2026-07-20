//! dhat heap profiles: dialogue materialize + session list collect.
//!
//! ```text
//! cargo bench -p sivtr --bench browse_dhat --features "perf-benches,dhat-heap"
//! # dhat-heap.json → https://nnethercote.github.io/dh_view/dh_view.html
//! ```

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use sivtr::commands::browse::perf::{HotPane, HydratedStore};

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    let n = std::env::var("SIVTR_DHAT_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200usize);
    let iters = std::env::var("SIVTR_DHAT_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100usize);

    // ── Dialogue: full body clone vs sparse materialize ───────────────────
    let pane = HotPane::prepare(n);

    let t0 = dhat::HeapStats::get();
    let mut naive_bodies = 0usize;
    for _ in 0..iters {
        naive_bodies = naive_bodies.saturating_add(pane.naive_full_clone());
    }
    let t1 = dhat::HeapStats::get();

    let mut sparse_bodies = 0usize;
    for _ in 0..iters {
        let (_, bodies) = pane.sparse_focus_materialize();
        sparse_bodies = sparse_bodies.saturating_add(bodies);
    }
    let t2 = dhat::HeapStats::get();

    let naive_bytes = t1.total_bytes.saturating_sub(t0.total_bytes);
    let naive_blocks = t1.total_blocks.saturating_sub(t0.total_blocks);
    let sparse_bytes = t2.total_bytes.saturating_sub(t1.total_bytes);
    let sparse_blocks = t2.total_blocks.saturating_sub(t1.total_blocks);

    eprintln!("=== dialogue materialize (n={n} dialogues × {iters} iters) ===");
    eprintln!("naive_full_body_clone:");
    eprintln!(
        "  body_clones={naive_bodies}  (expect ~ n*iters = {})",
        n * iters
    );
    eprintln!("  heap_bytes={naive_bytes}");
    eprintln!("  heap_blocks={naive_blocks}");
    eprintln!("sparse_titles_plus_focus_materialize:");
    eprintln!("  body_clones={sparse_bodies}  (expect ~ 1*iters = {iters})");
    eprintln!("  heap_bytes={sparse_bytes}");
    eprintln!("  heap_blocks={sparse_blocks}");
    if sparse_bytes > 0 {
        eprintln!(
            "heap_bytes ratio naive/sparse = {:.2}x",
            naive_bytes as f64 / sparse_bytes as f64
        );
    }
    if sparse_bodies > 0 {
        eprintln!(
            "body_clones ratio naive/sparse = {:.2}x",
            naive_bodies as f64 / sparse_bodies as f64
        );
    }

    // ── Session list: meta collect vs full body collect ───────────────────
    // 40 sessions × 50 fat turns (~4KiB text each) ≈ keep-set after hydrate.
    let n_sess = std::env::var("SIVTR_DHAT_SESSIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40usize);
    let n_recs = std::env::var("SIVTR_DHAT_RECORDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50usize);
    let list_iters = std::env::var("SIVTR_DHAT_LIST_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50usize);
    let store = HydratedStore::new(n_sess, n_recs);

    let t3 = dhat::HeapStats::get();
    let mut meta_n = 0usize;
    for _ in 0..list_iters {
        meta_n = store.project_meta();
    }
    let t4 = dhat::HeapStats::get();

    let mut full_n = 0usize;
    for _ in 0..list_iters {
        full_n = store.project_full();
    }
    let t5 = dhat::HeapStats::get();

    let meta_bytes = t4.total_bytes.saturating_sub(t3.total_bytes);
    let meta_blocks = t4.total_blocks.saturating_sub(t3.total_blocks);
    let full_bytes = t5.total_bytes.saturating_sub(t4.total_bytes);
    let full_blocks = t5.total_blocks.saturating_sub(t4.total_blocks);

    eprintln!();
    eprintln!(
        "=== session list collect ({n_sess} sessions × {n_recs} turns × {list_iters} iters) ==="
    );
    eprintln!("meta_only_collect:");
    eprintln!("  sessions={meta_n}");
    eprintln!("  heap_bytes={meta_bytes}");
    eprintln!("  heap_blocks={meta_blocks}");
    eprintln!("full_body_collect:");
    eprintln!("  sessions={full_n}");
    eprintln!("  heap_bytes={full_bytes}");
    eprintln!("  heap_blocks={full_blocks}");
    if meta_bytes > 0 {
        eprintln!(
            "heap_bytes ratio full/meta = {:.2}x",
            full_bytes as f64 / meta_bytes as f64
        );
    }
    eprintln!();
    eprintln!("peak_bytes={} peak_blocks={}", t5.max_bytes, t5.max_blocks);
    eprintln!("Open dhat-heap.json in dh_view for the full allocation tree.");
}
