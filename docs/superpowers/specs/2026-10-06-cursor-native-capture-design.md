# Cursor native capture and recovery

Date: 2026-10-06
Status: approved by the user on 2026-10-06; implementation has not started. A request-time authorization clarification is proposed explicitly in the implementation plan.

## Outcome

A developer working in an explicitly enrolled Cursor workspace can have supported visible development conversations captured privately, summarized locally, reviewed, and shared with an authorized teammate. The teammate can retrieve the approved finding and receive relevant file-overlap context. Missing capture must be visible rather than silently represented as complete.

The first supported target is Cursor desktop 3.23.23 on macOS, using local execution in the Agents window. Its installed commit during investigation was 2dac2428994fe34f12658d9ecad1541b98db2c00. Other Cursor versions and surfaces remain unverified until tested. Codex, Antigravity, Claude Code, and Gemini integration work is outside this first implementation slice; their current capability labels must distinguish unimplemented adapters from unavailable vendor features.

The user approved proceeding with the Cursor-first proposal. This document makes the implementation and acceptance boundaries reviewable.

## Evidence and current blockers

The investigation in docs/reviews/2026-10-06-native-capture-feasibility.md demonstrated additional_context delivery through a Cursor postToolUse hook. Cursor returned a synthetic marker supplied only by that hook. A stop callback supplied the exact transcript path. The transcript contained three completed visible turns even though only two callbacks were recorded overall. This verifies feasibility, not full pipeline reliability or the cause of missing callbacks.

Current code blocks native completion deliberately: crates/agent/src/process.rs requires agent_id managed-chat for every event. crates/agent/src/daemon.rs assigns unattributed-turn when the expected turn ID is absent. Neither behavior can be fixed by installation alone. The broad Cursor history importer is for explicit history operations and must not become the background capture discovery mechanism.

Two earlier reproduced server defects are prerequisites to a real-data pilot: activity upserts retain old file paths while replacing their service scope, and card corrections omit sensitive-content screening applied to new submissions.

## Architecture

Keep the existing local collector, encrypted SQLite journal, local inference endpoint, reviewed draft submission, PostgreSQL API, and MCP interface. Add a narrow Cursor session adapter and a persistent recovery worker inside the existing collector; do not introduce another service or a global chat-history scanner.

The flow is:

1. An operator enrolls the product/service and the developer approves native Cursor capture for that enrollment.
2. Project hooks deliver a bounded event to the authenticated local collector.
3. The collector validates the envelope and durably records it locally before acknowledging it.
4. A validated session-specific transcript path establishes a recovery source. The worker reconciles only this enrolled source.
5. A completed visible turn becomes a private summary candidate with independent completeness and scope assessments.
6. The owner reviews and submits the candidate; a reviewer publishes it under current server permissions.
7. Another authorized developer retrieves the card or receives a relevant overlap notice.

No automatic publication is enabled by this work.

## Enrollment and source binding

Native Cursor capture is explicit and scoped to a product/service enrollment. Existing installations do not silently begin reading transcripts after upgrade. Add an enrollment option and show its source boundary before installing the project hooks. Store the consent with the existing private state and bind it to the workspace approval fingerprint.

Accept only an authenticated local callback whose client version and workspace roots match the supported enrollment. Reject ambiguous multiroot sessions. Bind client, surface, workspace fingerprint, service, conversation ID, and canonical transcript path. Validate identifier syntax and length before constructing or comparing paths. The transcript must be under the current user's allowed Cursor transcript directory and have a filename/layout consistent with that conversation. Do not infer authorization from a project-directory slug alone.

A callback path is untrusted input. Validate owner, regular-file type, allowed ancestors and symlinks; use a descriptor-based no-follow open on the supported platform and recheck file identity around reads. An arbitrary local file, other session's transcript, or sibling project's source must not be accepted. Do not read payload content before path and enrollment checks pass.

If a callback has no transcript path, record partial hook coverage and wait for a valid binding. Do not scan Cursor storage to find a substitute. An entirely missed new session remains undetectable through this mechanism; expose that limit during setup and in coverage reporting.

## Durable event receipt and recovery

