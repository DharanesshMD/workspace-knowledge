# Contributor-aware code-change context

Workspace Knowledge can connect a file in an enrolled repository to reviewed contributor identity, confirmed rationale, related decisions and evidence. The goal is to help a developer or coding agent understand who owns an important invariant and why it exists before changing code.

## Setup and data flow

1. An operator registers a repository and its service boundaries. Developers enroll only a repository already inside their approved workspace and bind it to the server repository ID.
2. A contributor opts into a scoped profile. Code authorship starts as a candidate; a contributor confirms the attribution and the rationale-to-code link. Reviewers can add or withdraw a reviewed relationship. Git author strings alone never establish a person's identity.
3. `wk-agent context --file backend/retry.rs` observes local Git state and requests a source-backed brief. The REST and MCP `knowledge.change_context` methods provide advisory retrieval. They return relevant names, reasons, references, freshness and explicit gaps; they do not return conversation messages.
4. Before a covered edit, the separate Claude Code protection hook can create a five-minute, single-use preflight bound to the enrolled device, repository revision, target snapshot and exact retry input. The first attempt is denied and opens the hosted brief. Only the developer's authenticated browser session with CSRF protection can acknowledge it. The matching tool retry consumes the receipt, rechecks the local snapshot and then returns control to Claude Code's ordinary permission checks.

The device and repository must already be enrolled before protection can be enabled. Check status and install the separately owned Claude hook with:

```sh
wk-agent --state-dir "$HOME/.local/share/workspace-knowledge/collector" protect status
wk-agent --state-dir "$HOME/.local/share/workspace-knowledge/collector" protect enroll \
  --settings-file "$HOME/.claude/settings.json" --yes
```

Enrollment refuses unsupported Claude Code versions and labels a successful configuration `configured_unverified`. `protect status` reports exact covered tools and known bypasses; use `protect disable --settings-file ...` to remove only this hook.

Summaries and rationale are shared as reviewed knowledge. Local tool input, edit text, operation hashes, absolute paths and unreviewed conversation content are not sent as team context. The server stores only the bounded request and brief needed for a short-lived preflight; it expires for authorization after five minutes and is physically removed after the 24-hour cleanup deadline. Restore deletes every preflight in the same transaction that revokes restored device and browser authority.

## Decision meaning

The brief distinguishes confirmed contributor attribution from heuristic Git candidates, reviewed rationale from proposed notes, current source revisions from stale ones, and complete coverage from known gaps. Missing, withdrawn, out-of-scope, stale or unavailable provenance cannot be turned into permission by acknowledging the page. MCP and ordinary REST calls remain advisory and cannot create or consume a human acknowledgement.

An acknowledgement means the named developer reviewed that brief for that exact local operation. It does not prove the code change is correct, that tests passed, or that the host will only write the observed file. Those claims need separate code review or signed evidence.

## Protection coverage and limits

The first blocking adapter targets Claude Code `Edit` and `Write` pre-tool calls. A defensive `MultiEdit` matcher is configured, but the tested 2.1.296 host did not offer that tool; native acceptance remains absent. Enrollment requires Claude Code 2.1.295 or later because the hook configuration uses its fail-closed `onFailure: "block"` behavior ([Claude Code hook documentation](https://code.claude.com/docs/en/hooks)). Each installation is labeled `configured_unverified`; Workspace Knowledge does not call it certified until a named version, operating system and tool flow pass host acceptance.

The hook does not cover shell commands, `NotebookEdit`, manual or external file writes, other AI clients, or a user disabling the hook. Paths outside the enrolled service continue through normal host permissions and are explicitly uncovered. The final local snapshot check narrows races, but a different writer can still change a file after the hook returns and before the host writes it. Administrators should retain ordinary branch protection, code review and test gates.

If the server is unavailable, identity is revoked, a brief expires, a snapshot changes, an operation is retried differently, or receipt consumption fails, the covered hook blocks and requires a fresh review. The observation collector and this gate are separate: capture errors do not grant edit permission or alter existing capture behavior.

## Acceptance and operations

See [current local enrolled Edit/Write acceptance](../reviews/2026-10-12-v0.1-continuation.md) and [the earlier version-specific acceptance record](../reviews/2026-10-11-code-change-context-acceptance.md). `scripts/code-context-live.py` tests the local missing-enrollment block against a temporary synthetic repository without server credentials. `scripts/code-context-pilot.py` measures the advisory endpoint for five distinct synthetic identities against a loopback server and `/wk_test`; its private fixture must contain five bearer tokens and bounded request bodies. It reports latency and selected PostgreSQL relation sizes, and makes no capacity claim. A run without an owned fixture is explicitly `unverified`.

For deployment, preserve the existing server, database, OIDC, TLS proxy, backup and restore boundaries. Apply schema 17 through the normal migration process, run the bounded retention worker, and perform the restore drill before enabling host hooks. Do not advertise universal capture, production-provider acceptance or certified protection based on portal retrieval or local tests alone.
