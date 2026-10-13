# Optional private hybrid search implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:executing-plans task by task; implement inline with one fresh final reviewer.

**Goal:** Add measured private semantic discovery to reviewed team knowledge while preserving ordinary PostgreSQL/keyword operation.

**Architecture:** Optional pgvector schema, revision-safe bounded operator indexing, digest-checked loopback Ollama client and one shared HTTP/MCP hybrid retrieval path. Existing consent/publication/identity and file attribution stay authoritative.

**Tech Stack:** Existing Rust/Axum/SQLx/PostgreSQL/reqwest/React/Playwright; pgvector0.8.7; Ollama native embed API. No new Rust database/vector SDK.

**Spec:** `docs/superpowers/specs/2026-10-13-semantic-search-design.md`

## Global constraints

- Core schema versions1–17 and ordinary PostgreSQL remain unchanged; semantic migration/operator commands explicit.
- Published current summaries only; never raw/private/draft/environment/contact data; no text/vector/error-detail logging.
- Exact permission-filtered cosine search, initial Nomic similarity floor0.6; candidate limits20+20; result limit1–20; RRF60, stable ties; no new edit permissions.
- Loopback HTTP provider only, strict URL/model/digest/dimension bounds, no redirects/proxies, response cap1MiB, input cap16KiB, query3s/index30s, concurrency2; truncate:false.
- Namespace includes model/digest/dimensions/nomic-search-v1; wrong provider/model/dimensions never use old vectors.
- Operator indexing20/pass; failures retry after300s; runtime SELECT only. Optional feature disabled without explicit complete configuration.
- Preserve primary checkout/unrelated files; reuse clean attached candidate at674418e; no real data, merge, publication or global client changes.

## Review focus

1. Membership revocation or correction while embedding is in flight must exclude stale content at final read/write.
2. Missing optional schema/extension must fall back without breaking plain PostgreSQL deployments.
3. Close unauthorized neighbors must never affect returned names/snippets/ranks through a shared approximate index.
4. Long/malformed/local-provider responses and model replacements must stay bounded and explicit, not silently truncate or reuse incompatible vectors.
5. MCP/UI diagnostics must reflect actual retrieval; semantic similarity must not promote provenance or edit authorization.

### Task 1: Optional vector store and scoped candidates

**Files:** create `migrations/semantic/0001.sql`, `deploy/scripts/semantic-role.sql`, `crates/knowledge/src/semantic.rs`, `crates/knowledge/tests/hybrid_search.rs`; modify knowledge module declaration.

**Interfaces:** consume Knowledge pool/actor transactions and cards lifecycle; produce `Namespace { model,digest,dimensions }`, `Pending { id,revision,text }`, `semantic_migrate()`, `semantic_pending(org,product,namespace)`, `semantic_store(org,product,namespace,pending,vector)`, `semantic_failed(...)`, `semantic_candidates(actor,product,namespace,vector,limit)` and `search_hybrid(actor,product,query,limit,namespace,vector)`.

- [ ] Write tests using `common::setup` and restricted `common::runtime`: missing methods fail; current authorized card appears from semantic query with no keyword match; closer frontend/other-org vectors stay hidden; raw/personal/unpublished absent; correction/withdrawal reject index writes; mixed model/dimension rejected; runtime cannot write; revoke removes retrieval.
  ```rust
  assert!(f.k.search(&f.reader,"demo","duplicate money",5).await.unwrap().is_empty());
  let matches=runtime.search_hybrid(&f.reader,"demo","duplicate money",5,&ns,&[1.,0.,0.]).await.unwrap();
  assert_eq!(matches[0].id, reviewed_backend.id);
  assert_eq!(f.k.semantic_store(&f.alice.org,"demo",&ns,&old_source,&[1.,0.,0.]).await.unwrap(), false);
  ```
