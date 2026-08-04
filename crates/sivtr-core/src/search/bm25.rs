//! BM25 relevance ranking over a record corpus.
//!
//! Implemented literature techniques (see `docs/retrieval-literature.md`):
//! - **Fielded weighting** (BM25F, Robertson et al., CIKM 2004): the title
//!   (which for terminal records is the executed command) is a separate field
//!   with its own weight and length normalization, replacing the old hack of
//!   replicating the title three times into the ranked text.
//! - **BM25+ lower bound** (Lv & Zhai, CIKM 2011): every query term present in
//!   a document contributes at least `idf * δ`, so a long document that does
//!   match is scored strictly above a short document that does not match at
//!   all — important for sivtr's long agent turns whose error text sits far
//!   into the conversation.
//! - Head+tail token windows keep both protections: short command records stay
//!   fully indexed, mid-document chatter about common command words stays out
//!   of the postings, and end-of-session errors stay findable.

use std::collections::HashMap;

use crate::record::{WorkPartKind, WorkRecord, WorkRef};

/// Standard BM25 term-saturation and length-normalization constants.
const K1: f64 = 1.2;
const B: f64 = 0.75;

/// BM25+ lower bound added to every matching query term (Lv & Zhai, CIKM 2011).
const DELTA: f64 = 1.0;

/// Weight of the title field relative to the body field (BM25F-style).
/// The title of a terminal record is the command itself, so a title match is
/// far stronger evidence than a body mention. Applied only to multi-token
/// queries (see [`Bm25Index::rank_terms_with`]): a single-token query like
/// `grok` must not promote unrelated terminal records over provider sessions.
pub const TITLE_WEIGHT: f64 = 3.0;

/// Tokens indexed per document body, applied as a head window plus a tail
/// window of this many tokens each. A hard head-only cap silently dropped
/// terms that occur late in long agent turns — exactly where error text
/// lives — while indexing everything lets common command words inside long
/// chats (df 300+) outrank the short records that actually ran the command.
const WINDOW_TOKENS: usize = 800;

/// Splits on non-alphanumeric runs, lowercases Latin/digit runs (so `Error[E`
/// and `error[E0428]` share the token `error`), and emits overlapping CJK
/// bigrams (Lucene CJKAnalyzer style) so Chinese text is searchable without a
/// segmentation dictionary.
pub struct SimpleTokenizer;

