# Implementation and release gates

This ledger distinguishes implemented code from external certification. No unchecked item may be marketed as complete universal capture or production certification.

| Plan area | Code path | Remaining acceptance gate |
|---|---|---|
| Contracts / folder approval | Strict cards, consent-bound paths, private state | Generated card/event/policy schemas published; native Windows junction/replacement certification remains |
| Private collector | Native keyring, authenticated loopback receiver, encrypted journal, fixed TTL, durable closure guards, rotating session scans | Live keyring and user-session startup on each OS; secure-open race hardening |
| Correlation / adapters | Opt-in Cursor 3.23.23 macOS bound-session recovery; Claude Code/Gemini partial visible hooks; explicit export | Live version-specific fixtures; no universal transcript coverage |
| Shared service | OIDC verification, administrative identity provisioning, membership policy, revisions, withdrawal, graph | Synthetic OIDC refresh/expiry/rotation checks passed; live-provider/load certification remains; non-owner RLS exercised in disposable PostgreSQL |
| Summaries | Local constrained JSON inference, input/output screening, encrypted private drafts, explicit reviewed submission | Real-model privacy corpus; policy-auto remains disabled pending that gate |
| Retrieval | Eight MCP tools over stdio and authenticated stateless Streamable HTTP (`/mcp/{product}`, protected-resource metadata), CLI search/context | Synthetic remote/stdio MCP passed; real-client OAuth login certification and load benchmark remain |
| Desktop capture | Instrumented local chat; explicit bound Enterprise text-message pull | Enterprise entitlement/live pagination/edit/deletion proof; central multiuser broker deployment |
| Evidence / provenance | Scoped graph, reviewer relationships (API and console), evidence freshness, Ed25519-signed evidence ingestion (`wk-evidence`) with replay guard and revocation | Production CI/Git connectors that sign real commit/test outcomes; live source-revision monitoring |
| Environment sharing | OpenBao KV v2 schema/import/run/export; value suppression; separate token | TLS import, rotation and revocation passed with synthetic data; Windows export ACL certification remains |
| Console | Hosted OIDC browser sign-in, cookie sessions/CSRF, scoped product selection, logout; separate owner-only summary store/edit/review-copy/delete, explicit local sync; existing search/review/graph/evidence/policy/schema views | Build and fifteen browser checks passed; real local-provider/PostgreSQL owner workflow passed in public/confidential client modes. Production-provider certification, team feed, device UI, physical expiry/restore integration and admin operations remain |
| Team overlap notices | Activity table with RLS, overlap lookup (active → changed → related), `knowledge.overlaps` MCP tool, Claude Code and Cursor hook delivery, activity pause/resume, Cursor history → cards → replay evaluation | Live hook certification in Claude Code and Cursor; multi-person team evaluation; larger history and stronger local model |
| Jev decision layer | Advisory batched routing/priority/conflict assessment (`/decisions`, `knowledge.assess`), bounded cache, review-by-default, offline calibration script | Disabled until an approved endpoint is configured; hosted vs private deployment decision, representative calibration corpus and measured speedup remain |
| Optional semantic retrieval | Lexical baseline retained | On-prem embedding endpoint and comparative evaluation required before enabling optional pgvector |
| Release | Source builds, container, startup templates, encrypted backup/restore with tombstone replay, SHA256SUMS + SPDX inventory; dependency audit tracked | Known rsa RUSTSEC-2023-0071 remains in the dependency tree; restore drill passed on disposable PostgreSQL; native Windows/Linux runners, release-signing keys and publication authorization remain |

Native Codex and Antigravity capture adapters are not implemented in this build. Their vendor hook availability is distinct from our adapter implementation and certification. They can consume the standard MCP interface; this alone does not collect their chats. These are external integration gaps, not completed features hidden behind success labels.

Cursor native slice evidence: [results](../reviews/2026-10-06-cursor-native-capture-results.md). Fixture/keyring/local-model handoff and independent review passed. [Bounded live acceptance](../reviews/2026-10-08-cursor-live-acceptance.md) now demonstrates reviewed publication, teammate MCP/overlap, restart continuity and in-flight revocation; production certification remains pending.
