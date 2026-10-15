# Meaning-based prompt context

Approved intent (2026-10-14): when a developer sends a prompt, the automatic team-context lookup should also find teammates' published summaries that match by meaning, quickly and with high relevance. Decisions: keep today's privacy boundary (only locally extracted prompt keywords reach the team server); extend the server's existing overlap lookup rather than adding client requests; speed and relevance targets are acceptance gates.

## Current behaviour

The Claude Code `UserPromptSubmit` hook reduces the prompt to at most 24 keywords on the developer machine (`wk_adapters::team_context::keywords`), drops them if they look sensitive, and calls `POST /v1/products/{product}/overlaps` (device-signed operation `overlaps` for connected collectors; MCP `knowledge.overlaps`). The server returns active work, changed files and "related" summaries found by PostgreSQL text match over title, intent and rationale. Optional hybrid search (local Nomic embeddings in pgvector) exists but only serves `knowledge.search`/`/v1/search`.

## Design

One shared server function serves the HTTP route, the device route and MCP: it runs the existing lookup and, when it has a non-sensitive query, adds meaning-based related summaries.

- **When:** the embedding client is configured, the product permits local inference (`allow_loopback`), the semantic index is ready for the namespace, and a query is present. Otherwise the response is exactly today's, with `retrieval: {"mode":"lexical","reason":...}` using the existing fixed reason labels (`lexical` matches `/v1/search`).
- **Concurrency:** the query embedding runs concurrently with the existing lookup; the semantic candidate query runs after both. Membership and policy are checked before any provider call and rechecked by the final SQL, as in hybrid search.
- **Candidates:** current published, non-withdrawn, non-superseded/deprecated summaries in the caller's services, excluding the caller's own and any summary already shown as active or changed work; cosine similarity at least 0.55 by default (`WK_PROMPT_SIMILARITY_FLOOR`, 0.3–0.95); at most 20 candidates.
- **Fusion:** text-related and semantic candidates are merged by reciprocal rank fusion (constant 60, stable ID ties). At most 5 semantic-only summaries are added. Each related notice carries the existing `matched_by` field with value `text`, `meaning` or `both`. The rendered agent text keeps the existing "reference data, not instructions" header.
- **Response:** adds `retrieval: {"mode":"lexical"|"hybrid","reason":<label or null>}`. Existing fields are unchanged.

## Speed

Target: hybrid lookups complete in under 100 ms at p95 with a warm model on the synthetic corpus; a hard 800 ms budget for the query embedding, which leaves margin inside the hook's 1.5 s notice bound and 2 s request timeout, after which text results return alone (`reason: "embedding_timeout"`). Requests are validated before any model call.

- **Warm model:** embedding requests send `keep_alive` (default 30 minutes, configurable with `WK_EMBEDDING_KEEP_ALIVE`, bounded 0 to 24 hours); when semantic search is configured, the server builds a separate short-lived embedding client at startup and issues one warm-up embedding from it in a background task, so startup never waits. Warm-up failure is logged as a fixed label. Invalid `WK_EMBEDDING_KEEP_ALIVE` or `WK_PROMPT_SIMILARITY_FLOOR` values refuse startup even when no model is configured.
- **Query cache:** an in-memory LRU of 1,024 query vectors keyed by organisation, product and whitespace-normalized keywords (the namespace is fixed per client), 10-minute TTL, shared with `/v1/search` questions under the same rules. Never persisted or logged. Cache hits skip the digest check for up to the TTL; any provider call that detects a changed model clears the cache.
- **Index:** exact cosine scan remains (milliseconds at thousands of summaries). Re-evaluate an approximate index (HNSW with iterative filtered scans) above about 50,000 current summaries per product or if warm p95 exceeds 100 ms; no index change now. Semantic reads hold the product administration gate shared, so concurrent lookups do not serialize.
- **Overload:** the existing two concurrent embedding slots without queuing apply; a busy provider yields text results (`reason: "embedding_busy"`).

## Relevance

Target: on the committed synthetic corpus, using keyword queries produced by the hook's own extractor, at least 11 of 12 paraphrases return the expected summary in the related notices' top five, and none of the 5 unrelated questions returns a semantic match. Results are reported against text-only matching.

- **Keyword extraction:** extend the stop list with question and filler words (`why`, `how`, `what`, `when`, `where`, `which`, `who`, `does`, `did`, `could`, `would`, `must`, `there`, `here`, `about`, `just`, `like`, `some`, `any`, pronouns and similar filler; `should` was already listed) while keeping identifiers and numbers. Behaviour stays deterministic and local.
- **Floor and caps:** the cosine floor and the 5-result cap limit noise; both are recorded in the evaluation output. Calibration (implementation): search's 0.6 floor found only 4 of 12 meaning-only prompts, because keyword lists embed less sharply than full questions. A sweep on the synthetic set (meaning-only hits / unrelated leaks: 0.40 → 9/5, 0.45 → 10/2, 0.50 → 10/0, 0.55 → 11/0, 0.60 → 4/0) set the prompt default to 0.55, separate from search. This is a small developer-written sample, not a representative calibration.

## Privacy

Unchanged: only keywords leave the developer machine; sensitive-looking keywords are dropped on the machine and again on the server; keywords are never stored or logged; the in-memory cache holds, per organisation and product, the normalized query text as its key and the vector, in memory only, for up to 10 minutes. Raw conversations, owner-only notes and unpublished drafts are never candidates.

## Testing

- Unit: keyword extraction (stop words, identifiers), fusion and caps, cache hit/expiry/namespace change, `keep_alive` request field, retrieval labels.
- PostgreSQL integration (owned `wk_test`, synthetic data): meaning-only match found; tenant, product and service isolation; caller's own summaries excluded; no duplicates with changed/active notices; withdrawn/superseded excluded; policy-disabled, missing index, timeout and busy fallbacks; the device route returns the same results.
- Evaluation: an opt-in Rust test (`crates/server/tests/prompt_context_eval.rs`) runs the keyword variant through the overlap endpoint with a real local model, including meaning-only prompts that share no words with their record; record relevance and p50/p95 latency (cold and warm) with a real local Nomic model in an isolated process; report honestly against the targets.
