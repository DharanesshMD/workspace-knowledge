# Plan: knowledge graph from real Cursor history + "who else is working on this" notices

Decisions (2026-10-02): summarizer `gemma4:e2b-mlx` (local Ollama, MLX); first run on **10 transcripts**; Developer B simulated by **splitting one project's history by time**; in-progress activity is **team-visible with opt-out**.

## Goal

1. Build cards and graph from real transcripts plus the code they touched, and measure whether a second developer's agent retrieves the right prior work.
2. New feature: when Developer B's agent works on code that Developer A changed, is changing, or tried to change, B's agent receives an ordered chain of notices such as:
   - "Developer A is working on `backend/ratelimit.rs` right now: adding tenant limits (started 2 h ago)."
   - "Developer A changed this last week: tenant-aware middleware, merged in `abc123`."
   - "Developer A tried the same approach earlier and abandoned it: Redis-based counters (open question: cross-region)."

## Privacy boundaries (unchanged)

- Raw transcripts stay on this machine. Only sanitized summaries, repo-relative file paths and commit IDs reach the (disposable, local) server.
- Activity notices carry the intent summary, files and timing, never prompts.
- Team-visible by default; `wk-agent activity pause` opts out. Personal or unapproved folders are never reported.
- The test database is disposable and deleted after the run; nothing is published anywhere.

## Phase 0 — reader fixes
- [x] Drop "user" records that are Cursor-injected system context (no `<user_query>`, only tag blocks such as `<hooks_context>`).
- [x] Mark each agent message's closing report as the outcome; keep narration separately so summaries focus on outcomes.
- [x] Extract file paths from transcript `tool_use` inputs (metadata only, scoped to the project) as file observations.
- [x] Tests for all three with synthetic fixtures.

## Phase 1 — history replay into a test knowledge base
- [x] Pick one project with a local Git repo and ≥10 transcripts; take the 10 most recent, time-ordered. First 7 = Developer A, last 3 = Developer B.
- [x] Link each A session to commits that touched its files in its time window (`git log`, read-only).
- [x] Summarize each A session locally with gemma4 (existing constrained-JSON path); cards carry files, commits and capture source.
- [x] Load into a disposable PostgreSQL + `wk-server --dev` as user `developer-a`; publish; graph builds automatically.

## Phase 2 — activity and overlap feature
- [x] Data: `activity` table (org, product, user, session, repo-relative files, services, intent summary, status, last_seen, expiry 8 h after last event), with RLS.
- [x] Collector: file read/edit hook events send heartbeats (paths only, approved scope only); `activity pause/resume`.
- [x] Server: `POST /v1/activity`, `POST /v1/overlaps` (files, services, query) → ordered chain: active work → recent changes (cards with file/commit evidence) → earlier attempts/abandoned or conflicting decisions. Excludes the caller's own work; deduplicated per session.
- [x] Delivery: MCP tool `knowledge.overlaps`; Claude Code hook context injection (prompt submit, before edit); Cursor `postToolUse` injection (verified in Cursor docs: no pre-prompt or pre-edit injection). Notices are labelled as reference data, not instructions. Live client certification still pending.
- [x] Tests: permission filtering, expiry, pause, dedupe, ordering, no raw text, prompt-injection text in summaries treated as data.

## Phase 3 — Developer B evaluation
- [x] For each B session: use its first request and the files it touched to query `overlaps` + `context`, as B's agent would before acting.
- [x] Ground truth: A sessions sharing files with the B session. Measure recall/precision of notices, latency, and card quality (faithfulness, specificity) by spot-check.
- [x] Show the exact notice chain B's agent would receive for each session, for you to judge.

## Phase 4 — wrap-up
- [x] Docs (architecture + setup), completion matrix, progress ledger.
- [x] Full test suite, Clippy, integration tests; delete the disposable database.
- [x] Review section below.

## Approval
- `gemma4:e2b-mlx` was already installed; no download needed. Project chosen: the local repository with the most commits and 44 transcripts (name kept out of the repo).

## Review

**Result.** Developer B's agent was told about all overlapping earlier work at edit time (recall 100%; precision 60% because two capped same-folder notices count as misses) and about A's in-progress work in both sessions where it applied. At prompt time, any-word matching found 33–67% versus 0% for the old all-words search. Lookups take 3–7 ms. Details: `docs/architecture/overlap-notices.md`.

**What the evaluation caught (all fixed):** MLX ignoring OpenAI-style JSON schemas; a summarizer prompt that withheld 6/10 summaries as "sensitive" for mere uncertainty; history cards that could never be published (partial coverage); Cursor system text read as a user message; confirmation noise in card lists; `tasks/todo.md` causing false overlaps; commit linking that attached the same commits to every card.

**Not done / limits:** live hook delivery in real Claude Code and Cursor sessions; a real multi-person team; more than one project; heartbeat intent is generic ("Editing files in …"); one faithfulness error from the 2B model remains visible in a card. Cursor cannot inject notices before a prompt or edit, only after the first edit.

**Verification:** 95 workspace tests, 11 PostgreSQL tests, HTTP + stdio/remote MCP integration (9 tools), Clippy, formatting, demo.
