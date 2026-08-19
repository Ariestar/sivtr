//! On-disk cache for built BM25 indexes.
//!
//! Building an index re-tokenizes every passage of the corpus (measured:
//! ~3s on a full agent corpus), so a fresh process should reuse the previous
//! process's index instead of rebuilding it. The cache key is the corpus
//! fingerprint ([`crate::cache::records_fingerprint`]) plus an index-version
//! tag; a fingerprint change (new/changed records) or a layout/scoring change
//! naturally misses and rebuilds.

use serde::{Deserialize, Serialize};

use crate::cache::{index_cache_path, records_fingerprint, write_cache_atomic};
use crate::record::WorkRef;
use crate::search::bm25::{Bm25Index, INDEX_CACHE_VERSION};

fn fingerprint(records: &[crate::record::WorkRecord]) -> u64 {
    let refs: Vec<WorkRef> = records
        .iter()
        .map(|record| record.work_ref.whole())
        .collect();
    records_fingerprint(&refs)
}

/// Cache envelope: version tag plus the fingerprint, so a stale file from a
/// different corpus or code revision is rejected before deserialization.
#[derive(Serialize, Deserialize)]
struct CachedIndex {
    version: u32,
    fingerprint: u64,
    index: Bm25Index,
}

/// Load the cached index for a corpus, or `None` on any mismatch — a stale or
/// corrupt cache must never fail the search, only cost a rebuild.
pub fn load_index(records: &[crate::record::WorkRecord]) -> Option<Bm25Index> {
    let fingerprint = fingerprint(records);
    let bytes = std::fs::read(index_cache_path(fingerprint)).ok()?;
    let cached: CachedIndex = rmp_serde::from_slice(&bytes).ok()?;
    if cached.version != INDEX_CACHE_VERSION || cached.fingerprint != fingerprint {
        return None;
    }
    Some(cached.index)
}

/// Best-effort write of a built index; failures only cost a rebuild next run.
pub fn store_index(records: &[crate::record::WorkRecord], index: &Bm25Index) {
    let cached = CachedIndex {
        version: INDEX_CACHE_VERSION,
        fingerprint: fingerprint(records),
        index: index.clone(),
    };
    if let Ok(bytes) = rmp_serde::to_vec(&cached) {
        write_cache_atomic(&index_cache_path(cached.fingerprint), &bytes);
    }
}

/// Build or load the index for a corpus: reuse the cached index when it
/// matches, otherwise build and persist.
pub fn build_or_load(records: &[crate::record::WorkRecord]) -> Bm25Index {
    if let Some(index) = load_index(records) {
        return index;
    }
    let index = Bm25Index::build(records);
    store_index(records, &index);
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{
        WorkChannel, WorkOutcome, WorkPart, WorkPartData, WorkRecord, WorkRecordKind,
        WorkSessionRef, WorkSource, WorkStatus, WorkTime, RECORD_SCHEMA_VERSION,
    };
    use crate::search::bm25::Bm25Index;

    fn record(session: &str, index: usize, title: &str, text: &str) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::terminal(session, index),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session.to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: Some(WorkStatus {
                outcome: WorkOutcome::Success,
                exit_code: Some(0),
            }),
            title: title.to_string(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::Output {
                    content: text.to_string(),
                    ansi: None,
                },
            }],
        }
    }

    #[test]
    fn cache_round_trips_a_built_index() {
        let records = vec![
            record("s1", 1, "cargo install", "building project"),
            record("s1", 2, "run tests", "tests passed"),
        ];
        let built = Bm25Index::build(&records);
        store_index(&records, &built);
        let loaded = load_index(&records).expect("cache hit");
        assert_eq!(loaded, built);
        let _ = std::fs::remove_file(index_cache_path(fingerprint(&records)));
    }

    #[test]
    fn cache_misses_on_different_corpus() {
        let records = vec![record("s1", 1, "a", "one")];
        let built = Bm25Index::build(&records);
        store_index(&records, &built);
        let other = vec![record("s9", 9, "b", "two")];
        assert!(load_index(&other).is_none());
        let _ = std::fs::remove_file(index_cache_path(fingerprint(&records)));
    }
}
