# Meaning-based prompt context acceptance — 2026-10-14

Scope: source implementation on macOS, release build, PostgreSQL with pgvector and a real local Ollama `nomic-embed-text:v1.5` (768 dimensions) on one machine. Only the committed synthetic corpus `synthetic-feature-reuse-v1` (`scripts/fixtures/semantic-search.json`) was used. No real conversation corpus was indexed.

## What was measured

Prompt-time overlap lookups (`POST /v1/products/{product}/overlaps`) through the real HTTP router, with the hook's own keyword extractor (`wk_adapters::team_context::keywords`) turning each question into the query. Sixteen published records (twelve targets, four distractors) were indexed. Query sets:

- 12 paraphrases (the fixture's `query` per target record).
- 12 meaning-only prompts whose extracted keywords share no word with their record's title, intent or rationale (asserted by the test), so text matching alone cannot find them.
- 5 unanswerable questions.

A hit is the target card among the first five related notices. Hybrid runs use the default prompt similarity floor of 0.55; text-only runs use the same router without embeddings.

## Relevance

| Query set | Text only | Hybrid |
| --- | --- | --- |
| 12 paraphrases, target in top five | 12/12 (MRR@5 0.958) | 12/12 (MRR@5 1.0) |
| 12 meaning-only prompts, target in top five | 0/12 | 11/12 (MRR@5 about 0.48–0.53 across runs) |
| 5 unanswerable questions, meaning matches shown | — | 0 (highest similarity 0.48) |

The meaning-only miss was "Can a contractor read decisions about systems outside their team?" (record `scope`). Meaning-only targets are usually found but often not ranked first, which the MRR reflects.

## Similarity floor calibration

Search's 0.6 floor found only 4 of 12 meaning-only prompts: short keyword lists embed less sharply than full questions. The test replays fusion on card IDs at several floors (hits / unanswerable questions with any visible card at or above the floor):

| Floor | Meaning-only hits | Unanswerable leaks |
| --- | --- | --- |
| 0.40 | 9 | 5 |
| 0.45 | 10 | 2 |
| 0.50 | 10 | 0 |
| 0.55 | 11 | 0 |
| 0.60 | 4 | 0 |

The prompt default is 0.55 (`WK_PROMPT_SIMILARITY_FLOOR`, 0.3–0.95); search keeps 0.6. The margin between 0.48 (highest unanswerable similarity) and 0.55 is small and was chosen on this set, so it is a heuristic rather than a calibrated threshold.

## Latency

Sequential lookups in one process, milliseconds:

| Run | p50 | p95 |
| --- | --- | --- |
| Hybrid, cold (empty query cache, model already loaded by indexing) | 29.1 | 60.7 |
| Hybrid, warm (query cache filled) | 14.8 | 18.8 |
| Text only | 2.7 | 4.0 |

Warm p95 reached up to 38 ms in other runs while the machine was under load. All runs were below the 100 ms warm p95 target and well within the 800 ms embedding budget. A cold model load (first request after Ollama unloads the model) was not measured; the startup warm-up and 30-minute `keep_alive` exist to avoid it.

## Reproduce

Needs a disposable pgvector PostgreSQL `wk_test` database and a local Ollama with `nomic-embed-text:v1.5` pulled. Read the digest from `http://127.0.0.1:11434/api/tags`.

```sh
export WK_TEST_DATABASE_URL=postgres://...@127.0.0.1/wk_test
export WK_EVAL_EMBEDDING_URL=http://127.0.0.1:11434/api/embed
export WK_EVAL_EMBEDDING_MODEL=nomic-embed-text:v1.5
export WK_EVAL_EMBEDDING_DIGEST=<64-character digest>
./scripts/cargo test -p wk-server --release --test prompt_context_eval -- --include-ignored --nocapture
```

The test prints a JSON report (per-query ranks and `matched_by`, MRR, latency percentiles and the floor sweep) and fails if hybrid paraphrase hits fall below 11/12, meaning-only hits below 9, any unanswerable question leaks, or warm p95 reaches 100 ms. Set `WK_PROMPT_SIMILARITY_FLOOR` to evaluate another floor. Without the `WK_EVAL_*` variables the test is skipped.

## Limits

- Small synthetic sample written by the developers who tuned the floor on it; there is no held-out set. It is not evidence of accuracy on a real team's summaries, other languages or code-heavy text.
- One machine, sequential requests, a 16-record corpus. Not a team-scale, concurrent or production latency benchmark.
- Tie order between equally ranked cards depends on random card IDs, so MRR varies slightly between runs.
- Meaning matches are reference context, not evidence that two pieces of work are related; file and revision evidence remain authoritative.
