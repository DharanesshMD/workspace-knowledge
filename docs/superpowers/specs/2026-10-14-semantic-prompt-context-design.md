# Meaning-based prompt context

Approved intent (2026-10-14): when a developer sends a prompt, the automatic team-context lookup should also find teammates' published summaries that match by meaning, quickly and with high relevance. Decisions: keep today's privacy boundary (only locally extracted prompt keywords reach the team server); extend the server's existing overlap lookup rather than adding client requests; speed and relevance targets are acceptance gates.

## Current behaviour

The Claude Code `UserPromptSubmit` hook reduces the prompt to at most 24 keywords on the developer machine (`wk_adapters::team_context::keywords`), drops them if they look sensitive, and calls `POST /v1/products/{product}/overlaps` (device-signed operation `overlaps` for connected collectors; MCP `knowledge.overlaps`). The server returns active work, changed files and "related" summaries found by PostgreSQL text match over title, intent and rationale. Optional hybrid search (local Nomic embeddings in pgvector) exists but only serves `knowledge.search`/`/v1/search`.

## Design

One shared server function serves the HTTP route, the device route and MCP: it runs the existing lookup and, when it has a non-sensitive query, adds meaning-based related summaries.

- **When:** the embedding client is configured, the product permits local inference (`allow_loopback`), the semantic index is ready for the namespace, and a query is present. Otherwise the response is exactly today's, with `retrieval: {"mode":"text","reason":...}` using the existing fixed reason labels.
- **Concurrency:** the query embedding runs concurrently with the existing lookup; the semantic candidate query runs after both. Membership and policy are checked before any provider call and rechecked by the final SQL, as in hybrid search.
- **Candidates:** current published, non-withdrawn, non-superseded/deprecated summaries in the caller's services, excluding the caller's own and any summary already in the notices; cosine similarity at least 0.6; at most 20 candidates.
- **Fusion:** text-related and semantic candidates are merged by reciprocal rank fusion (constant 60, stable ID ties). At most 5 semantic-only summaries are added. Each related notice carries `match: "text" | "meaning" | "both"`. The rendered agent text keeps the existing "reference data, not instructions" header.
- **Response:** adds `retrieval: {"mode":"text"|"hybrid","reason":<label or null>}`. Existing fields are unchanged.

## Speed

Target: hybrid lookups complete in under 100 ms at p95 with a warm model on the synthetic corpus; a hard 1.5 s budget for the semantic part, after which text results return alone (`reason: "embedding_timeout"`).

- **Warm model:** embedding requests send `keep_alive` (default 30 minutes, configurable with `WK_EMBEDDING_KEEP_ALIVE`, bounded 0 to 24 hours); the server issues one warm-up embedding at startup when semantic search is configured. Warm-up failure is logged as a fixed label and never blocks startup.
- **Query cache:** an in-memory LRU of 1,024 query vectors keyed by (namespace, normalized keywords), 10-minute TTL. Never persisted or logged. Cleared when the namespace changes.
- **Index:** exact cosine scan remains (milliseconds at thousands of summaries). The docs state the corpus size at which an approximate index should be evaluated; no index change now.
- **Overload:** the existing two concurrent embedding slots without queuing apply; a busy provider yields text results (`reason: "embedding_busy"`).

## Relevance

Target: on the committed synthetic corpus, using keyword queries produced by the hook's own extractor, at least 11 of 12 paraphrases return the expected summary in the related notices' top five, and none of the 5 unrelated questions returns a semantic match. Results are reported against text-only matching.

- **Keyword extraction:** extend the stop list with question and filler words (`why`, `how`, `what`, `when`, `where`, `which`, `does`, `did`, `should`, `could`, `would`, `must`, `there`, `here`, `about`, `just`, `like`, `some`, `any`) while keeping identifiers and numbers. Behaviour stays deterministic and local.
- **Floor and caps:** the 0.6 cosine floor and the 5-result cap limit noise; both are recorded in the evaluation output.

## Privacy

Unchanged: only keywords leave the developer machine; sensitive-looking keywords are dropped on the machine and again on the server; keywords are never stored or logged; the in-memory cache holds vectors only. Raw conversations, owner-only notes and unpublished drafts are never candidates.

## Testing

- Unit: keyword extraction (stop words, identifiers), fusion and caps, cache hit/expiry/namespace change, `keep_alive` request field, retrieval labels.
- PostgreSQL integration (owned `wk_test`, synthetic data): meaning-only match found; tenant, product and service isolation; caller's own summaries excluded; no duplicates with changed/active notices; withdrawn/superseded excluded; policy-disabled, missing index, timeout and busy fallbacks; the device route returns the same results.
- Evaluation: extend `scripts/semantic-eval.py` with the keyword variant and overlap endpoint; record relevance and p50/p95 latency (cold and warm) with a real local Nomic model in an isolated process; report honestly against the targets.
