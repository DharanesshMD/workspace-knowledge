# Local Codex and Antigravity acceptance — 2026-11-07

Status: passed for the local MCP workflow described below. This is a bounded local integration check, not production or native-capture certification.

## Environment and configuration

- Source baseline: `6188cfe`; fixes on `codex/local-ide-acceptance-20261107`.
- ChatGPT desktop: `26.1103.31037`; bundled Codex CLI: `0.167.0-alpha.3.2`. Real Codex runs used the installed user's `gpt-6.1-sol`/high settings and existing authentication. Desktop UI automation was denied by the computer-use tool, so no desktop UI result is claimed.
- Antigravity IDE: `2.6.0`, bundle `com.google.antigravity-ide`; real agent-panel runs used its selected Gemini 3.8 Flash Medium model.
- Existing Linux arm64 Docker stack: PostgreSQL 17, Keycloak, HTTPS proxy, restricted application database role, and product `demo`. The running application was rebuilt and restarted during testing; its database volumes were preserved.
- Codex uses identity `ravi`; Antigravity uses `anika`. These are existing local demonstration identities, not newly granted production accounts.
- MCP server name: `workspace-knowledge-local`. Codex configuration is in `~/.codex/config.toml`; the current Antigravity configuration is in `~/.gemini/config/mcp_config.json`. Existing server entries were preserved and backed up privately.
- Private launchers read the existing token files and private CA bundle at process startup and execute `wk-mcp proxy`. Tokens are absent from project rules and MCP configuration. No OS certificate-validation bypass was added.
- Synthetic projects: `~/wk-demo/antigravity-anika` and `~/wk-demo/codex-ravi`. The existing Antigravity editor with an unsaved unrelated file was preserved in its original window.
- Private configuration backups and raw synthetic test evidence: `~/.local/share/workspace-knowledge/local-ide-test-20261107/`. Do not commit that directory or its backups.

## Real model-driven workflow

1. Both private launchers discovered all 12 hosted tools. Antigravity called `knowledge.team_work` for an absent file and received `no_matches`; the UI's expanded tool arguments and output were inspected.
2. Antigravity checked overlap and team context, changed a synthetic retry schedule from `[100,100,100]` to `[100,200,400]` milliseconds, and ran a Python unittest. Its explicit `knowledge.share_session` returned `published: true`, card `9a4d4ff7-34c9-4824-8c69-86f512510539`, titled `WK-LIVE-20261107 Antigravity retry backoff`.
3. After an application-container restart, Codex called `knowledge.team_work` as Ravi and retrieved that exact card, Anika's name, the file, rationale, and the constraint to retain exactly three attempts without jitter, dependencies, or network calls. The read succeeded without an approval override after the annotation fix.
4. Anika's synthetic source and test were copied to Ravi's checkout to represent pulling a teammate's code. Codex checked both overlap and team-work tools before editing, added an optional positive delay cap while preserving the default schedule, and passed four unittest cases. Explicit sharing returned `published: true`, card `48a1cb40-17dd-4113-98d6-d28d6aeba045`, titled `WK-LIVE-20261107 Codex capped retry`, with both changed source/test paths.
5. Independent local reruns confirmed Antigravity's one test and Codex's four tests. The synthetic cards are retained so the demonstration remains usable. File lists are what the agents reported, not independently verified Git observations.
6. On the final image, Antigravity retrieved Ravi's exact card and both changed files, repeated the recorded four-test result and preserved constraints, and correctly distinguished `published: true` from `lifecycle: proposed`. A fresh real Codex read also correctly explained that `proposed` does not mean unpublished. Fresh processes using the updated local proxy independently retrieved the opposite developer's exact card after the final restart.

## Reproduced issues and fixes

