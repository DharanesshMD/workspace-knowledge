# Code-change context acceptance — 2026-10-11

Status: contributor-aware context and the narrow Claude Code pre-edit integration are implemented. Synthetic contract, database, browser, build and restore checks are recorded below. **The installed host integration is not certified**, and this work does not certify deployment capacity or universal conversation capture.

## What is verified

- Repository, contributor, rationale, reviewed relationship and revision contracts distinguish confirmed identity from Git heuristic candidates. Code-change briefs include source references and gaps; they do not include raw conversation messages.
- The local observer sends bounded relative targets, base revision, operation class and an opaque operation handle. Local file digests and tool input remain in private collector state.
- The hosted brief is owner-authenticated, CSRF-protected and short-lived. Acknowledgement is unavailable through bearer, collector or MCP paths. Denied and unavailable briefs cannot be acknowledged.
- The Claude Code hook handles `Edit`, `Write` and `MultiEdit`. A first edit request produces a denial and review URL; a matching retry must reobserve the same snapshot and consume a single-use, device-bound receipt. Changed retries, stale snapshots and unavailable server authority block.
- Only the exact hosted portal, API and local hook statuses described in [the architecture](../architecture/code-change-context.md) are covered. A successful retrieval is advisory; it is not an edit permission or a correctness claim.

## Results

| Check | Result | Scope |
|---|---|---|
| Rust workspace with PostgreSQL tests | 277 passed, 0 failed | `cargo test --workspace --locked -- --include-ignored`; disposable `wk_test` only |
| Change-guard adapter tests | 2 passed | Synthetic `Edit`/`Write`/`MultiEdit` parsing, minimum-version refusal, denial JSON and fail-closed hook configuration |
| Agent pre-edit tests | 3 passed | Exact retry snapshot/expiry binding, explicit denial without enrollment, and expired local receipt cleanup |
| Provenance, preflight and context database tests | 6 provenance, 6 preflight, 5 context and 5 retention passed | Disposable PostgreSQL `wk_test`; not a production database |
| Server change-context and MCP integration | 3 passed | Synthetic browser acknowledgement, advisory MCP boundary and REST scope/authentication |
| Full synthetic HTTP/MCP smoke and consent demo | Passed | Real PostgreSQL `wk_test`, HTTP auth/publication/MCP discovery, and local scope/replay/pause fixture |
| Hosted browser tests | 34 passed | Playwright; desktop and mobile views were inspected |
| Python harness tests | 4 passed | Includes explicit unverified response when a five-user pilot fixture is absent |
| Retention deadline regression | 1 passed | Only the selected expired preflight is removed; a future cleanup row remains |
| Local synthetic hook harness | Passed | Missing enrollment denied a synthetic edit; target stayed unchanged and private canary was absent from hook output |
| Encrypted restore drill | 14 checks passed in 4.65 seconds | New disposable synthetic databases; included invalidation of an acknowledged preflight |
| Five-developer code-context endpoint pilot | Unverified | Requires a private fixture for five synthetic identities and a running loopback server; no throughput or database-growth claim is made |

The restore drill ran with `age v1.3.2` and PostgreSQL tools 17.11. It passed encrypted snapshot handling, authority quarantine, preflight invalidation, replay protection, incompatible/tampered backup rejection and rollback checks. It used disposable databases only.

The local synthetic hook harness checks that missing enrollment returns an explicit denial, a temporary target file remains byte-for-byte unchanged, and a private canary is absent from the response. Its output reports the installed client version and does not claim browser/server acceptance.

## Client and operating-system boundary

| Client / surface | Version and platform | Status |
|---|---|---|
| Claude Code `Edit`, `Write`, `MultiEdit` pre-tool hook | Fail-closed configuration requires 2.1.295+; installed test CLI was 2.1.15 on macOS | Enrollment correctly refuses this version; exact-file live confirmation remains unverified |
| Claude Code shell, `NotebookEdit`, external/manual file changes | All versions | Explicitly uncovered |
| Cursor, Codex, Gemini CLI, Antigravity, other Claude surfaces | Current installations | Advisory retrieval only; no protected pre-edit certification |

The hook's `onFailure: "block"` setting is specifically necessary because earlier Claude Code hook behavior can fail open on a hook crash or timeout. Configuration tests confirm the setting is present; behavior on a supported live client still needs acceptance on that client version. No client was upgraded or reconfigured for this test.

## Operational limits

The server does not receive the patch, raw tool input, local file digest, absolute path or private conversation. It retains preflight request/brief material for at most 24 hours for cleanup, while the permission itself expires after five minutes. The operator retention worker deletes in bounded batches of at most 1,000; the application runtime is not granted cleanup or restore authority. Restore removes preflights before making the restored database available.

The adapter narrows but cannot eliminate a local time-of-check/time-of-use window between the final snapshot check and the host writing the file. Shell commands, manual edits, hook disablement and other writers can bypass it. Keep normal code review, tests and branch protections enabled.

Schema 16, supported-host acceptance, production OIDC/TLS, native client capture, the known runtime dependency advisory `RUSTSEC-2023-0071`, and workload-specific capacity remain separate release gates. The five-developer harness measures only the advisory endpoint when explicitly configured; portal-only load does not establish protected-edit throughput or server capacity.