Move remote membership calls and inference out of the hook acknowledgement path. A hook may enqueue private local data only under current local consent and the existing fail-closed membership policy. A server outage must not silently authorize continued capture: report unavailable membership and a capture gap; do not broaden the existing permission contract in the name of offline support.

The receiver acknowledges success only after durable encrypted storage. Delivery retries use stable event identities; duplicate delivery does not duplicate messages, summaries, or activity. Acknowledgement must never mean published or complete. Hooks must retain their non-blocking behavior toward Cursor when collection fails.

Use the existing 15-second worker cadence for already-bound sources. Persist source identity, committed byte boundary, prefix digest, last observed state, and turn progress in encrypted private storage. Read only complete newline-terminated records; wait for an incomplete trailing record. Bound file size, per-scan bytes and turns processed. Oversized input, unknown schema, source replacement, truncation, or a changed processed prefix produces an explicit hold, not silent truncation or guessed continuity.

Checkpoint advancement and imported events must be atomic, or use replay-safe event IDs so a crash between them cannot lose or duplicate work. Exercise the chosen mechanism through crash/restart tests. When reconciling a growing file, confirm the consumed prefix is unchanged before committing progress.

Preserve the fixed 24-hour raw retention boundary; recovery cannot recreate closed or expired session IDs. Source bindings also expire and cannot extend raw retention. A session spanning that boundary needs a supported fresh-session transition; otherwise stop capture and report it. Do not reread old source text after consent revocation, enrollment change, hold, or expiry. Retention cleanup must run independently of remote membership/inference availability.

## Message parsing and turn correlation

Extract a single-transcript parser from reusable existing logic. Its inputs are a validated byte stream and source identity; it must not enumerate directories.

Allowlist visible user and assistant text. Exclude reasoning/thinking blocks, tool-output bodies, injected system context, credentials, and attachment contents. Parse supported tool references only as scope/evidence metadata. Unknown message forms must not be interpreted as ordinary user speech.

Use a native stable generation ID where available. For transcript recovery, derive stable per-turn identities from the bound session and structural transcript position, with content digests detecting edits. Do not deduplicate solely by text: two identical user requests can be distinct turns. Connect callback and transcript identities explicitly so the same turn is not summarized twice. Remove the unattributed-turn merge behavior for native capture; ambiguous correlation stays partial or held.

A visible-turn completion requires a user message, assistant response, a supported terminal record or correlated completed callback, and no parse gap. A final assistant message alone is not proof of completion. Interruptions, errors, pending tools, compaction, resumed sessions, and child-agent references must be represented explicitly. Unsupported child/compaction forms remain partial or held instead of being certified by omission.

## Independent coverage and privacy states

Track these dimensions separately:

- Visible messages: unavailable, partial, or complete within the declared text-only source boundary.
- Workspace scope: verified for supported operations, unknown, or violated.
- Recovery health: waiting for binding, watching, recovering, stale, held, or expired.
- Publication: private draft, owner-reviewed submission, or reviewer-published card.

A complete transcript does not establish workspace scope. Validate supported file-tool paths against the enrolled service. Shell commands, MCP calls, attachments, other workspaces, unknown tools, and subagents may access context not provable from the transcript. Mark those cases unknown or violated as appropriate; they must not become publishable automatically. Pasted sensitive or unrelated prose still requires existing content screening and owner review.

Only a supported completed turn with acceptable scope evidence may replace the managed-chat-only completion condition. Persist the evidence with the private candidate, and revalidate source digest, local consent, membership and policy revision at submission. Keep server lifecycle promotion and trusted code/test verification separate from conversational claims.

## Developer-facing coverage

Extend local wk-agent doctor with Cursor source-boundary/version status, number of bound sessions, last successful reconciliation time, recovered-turn count, and fixed-label hold reasons. Do not print raw messages, full transcript paths, user emails or credentials in diagnostics.

Show when no binding exists or the collector is unavailable. Silence is not evidence that every Cursor session was observed. Staleness describes the collector/source check, not merely an absence of user activity. Preserve scope-specific pause, reset and revocation controls; resume must not reopen a contaminated or expired session.

Expose the additional native status locally first. A team-wide capture dashboard is outside this slice and must not leak private local session metadata.

## Context delivery and team handoff

Retain bounded authorized overlap retrieval through postToolUse, plus explicit MCP lookup. Where tested, use session-start context as an additional delivery path, not a reliability prerequisite. A missed callback must not stall the agent.

