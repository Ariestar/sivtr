use std::collections::HashMap;

use super::model::WorkRecord;
use super::refs::WorkRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarityMatch {
    pub record_ref: WorkRef,
    pub score: u32,
    pub matched_terms: Vec<String>,
}

pub fn semantic_search(
    records: &[WorkRecord],
    query: &str,
    limit: usize,
    include: impl Fn(&WorkRecord) -> bool,
) -> Vec<SimilarityMatch> {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return Vec::new();
    }

    let idf = compute_idf(records, &query_terms);

    let mut scored: Vec<SimilarityMatch> = records
        .iter()
        .filter(|record| include(record))
        .filter_map(|record| {
            let (score, matched) = compute_relevance(record, &query_terms, &idf);
            if score > 0 {
                Some(SimilarityMatch {
                    record_ref: record.work_ref.clone(),
                    score,
                    matched_terms: matched,
                })
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| b.score.cmp(&a.score));
    scored.truncate(limit);
    scored
}

fn compute_idf(records: &[WorkRecord], query_terms: &[String]) -> HashMap<String, f64> {
    let total = records.len() as f64;
    if total == 0.0 {
        return HashMap::new();
    }

    let mut doc_freq: HashMap<String, usize> = HashMap::new();
    for record in records {
        let mut seen = tokenize(&record.combined_text())
            .into_iter()
            .collect::<Vec<_>>();
        seen.sort_unstable();
        seen.dedup();
        for term in seen {
            *doc_freq.entry(term).or_insert(0) += 1;
        }
    }

    query_terms
        .iter()
        .map(|term| {
            let df = *doc_freq.get(term).unwrap_or(&1) as f64;
            let idf = (total / df).ln() + 1.0;
            (term.clone(), idf)
        })
        .collect()
}

fn compute_relevance(
    record: &WorkRecord,
    query_terms: &[String],
    idf: &HashMap<String, f64>,
) -> (u32, Vec<String>) {
    let mut bag = HashMap::new();
    let mut total = 0u32;
    for term in tokenize(&record.combined_text()) {
        *bag.entry(term).or_insert(0u32) += 1;
        total += 1;
    }
    if total == 0 {
        return (0, Vec::new());
    }

    let mut score = 0u32;
    let mut matched = Vec::new();
    let title_terms = tokenize(&record.title);
    let part_terms: Vec<Vec<String>> = record.parts.iter().map(|p| tokenize(&p.text)).collect();
    for term in query_terms {
        let in_content = if let Some(&count) = bag.get(term) {
            let tf = count as f64 / total as f64;
            let idf_weight = idf.get(term).copied().unwrap_or(1.0);
            score += (tf * 10.0 * idf_weight).round() as u32;
            true
        } else {
            false
        };
        let in_title = title_terms.iter().any(|t| t == term);
        let in_parts = part_terms
            .iter()
            .any(|terms| terms.iter().any(|t| t == term));
        if in_content || in_title || in_parts {
            matched.push(term.clone());
            if in_title {
                score += 5;
            }
            if in_parts {
                score += 2;
            }
        }
    }
    (score, matched)
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty() && s.len() > 1)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::model::{
        WorkChannel, WorkOutcome, WorkPartIo, WorkPartKind, WorkRecordKind, WorkSessionRef,
        WorkSource, WorkStatus, WorkTime,
    };

    #[test]
    fn finds_relevant_records_by_content() {
        let records = vec![
            test_record("terminal/s1/1", "cargo build failed with linker error"),
            test_record("terminal/s1/2", "git push origin main"),
            test_record("terminal/s1/3", "error: linker not found"),
        ];
        let results = semantic_search(&records, "linker error", 10, |_| true);
        assert!(!results.is_empty());
        assert_eq!(results[0].record_ref, WorkRef::terminal_record("s1", 3));
    }

    #[test]
    fn respects_limit() {
        let records = vec![
            test_record("terminal/s1/1", "error error error"),
            test_record("terminal/s1/2", "error error"),
            test_record("terminal/s1/3", "warn error"),
        ];
        let results = semantic_search(&records, "error", 2, |_| true);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn empty_query_returns_nothing() {
        let records = vec![test_record("terminal/s1/1", "some content")];
        assert!(semantic_search(&records, "", 10, |_| true).is_empty());
    }

    #[test]
    fn title_match_boosts_score() {
        let records = vec![
            test_record("terminal/s1/1", "regular output"),
            test_record("terminal/s1/2", "error in output"),
        ];
        let results = semantic_search(&records, "title", 10, |_| true);
        assert!(!results.is_empty());
        assert_eq!(results[0].record_ref, WorkRef::terminal_record("s1", 1));
    }

    #[test]
    fn rare_terms_score_higher_than_common() {
        let records = vec![
            test_record("terminal/s1/1", "build build build deploy"),
            test_record("terminal/s1/2", "build build build"),
            test_record("terminal/s1/3", "build build"),
        ];
        let results = semantic_search(&records, "deploy", 10, |_| true);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].record_ref, WorkRef::terminal_record("s1", 1));
        let common_results = semantic_search(&records, "build", 10, |_| true);
        assert!(common_results[0].score < results[0].score * 3);
    }

    #[test]
    fn returns_matched_terms() {
        let records = vec![test_record("terminal/s1/1", "cargo build linker error")];
        let results = semantic_search(&records, "linker error", 10, |_| true);
        assert_eq!(results.len(), 1);
        assert!(results[0].matched_terms.contains(&"linker".to_string()));
        assert!(results[0].matched_terms.contains(&"error".to_string()));
    }

    fn test_record(ref_id: &str, combined: &str) -> WorkRecord {
        let work_ref: WorkRef = ref_id.parse().unwrap();
        let (session, index) = match &work_ref {
            WorkRef::Terminal {
                session,
                record_index,
                ..
            } => (session.clone(), *record_index),
            _ => unreachable!(),
        };
        WorkRecord {
            schema_version: 1,
            work_ref: work_ref.clone(),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session.clone(),
                canonical_id: Some(session.clone()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: Some(WorkStatus {
                outcome: WorkOutcome::Success,
                exit_code: Some(0),
            }),
            title: "title".to_string(),
            parts: vec![WorkPart {
                io: WorkPartIo::Output,
                kind: WorkPartKind::Command,
                index: index - 1,
                occurred_at: None,
                label: None,
                text: combined.to_string(),
                ansi: None,
                tags: Vec::new(),
            }],
        }
    }
}
