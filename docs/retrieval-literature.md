# Retrieval Literature — Survey & Implementation Record

> Companion to `docs/retrieval-eval.md`. Summarizes the surveyed retrieval
> literature and records, per recommended technique, whether and how it was
> implemented in sivtr and its measured effect on the frozen eval snapshot
> (`eval-snapshot.json`, 1328 records / 22 labeled queries, k=5).
>
> Baseline (committed `f5a0536`, head+tail token windows): recall@5 0.276 /
> prec@5 0.673 / mrr 0.864 / **ndcg@5 0.734**.
> Goal: ndcg@5 strictly > 0.734 and recall@5 ≥ 0.276 on the unchanged snapshot.
>
> Current (passage retrieval over typed WorkParts, see table below):
> recall@5 **0.403** / prec@5 **0.773** / mrr **0.964** / **ndcg@5 0.861**.

## Survey summary

Sources verified during research (playwright browser, 2026-08-04):
Wikipedia "Okapi BM25", Wikipedia "Query expansion" (wikitext), Semantic
Scholar API (RRF), arXiv:2312.10997 (RAG survey), Tongji-KGLLM/RAG-Survey.

### 1. Term weighting & ranking functions (BM25 family)

- **BM25** — Robertson & Zaragoza, *Foundations and Trends in IR* 3(4), 2009.
  The probabilistic relevance framework: tf saturation (k1), length
  normalization (b), Robertson idf. sivtr's current index is this, self-contained.
