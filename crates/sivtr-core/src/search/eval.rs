//! Retrieval evaluation: golden queries, IR metrics, and the snapshot format.
//!
//! `sivtr eval` measures retrieval quality against a **snapshot of real data**
//! (frozen corpus + labeled queries), so ranking changes are gated on
//! measurable improvement over a fixed baseline instead of feel.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::record::WorkRecord;

use super::types::Field;

/// One golden query: what a good retrieval should surface for `query`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenQuery {
    pub name: String,
    pub query: String,
    #[serde(default)]
    pub field: Field,
    /// Expected relevant records as whole WorkRef strings (e.g. `terminal/dev/2`).
    pub relevant: Vec<String>,
}

/// A frozen evaluation fixture: a real-data corpus plus labeled queries.
/// Reproducible by construction — the same snapshot always yields the same report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalSnapshot {
    pub queries: Vec<GoldenQuery>,
    pub corpus: Vec<WorkRecord>,
}

/// Per-query metric results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryEval {
    pub name: String,
    /// Number of relevant records labeled for this query.
    pub relevant: usize,
    /// Number of records the pipeline retrieved (ranked length).
    pub retrieved: usize,
    pub recall_at_k: f64,
    pub precision_at_k: f64,
    pub mrr: f64,
    pub ndcg_at_k: f64,
}

/// Aggregated metrics across all queries.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AggregateMetrics {
    pub mean_recall_at_k: f64,
    pub mean_precision_at_k: f64,
    pub mean_mrr: f64,
    pub mean_ndcg_at_k: f64,
}

/// Full evaluation report for one pipeline run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    pub k: usize,
    pub queries: Vec<QueryEval>,
    pub aggregate: AggregateMetrics,
}

/// Compute metrics from precomputed ranked lists, one per query (aligned by index).
pub fn evaluate_with_ranked(
    queries: &[GoldenQuery],
    ranked: &[Vec<String>],
    k: usize,
) -> EvalReport {
    let per_query: Vec<QueryEval> = queries
        .iter()
        .zip(ranked)
        .map(|(query, ranked)| evaluate_query(query, ranked, k))
        .collect();
    let count = per_query.len().max(1) as f64;
    let aggregate = AggregateMetrics {
        mean_recall_at_k: per_query.iter().map(|q| q.recall_at_k).sum::<f64>() / count,
        mean_precision_at_k: per_query.iter().map(|q| q.precision_at_k).sum::<f64>() / count,
        mean_mrr: per_query.iter().map(|q| q.mrr).sum::<f64>() / count,
        mean_ndcg_at_k: per_query.iter().map(|q| q.ndcg_at_k).sum::<f64>() / count,
    };
    EvalReport {
        k,
        queries: per_query,
        aggregate,
    }
}

