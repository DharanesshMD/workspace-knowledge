# Cursor native capture results

Follow-up: [2026-10-08 live acceptance](2026-10-08-cursor-live-acceptance.md) adds successful bounded live handoff, restart/continuation and in-flight revocation evidence. The historical pending gates below describe the earlier run.

Updated 2026-10-08. Branch: `codex/cursor-native-capture`. This is the first explicitly enrolled Cursor slice, not production deployment or universal-client certification. Code and automated verification are complete for this slice; live acceptance is partial.

## Implemented

- Immutable activity-session service scope; shared submit/correct/publish sensitive-content screening; revision-bearing overlap references.
- Cursor 3.23.23 local Agents parser, explicit enrollment, exact callback-bound transcript source, descriptor-based no-follow reads, encrypted checkpoint recovery, fixed 24-hour source lifetime and one-hour-or-shorter private draft lifetime.
- Unknown/outside callback tool scope permanently holds the source even when the transcript omits tool metadata. Unsupported shapes fail closed.
- Concurrent reconciliation cannot replace a newer committed prefix with an older one. Holds are monotonic.
- Every native inference attempt rechecks current source and authorization. A failed reconciliation stops that session. Native inference is bounded to ten attempts and a 120-second batch deadline; retention has its own 15-second task independent of inference.
- Reviewed native candidates retain proposed lifecycle and validated workspace-relative transcript file evidence. Replay/submission guards prevent duplicate cards.
- Search now includes decisions, constraints and open questions, in addition to title, intent and rationale. The end-to-end fixture exposed the prior omission when “idempotency” occurred only in decisions. Migration 0005 rebuilds the matching GIN index; run with the migration role before starting application processes. The index rebuild blocks writes while running.

## Verification

Final workspace suite: 120 passed, zero failed (database-dependent tests are run separately). Formatting, warnings-as-errors Clippy, workspace build and diff whitespace checks passed.

Separate PostgreSQL access suite: 16 passed, including structured-field search, unpublished exclusion, revocation, service/organization isolation, immutable activity scope and corrected-card screening. The HTTP suite's three ordinary tests passed in the workspace suite; its additional database test passed separately. The five-identity handoff test passed with the previously generated real local-model card.

The final deterministic end-to-end harness passed with real macOS Keychain, PostgreSQL and synthetic native callbacks/transcripts. It verifies private draft generation, replay/restart deduplication, reviewed publication, teammate REST and remote MCP retrieval, overlap card revision, later-turn recovery without another callback, sensitive-continuation hold and denied submission/access. It uses a loopback fixture model and does not prove model quality.

The general HTTP/stdio MCP/remote MCP integration passed, including authentication, publication, discovery, bounded context and revision-safe supersession. `cargo tree -i rsa --target all --locked` reports no active dependency path. RSA remains in the lockfile; this check is not a new advisory audit or a claim that the entire dependency tree is vulnerability-free.

Before the final review fixes, the real Ollama `gemma4:e2b-mlx` harness also passed. Its actual card preserved idempotency keys, retry limit of three, proposed lifecycle and pending implementation review, without an absolute transcript path. That earlier model-quality observation is separate from the final deterministic rerun.

## Observed live Cursor behavior

Tests used only an owned synthetic project and disposable `wk_test` database. No global hooks or real developer projects were enrolled.

1. Cursor 3.23.23 edited a synthetic retry constant, then stalled. Cancellation produced `turn_ended/status:error`. A previously unobserved timestamp wrapper caused a safe hold; a failing regression pinned support for that wrapper. The original held source stayed closed.
2. Rebuilt unsigned binaries triggered macOS Keychain prompts. The user approved the expected synthetic items. SecurityAgent is blocked to desktop automation. This remains a packaging/usability risk and does not establish unattended installation reliability.
3. After restoring stopped test services, a fresh read-only Cursor turn and a no-tool continuation delivered seven callbacks. Completion callbacks supplied the transcript path. The collector recovered two complete turns with no delivery gaps. Native counters count observations, so legacy prompt/response counters remained zero.
4. The later local-model run did not create a draft. The machine had about 2.4 GB free before model loading; model memory reached roughly 7 GB and requests exceeded the 120-second timeout. The source became stale while the old single worker waited. Final code now bounds native attempts and runs retention independently; final live model behavior after those changes was not retested. This is a resource observation, not a server-capacity benchmark.
5. Removing the synthetic developer's server membership held the recovered live source and cleared all recovered turns. Doctor showed membership unavailable and two held sessions. The test did not demonstrate revocation in the middle of a successful model call.

Live instrumentation logged callback names, presence of a transcript binding and process status, without recording prompts or transcript contents. Test hooks are disabled and owned test services/credentials are cleaned up.

## Review and remaining gates

Independent review found four important issues (callback scope loss, cached inference after a hold, unbounded failed inference delaying retention, and checkpoint rollback). All four were fixed and re-reviewed without remaining critical or important findings. The search correction was separately reviewed without actionable findings. Regression evidence includes failing-before/passing-after scope/checkpoint/budget/retention/search checks.

Still unverified: full fresh-live capture → successful local inference → human review → teammate MCP/overlap, live cancellation/resume after the timestamp fix, and live collector restart continuity after the final fixes. Unit and synthetic integration coverage do not replace these live gates. Inference-time revocation and concurrent asynchronous reconciliation are covered by guards/helper regressions, not a full adversarial end-to-end scheduler test.

Other limits:

- Exact macOS Cursor installation/version/surface only. Alternate installations, shell/MCP/attachments/compaction/subagents and unknown callback/transcript shapes are unsupported and held.
- Entirely missed initial sessions cannot be discovered. Recovery applies only to already bound sources. An outage blocks capture because membership is checked on every request.
- Callback-only file scope is checked but does not become card file evidence; transcript file metadata is still needed for overlap references. Deleted/nonexistent paths can cause a conservative hold.
- Source checks detect prefix mutation, inode replacement and symlink traversal; they do not provide an OS-level immutable snapshot against a hostile same-user process racing every check.
- Sensitive-content screening is heuristic, not comprehensive DLP. Shared publication still requires review.
- Native Codex and Antigravity adapters remain unimplemented. Claude Code and Gemini coverage remains partial. MCP retrieval is not conversation capture.
- No five-developer production load test, database-growth benchmark, native signing/notarization or deployment certification was performed in this slice.

## Execution rulings

1. Kept the approved per-request fail-closed membership rule; outages can prevent capture.
2. Prepared later-task tests while earlier verification compiled, but kept implementation after observed failures and reran task verification. Cost: intermediate combined runs can see unfinished tests.
3. Hook version is optional in observed envelopes; each consent load checks the installed `/Applications/Cursor.app` version and rejects conflicting callback versions. Cost: alternate installations are unsupported; this is not process authentication against the same OS user.
4. Native guard identity binds root/service/conversation, not approval hash. A policy change cannot reopen the same conversation. Cost: moving a workspace creates a distinct enrollment context.
5. Committed tightly coupled source/recovery/CLI work together, retaining separate task evidence. Cost: a larger integration commit.
6. Extended reviewed-card search after the final handoff regression exposed missing structured fields. Cost: one blocking index rebuild during deployment migration.
7. Kept live certification incomplete after the resource-limited model run instead of treating synthetic or earlier model tests as equivalent evidence. Cost: the live acceptance gates above remain release blockers.

Detailed local logs are retained under `.superpowers/sdd/2026-10-06-cursor-native-capture/` in the managed worktree. Nothing has been merged, pushed or deployed.
