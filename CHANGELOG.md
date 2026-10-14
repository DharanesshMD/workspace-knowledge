# Changelog

All notable changes are recorded here.

## Unreleased

- Code of conduct, Dependabot updates, design-history docs layout and README title icon sizing.

## [v0.1.0-alpha.1](https://github.com/DharanesshMD/workspace-knowledge/releases/tag/v0.1.0-alpha.1) — 2026-10-14 (source-only prerelease)

### Included

- Explicit product-folder approval, private local capture boundaries and client/version capability reporting.
- Private owner summaries and reviewed, scoped team summaries in a hosted portal; raw captured messages are not exposed through team retrieval.
- Prominent owner device warnings for reported capture gaps, stale/missing reports and limited/unknown coverage, with explicit local recovery consequences; hook capture failures let development continue.
- Optional pgvector/Nomic hybrid search of published current summaries, with revision-safe indexing, scoped retrieval and explicit keyword fallback; ordinary PostgreSQL/keyword installations remain supported.
- Contributor attribution, revision-bound rationale and known impact references in advisory code-change briefs.
- Optional Claude Code edit confirmation, locally tested with version 2.1.296 Edit/Write against an isolated synthetic model and enrolled browser flow.
- OIDC portal/device workflows, authenticated API/MCP retrieval, device revocation and schema-17 backup/restore controls with synthetic/local acceptance.
- Source setup, a real CLI synthetic demo, Linux/macOS/Windows build CI, PostgreSQL/browser suites and a bounded local five-identity soak.

- Strict OIDC discovery/signature/claim regressions and positive/negative access-token-hash acceptance; no custom JWT protocol verifier.

### Known limits

- Linux Docker server + macOS developers is the alpha evaluation scope. Windows builds, universal capture, MultiEdit, cloud-backed hooks and Antigravity publication are not qualified.
- RUSTSEC-2023-0071 package removed using the pinned Convex AWS-LC OIDC fork and PostgreSQL-only SQLx dependencies; audits remain unsuppressed. One informational rustls-pemfile maintenance warning remains.
- The strict local Qwen model-sensitive corpus failed its contact-sensitive case. Application screening held the observed output; this is not model/privacy certification. Automatic publication remains disabled.
- Production OIDC/TLS, real-server/team and long-duration capacity testing, Apple signing/notarization and binary distribution acceptance remain pending.
- Code context does not cover undocumented decisions or guarantee safe edits; shell/manual/external writes and other clients bypass the optional hook.

See the [publication review](docs/reviews/2026-10-12-public-alpha.md), [local acceptance](docs/reviews/2026-10-12-v0.1-continuation.md) and [launch runbook](docs/operations/public-alpha.md). There are no downloadable signed binaries in this candidate.