Include card identity, revision and freshness with context. Label retrieved text as reference data, not instructions. Maintain token/character limits and per-session deduplication. Treat delivery as observable; do not claim the model will always obey or that retrieval guarantees correctness.

The acceptance handoff uses two distinct synthetic identities with different permissions. Developer A's private raw transcript must remain local. After owner review and reviewer publication, developer B can retrieve the approved card, and a file-aware lookup surfaces it. A third unauthorized identity must receive neither the card nor activity metadata.

## Server security prerequisites

Activity: do not narrow an existing session's permission scope while retaining paths from the old scope. Reject a service-scope change for the same activity session, requiring a new session identity, or implement equally restrictive immutable per-observation scope. The first implementation should prefer rejecting scope changes. Exercise the existing upsert through the API and under a non-owner RLS role.

Corrections: apply the same sensitive-content screening as new submissions before storing a correction or revision. Publication must also reject sensitive cards already stored through an older path. Reuse one shared validation path where practical, including draft writes through MCP. Fixed error labels must not echo rejected content. Add regressions for the synthetic bypass demonstrated in the investigation.

## Files and interfaces affected

- crates/adapters: a single-source Cursor parser and version-specific hook envelope validation; existing broad history import remains an explicit operation.
- crates/agent: source enrollment/binding, durable receipt, recovery worker, coverage diagnostics and summary eligibility checks.
- crates/local / crates/capture: persistent checkpoint/event atomicity and bounded retention operations only where required by recovery.
- crates/knowledge/src/activity.rs: prevent activity scope narrowing with retained paths.
- crates/server/src/lib.rs and shared validation: consistent correction/publication screening.
- docs/adapters/setup.md, compatibility.json and completion-matrix.md: precise implementation versus certification status.
- Test fixtures contain synthetic data only, not developer transcripts or personal paths.

## Acceptance and verification

Before calling the slice complete:

1. Parser fixtures cover repeated identical messages, injected wrappers, unknown records, partial lines, missing responses, completed versus aborted status, and ignored reasoning blocks.
2. Source validation rejects wrong owners, symlinks, traversal, other sessions/projects, multiroot ambiguity, replaced files and consent changes.
3. Recovery tests exercise duplicate callbacks, missed callbacks after binding, restart at each checkpoint boundary, prefix changes, truncation, quota/expiry and fixed raw TTL.
4. Scope tests cover permitted edits, sibling paths, outside-workspace tools and ambiguous shell/MCP activity; unsafe turns remain unpublished.
5. Regression tests reproduce and fix both security defects, including a non-owner PostgreSQL RLS read.
6. Live Cursor desktop exercises a file-edit turn and multiple no-tool turns, collector restart, cancellation and resumed work. Record exactly which event forms the client delivered and which recovery paths succeeded.
7. A local inference endpoint produces a reviewable candidate from the synthetic source. A deterministic mock may test transport and error handling but cannot establish model summary quality.
8. Two synthetic developers complete capture, review, publish, authorized retrieval and overlap delivery. An unauthorized identity is denied.
9. Relevant Rust tests, PostgreSQL integration checks and build/lint pass. Report unavailable live checks separately; do not promote compatibility status on fixture results alone.

## Operational boundaries and rollout

Keep the feature opt-in until the target-version acceptance matrix passes. Test in a disposable local workspace and database before a team pilot. Do not change global hooks or enroll existing projects without explicit user authorization. Disable only owned hooks on rollback, retain existing reviewed cards, and close pending native source bindings without reopening expired sessions.

This slice does not implement universal capture, auto-publication, cloud-agent support, semantic retrieval, a new database retention policy for shared cards, or automatic production deployment. The preceding capacity estimates remain estimates until a representative load benchmark is run.

## Remaining deployment decisions

No product behavior is left to a silent assumption: native capture remains opt-in, text-only and review-required; unknown scope is held; unsupported client versions report unverified; missed initial sessions are not discovered by a global scan. Exact supported transcript status forms must be extracted from the synthetic live fixture during implementation and pinned by tests. If the client cannot supply a trustworthy completion or scope signal for an operation, that operation remains partial rather than weakening the contract.
