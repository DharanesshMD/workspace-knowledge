# Optional private semantic search

Hybrid search combines PostgreSQL full-text ranks with exact pgvector cosine matches over current published summaries. It helps discover recorded rationale when a question uses different words. It never establishes authorship, dependencies, implementation status or edit permission. Use existing file/revision code-change briefs for confirmed attribution and applicability.

The first provider is native Ollama `/api/embed` with Nomic search prefixes. Local acceptance uses Ollama0.35.1, `nomic-embed-text:v1.5` and768 dimensions. Only literal `127.0.0.1`/`::1` HTTP is allowed; the application/indexer and model must share a host or network namespace. No remote/cloud embedding endpoint, application-token forwarding, proxy or redirect is supported.

## Enable explicitly

1. Keep the normal core migrations and restricted application-role grants. Existing PostgreSQL installations work with semantic search disabled. Back up before adding/changing extensions or database images.
2. Install pgvector0.8.7 into the PostgreSQL17 server through the DBA. With separately protected migration-owner credentials, run `wk-server semantic-migrate`; then apply `deploy/scripts/semantic-role.sql` as owner. The runtime receives SELECT only. Core schema versions remain1–17; the optional migration has its own version guard and is not repeated during searches/startup.
3. Run the reviewed embedding model privately. Model downloads are operator actions, never application startup behavior:

   ```sh
   ollama pull nomic-embed-text:v1.5
   curl --fail http://127.0.0.1:11434/api/tags
   ```

   Inspect the `nomic-embed-text:v1.5` entry's digest and retain the approved model weights/configuration. The application verifies this declared digest before and after each computation; metadata checks are not independent cryptographic model certification.
4. Set all four values for both the application and the separate indexer. Keep database credentials in the existing protected deployment configuration; the HTTP process always uses its restricted runtime role.

   ```sh
   export WK_EMBEDDING_URL=http://127.0.0.1:11434/api/embed
   export WK_EMBEDDING_MODEL=nomic-embed-text:v1.5
   export WK_EMBEDDING_DIGEST=APPROVED_64_CHARACTER_LOWERCASE_HEX_DIGEST
   export WK_EMBEDDING_DIMENSIONS=768
   ```

   The digest above is a placeholder, not a working default. All four unset disables semantic search. Partial/malformed configuration refuses application startup. Changing a model tag/digest/dimension needs a reviewed reindex; incompatible namespaces are never mixed.
5. Using separately protected operator database credentials, pipe the exact organization/product scope into a bounded index pass:

   ```sh
   printf '%s\n' '{"org":"TEAM_ORG","product":"PRODUCT"}' | wk-server semantic-index
   ```

   The indexer emits selected/indexed/held/changed counts only. A pass considers at most20 records; successful unchanged records are not processed again. Held records back off300seconds. Repeat passes to catch newly published revisions, or schedule the same scoped operator command externally. Initial20-record passes are not an automatic publication/indexing SLA. Use one indexer/model configuration per product; stop old jobs before switching model configuration.

For Linux Docker, `deploy/compose/semantic.yaml` is an explicit override for `compose.yaml`. It pins the pgvector Postgres17 image and adds an Ollama service sharing the app's network namespace, with no model port published. Populate its private model volume with `docker compose ... exec embeddings ollama pull nomic-embed-text:v1.5`. Supply model configuration explicitly; it does not migrate existing volumes or download a model automatically. The optional `semantic-index` profile runs in a separate process/container with operator credentials while sharing only the app's network namespace. Provision its credential separately through the deployment's protected environment configuration:

```sh
printf '%s\n' '{"org":"TEAM_ORG","product":"PRODUCT"}' |
  docker compose -f deploy/compose/compose.yaml -f deploy/compose/semantic.yaml \
    --profile semantic-tools run --rm -T --no-deps semantic-index
```

Use the protected owner migration command and optional grant script before starting restricted hybrid queries. Production OIDC/TLS and target-server acceptance remain separate gates; local source acceptance does not certify this deployment template on your host.

## Privacy, revision and permission boundaries

Only published, nonwithdrawn, nonsuperseded/nondeprecated cards are selected. Index text contains title/intent/rationale/decisions/constraints/open questions; it does not read raw capture, personal summaries, drafts, environment values, contributor contacts or file contents. Existing screening and human review remain necessary; embeddings are sensitive derived data, not anonymization.

The namespace includes model name, digest, dimension and `nomic-search-v1` input format. A source revision and exact current text are rechecked after inference, so a correction/withdrawal cannot accept an old computation. An owner trigger deletes derived vectors on source changes without giving runtime DELETE permission. Reads also recheck current revisions and every required service permission. Ordinary role access is protected by RLS plus explicit SQL filters. Exact sorting avoids approximate global-index neighbors affecting permitted result ranks.

The indexer prunes obsolete namespaces for its selected product when reindexing. Logical deletion does not erase existing WAL or backups. Restored vectors cannot grant visibility to withdrawn or unauthorized cards; replay the existing tombstone/deletion journals during restore. Provider, database and host operators remain trusted.

## Retrieval and fallback

The product must permit local inference (`allow_loopback=true`). Disabled policy prevents query/index provider calls; indexing rechecks before each document and the storage transaction rejects results after policy removal. A query whose policy is removed during inference falls back to keyword results (or denies a concurrent authorization change). Already transmitted inference input cannot be recalled.

