//! Pseudo-relevance feedback (PRF) query expansion with a query-difficulty
//! gate.
//!
//! Literature: relevance-model style expansion (Lavrenko & Croft, SIGIR 2001)
//! with selective application (Amati, Carpineto & Romano, ECIR 2004): expansion
//! helps when the pseudo-relevant set is genuine (rare query terms), and
//! damages results when the top documents are noise (common query terms with
//! document frequency above the gate threshold — exactly the `share`/`status`
//! df 300+ case measured in the eval).

use std::collections::{HashMap, HashSet};

/// Pseudo-relevance feedback configuration.
pub struct Prf {
    /// Number of top-ranked documents treated as pseudo-relevant.
    pub top_k: usize,
    /// Maximum number of expansion terms added to the query.
    pub max_terms: usize,
    /// Weight applied to expansion terms when re-ranking (original terms 1.0).
    pub lambda: f64,
    /// Expansion is suppressed unless at least one query token has df/n below
    /// this ratio (rare terms -> pseudo-relevant set is likely genuine).
    pub max_df_ratio: f64,
}

impl Default for Prf {
    fn default() -> Self {
        Self {
            top_k: 5,
            max_terms: 4,
            lambda: 0.25,
            max_df_ratio: 0.1,
        }
    }
}

impl Prf {
    /// Difficulty gate: expand only when at least one query token is rare
    /// enough that the top documents are likely genuine matches, not noise.
    pub fn gate(&self, query_tokens: &[String], df_ratio: impl Fn(&str) -> f64) -> bool {
        if query_tokens.is_empty() {
            return false;
        }
        query_tokens
            .iter()
            .any(|token| df_ratio(token) < self.max_df_ratio)
    }

    /// Select expansion terms from pseudo-relevant documents ordered best-first
    /// (rank position `i` carries weight `1 / (i + 1)`). Query tokens are
    /// excluded; returns at most `max_terms` terms by descending weight.
    pub fn select_terms(&self, query_tokens: &[String], docs: &[Vec<String>]) -> Vec<String> {
        let query_set: HashSet<&str> = query_tokens.iter().map(String::as_str).collect();
        let mut weight: HashMap<String, f64> = HashMap::new();
        for (rank, doc) in docs.iter().enumerate() {
            let doc_weight = 1.0 / (rank as f64 + 1.0);
            for token in doc {
                if query_set.contains(token.as_str()) {
                    continue;
                }
                *weight.entry(token.clone()).or_insert(0.0) += doc_weight;
            }
        }
        let mut terms: Vec<(String, f64)> = weight.into_iter().collect();
        terms.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        terms
            .into_iter()
            .take(self.max_terms)
            .map(|(token, _)| token)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_opens_for_rare_term_query() {
        let prf = Prf::default();
        let tokens = vec!["connection".to_string(), "refused".to_string()];
        let df_ratio = |token: &str| match token {
            "connection" => 0.08,
            "refused" => 0.04,
            _ => 1.0,
        };
        assert!(prf.gate(&tokens, df_ratio));
    }

    #[test]
    fn gate_suppresses_common_word_query() {
        let prf = Prf::default();
        let tokens = vec![
            "sivtr".to_string(),
            "serve".to_string(),
            "status".to_string(),
        ];
        let df_ratio = |_token: &str| 0.3;
        assert!(!prf.gate(&tokens, df_ratio));
    }

    #[test]
    fn gate_handles_empty_query() {
        let prf = Prf::default();
        assert!(!prf.gate(&[], |_| 0.0));
    }

    #[test]
    fn select_terms_harvests_cooccurring_terms() {
        let prf = Prf::default();
        let query = vec!["panic".to_string()];
        let docs = vec![
            vec![
                "panic".to_string(),
                "cargo".to_string(),
                "run".to_string(),
                "server".to_string(),
            ],
            vec!["panic".to_string(), "cargo".to_string(), "test".to_string()],
        ];
        let terms = prf.select_terms(&query, &docs);
        // `cargo` co-occurs in both pseudo-relevant docs -> top expansion term.
        assert_eq!(terms[0], "cargo");
        // Query token is never re-added.
        assert!(!terms.contains(&"panic".to_string()));
        assert!(terms.len() <= prf.max_terms);
    }

    #[test]
    fn select_terms_respects_cap_and_order() {
        let prf = Prf {
            max_terms: 2,
            ..Prf::default()
        };
        let query = vec!["q".to_string()];
        let docs = vec![vec![
            "q".to_string(),
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
        ]];
        let terms = prf.select_terms(&query, &docs);
        assert_eq!(terms.len(), 2);
        assert_eq!(terms[0], "alpha");
        assert_eq!(terms[1], "beta");
    }
}
