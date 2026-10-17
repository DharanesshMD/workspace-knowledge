# Changelog

All notable changes are recorded here.

## Unreleased

Nothing yet.

## [v0.1.0-alpha.2](https://github.com/DharanesshMD/workspace-knowledge/releases/tag/v0.1.0-alpha.2) — 2026-10-15 (source-only prerelease)

First published prerelease. It supersedes v0.1.0-alpha.1, which was tagged on 2026-10-14 but never published and predates the rustls security update.

### Included

- Code of conduct, Dependabot updates, design-history docs layout and README title icon sizing.
- Explicit product-folder approval, private local capture boundaries and client/version capability reporting.
- Private owner summaries and reviewed, scoped team summaries in a hosted portal; raw captured messages are not exposed through team retrieval.
- Prominent owner device warnings for reported capture gaps, stale/missing reports and limited/unknown coverage, with explicit local recovery consequences; hook capture failures let development continue.
- Optional pgvector/Nomic hybrid search of published current summaries, with revision-safe indexing, scoped retrieval and explicit keyword fallback; ordinary PostgreSQL/keyword installations remain supported.
- With semantic search enabled, prompt-time overlap notices (HTTP, MCP and device-signed) add up to five summaries matched by meaning to the prompt keywords, labelled `matched_by` `meaning`/`both` with a `retrieval` mode and fallback reason; query vectors for search and overlaps are cached in memory per organisation and product, the model is kept loaded and warmed at startup, and device-signed lookups now honour the device's service limits for changed and text-related notices too (previously only activity did), drop possibly sensitive query text, and return `query_ignored` and `retrieval`. See the [acceptance report](docs/reviews/2026-10-14-semantic-prompt-context-acceptance.md).
- Contributor attribution, revision-bound rationale and known impact references in advisory code-change briefs.
- Optional Claude Code edit confirmation, locally tested with version 2.1.296 Edit/Write against an isolated synthetic model and enrolled browser flow.
- OIDC portal/device workflows, authenticated API/MCP retrieval, device revocation and schema-17 backup/restore controls with synthetic/local acceptance.
- Source setup, a real CLI synthetic demo, Linux/macOS/Windows build CI, PostgreSQL/browser suites and a bounded local five-identity soak.

- Strict OIDC discovery/signature/claim regressions and positive/negative access-token-hash acceptance; no custom JWT protocol verifier.
- `wk-agent memory scan|import`: private drafts from `CLAUDE.md`, `AGENTS.md` and explicitly approved Claude Code project notes, with re-import that skips unchanged sections and links updates to submitted summaries.
- `wk agent-history --client claude-code|codex|copilot|cursor list|show|export`: read-only readers for Claude Code, Codex and VS Code Copilot Chat sessions (Copilot `.json` and `.jsonl` logs) next to Cursor, with list output limited to IDs, titles and counts, masked `show`, and export to private `wk-agent import` files; `wk cursor-history` remains as an alias. Client-injected prompt context is dropped and counted in `dropped_injected`.
- `wk-agent import` records one private journal session per part (at most 96 KiB and 256 messages, whole exchanges) so `process` drafts one summary per part instead of one per reply; this also improves Cursor imports. Limits rise to 5,000 messages and 8 MiB per export; re-importing a continued session drafts every part again.
- `wk-agent memory scan|import` also reads GitHub Copilot instruction files (`.github/copilot-instructions.md`, `.github/instructions/*.instructions.md`). Frontmatter hints are now unquoted, which also affects single-quoted and unbalanced double-quoted hints of existing notes, so such notes redraft once.
- History titles, `show` and exports now mask credentials by name as well as long tokens: assignments of any length (`password=…`, `accessToken: …`, quoted JSON values), URL passwords and bare tokens, `curl -u` and long credential flag values, `Cookie` headers and `sig`/`signature` query parameters, and indented YAML blocks under credential keys. See the [setup guide](docs/adapters/setup.md#import-past-agent-sessions) for what is not masked.
- `wk cursor-history export` and `wk agent-history export` now refuse, before writing anything, an existing `--out` directory that is accessible to group or others (Unix), `..` in `--out`, a `--session` named twice and file names that differ only in case; files written by a failed run are removed. Export file names for unusual session IDs get a hash suffix.
- `wk cursor-history list` and `wk agent-history list` titles are now masked, limited to the first line and clipped to 80 characters (masking happens before clipping).
- Setup guide for VS Code and GitHub Copilot MCP (`.vscode/mcp.json` for `wk-mcp` and the remote endpoint); Copilot gets no automatic prompt-time notices yet.

### Known limits

- Linux Docker server + macOS developers is the alpha evaluation scope. Windows builds, universal capture, MultiEdit, cloud-backed hooks and Antigravity publication are not qualified.
- rustls updated to 0.23.46 for RUSTSEC-2026-0346 (medium) and RUSTSEC-2026-0347/0348/0349 (low), published 2026-10-14.
- RUSTSEC-2023-0071 package removed using the pinned Convex AWS-LC OIDC fork and PostgreSQL-only SQLx dependencies; audits remain unsuppressed. One informational rustls-pemfile maintenance warning remains.
- The strict local Qwen model-sensitive corpus failed its contact-sensitive case. Application screening held the observed output; this is not model/privacy certification. Automatic publication remains disabled.
- Production OIDC/TLS, real-server/team and long-duration capacity testing, Apple signing/notarization and binary distribution acceptance remain pending.
- Code context does not cover undocumented decisions or guarantee safe edits; shell/manual/external writes and other clients bypass the optional hook.

See the [publication review](docs/reviews/2026-10-12-public-alpha.md), [local acceptance](docs/reviews/2026-10-12-v0.1-continuation.md) and [launch runbook](docs/operations/public-alpha.md). There are no downloadable signed binaries in this candidate.