impl SimpleTokenizer {
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(&ch) = chars.peek() {
            if is_cjk(ch) {
                let mut run = String::new();
                while let Some(&c) = chars.peek() {
                    if !is_cjk(c) {
                        break;
                    }
                    run.push(c);
                    chars.next();
                }
                let run_chars: Vec<char> = run.chars().collect();
                if run_chars.len() == 1 {
                    tokens.push(run);
                } else {
                    for bigram in run_chars.windows(2) {
                        tokens.push(bigram.iter().collect());
                    }
                }
            } else if ch.is_alphanumeric() {
                let mut run = String::new();
                while let Some(&c) = chars.peek() {
                    if is_cjk(c) || !c.is_alphanumeric() {
                        break;
                    }
                    run.push(c);
                    chars.next();
                }
                if !run.is_empty() {
                    tokens.push(run.to_lowercase());
                }
            } else {
                chars.next();
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

/// An in-memory fielded BM25 index over a record corpus. Build once, query
/// many times. Each document is two fields: `title` (short, weight
/// [`TITLE_WEIGHT`]) and `body` (head+tail windowed, weight 1).
pub struct Bm25Index {
    n: usize,
    avgdl_title: f64,
    avgdl_body: f64,
    /// Token -> number of documents containing it.
    df: HashMap<String, f64>,
    /// Token -> [(document id, title term frequency, body term frequency)].
    postings: HashMap<String, Vec<(usize, f64, f64)>>,
    doc_len_title: Vec<f64>,
    doc_len_body: Vec<f64>,
    refs: Vec<WorkRef>,
}

impl Bm25Index {
    pub fn build(records: &[WorkRecord]) -> Self {
        let mut df = HashMap::new();
        let mut postings: HashMap<String, Vec<(usize, f64, f64)>> = HashMap::new();
        let mut doc_len_title = Vec::with_capacity(records.len());
        let mut doc_len_body = Vec::with_capacity(records.len());
        let refs = records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect::<Vec<_>>();

        for (doc_id, record) in records.iter().enumerate() {
            let mut tf_title: HashMap<String, usize> = HashMap::new();
            for token in SimpleTokenizer.tokenize(&record.title) {
                *tf_title.entry(token).or_insert(0) += 1;
            }
            let mut tf_body: HashMap<String, usize> = HashMap::new();
            let body_tokens = SimpleTokenizer.tokenize(&body_text(record));
            if body_tokens.len() <= 2 * WINDOW_TOKENS {
                for token in body_tokens {
                    *tf_body.entry(token).or_insert(0) += 1;
                }
            } else {
                // Head + tail windows; disjoint because len > 2 * WINDOW_TOKENS.
                for token in body_tokens
                    .iter()
                    .take(WINDOW_TOKENS)
                    .chain(body_tokens.iter().skip(body_tokens.len() - WINDOW_TOKENS))
                {
                    *tf_body.entry(token.clone()).or_insert(0) += 1;
                }
            }
            doc_len_title.push(tf_title.values().sum::<usize>() as f64);
            doc_len_body.push(tf_body.values().sum::<usize>() as f64);

            // Merge both fields into df/postings; a token in either field marks
            // the document as containing it.
            let mut seen = std::collections::HashSet::new();
            for token in tf_title.keys().chain(tf_body.keys()) {
                if !seen.insert(token) {
                    continue;
                }
                *df.entry(token.clone()).or_insert(0.0) += 1.0;
                postings.entry(token.clone()).or_default().push((
                    doc_id,
                    *tf_title.get(token).unwrap_or(&0) as f64,
                    *tf_body.get(token).unwrap_or(&0) as f64,
                ));
            }
        }

        let n = records.len();
        let avg = |values: &[f64]| -> f64 {
            if n > 0 {
                values.iter().sum::<f64>() / n as f64
            } else {
                1.0
            }
        };
        Self {
            n,
            avgdl_title: avg(&doc_len_title),
            avgdl_body: avg(&doc_len_body),
            df,
            postings,
            doc_len_title,
            doc_len_body,
            refs,
        }
    }

    /// Document-frequency ratio of a token (0.0 when absent). Used by the PRF
    /// difficulty gate: expansion is suppressed for common-word queries.
    pub fn df_ratio(&self, token: &str) -> f64 {
        self.df.get(token).copied().unwrap_or(0.0) / self.n.max(1) as f64
    }

    /// Rank the corpus by BM25 relevance to `query`, best first. Only records
    /// sharing at least one query token appear in the result.
    pub fn rank(&self, query: &str) -> Vec<(WorkRef, f32)> {
        let terms = SimpleTokenizer
            .tokenize(query)
            .into_iter()
            .map(|token| (token, 1.0))
            .collect::<Vec<_>>();
        self.rank_terms(&terms)
    }

    /// Rank with explicit per-term weights (expansion terms carry `lambda`).
    pub fn rank_terms(&self, terms: &[(String, f64)]) -> Vec<(WorkRef, f32)> {
        self.rank_terms_with(terms, TITLE_WEIGHT)
    }

    /// Rank with explicit per-term weights and a configurable title-field
    /// weight. Single-token queries pass 1.0 so a lone keyword does not get
    /// boosted into unrelated terminal records; multi-token command queries
    /// keep the full [`TITLE_WEIGHT`].
    pub fn rank_terms_with(
        &self,
        terms: &[(String, f64)],
        title_weight: f64,
    ) -> Vec<(WorkRef, f32)> {
        self.score_terms(terms, title_weight)
            .into_iter()
            .map(|(doc_id, score)| (self.refs[doc_id].clone(), score as f32))
            .collect()
    }

    /// Ranked document ids (aligned with `records`) for the same term weights.
    /// Used by the RRF fusion to build per-signal rank lists without re-finding
    /// records by reference.
    pub fn ranked_ids(&self, terms: &[(String, f64)]) -> Vec<usize> {
        self.ranked_ids_with(terms, TITLE_WEIGHT)
    }

    /// Ranked document ids with a configurable title weight (see
    /// [`Self::rank_terms_with`]).
    pub fn ranked_ids_with(&self, terms: &[(String, f64)], title_weight: f64) -> Vec<usize> {
        self.score_terms(terms, title_weight)
            .into_iter()
            .map(|(doc_id, _)| doc_id)
            .collect()
    }

    fn score_terms(&self, terms: &[(String, f64)], title_weight: f64) -> Vec<(usize, f64)> {
        let mut scores: HashMap<usize, f64> = HashMap::new();
        for (token, weight) in terms {
            let Some(&df) = self.df.get(token) else {
                continue;
            };
            let Some(postings) = self.postings.get(token) else {
                continue;
            };
            // Robertson idf; non-negative for every token frequency.
            let idf = ((self.n as f64 - df + 0.5) / (df + 0.5) + 1.0).ln();
            for &(doc_id, tf_title, tf_body) in postings {
                // BM25F-style per-field saturation with per-field length
                // normalization, plus the BM25+ lower bound per matching term.
                let sat = |tf: f64, len: f64, avgdl: f64| {
                    tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * len / avgdl.max(1.0)))
                };
                let contribution = idf
                    * (title_weight * sat(tf_title, self.doc_len_title[doc_id], self.avgdl_title)
                        + sat(tf_body, self.doc_len_body[doc_id], self.avgdl_body)
                        + DELTA);
                *scores.entry(doc_id).or_insert(0.0) += weight * contribution;
            }
        }

        let mut ranked: Vec<(usize, f64)> = scores.into_iter().collect();
        // Deterministic order: score desc, then reference asc (HashMap iteration
        // order is randomized per process, so ties must be broken explicitly).
        ranked.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| self.refs[a.0].to_string().cmp(&self.refs[b.0].to_string()))
        });
        ranked
    }
}