- [ ] Run `cargo test -p wk-knowledge --test hybrid_search -- --include-ignored`; observe missing-feature failure, then real behavior failure if necessary.
- [ ] Implement exact SQL filtering, RLS SELECT, owner guards and revision-trigger invalidation. Namespace validation and finite/nonzero vectors serialize through JSON text cast to vector; SQL parameters only. SQL indexer SELECT uses published eligibility and retry guards, no private table access. RRF deduplication uses ordered candidates and stable IDs.
- [ ] Verify all targeted tests against owned pgvector database; verify default core migrations still pass with ordinary PostgreSQL. Commit task1.

### Task 2: Bounded private embedding and integrated search

**Files:** create `crates/server/src/embeddings.rs`, `crates/server/src/semantic.rs`, `crates/server/tests/semantic.rs`; modify server lib/main/remote MCP and stdio MCP diagnostics.

**Interfaces:** consume Task1 Namespace/Pending/query APIs; produce `Embeddings::new(config)`, `from_env() -> Result<Option<Embeddings>,String>`, `embed_query(text)`, `embed_document(text)` and shared `search_for(...) -> {cards,retrieval}`. Configure server through explicit optional embedding argument, existing router constructors keep semantics disabled for fixtures. Operator CLI `semantic-migrate`, `semantic-index` consumes scoped stdin `{org,product}` and env config, outputs counts only.

- [ ] First write local HTTP fixture tests: model digest flips, wrong output/count/dimensions, zero vector, redirect and timeout all reject; loopback URL validation rejects remote/credential/query URLs. Input capture asserts only synthetic query/document text and no application credentials.
  ```rust
  assert!(Embeddings::new(config("https://example.org/api/embed")).is_err());
  assert!(client.embed_query("idempotency").await.is_err()); // changed fixture digest
  ```
- [ ] Run `cargo test -p wk-server embeddings`; observe missing-feature RED then implement bounded reqwest calls, two slots and end-to-end deadlines; stable fixed error labels. Prefix documents/queries consistently under model namespace.
- [ ] Write HTTP/remote MCP regressions: a paraphrase matches authorized indexed card; sensitive/unauthorized input never reaches fixture; stopped/changed provider/missing schema return lexical plus diagnostics; final membership revocation remains denied/excluded. Default router response remains compatible. Implement shared search and CLI, with authorization before provider calls.
- [ ] Run all server/knowledge tests with PostgreSQL; stdio MCP regression must preserve `retrieval` in previews/context. Commit task2.

### Task 3: UI, setup, real-model comparison and acceptance

**Files:** update `web/src/App.tsx`, `web/src/types.ts`, relevant browser tests, README/completion matrix; create `docs/architecture/semantic-search.md`, `deploy/compose/semantic.yaml`, `scripts/semantic-eval.py`, synthetic corpus and acceptance report; update CI with pgvector test database while retaining plain PostgreSQL compatibility check.

**Interfaces:** consume Task2 additive `{retrieval:{mode,reason}}`; UI shows completed search diagnostics without inventing unavailable status. Eval consumes running owned PostgreSQL/API and approved synthetic local embedding model.

- [ ] Write browser tests for hybrid/lexical fallback and older responses without diagnostics; observe RED before UI, then implement compact status message using existing styles, clear it on new search/product change/error.
- [ ] Document optional migration/grants, namespace/digest pinning, bounded index pass and reindex/model-change behavior; optional Compose override changes DB image only, no global model installation/volume migration. Explain shared host/network namespace requirement for loopback model.
- [ ] Run real local Nomic embeddings on synthetic paraphrase/exact/distractor corpus; compare keyword and hybrid ranking/recall and record latency. Never infer production/team/model certification from synthetic success. Verify corrections/withdrawals/revocations and provider outage through actual API/MCP.
- [ ] Run fmt/clippy/full Rust and PostgreSQL examples, Python, browser/build, unsuppressed Cargo/npm audits and redacted staged/history scans. Verify plain PostgreSQL core readiness separately. Inspect desktop/mobile UI once; bounded correction pass.
- [ ] One fresh final review against674418e..HEAD, regrade and fix Critical/Important with reproducing RED→GREEN tests once; defer Minor explicitly. Commit/push existing draft PR, confirm exact-head CI and retain limits. Archive private evidence, clean only owned processes/DB/plan scratch; no public release/deployment.
