//! BM25 relevance ranking over a record corpus. A standard implementation with
//! tf saturation (`k1`) and length normalization (`b`), built once per corpus
//! and answering many queries. The previous `bm25` crate scored `idf * tf`
//! without either term, so long documents with many query-word mentions (agent
//! sessions, tool listings) outranked short semantic matches.

use std::collections::HashMap;

use crate::record::{WorkPartKind, WorkRecord, WorkRef};

/// Standard BM25 term-saturation and length-normalization constants.
const K1: f64 = 1.2;
const B: f64 = 0.75;

/// Cap on tokens indexed per document. Agent turns and tool listings can run
/// to tens of thousands of tokens; without a cap their raw term frequencies
/// outscore short semantic matches (BM25's long-document bias).
const MAX_TOKENS_PER_DOC: usize = 800;

/// Splits on non-alphanumeric runs, lowercases Latin/digit runs, and emits
/// overlapping CJK bigrams (Lucene CJKAnalyzer style) so Chinese text is
/// searchable without a segmentation dictionary.
pub struct SimpleTokenizer;

impl SimpleTokenizer {
    pub fn tokenize(&self, text: &str) -> Vec<String> {
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
    n: usize,
    avgdl: f64,
    /// Token -> number of documents containing it.
    df: HashMap<String, f64>,
    /// Token -> [(document id, term frequency)].
    postings: HashMap<String, Vec<(usize, f64)>>,
    doc_len: Vec<f64>,
    refs: Vec<WorkRef>,
}

impl Bm25Index {
    pub fn build(records: &[WorkRecord]) -> Self {
        let mut df = HashMap::new();
        let mut postings: HashMap<String, Vec<(usize, f64)>> = HashMap::new();
        let mut doc_len = Vec::with_capacity(records.len());
        let refs = records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect::<Vec<_>>();

        for (doc_id, record) in records.iter().enumerate() {
            let mut term_freq: HashMap<String, usize> = HashMap::new();
            for token in SimpleTokenizer
                .tokenize(&doc_text(record))
                .into_iter()
                .take(MAX_TOKENS_PER_DOC)
            {
                *term_freq.entry(token).or_insert(0) += 1;
            }
            doc_len.push(term_freq.values().sum::<usize>() as f64);
            for (token, count) in term_freq {
                *df.entry(token.clone()).or_insert(0.0) += 1.0;
                postings
                    .entry(token)
                    .or_default()
                    .push((doc_id, count as f64));
            }
        }

        let n = records.len();
        let avgdl = if n > 0 {
            doc_len.iter().sum::<f64>() / n as f64
        } else {
            1.0
        };
        Self {
            n,
            avgdl,
            df,
            postings,
            doc_len,
            refs,
        }
    }

    /// Rank the corpus by BM25 relevance to `query`, best first. Only records
    /// sharing at least one query token appear in the result.
    pub fn rank(&self, query: &str) -> Vec<(WorkRef, f32)> {
        let tokens = SimpleTokenizer.tokenize(query);
        let mut scores: HashMap<usize, f64> = HashMap::new();
        for token in tokens {
            let Some(&df) = self.df.get(&token) else {
                continue;
            };
            let Some(postings) = self.postings.get(&token) else {
                continue;
            };
            // Robertson idf; non-negative for every token frequency.
            let idf = ((self.n as f64 - df + 0.5) / (df + 0.5) + 1.0).ln();
            for (doc_id, tf) in postings {
                let denom = tf + K1 * (1.0 - B + B * self.doc_len[*doc_id] / self.avgdl);
                let contribution = idf * tf * (K1 + 1.0) / denom;
                *scores.entry(*doc_id).or_insert(0.0) += contribution;
            }
        }

        let mut ranked: Vec<(usize, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked
            .into_iter()
            .map(|(doc_id, score)| (self.refs[doc_id].clone(), score as f32))
            .collect()
    }
}

/// Document text for BM25: the title repeated three times (cheap field boost)
/// followed by every part's text. Covers the same parts the default content
/// pattern matches — dialogue, output, tool results, thinking, errors — and
/// leaves tool-call payloads and skill text out, so the ranked text is the
/// same source the boolean filter reads.
fn doc_text(record: &WorkRecord) -> String {
    let mut text = String::new();
    for _ in 0..3 {
        text.push_str(&record.title);
        text.push('\n');
    }
    for part in &record.parts {
        if matches!(part.kind(), WorkPartKind::ToolCall | WorkPartKind::Skill) {
            continue;
        }
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

    #[test]
    fn long_noise_document_does_not_outrank_short_match() {
        // The long document repeats one query term ten times but only matches
        // a single term; the short one matches all three terms. tf saturation
        // and length normalization keep the short match on top.
        let long_noise =
            "command command command command command command command command command command \
            setup tooling docs and miscellaneous text";
        let corpus = vec![
            record("dev", 1, "command error", "command not found: get-item"),
            record("dev", 2, "unrelated", "the found command ran fine"),
            record("dev", 3, "setup log", long_noise),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = index.rank("command not found");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn multi_word_query_prefers_documents_matching_all_terms() {
        let corpus = vec![
            record("dev", 1, "command error", "command not found: get-item"),
            record("dev", 2, "unrelated", "the found command ran fine"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = index.rank("command not found");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }
}