- **BM25+** — Lv & Zhai, *CIKM 2011* ("Lower-Bounding Term Frequency
  Normalization"). Adds a δ lower bound (default 1.0) per matching query term so
  that a long document that *does* match is scored strictly above a short
  document that does not match at all. Directly relevant to sivtr's long
  agent-turn documents whose error text sits far into the conversation.
- **BM25F** — Robertson, Zaragoza & Taylor, *CIKM 2004* ("Simple BM25 Extension
  to Multiple Weighted Fields"). Documents are treated as multiple weighted
  fields (title/command/body) with independent weights and per-field length
  normalization. sivtr previously faked a title boost by replicating the title
  3× into the ranked text; a real fielded weight replaces that hack.
- **Pivoted length normalization** — Singhal, Buckley & Mitra, *SIGIR 1996*.
  Length-bias correction; the `b` parameter covers the same ground for BM25.
- **Passage retrieval / best-match scoring** — Callan, *SIGIR 1994* ("Passage-Level
  Evidence in Document Retrieval"). Score a document by its *best* passage
  rather than the whole document; passage-length normalization keeps a long
  turn's local evidence from being diluted by the rest of the turn. In sivtr
  the passages are not arbitrary text windows but the corpus's own structural
  unit: each typed `WorkPart` (command, output, error, assistant turn, …) is
  one passage, scored with per-passage-kind length normalization and aggregated
  by max-over-parts per record. This is the elegant fit for sivtr's data model —
  the middle of a conversation stops being "discarded"; it is simply *many
  short passages*.
- **LM smoothing** — Zhai & Lafferty, *SIGIR 2001*. Alternative ranking family;
  kept as background only (no behavioral change).

### 2. Tokenization (CJK-specific)

- CJK **character bigrams** (Lucene CJKAnalyzer style) are the standard
  dictionary-free tokenization for Chinese/Japanese; robust to out-of-vocabulary
  terms, error text, and code identifiers. sivtr uses overlapping bigrams —
  keep, do not switch to dictionary segmentation.
- **Case normalization**: Latin runs are lowercased at tokenization so
  `error[E` matches `Error[E0428]`. Verify with a dedicated unit test.

### 3. Query-side optimization (query understanding)

- **Query expansion (QE)** — Rocchio, *1971*; Carpineto & Romano survey, *ACM
  Computing Surveys* 44(1), 2012. Reformulate the query to improve recall.
- **Pseudo-relevance feedback (PRF) / relevance models (RM1/RM3)** — Buckley,
  *TREC-3*, 1995; Lavrenko & Croft, *SIGIR 2001*. Take the top-ranked documents
  as pseudo-relevant, harvest co-occurring terms, add them to the query. Fits
  sivtr's personal work-memory corpus: a debugging session's later fix terms
  co-occur with its error text.
- **Selective application / query difficulty** — Amati, Carpineto & Romano,
  *ECIR 2004* ("Query Difficulty, Robustness, and Selective Application of
  Query Expansion"). QE helps on average but damages hard queries where the
  pseudo-relevant set is noise. Gate expansion: suppress when query terms have
  high document frequency (common words — exactly the `share`/`status` df 300+
  case measured in the eval).
- **Positional/proximity relevance models** — Lv & Zhai, *SIGIR 2010*;
  Ermakova et al., *SAC 2016*. Deferred (needs positional indexes).
- **Embedding-based QE** — Kuzi et al., *CIKM 2016*. Deferred (model dependency).

### 4. Multi-signal fusion

- **Reciprocal Rank Fusion (RRF)** — Cormack, Clarke & Büttcher, *SIGIR 2009*
  (DOI 10.1145/1571941.1572114). Fuse rank lists via Σ 1/(k + rank); no score
  calibration needed. Standard in modern hybrid retrieval. sivtr fuses
  relevance + recency + command-field match.

### 5. Re-ranking

- Cross-encoders (monoT5: Nogueira & Cho, 2019) and late interaction (ColBERT:
  Khattab & Zaharia, *SIGIR 2020**) — **deferred (non-goal)**: neural models
  conflict with sivtr's local-first / no-heavy-dependencies rule and this
  sandbox has no model access.

### 6. Evaluation

- NDCG (Järvelin & Kekäläinen, *TOIS* 2002), MRR, recall@k — implemented in
  `sivtr eval`. TREC-style frozen snapshot + labeled queries; per-query failure
  analysis beats mean-only tracking. The eval is the arbiter for every change
  below.

## Implementation status table

| Technique | Status | Where | Measured effect (frozen eval, k=5) |
| --- | --- | --- | --- |
| BM25+ δ lower bound | **Implemented, live** | `crates/sivtr-core/src/search/bm25.rs` | Part of the shipped ranking; matching long docs score strictly above non-matching short docs (unit-tested) |
| Fielded title/command weighting (replaces title×3) | **Implemented, live** | `crates/sivtr-core/src/search/bm25.rs` | Title weight 3.0 for multi-token queries, 0.0 for single-token (the 0-gate keeps `grok`-style content queries from being overrun by terminal records whose title is the keyword) |
| **Passage retrieval over typed WorkParts** | **Implemented, live** | `crates/sivtr-core/src/search/bm25.rs` | Index unit = each typed `WorkPart` (Callan SIGIR'94 best-match), per-passage-kind length normalization, record score = max over parts. Replaces the head+tail token window. Uniform content-kind weights 1.0 (weights >1 drowned terminal records in content queries); Title/Command 3.0 gated by query length. k1 2.0. Measured jump: ndcg@5 0.828→**0.861**, recall@5 0.365→**0.403**; `kubectl` 0.339→1.0, `command not found` 0→1.0, `connection refused` 0.832→0.956 |
| Tokenizer case normalization | **Implemented, live** | `crates/sivtr-core/src/search/bm25.rs` | No metric change (already lowercased); now explicitly unit-tested: `error[E` shares the `error` token with `Error[E0428]` |
| PRF query expansion + difficulty gate | **Implemented, live (tuned)** | `crates/sivtr-core/src/search/expand.rs` | Tuned λ=0.25 / max_terms=4 / gate df<0.1. vs PRF-off: ndcg@5 0.830→0.828 (−0.2%), mrr equal 0.924; untuned (λ=0.5/6 terms) dropped to 0.803. Kept live for the expansion capability; can be disabled via `PRF_ENABLED` for +0.002 ndcg |
| RRF fusion (relevance + recency + command match) | **Implemented, unit-tested, disabled by default** | `crates/sivtr-core/src/search/fusion.rs` | Fusing relevance+recency+command: ndcg@5 0.596 (recency over the corpus let recent chatter outrank older genuine matches — recency is a known dataset-specific heuristic, see eval doc). Fusing relevance+command only: 0.813 < 0.830 no-fusion. Kept behind `FUSION_ENABLED` with the reason recorded |
| Learned/neural re-ranking (ColBERT, monoT5, cross-encoders) | Deferred — non-goal | — | No model access; conflicts with local-first rule |
| Vector/embedding index | Deferred — non-goal | — | No model access; roadmap "Semantic" track |
| Dictionary-based CJK segmentation | Deferred — non-goal | — | Bigram tokenization measured better for code/error text |

## Final measured numbers

Frozen snapshot (`eval-snapshot.json`, 1328 records / 22 labeled queries, k=5), `sivtr eval --snapshot eval-snapshot.json --sort relevance`:

| Metric | Baseline (f5a0536, head+tail) | Fielded + BM25+ + PRF (af1d613) | **Passage retrieval (current)** | Delta vs baseline |
| --- | --- | --- | --- | --- |
| recall@5 | 0.276 | 0.365 | **0.403** | +46% |
| prec@5 | 0.673 | 0.736 | **0.773** | +15% |
| mrr | 0.864 | 0.924 | **0.964** | +12% |
| ndcg@5 | 0.734 | 0.828 | **0.861** | +17% |

Deterministic across repeated runs (run twice, identical); captured output in `{SCRATCH}/eval-baseline.txt` and `{SCRATCH}/eval-after.txt`.

Per-query highlights of the passage model vs af1d613: the two residual weak
queries from the head+tail era are now perfect — `kubectl` 0.339→**1.0**,
`command not found` 0→**1.0**; `connection refused` 0.832→0.956. Residuals
remaining (all content-queries where the top-5 has few *relevant* records):
`provider grok` 0.470, `permission denied` 0.485, `docker` 0.616,
`rust compile error` 0.131 (56 relevant — its error terms now *do* sit in
bounded parts; the 0.131 is a precision problem: only 1 of the top-5 is
relevant, recall itself is capped by k=5 over 56 relevant records).