| Issue observed | Fix and regression |
|---|---|
| A real Codex read lookup failed with "MCP tool call requires approval, but approval policy is never" because tool annotations were absent | Mark the eight hosted read tools and nine bridge read tools with `read_only_hint = true`. Publication, recording, feedback, assessment and approval-request tools are not marked read-only. Both metadata regressions were observed failing and then passing; the real Codex lookup subsequently succeeded. |
| Codex interpreted a shared card's `lifecycle: proposed` as meaning it was unpublished | Team-work entries now include their actual `published` flag, preserved during compaction, and the brief explains that implementation lifecycle is separate. A PostgreSQL regression failed before the fix and passed afterwards for full and compact replies. Implementation claims are not upgraded merely because a card was shared. |
| Setting the documented `WK_AUTO_PUBLISH_PRODUCTS=none` on the host did not reach the Compose application | Forward it in Compose; a real rendered-configuration regression failed before the fix and passed afterwards. Existing publication defaults are preserved. |
| The team-work verifier failed once real retry summaries populated the server: relevance ranking placed another card first | Query using this run's unique alphanumeric marker rather than generic retry keywords. The live verifier then passed while both IDE-generated cards remained present, and withdrew only its own fixture. |
| Installation copy described schema 18 and mandatory review despite newer main behavior | Document migrations 1–21, runtime-role grants, and eligible assistant-card publication defaults; distinguish historical alpha.3 behavior from current main. |

## Access, privacy and recovery checks

- Seven direct live MCP checks passed: unknown developer is explicit; partial file/directory names do not match; traversal is refused; a synthetic sensitive query and sensitive share are rejected without echoing their content; an out-of-scope service reveals no work/activity; the rejected summary is absent from shared retrieval.
- Portal sign-in and token lifecycle passed: missing CSRF refused; token secret shown only on creation; listings contain no secret; token accepted on the intended MCP endpoint and rejected on REST and another product; tokens cannot mint/list tokens; revocation immediately rejects subsequent use.
- Admin lifecycle passed: create a scoped token-only developer, list metadata, authenticate, reissue and invalidate the old token, remove and revoke, and recreate without inheriting old access. Non-admin access, another developer's identity, invalid names, excess lifetime and out-of-scope services were refused. The verifier removed its disposable account.
- Rebuilt container passed non-root/assets checks and refused production startup without required configuration. The application restarted without losing either model-generated summary.
- An initial debug build exhausted local disk space. Only generated artifacts for this repository's server/MCP packages were cleaned; subsequent full validation uses disabled debug information and bounded build jobs. No project source, Docker volume, real conversation, or user credential was deleted.

## Verification and support boundary

| Final check | Result |
|---|---|
| Rust workspace, including PostgreSQL/pgvector tests (`--include-ignored`) | 799 passed, zero failures, zero ignored |
| Browser suite | 77 passed |
| Python checks | 46 passed; one explicit Gitleaks-dependent skip |
| Strict workspace/all-target Clippy, formatting, whitespace checks | Passed |
| Rust workspace build; Linux container including production web build | Passed |
| Real PostgreSQL/HTTP/stdio and hosted MCP integration script | Passed |
| Final deployed two-user verifier and new-process cross-user retrieval | Passed |
| Focused independent code review | No actionable findings |

The running application uses `wk-server:local-ide-20261107-final`, also tagged `wk-server:local`; Docker image identity `sha256:ce5f820d626ed9c85d8621ad4507f6c3ca5116a8efc7d991cc401220a8ea32f5`. The original image is retained as `wk-server:pre-local-ide-20261107` for rollback. Database volumes and identity-provider state were preserved. The updated `wk-mcp` was installed atomically both in `~/.local/bin` and in the private acceptance launchers' binary directory; prior binaries are retained in the private backup directory.

To repeat the shared-work demonstration, open the two synthetic projects in their respective clients and ask about the other developer's `WK-LIVE-20261107` work on `backend/retry.py`. New Codex sessions read the configured server; the existing chat's tool inventory was not hot-reloaded. Sharing remains on explicit request in the project rules.

This qualifies the tested MCP sharing/retrieval workflow only. Native capture remains unenabled for these installed versions: the existing adapters target Codex CLI `0.162.0-alpha.2` and the older Antigravity desktop `2.19.1`; Antigravity native publication remains held. No client pin, capture hold, credential policy, or hook-trust check was bypassed. Automatic conversation capture, every model/client, other operating systems, public identity providers, signed distribution and real-team production capacity remain outside this test.

Configuration references: [OpenAI MCP documentation](https://learn.chatgpt.com/docs/extend/mcp?surface=cli), [Google MCP documentation](https://antigravity.google/docs/mcp), and [Google's current MCP configuration location](https://codelabs.developers.google.com/developer-knowledge-mcp-antigravity).
