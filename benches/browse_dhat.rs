//! dhat heap profiles: full-body clone vs sparse materialize.
//!
//! ```text
//! cargo bench -p sivtr --bench browse_dhat --features "perf-benches,dhat-heap"
//! # dhat-heap.json → https://nnethercote.github.io/dh_view/dh_view.html
//! ```

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use sivtr::commands::browse::perf::HotPane;

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

    // Setup outside measured deltas as much as possible.
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

    eprintln!("=== browse_dhat (n={n} dialogues × {iters} iters) ===");
    eprintln!("naive_full_body_clone:");
    eprintln!("  body_clones={naive_bodies}  (expect ~ n*iters = {})", n * iters);
    eprintln!("  heap_bytes={naive_bytes}");
    eprintln!("  heap_blocks={naive_blocks}");
    eprintln!("sparse_titles_plus_focus_materialize:");
    eprintln!(
        "  body_clones={sparse_bodies}  (expect ~ 1*iters = {iters})"
    );
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
    eprintln!("peak_bytes={} peak_blocks={}", t2.max_bytes, t2.max_blocks);
    eprintln!("Open dhat-heap.json in dh_view for the full allocation tree.");
}