Search checks membership before any provider request and rechecks it in final queries. Candidate sets are at most20 keyword and20 vector results; reciprocal rank fusion uses constant60 and stable ID ties. Initial Nomic matches require cosine similarity at least0.6, a heuristic tested on the synthetic corpus rather than a truth/confidence threshold. Weaker useful matches may be omitted; broader team/model/language calibration remains necessary. Search does not send content to a generative model or modify knowledge relationships.

Requests have a3-second total semantic deadline,2 concurrent embedding operations per client/process, no queued overload and a1MiB response cap. Index calls have a30-second deadline. Input caps are1024 query bytes/16KiB document bytes, with `truncate:false`; inputs exceeding the provider's token window are held rather than silently shortened. Nomic's Ollama packaging may use a smaller context than its model-card maximum. Configure provider CPU/memory/queue limits separately; HTTP cancellation does not certify that provider computation stopped immediately.

Semantic reads (readiness checks, search and prompt-time candidates) hold the product administration gate in shared mode: concurrent lookups do not queue behind each other, while role, device and membership changes still exclude them.

HTTP search adds `retrieval.mode` (`hybrid`/`lexical`) and fixed fallback reasons. Missing current vectors/schema, provider downtime, changed model, invalid responses, timeout or capacity return keyword results. Database/authorization failures are not reported as semantic success. Remote and stdio MCP preserve diagnostics; minimal context budgets may omit the reason and set `omitted:true` while retaining mode. The portal reports the completed search method; absent diagnostics remain unknown, and failed/new requests clear old status. Owner/private stores and shared-summary completeness labels are unchanged.

Every semantic query, from search and from prompt-time overlap lookups alike, shares these rules:

- **Query cache:** up to 1,024 query vectors for 10 minutes, in process memory only (never persisted or logged). Each entry is keyed by organisation, product and the whitespace-normalized query text, and holds that text as its key plus the vector; one organisation's queries are never cache hits for another, so response timing does not reveal them. Least recently used entries are evicted. A cache hit skips the provider digest check for up to the 10-minute lifetime; any provider call that detects a changed model clears the cache.
- **Warm model:** every embedding request sends Ollama `keep_alive` (default `30m`; `WK_EMBEDDING_KEEP_ALIVE` takes an integer with `s`, `m` or `h`, at most 24 hours). At startup the server sends one warm-up embedding from a separate short-lived client in a background task; startup never waits, and a failure is logged as a fixed label.

## Prompt-time overlap notices

When semantic search is enabled, the [overlap lookup](overlap-notices.md) used by the prompt hook, MCP `knowledge.overlaps` and device-signed collectors also adds related summaries that match the query keywords by meaning. The same configuration, local-inference policy and index readiness apply, and `retrieval` reports `hybrid` or `lexical` with the same fixed reasons. The query cache and warm model above apply unchanged. The only differences from search:

- **Similarity floor:** 0.55 by default, below search's 0.6, because short keyword lists embed less sharply than full questions. `WK_PROMPT_SIMILARITY_FLOOR` overrides it (0.3–0.95). The default was chosen on the small synthetic set in the [prompt context acceptance report](../reviews/2026-10-14-semantic-prompt-context-acceptance.md); it is a heuristic, not a confidence threshold.
- **Budget:** the query embedding has 800 ms, running concurrently with the text lookup, so the result stays inside the hook's 1.5 s notice bound. On timeout the text notices return alone (`embedding_timeout`).

Invalid `WK_EMBEDDING_KEEP_ALIVE` or `WK_PROMPT_SIMILARITY_FLOOR` values refuse startup even when the embedding model is not configured.

Candidates are found by an exact cosine scan over current vectors for the caller's permitted services. On the synthetic corpus a warm lookup takes milliseconds. Re-evaluate an approximate index (HNSW with pgvector iterative filtered scans, so service filters do not starve results) when a product exceeds about 50,000 current summaries or warm prompt-time p95 exceeds 100 ms.

## Evaluate

`scripts/semantic-eval.py` compares two explicitly selected literal-loopback APIs against the committed synthetic corpus. Set `WK_SEMANTIC_EVAL_SYNTHETIC=1` and a synthetic `WK_ACCESS_TOKEN`; `--seed` explicitly creates/publishes proposed fixture records in the development `demo` product. Use owned disposable databases only. Index those records before evaluating a hybrid-enabled API. Keep a second keyword-only API for a fair current-baseline comparison. Outputs include top1/top5/ranking and observed local latency for paraphrases/exact terms, plus unrelated-question empty-result checks.

The current lexical baseline uses all-term `simple` full-text matching; verbose natural-language questions often fail that baseline. The comparison is not a claim against every optimized keyword system. Small synthetic results do not establish representative team accuracy, multilingual/code-heavy coverage, deployed-server capacity or guaranteed correctness. See the [dated semantic acceptance report](../reviews/2026-10-13-semantic-search-acceptance.md) for exact measured evidence.

Prompt-time relevance and latency are measured by the opt-in Rust test `crates/server/tests/prompt_context_eval.rs` against the same fixture and a real local model; see the [prompt context acceptance report](../reviews/2026-10-14-semantic-prompt-context-acceptance.md).

Primary references: [pgvector](https://github.com/pgvector/pgvector), [Ollama embed](https://docs.ollama.com/api/embed), [Ollama tags](https://docs.ollama.com/api/tags), [Nomic prefixes/model license](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5).
