//! Reciprocal rank fusion (RRF) of independent rank signals.
//!
//! Literature: Cormack, Clarke & Büttcher, SIGIR 2009 — fuse rank lists via
//! Σ 1/(k + rank); no score calibration required. Standard fusion in modern
//! hybrid retrieval.

/// RRF scores for items `0..n` from per-signal ranked id lists (best first).
/// Items absent from a signal list receive no contribution from it.
pub fn rrf_scores(n: usize, ranked_lists: &[Vec<usize>], k: usize) -> Vec<f64> {
    let mut scores = vec![0.0; n];
    for list in ranked_lists {
        for (rank, &item) in list.iter().enumerate() {
            if item < n {
                scores[item] += 1.0 / (k as f64 + (rank + 1) as f64);
            }
        }
    }
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fused_ordering_beats_either_single_signal() {
        // Doc 1 ranks 2nd in BOTH signals. Doc 0 is #1 on relevance, doc 3 is
        // #1 on recency — so neither single signal puts doc 1 on top, but the
        // fusion does (best average rank).
        let n = 4;
        let relevance = vec![0, 1, 2, 3];
        let recency = vec![3, 1, 2, 0];
        let fused = rrf_scores(n, &[relevance, recency], 60);
        let order: Vec<usize> = {
            let mut idx: Vec<usize> = (0..n).collect();
            idx.sort_by(|&a, &b| fused[b].total_cmp(&fused[a]));
            idx
        };
        assert_eq!(
            order[0], 1,
            "fusion must promote the doc both signals rank well"
        );
        assert!(fused[1] > fused[0]);
        assert!(fused[1] > fused[3]);
    }

    #[test]
    fn absent_items_get_no_contribution() {
        let n = 4;
        // Signal 1 only ranks items 0..2; item 3 appears in no signal.
        let fused = rrf_scores(n, &[vec![0, 1, 2]], 60);
        assert_eq!(fused[3], 0.0);
        assert!(fused[0] > 0.0);
    }

    #[test]
    fn k_controls_signal_influence() {
        let n = 2;
        let fused = rrf_scores(n, &[vec![1, 0]], 1);
        // rank1 = 1/2 = 0.5, rank2 = 1/3 ≈ 0.333
        assert!((fused[1] - 0.5).abs() < 1e-9);
        assert!((fused[0] - 1.0 / 3.0).abs() < 1e-9);
    }
}
