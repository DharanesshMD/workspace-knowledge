# Native capture implementation and bounded acceptance — 2026-10-09

## Result

Codex CLI 0.162.0-alpha.2 on macOS completed a real no-tool capture-to-sharing test. Its actual UserPromptSubmit and Stop commands passed normal hook trust review, verified the emitting process, and delivered authenticated callbacks to the rebuilt collector. Local Qwen3.5:2b inference produced a private draft retaining the idempotency requirement, three-retry limit and proposed lifecycle. A reviewer corrected its title, creating revision 2, then published it. A second authorized identity retrieved it through actual MCP knowledge.search and knowledge.get. Before publication that identity could not find it.

Synthetic card: `3485786d-4eb2-4b19-8242-77ebd3e21bb9`, revision 2. This is a bounded live success, not client certification or an unattended installation guarantee.

Antigravity desktop 2.19.1 now has a callback-bound source reader and a strict visible-message parser based on the previously observed synthetic transcript. All Antigravity native turns remain held: no reliable association between delayed terminal callbacks and individual turns has been established. No live end-to-end Antigravity collector/publication success is claimed. Claude Code's earlier subscription/MCP/prompt-response success is unchanged; its partial turn correlation was outside this implementation.

## Checks and fixes

The workspace suite passed 144 tests; its 18 database-dependent tests were separately enabled and passed (16 knowledge tests, one HTTP test and one reviewed handoff test). Agent regressions passed 26 tests after the final lock/diagnostic/budget adjustments. Clippy with warnings denied and the full workspace suite passed again after the final adjustments.

An independent review found five issues, addressed in the implementation: unsupported Codex context now holds the entire session; permission waits are followed by fresh source/enrollment checks; native state transitions and draft submission use a cross-process lock; emitter ancestry is verified against the pinned binary; disabling one client preserves the other clients and makes its remaining callbacks harmless. Regression tests cover sticky scope, sequential turns, cancellation replay, fixed expiry, lock exclusion, selective cleanup, changed enrollment, changed draft evidence and executable-path mismatch. The async test exercises the final storage check after a simulated permission wait; a full live permission-server barrier matrix remains unverified.

Live testing exposed two installation realities. macOS `ps comm` can report only `codex`, so executable verification uses `proc_pidpath`. Rebuilding the unsigned collector can require Keychain access again. While access was pending, two-second hooks timed out; the resulting capture gap prevented publication. After the user allowed the synthetic items, the test used an explicit clean-session reset and a new chat. Both callbacks then returned exit 0, and the handoff succeeded. No timeout was treated as successful capture.

## Boundaries still open

- Codex shell reads, patch/file tools, subagents, compaction and unknown contexts remain held. The live read/edit probe observed Bash and apply_patch and was interrupted; it does not certify file-operation coverage.
- Antigravity terminal attribution, launcher ancestry acceptance and full collector live matrix remain unverified. New wrappers/tool records are rejected.
- Normal Codex trust was tested with generated inline config and the real hook command, plus a metadata-only diagnostic wrapper. Project-local installation/trust, live restart/outage/disable/revocation combinations and full cross-platform packaging are not all certified.
- Entirely missed initial callbacks remain undetectable. Native evidence expires after 24 hours, with no replay extension. A new client session is needed after an expired/closed identity or a session scope hold.
- Five-developer load, storage sizing measurements, signing/notarization and production release review are separate work. No performance or universal-capture claim follows from these tests.

## Operational cleanup

Only synthetic workspaces and callback-named synthetic transcripts were accessed. No real conversation history scan or traffic interception was used. Test hook definitions were inline to the owned CLI processes; no new global hook registration was installed. Owned test API/collector and local inference are stopped after acceptance; temporary synthetic Keychain entries and private collector state are removed. The disposable database retains the synthetic published card for traceability. User Claude subscription login is preserved.