/// Body text for BM25: every part's text, skipping tool-call payloads and
/// skill text (the same source the default content boolean filter reads).
pub fn body_text(record: &WorkRecord) -> String {
    let mut text = String::new();
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
    fn tokenizer_normalizes_case_for_error_codes() {
        // Query `error[E` must share the `error` token with `Error[E0428]`.
        let doc_tokens = SimpleTokenizer.tokenize("thread panicked: Error[E0428]");
        let query_tokens = SimpleTokenizer.tokenize("error[E");
        assert!(doc_tokens.contains(&"error".to_string()));
        assert!(query_tokens.contains(&"error".to_string()));
        assert!(doc_tokens.contains(&"e0428".to_string()));
        assert!(!query_tokens.contains(&"E".to_string()));
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

    #[test]
    fn late_term_in_long_document_is_still_indexed_and_ranked() {
        // Regression: a head-only per-document token cap silently dropped terms
        // that occur late in long agent turns — exactly where error text lives.
        // The tail window must keep them findable.
        let mut noise = String::new();
        for i in 0..2_000 {
            noise.push_str(&format!("padding token {i} "));
        }
        let long_turn = format!("{noise}panicked at src/main.rs:42");
        let corpus = vec![
            record("dev", 1, "long agent turn", &long_turn),
            record("dev", 2, "short chatter", "no signal here at all"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = index.rank("panicked");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn matching_long_document_outranks_nonmatching_short_document() {
        // BM25+ lower bound: a long document that does match the query is
        // scored strictly above a short document that does not match at all.
        let mut noise = String::new();
        for i in 0..3_000 {
            noise.push_str(&format!("filler token {i} "));
        }
        let long_match = format!("{noise}kubectl gotcha at the very end");
        let corpus = vec![
            record("dev", 1, "long session", &long_match),
            record("dev", 2, "short unrelated", "just a short line"),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = index.rank("kubectl");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
        assert_eq!(ranked.len(), 1, "non-matching doc must not score");
    }

    #[test]
    fn title_field_boost_beats_body_only_mention() {
        // Fielded weighting: a title match (the command itself for terminal
        // records) outranks a body-only mention of the same common words.
        let corpus = vec![
            record("dev", 1, "sivtr serve status", "ok"),
            record(
                "dev",
                2,
                "chatter",
                "user asked about sivtr serve status and remote list today",
            ),
        ];
        let index = Bm25Index::build(&corpus);
        let ranked = index.rank("sivtr serve status");
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }

    #[test]
    fn rank_terms_applies_per_term_weights() {
        let corpus = vec![
            record("dev", 1, "fix", "cargo run panicked"),
            record("dev", 2, "other", "cargo build passed"),
        ];
        let index = Bm25Index::build(&corpus);
        // Expansion term `panicked` with weight 1 pulls dev/1 to the top even
        // though `cargo` alone matches both documents.
        let terms = vec![("cargo".to_string(), 1.0), ("panicked".to_string(), 1.0)];
        let ranked = index.rank_terms(&terms);
        assert_eq!(ranked[0].0.to_string(), "terminal/dev/1");
    }
}