fn evaluate_query(query: &GoldenQuery, ranked: &[String], k: usize) -> QueryEval {
    let relevant: HashSet<&str> = query.relevant.iter().map(String::as_str).collect();
    let relevant_count = relevant.len();
    let k_used = k.max(1);
    let top_k: Vec<&String> = ranked.iter().take(k_used).collect();
    let is_hit = |position: usize| -> bool {
        top_k
            .get(position)
            .is_some_and(|ref_| relevant.contains(ref_.as_str()))
    };
    let hits = (0..top_k.len()).filter(|&i| is_hit(i)).count();

    // MRR scans the full ranked list, not just the top-k window.
    let mrr = match ranked
        .iter()
        .position(|ref_| relevant.contains(ref_.as_str()))
    {
        Some(position) => 1.0 / (position + 1) as f64,
        None => 0.0,
    };

    // NDCG@k with binary gains (1 for relevant, 0 otherwise).
    let dcg: f64 = (0..top_k.len())
        .map(|i| {
            if is_hit(i) {
                1.0 / (i as f64 + 2.0).log2()
            } else {
                0.0
            }
        })
        .sum();
    let ideal = (0..relevant_count.min(k_used))
        .map(|i| 1.0 / (i as f64 + 2.0).log2())
        .sum::<f64>();
    let ndcg = if ideal > 0.0 { dcg / ideal } else { 0.0 };

    QueryEval {
        name: query.name.clone(),
        relevant: relevant_count,
        retrieved: ranked.len(),
        recall_at_k: hits as f64 / relevant_count.max(1) as f64,
        precision_at_k: hits as f64 / k_used as f64,
        mrr,
        ndcg_at_k: ndcg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(name: &str, relevant: &[&str]) -> GoldenQuery {
        GoldenQuery {
            name: name.to_string(),
            query: name.to_string(),
            field: Field::Content,
            relevant: relevant.iter().map(|ref_| ref_.to_string()).collect(),
        }
    }

    #[test]
    fn metrics_hand_computed_for_perfect_ranking() {
        let query = q("q", &["a", "b", "c"]);
        let ranked = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let result = evaluate_query(&query, &ranked, 5);
        assert_eq!(result.recall_at_k, 1.0);
        assert_eq!(result.precision_at_k, 0.6);
        assert_eq!(result.mrr, 1.0);
        assert_eq!(result.ndcg_at_k, 1.0);
    }

    #[test]
    fn metrics_penalize_relevant_after_top_k() {
        let query = q("q", &["x", "y", "z"]);
        let ranked = vec![
            "a".to_string(),
            "x".to_string(),
            "b".to_string(),
            "y".to_string(),
        ];
        let result = evaluate_query(&query, &ranked, 2);
        // Only x lands in the top-2 window.
        assert_eq!(result.recall_at_k, 1.0 / 3.0);
        assert_eq!(result.precision_at_k, 0.5);
        assert_eq!(result.mrr, 0.5);
        // NDCG: dcg = 1/log2(3) over ideal 1/log2(2) + 1/log2(3).
        let expected_ndcg = (1.0 / 3.0_f64.log2()) / (1.0 + 1.0 / 3.0_f64.log2());
        assert!((result.ndcg_at_k - expected_ndcg).abs() < 1e-9);
    }

    #[test]
    fn no_hits_metrics_are_zero() {
        let query = q("q", &["a"]);
        let ranked = vec!["b".to_string(), "c".to_string()];
        let result = evaluate_query(&query, &ranked, 5);
        assert_eq!(result.recall_at_k, 0.0);
        assert_eq!(result.precision_at_k, 0.0);
        assert_eq!(result.mrr, 0.0);
        assert_eq!(result.ndcg_at_k, 0.0);
    }

    #[test]
    fn mrr_scans_full_list_beyond_k() {
        let query = q("q", &["z"]);
        let ranked = vec!["a".to_string(), "b".to_string(), "z".to_string()];
        let result = evaluate_query(&query, &ranked, 2);
        // recall@2 = 0 but the relevant record is rank 3.
        assert_eq!(result.recall_at_k, 0.0);
        assert_eq!(result.mrr, 1.0 / 3.0);
        assert_eq!(result.ndcg_at_k, 0.0);
    }

    #[test]
    fn evaluate_aggregates_over_queries() {
        let queries = vec![q("perfect", &["a"]), q("missing", &["zz"])];
        let ranked = vec![vec!["a".to_string()], vec!["b".to_string()]];
        let report = evaluate_with_ranked(&queries, &ranked, 5);
        assert_eq!(report.queries.len(), 2);
        assert_eq!(report.aggregate.mean_recall_at_k, 0.5);
        assert_eq!(report.aggregate.mean_ndcg_at_k, 0.5);
    }

    #[test]
    fn snapshot_round_trips() {
        let snapshot = EvalSnapshot {
            queries: vec![q("panic", &["terminal/dev/2"])],
            corpus: Vec::new(),
        };
        let json = serde_json::to_string(&snapshot).expect("serialize");
        let back: EvalSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, snapshot);
    }
}
