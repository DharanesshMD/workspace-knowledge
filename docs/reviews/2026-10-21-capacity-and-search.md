# Capacity and search performance (2026-10-21)

**Status: root cause found, fixed and re-measured on a local Docker VM with synthetic content. Read the [limits](#limits) before quoting a number.** This is adoption-friction item 3. It explains why the hosted server's two-core database hit its CPU cap under search/MCP load, records the fix, and gives a measured operating envelope for a *small* team. It is not a forecast for real content, larger teams or a provisioned server.

## Question

The five-identity synthetic pilot showed search and MCP p95 near 0.9 s (p50 about 15 ms) while the 2-core PostgreSQL sat at its CPU cap. What is the database doing, is it cheap to fix, and how much load does the fixed server carry?

## Root cause

Profiled with `pg_stat_statements` in the pilot's disposable PostgreSQL (`hosted-pilot.py --statement-stats`), then `EXPLAIN (ANALYZE, BUFFERS)` as the restricted application role against a local disposable database with the same card shape.

1. **One statement is the database load.** In the unmodified five-identity pilot the lexical card search (the statement behind both `/v1/search` and `knowledge.search`) took 157 s of the 168 s of execution time in the 15 most expensive statements (94%; mean 145 ms, max 1.36 s, 2.0 million shared-buffer hits). Every other statement had a mean below 5 ms.
2. **It tokenized every candidate card on every request.** The query computes `to_tsvector(...)` over title, intent, rationale, decisions, constraints and open questions of each card in scope, at about 1.2 to 2.2 ms for a 10 KiB card. It never used the GIN index `cards_search`, even though the index expression matched exactly:
   - With the restricted role, **row-level security keeps PostgreSQL from using a GIN index for `@@`**: the operator's function is not marked leakproof, so it may not be applied before the security policy. Forcing `enable_seqscan`/`enable_indexscan` off did not change that; the plan was a bitmap scan on `cards_scope` with the tokenizing filter (6.9 s on 6,000 cards).
   - Even without row-level security the planner picked a sequential scan on small tables, because it costs `to_tsvector` as one unit.
   - For a developer limited to some services, the planner also switched to a sequential scan plus a sort by id, tokenizing every card in scope instead of stopping after the first eight matches. That is why p50 was low and p95 high: a database that is already saturated makes the unlucky requests queue.
3. **Numbers (local, restricted role, synthetic 10 KiB cards, one product):**

   | Query | Cards | Before | After (stored vector) |
   |---|---|---|---|
   | Term in every card (early exit) | 600 | 9 to 13 ms | 0.7 ms |
   | Term in every card | 6,000 | 16 to 33 ms | 0.5 to 1.1 ms |
   | Term in no card | 600 | 1,333 ms | 9 ms |
   | Term in no card | 6,000 | 9,816 ms | 264 ms |
   | Similar-work lookup (prompt context), term in no card | 3,021 | 3,289 ms | 49 ms |

   The after column is still linear in the cards in scope (about 0.04 ms per card for the vector match, 0.14 ms per card measured through the application with realistic vocabulary), because the GIN index cannot be used under row-level security. It is now cheap enough for thousands of cards, not for hundreds of thousands.

The same pattern existed in two more places: the hybrid retrieval's lexical leg and the "similar work elsewhere" step of prompt context (`overlaps`, which runs on prompts). The second was measured separately (table above) and fixed the same way.

## A second cause, found by the soak

The first 30-minute soak (50 identities, search fix only) ran cleanly for 25 minutes and then collapsed (table in the results). `pg_stat_statements` pointed at publication: `INSERT INTO graph_edges ... SELECT ... FROM graph_nodes WHERE org, product, card_id` rose from 2.3 ms per call in a one-minute run to 87 ms, and `SELECT count(*) FROM graph_nodes WHERE ... card_id` from 0.45 ms to 22 ms. `graph_nodes` has a foreign key to `cards` but no index on the referencing columns, so every publication scanned the whole table (28 ms plus a 29 ms JIT compile at 24,000 nodes in `EXPLAIN ANALYZE`). Writes also take a per-product gate, so slow publications queued everyone behind them and the 5 s pool and 3 s lock limits turned that into 503s. The same lookup runs when a card is withdrawn or deleted.

## Fix

Migration `0018.sql` (schema version 18):

- Adds `cards.search_vector`, a **stored generated column** over the same six fields. It follows every later edit automatically, adds no application write path, and is not part of row-level-security policy: all access controls, service scoping and privacy screening are unchanged.
- Drops `cards_search`, which the application role could never use and which made every insert tokenize the card a second time. **No GIN index replaces it**: the restricted role could not use one (see above). If a deployment ever needs to search hundreds of thousands of cards, that needs a different design (for example a leakproof matching function installed by a superuser, or a separate search store), decided deliberately.
- Adds `graph_nodes_card` on `graph_nodes(org, product, card_id)`, the index the foreign key never had (second cause above).
- `search`, `search_hybrid` and `overlaps` now match `search_vector`. One deliberate behaviour change: the similar-work step used to match only title, intent and rationale; it now also matches decisions, constraints and open questions, as ordinary search always did.

Also required by the schema bump: readiness, `backup.sh`, `restore.sh` and the operations guides now require migrations 1–18. **Version 2 snapshots taken at schema 17 are refused by the schema-18 restore script and need the documented offline upgrade** (migrate in an isolated database with the matching release, then take a new snapshot). `hosted-restore-drill.py` passed all 14 checks at schema 18. The stored vector costs space: with about 700 distinct words per card it is about 4 KB per card (measured on a scratch table of 2,000 such cards), about the size of the compressed body. The index that was dropped was never measured, and the pilot's 25-word text understates both, so plan disk from your own content.

Tests (`crates/knowledge/tests/search_cost.rs`, restricted application role, 600 cards of about 10 KiB): each failed on the old code and passes now. Before the fix, search over 400 such cards for an absent term took 450 ms and the similar-work lookup 457 to 1,051 ms against a 250 ms budget (the budget is deliberately loose); after it, search takes about 57 ms. A third test seeds three graph nodes per card and fails if publication's graph lookup is a sequential scan. The search test also checks that edits to each of the six searchable fields are found. The existing access, team, activity, hybrid-search and personal tests pass unchanged on schema 18, including upgrading a schema-17 database that already held cards.

## Method

- **Harness.** `scripts/hosted-pilot.py`: Linux arm64 containers, app capped at 2 CPU / 1 GiB, PostgreSQL 17 at 2 CPU / 2 GiB with 50 connections, 256 MiB shared buffers, 4 MiB work memory; real HTTP, signed collector requests and MCP reads; five to 100 synthetic upstream-signed identities; each identity runs a closed loop of 10 requests (policy, two idempotent personal writes, a shared card, its publication by the reviewer, owner list, team context, search, MCP search, review queue) with a pause between cycles. Added for this work: `--identities N`, `--statement-stats` (loads `pg_stat_statements`; statement shapes and timings only, no content), `--rare-term-search` / `--rare-term-every N` (adds a search for a term no card contains, the worst case for lexical search, reported as `search_rare`), `--tolerate-errors` (count failures and keep running, for soaks), a per-minute timeline, and `--server-binary` / `--deploy-scripts` so an older schema can be compared. Output is timings, counts and bytes only.
- **Before and after.** Both builds come from `docker build --target pilot`: *before* is the tree at `af67ff2`, *after* the same tree plus this change. Same host, same Docker VM, runs one at a time.
- **Content.** Each card body is 10 KiB of text drawn from 25 words. The search term "middleware" is in every card; "absentlexeme" is in none.

## Results

All rows are 60 s steady load, closed loop, 50 ms pause between cycles (each identity sends a cycle as soon as the previous one finishes, so these are saturation tests, not forecasts of real use), with the extra worst-case search, so each cycle is 11 requests. Latencies are milliseconds; "writes" is the shared-card write (`collector_cards`). The "before" build is `af67ff2`. One failed request per run is the planned application restart's readiness probe.

| Identities | Build | Req/s | Search p50 / p95 / max | MCP search p95 | Absent-term search p95 | Writes p95 | DB CPU mean / peak (cap 200) | DB throttled | Failed |
|---|---|---|---|---|---|---|---|---|---|
| 5 | before | 60 | 27 / **632** / 894 | 625 | 636 | 86 | 181 / 204 | 95 s | 0 |
| 5 | **after** | 193 | 16 / **34** / 53 | 36 | 32 | 34 | 112 / 160 | 0.4 s | 0 |
| 20 | before | 55 | 371 / **1,891** / 2,711 | 2,086 | 1,991 | 984 | 193 / 208 | 346 s | 0 |
| 20 | **after** | 233 | 40 / **117** / 273 | 128 | 161 | 186 | 176 / 203 | 59 s | 0 |
| 50 | before | not completed | | | | | | | write requests failed with 503 and the run aborted |
| 50 | **after** | 200 | 178 / **360** / 677 | 427 | 362 | 582 | 178 / 205 | 126 s | 0 |

The unmodified pilot mix (no absent-term search) at five identities, same before build: 91 req/s, search p95 890 ms, MCP p95 943 ms, database 154% mean and 203% peak, 88 s throttled. The 50-identity "after" row is one run; an earlier run of the build with only the search fix measured 305 req/s there, so expect roughly 30% run-to-run spread at saturation.

Paced, 60 s, one cycle per identity per second:

| Identities | Build | Req/s | Search p50 / p95 / max | Absent-term p95 | Writes p95 | DB CPU mean / peak | DB throttled | Pool |
|---|---|---|---|---|---|---|---|---|
| 20 | **after** | 168 | 21 / **54** / 171 | 53 | 110 | 98 / 167 | 8 s | 10 of 10 in use at peak |
| 50 | before | 53 | 1,589 / **3,503** / 5,381 | 3,585 | 2,765 | 177 / 210 | 473 s | 10 of 10 |
| 50 | **after** | 191 | 118 / **305** / 476 | 331 | 480 | 164 / 208 | 115 s | 10 of 10 |

Where the database time goes after the fix (50 identities, saturated): the card search is still the largest statement (about 54%, mean 36 ms, because each cycle includes an absent-term search over a growing table), then the per-owner credential advisory lock (33%, which is waiting rather than CPU, caused by saturation). In the five-identity run, where the database has headroom, the search is 63% of statement time at 9.4 ms mean but nothing is throttled.

### Thirty-minute soak

50 identities, one cycle per identity every 10 s (about 4.9 cycles/s, about 49 requests/s, a worst-case search every 20th cycle), 1,810 s steady load, then a 1,000-summary catch-up burst, an application restart and a retention and backup pass. Cards grew from 50 to 9,707; the database from 10.7 to 154.8 MiB; WAL 234 MiB; an encrypted snapshot is 38.3 MiB.

| | Search | MCP search | Absent-term search | Shared-card write | Private write | Publish |
|---|---|---|---|---|---|---|
| Requests | 8,657 | 8,657 | 400 | 9,657 | 18,314 | 9,657 |
| p50 (ms) | 8.7 | 8.6 | 56.6 | 21.0 | 17.1 | 12.2 |
| p95 (ms) | 70.6 | 72.8 | 330.0 | 470.3 | 316.0 | 189.7 |
| max (ms) | 3,292 | 3,070 | 720 | 3,107 | 2,491 | 2,270 |

- **89,970 requests, 1 failed**: the planned restart's readiness probe. All 50 owners reconnected and found their queued summaries after the restart.
- **Database** CPU averaged 41% of its 200% cap (peak 205%, 215 s throttled in total, in short bursts); memory peaked at 531 MiB of 2 GiB. **Application** CPU averaged 18% (peak 106%), memory 25 MiB of 1 GiB. The 10-connection pool was fully busy at peak (8 lock waits seen), with no acquire timeouts.
- **Per-minute p95** stayed at 15 to 102 ms for search and 21 to 350 ms for writes in 28 of the 30 steady minutes. One stall at about minute 11 (about 7% of the run, with minutes 12 and 13 recovering) took search p95 to 1.7 s and write p95 to 1.6 s, with throughput down about a quarter for that minute; a repeat of the soak with PostgreSQL checkpoint and autovacuum logging (`--database-events`) did not reproduce it: 91,340 requests, none failed, search p95 70 ms, and only a milder slow stretch at minutes 8 to 11 (search p95 150 to 384 ms, writes p95 under 120 ms). The log shows checkpoints every five minutes spread over 270 s (3,300 to 4,600 buffers each, none triggered by WAL volume) and autovacuum or analyze of one table or another nearly every minute, including `cards` and its TOAST tables at minutes 8 to 10. That is consistent with background maintenance competing with a two-core database but does not prove it; treat the stall as an unexplained transient of a minute or two. The catch-up burst of 1,000 writes at the end ran at p95 1.1 to 1.2 s.
- Absent-term search grew with the table, noisily: p95 28 ms at 1,200 to 2,000 cards, 50 to 460 ms between 3,000 and 8,000 cards (50 samples per minute).
- The only table still read sequentially at volume is `cards` (42 million rows read in total), which is the search described above.

The soak before the graph index (search fix only, same load, saved as the first soak) ran cleanly for 25 minutes and then collapsed at about 7,000 cards: writes p95 1.9 s, 390 requests failed with 503 (182 publications, 131 card writes, 59 review-queue reads), 38 of 50 owners found their queued summaries after the restart, and throughput fell from about 2,900 to under 900 requests per minute.

## Operating envelope (small team only)

On this local Docker VM (app 2 CPU, database 2 CPU, 50-connection database, 10-connection pool), synthetic 10 KiB cards, no real model, no production identity provider, no TLS proxy:

- **Supported with margin**: 20 identities each completing a full capture-and-read cycle every second (168 req/s). Search p95 54 ms, writes p95 110 ms, database at about half its CPU cap, no failures. This is far above what 20 developers produce.
- **Supported over time**: 50 identities at one cycle per 10 s for 30 minutes (49 req/s, about 9,700 cards): search p95 71 ms, shared-card writes p95 470 ms, one 1-minute stall (p95 about 1.7 s), no failures, database at 41% of its cap on average.
- **Limit found**: 50 identities each cycling every second saturates the database (cap reached, writes p95 about 0.5 s, search p95 about 0.3 s, still no failures); 50 identities at saturation without pacing is the same. The write path (one writer per product at a time, plus the 10-connection pool) is what queues first.
- **Cards in scope drive search cost**: matching searches stop after the first few hits, but a term no card contains is a linear scan of the cards in scope for the person asking (local measurement about 0.04 to 0.14 ms per card; in the soak p95 was 28 ms at 1,200 to 2,000 cards and 50 to 460 ms at 3,000 to 8,000). That is fine for a small team for a long time; it is not indexed and would need a design change for hundreds of thousands of cards.
- **Disk**: about 16 KB of database and 20 KB of WAL per captured cycle (a private note, a shared card, its revision and graph rows) on this compressible text; real prose is larger. A 30-minute soak at 4.9 cycles/s therefore wrote 235 MiB of WAL.

This is not a forecast. Real summaries, model-driven capture, a real identity provider and proxy, a slower disk, other clients on the same server and larger teams will move every number.

## Limits

- Local Docker VM on one Apple-silicon Mac, Linux arm64 containers; not a provisioned remote server. Runs were sequential but the host was not otherwise idle.
- Synthetic content: 10 KiB of text from 25 words is very compressible and has few distinct words, so storage, search-vector size and per-card match cost are understated. The absent-term probe is the worst case for search; real queries mostly hit.
- A closed-loop load with identical behaviour per identity is not a human usage pattern. No real model, native capture, browser, TLS proxy or production OIDC. JWTs come from a synthetic signer.
- One run per configuration except where stated; saturated runs vary by about 30%. The 50-identity "before" run did not complete, so no before figure exists there.
- The profile covers search/MCP and the publish write path. Other read paths (team context, owner listing, review queue) cost 3 to 11 ms mean in the soak and were not tuned. The per-owner collector-credential advisory lock and the single per-product writer gate are design choices that bound write concurrency; they were not changed.
- The real-team pilot on a provisioned server remains an owner-scoped open gate in [production readiness](../operations/production-readiness.md).

## Reproduce

```sh
docker build --target pilot -f deploy/Dockerfile -t wk-hosted-pilot:capacity .
./scripts/cargo build -p wk-server
python3 scripts/hosted-pilot.py --image wk-hosted-pilot:capacity --duration 60 --identities 20 \
  --cycle-pause-ms 1000 --statement-stats --rare-term-search --output /private/pilot-20.json
python3 scripts/hosted-pilot.py --image wk-hosted-pilot:capacity --duration 1800 --identities 50 \
  --cycle-pause-ms 10000 --rare-term-every 20 --tolerate-errors --statement-stats --output /private/soak-50.json
```

The repository's `.dockerignore` does not exclude `.claude/`, which can make the Docker build context many gigabytes; build from a clean export (`git archive HEAD`) if that applies to your checkout. To compare with an older build, pass `--server-binary` (a `wk-server` of the same schema) and `--deploy-scripts` (that tree's `deploy/scripts`).
