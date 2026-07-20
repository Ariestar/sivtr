//! Criterion microbenchmarks for browse hot path.
//!
//! ```text
//! cargo bench -p sivtr --bench browse_hot --features perf-benches
//! # HTML: target/criterion/report/index.html
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sivtr::commands::browse::perf::{run_ensure_growth, HotPane, HydratedStore};

fn bench_materialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("dialogue_materialize");
    for n in [50usize, 200, 500] {
        let pane = HotPane::prepare(n);
        group.throughput(Throughput::Elements(n as u64));

        group.bench_with_input(BenchmarkId::new("naive_full_body_clone", n), &n, |b, _| {
            b.iter(|| black_box(pane.naive_full_clone()))
        });

        group.bench_with_input(
            BenchmarkId::new("sparse_titles_plus_focus_materialize", n),
            &n,
            |b, _| b.iter(|| black_box(pane.sparse_focus_materialize())),
        );
    }
    group.finish();
}

fn bench_ensure_growth(c: &mut Criterion) {
    c.bench_function("dialogue_ensure_meta_grow_viewport", |b| {
        b.iter(|| black_box(run_ensure_growth()))
    });
}

fn bench_titles_iter(c: &mut Criterion) {
    let pane = HotPane::prepare(500);
    c.bench_function("dialogue_titles_collect_500", |b| {
        b.iter(|| black_box(pane.titles_count()))
    });
}

/// Session list projection: meta-only collect vs full body clone.
///
/// Fixture: 40 sessions × 50 fat turns (4KiB each) — mimics a hydrated keep set.
fn bench_session_list_project(c: &mut Criterion) {
    let store = HydratedStore::new(40, 50);
    let mut group = c.benchmark_group("session_list_project");
    group.throughput(Throughput::Elements(40));
    group.bench_function("meta_only_collect", |b| {
        b.iter(|| black_box(store.project_meta()))
    });
    group.bench_function("full_body_collect", |b| {
        b.iter(|| black_box(store.project_full()))
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_materialize,
    bench_ensure_growth,
    bench_titles_iter,
    bench_session_list_project
);
criterion_main!(benches);
