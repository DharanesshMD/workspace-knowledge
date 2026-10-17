# Changelog

Changes are recorded here for the first source-only alpha candidate. No tag or release has been published.

## Unreleased — proposed v0.1.0-alpha.1

### Included

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
- No local model needed: `wk-agent init` and `wk-agent device connect` take an optional `--summarizer local|rules`. In `rules` mode `process` builds drafts without a model from the first prompt and last reply of a turn or imported part, or from a memory note's heading and text. Titles start `[auto-drafted] `, cards are `partial`, quoted text is masked and held when sensitive, and drafts carry file evidence only where the source already provides it (Cursor turns only, not Claude Code, Codex or Copilot imports or native Codex capture). They quote the conversation, so review them before submitting. `doctor` reports the mode, and managed `wk-agent chat` requires `local`. See the [setup guide](docs/adapters/setup.md#set-up-without-a-local-model).
- `wk-agent prompt --client claude-code|copilot|codex` prints instructions for a `/wk-summarize` command (Claude Code slash command, Copilot prompt file, Codex `AGENTS.md` text). The assistant writes a card and pipes it to the new `wk-agent draft`, which validates it (strict fields, 64 KiB, sensitive-content screening) and stores a private local draft for 24 hours; review it with `drafts` and send it with `submit --id ID --reviewed`. Nothing reaches the team server before review. See [Summarize with your assistant](docs/adapters/setup.md#summarize-with-your-assistant-wk-summarize).
- Behaviour change: `wk-agent init` and `wk-agent device connect` without `--inference` now select `rules` (previously they defaulted to a local model URL). Pass `--inference URL` for the earlier behaviour. Existing configurations are unchanged and stay `local`; local configurations are written exactly as before so older binaries still read them, but `rules` configurations contain a `summarizer` key that older binaries cannot read. A setup's mode cannot be changed in place, and owner-sync consent records it.

### Known limits

- Linux Docker server + macOS developers is the alpha evaluation scope. Windows builds, universal capture, MultiEdit, cloud-backed hooks and Antigravity publication are not qualified.
- rustls updated to 0.23.46 for RUSTSEC-2026-0346 (medium) and RUSTSEC-2026-0347/0348/0349 (low), published 2026-10-14.
- RUSTSEC-2023-0071 package removed using the pinned Convex AWS-LC OIDC fork and PostgreSQL-only SQLx dependencies; audits remain unsuppressed. One informational rustls-pemfile maintenance warning remains.
- The strict local Qwen model-sensitive corpus failed its contact-sensitive case. Application screening held the observed output; this is not model/privacy certification. Automatic publication remains disabled.
- Production OIDC/TLS, real-server/team and long-duration capacity testing, Apple signing/notarization and binary distribution acceptance remain pending.
- Code context does not cover undocumented decisions or guarantee safe edits; shell/manual/external writes and other clients bypass the optional hook.

See the [publication review](docs/reviews/2026-10-12-public-alpha.md), [local acceptance](docs/reviews/2026-10-12-v0.1-continuation.md) and [launch runbook](docs/operations/public-alpha.md). There are no downloadable signed binaries in this candidate.
