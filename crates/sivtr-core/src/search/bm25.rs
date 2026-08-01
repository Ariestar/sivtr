//! BM25 relevance ranking over a record corpus, powered by the `bm25` crate
//! (Lucene/Elasticsearch-style scoring). `Bm25Index` is built once per corpus
//! and answers many queries.

use bm25::{SearchEngine, SearchEngineBuilder, Tokenizer};

use crate::record::{WorkRecord, WorkRef};

/// Splits on non-alphanumeric runs, lowercases Latin/digit runs, and emits
/// overlapping CJK bigrams (Lucene CJKAnalyzer style) so Chinese text is
/// searchable without a segmentation dictionary.
// ponytail: no stemming or idf clamping — the crate uses raw Robertson idf, so
// very common tokens get negative idf. Measured still beats recency; revisit if
// command-history queries matter.
pub struct SimpleTokenizer;

impl Tokenizer for SimpleTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            if is_cjk(ch) {
                let mut run = String::new();
                while i < chars.len() && is_cjk(chars[i]) {
                    run.push(chars[i]);
                    i += 1;
                }
                let run: Vec<char> = run.chars().collect();
                if run.len() == 1 {
                    tokens.push(run[0].to_string());
                } else {
                    for bigram in run.windows(2) {
                        tokens.push(bigram.iter().collect());
                    }
                }
            } else if ch.is_alphanumeric() {
                let mut run = String::new();
                while i < chars.len() && !is_cjk(chars[i]) && chars[i].is_alphanumeric() {
                    run.push(chars[i]);
                    i += 1;
                }
                if !run.is_empty() {
                    tokens.push(run.to_lowercase());
                }
            } else {
                i += 1;
            }
        }
        tokens
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4DBF     // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0xF900..=0xFAFF   // CJK Compatibility Ideographs
        | 0x20000..=0x2FA1F // CJK Extension B..F + Compatibility Supplement
    )
}

/// An in-memory BM25 index over a record corpus. Build once, query many times.
pub struct Bm25Index {
    engine: SearchEngine<u32, u32, SimpleTokenizer>,
    refs: Vec<WorkRef>,
}

impl Bm25Index {
    pub fn build(records: &[WorkRecord]) -> Self {
        let corpus = records.iter().map(doc_text).collect::<Vec<_>>();
        let engine = SearchEngineBuilder::<u32, u32, SimpleTokenizer>::with_tokenizer_and_corpus(
            SimpleTokenizer,
            corpus,
        )
        .build();
        Self {
            engine,
            refs: records
                .iter()
                .map(|record| record.work_ref.whole())
                .collect(),
        }
    }

    /// Rank the corpus by BM25 relevance to `query`, best first. Only records
    /// sharing at least one query token appear in the result.
    pub fn rank(&self, query: &str) -> Vec<(WorkRef, f32)> {
        self.engine
            .search(query, None)
            .into_iter()
            .map(|result| (self.refs[result.document.id as usize].clone(), result.score))
            .collect()
    }
}

/// Document text for BM25: the title repeated three times (cheap field boost)
/// followed by every part's text.
fn doc_text(record: &WorkRecord) -> String {
    let mut text = String::new();
    for _ in 0..3 {
        text.push_str(&record.title);
        text.push('\n');
    }
    for part in &record.parts {
        text.push_str(&part.text());
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{
        WorkChannel, WorkOutcome, WorkPart, WorkPartData, WorkRecord, WorkRecordKind,
        WorkSessionRef, WorkSource, WorkStatus, WorkTime, RECORD_SCHEMA_VERSION,
    };

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
    fn tokenizer_splits_and_lowercases() {
        let tokens = SimpleTokenizer.tokenize("Deploy-web: Foo.Bar_1");
        assert_eq!(tokens, vec!["deploy", "web", "foo", "bar", "1"]);
    }

    #[test]
    fn tokenizer_emits_cjk_bigrams() {
        // 4-char run -> 3 overlapping bigrams; 2-char word survives as one bigram.
        assert_eq!(
            SimpleTokenizer.tokenize("重构逻辑"),
            vec!["重构", "构逻", "逻辑"]
        );
        assert_eq!(SimpleTokenizer.tokenize("重构"), vec!["重构"]);
        // Mixed CJK + Latin keeps both token types.
        assert_eq!(SimpleTokenizer.tokenize("重构cargo"), vec!["重构", "cargo"]);
    }

    #[test]
    fn chinese_query_matches_documents_containing_the_words() {
        let corpus = vec![
            record("dev", 1, "重构下", "重构下，保证逻辑是最干净的"),
            record("dev", 2, "cargo build", "Finished dev profile"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = index.rank("重构");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn query_surfaces_matching_records_first() {
        let corpus = vec![
            record(
                "dev",
                1,
                "sqlite schema",
                "CREATE TABLE records (id TEXT PRIMARY KEY)",
            ),
            record(
                "dev",
                2,
                "rollback procedure",
                "kubectl rollout undo deploy/web",
            ),
            record("dev", 3, "git log", "7227bd8 refactor tui"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = index.rank("rollback");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/2");
    }

    #[test]
    fn unmatched_query_scores_nothing() {
        let corpus = vec![record("dev", 1, "cargo build", "Finished dev profile")];
        let index = Bm25Index::build(&corpus);
        assert!(index.rank("zzzznothing").is_empty());
    }
}
