# Optional semantic search acceptance — 2026-10-13

Scope: source implementation on macOS, PostgreSQL 17 with pgvector 0.8.7, and isolated native Ollama 0.35.1 with `nomic-embed-text:v1.5` (768 dimensions). Only disposable synthetic summaries were used. No real conversation corpus was indexed.

## Retrieval evidence

Sixteen published synthetic summaries include twelve target records and four distractors. The committed `scripts/fixtures/semantic-search.json` and `scripts/semantic-eval.py` reproduce the comparison against explicitly selected local APIs. The first indexing pass stored all sixteen; a repeated pass selected zero unchanged records.

| Query set | Current keyword search | Hybrid search |
| --- | --- | --- |
| 12 paraphrases, expected record in top five | 0/12 | 12/12 |
| 12 paraphrases, expected record first | 0/12 | 11/12 |
| 12 exact terms, expected record first | 12/12 | 12/12 |
| 5 unrelated questions, empty results | 5/5 | 5/5 |

Hybrid paraphrase mean reciprocal rank at five was 0.9583. One payment question ranked a distractor first and its expected record second. The cosine similarity floor of 0.6 was selected using exploratory positive/unrelated questions; two of the five unrelated topics were added afterward as small holdouts. This is not an independent representative accuracy benchmark. The keyword baseline uses all-term `simple` full-text matching, which is particularly weak for verbose paraphrases.

Observed sequential local latency (milliseconds): paraphrases keyword p50 4.78/p95 10.02, hybrid p50 32.215/p95 607.95; exact queries keyword p50 4.855/p95 6.21, hybrid p50 28.155/p95 39.17. The larger hybrid tail includes model warmup. These tiny samples do not establish a production latency SLA or five-developer capacity.

## Verification

- Full Rust workspace with database tests included: 294 passed, zero failed or ignored. Formatting, warnings-as-errors Clippy and workspace build passed.
- Real loopback provider tests cover model/digest changes, malformed/count/dimension/zero/overflow vectors, input screening, redirects, deadlines and response size.
- Database and HTTP/MCP tests cover tenant/service/device isolation, current revision, correction, withdrawal, supersession, restricted-role access, revocation during inference, missing index and provider failure fallback.
- Ordinary PostgreSQL without the vector extension passed real HTTP, remote/stdio MCP, publication and supersession checks with semantic search disabled and with complete embedding configuration but no optional index. Missing-index fallback is asserted explicitly. The disposable Docker harness removes its own container/volume.
- Portal build and all 38 browser tests passed. Desktop 1440px and mobile 390px screenshots show the search method and remain within the viewport. Failed requests and legacy responses do not retain invented diagnostics.
- Eight Python script tests passed, including the explicit Gitleaks negative control.
- Unsuppressed Cargo audit: zero vulnerabilities, existing `rustls-pemfile` maintenance warning RUSTSEC-2025-0134. npm audit: zero vulnerabilities.
- Optional Compose overlay parses successfully. This is configuration validation, not a Linux Ollama deployment acceptance result.

## Boundaries

Semantic indexing is opt-in, uses a bounded operator pass and requires explicit scheduling for continuous updates. It embeds only selected published summary fields. It does not fill missing capture, establish authorship or certify why code was written. Existing contributor/file/revision evidence and change-confirmation rules remain authoritative.

Production OIDC/TLS, real-team long-duration capacity, representative private-corpus accuracy, the optional Linux model stack and signed distributions remain pending. No public release, merge or publication is implied by these local checks.
