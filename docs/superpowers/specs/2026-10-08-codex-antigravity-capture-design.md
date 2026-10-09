# Codex and Antigravity native capture

Status: design and implementation plan approved; experimental implementation added on 2026-10-09. Codex no-tool handoff passed; Antigravity publication remains held. See the native capture acceptance report.

## Outcome and boundaries

Extend Workspace Knowledge so visible conversations in an explicitly enrolled product workspace can become private, reviewable knowledge drafts. Reuse the existing local encryption, membership checks, retention and reviewed publication. Success means a live client turn produces one correctly scoped draft, a reviewer publishes it, and another authorized identity retrieves the resulting card without accessing the source conversation.

Start with Codex CLI 0.162.0-alpha.2 and Antigravity desktop 2.19.1 on macOS. Other versions and surfaces remain unverified. Claude Code 2.1.293 now has a successful live MCP and prompt/response hook test, but its existing partial adapter and ambiguous turn correlation are outside this implementation slice. No claim of universal coverage, unattended operation or production certification is permitted.

## Evidence obtained before implementation

- Codex's actual UserPromptSubmit and Stop callbacks carried the same session_id and turn_id, the submitted prompt, and the correct final visible response. transcript_path was null in the ephemeral CLI test. The isolated probe used an independently reviewed local script and the documented per-invocation hook-trust bypass; it did not establish the normal installation/trust-review experience.
- Antigravity's project-local hooks delivered PostInvocation and Stop for the first no-tool turn, then PreInvocation, PostInvocation and Stop for a continuation. Both turns used invocationNum 0 and executionNum 0. Those numbers cannot serve as unique turn IDs. Both callbacks supplied the exact session transcript_full.jsonl path and the sole synthetic workspace.
- That exact synthetic transcript had four records: USER_INPUT, PLANNER_RESPONSE, USER_INPUT, PLANNER_RESPONSE. Each had a distinct step_index and DONE status. The two visible responses matched the UI. The first PreInvocation was not observed; initially missed callbacks must remain measurable uncertainty.
- The production code still explicitly rejects both native adapters; its legacy missing-turn fallback and managed-chat-only completeness rule cannot be reused as a complete native capture contract.

Official interfaces consulted: [Codex hooks](https://learn.chatgpt.com/docs/hooks) and [Antigravity hooks](https://antigravity.google/docs/hooks). Documentation is not client certification; the observations above are the limited empirical basis.

## Approach

Use separate client-specific adapters with a common private storage contract. Codex consumes direct hook fields, while Antigravity reconciles only the exact transcript named by a validated callback. Preserve the working Cursor adapter while extracting only small, proven storage utilities where necessary.

Alternatives considered: direct hooks alone avoid transcript parsing but cannot recover Antigravity's messages; a single generic transcript reader reduces file count but hides incompatible identities and schemas. Neither is adequate. Broad history scanning, screen scraping and network interception are excluded.

## Enrollment and event ingestion

Add explicit opt-in enrollment recording client, exact supported version, surface, device, canonical workspace, service and policy hash. Preserve existing configs through merge-only installation and provide disable/revoke controls. Codex installations must use normal hook review/trust; the diagnostic bypass is not an installation default. Antigravity config uses its named-hook schema in the chosen workspace's .agents/hooks.json.

Hook commands acknowledge quickly through the authenticated loopback collector. Validate payload size, client identity, explicit version enrollment and current binding before retaining content. Missing workspace identity, unexpected extra workspace roots or unsupported context marks the source held. Do not use unattributed-turn for either adapter. Retain only recognized visible message fields and scope evidence, never reasoning fields, system prompts, raw tool output or arbitrary attachment text.

## Codex adapter

Key state by client/device/session/agent/turn. Store prompt and final response as separate observations; exact duplicate callbacks are idempotent. Changed duplicate content, missing identities, conflicting prompts or an orphan Stop hold the turn. Stop is an observation of a final response, not proof of a successfully implemented requirement.

Track PreToolUse/PostToolUse for scope. Initially support only tested file-read/edit tool shapes whose canonical paths resolve within the enrolled service. Unknown tools, shell execution, external MCP context, attachments, subagents and compaction hold the affected turn until their attribution rules have explicit tests. Interrupt records cancellation and prevents a later stale Stop from silently completing the turn. Sequential turns must remain distinct even after collector restart.

A prompt+response pair may be marked message-complete only when its event sequence is consistent and supported; publication eligibility additionally requires verified scope, live membership and review. Missing-hook recovery is not available for ephemeral sessions with no transcript. Expose that limitation instead of treating a quiet collector as healthy or scanning user histories. A timeout marks an open turn stale/partial, never complete.

## Antigravity adapter

Accept only callbacks with exactly the enrolled workspace and a valid conversation ID. Bind only the canonical per-conversation transcript under the expected desktop data root. Apply the existing Cursor source-safety principles: owner checks, no symlink traversal, regular-file checks, bounded reads, stable file identity, retained-prefix digest, fixed expiry and no discovery by directory scanning.

Use conversation ID plus USER_INPUT step_index as turn identity; invocationNum and executionNum reset and are diagnostic only. Read complete JSONL records incrementally. Pair a supported USER_INPUT with visible PLANNER_RESPONSE records, and require a matching settled Stop plus an unchanged reconciled snapshot before completing the current turn. Do not infer success solely from fullyIdle. Maintain an explicit allowlist of observed normal terminal reasons; unknown/error/cancelled termination holds the current turn. Delayed Stop or transcript writes must not complete the wrong turn.

Parse the exact observed USER_REQUEST wrapper with a strict grammar; permit only tested non-content metadata around it. Unknown wrappers, attachments or external context hold the source. Exclude reasoning/planner-internal fields. Tool and mutation record formats require synthetic live examples before adding them to the parser. A recognized assistant record is eligible only when verified to represent visible UI output on the pinned version.

The first implementation can accept no-tool turns and tested file operations; unsupported records remain private and held. Prefix mutation, truncation, source replacement or incompatible schemas invalidates the source and its derived drafts. A background watcher may reconcile an already-enrolled source, but cannot enroll chats by scanning history. Entirely missed sessions remain undetectable and must be disclosed.

## Draft lifecycle and privacy

Store adapter evidence independently from model output: message completeness, scope confidence, terminal state, source digest, policy revision, supported client version and adapter parser version. The model cannot promote these properties. Recheck membership before inference and after its return, then again at submission. Revocation clears retained raw content and blocks derived draft submission. Apply fixed bounded retention independently of whether inference succeeds.

Replace managed-chat-only draft eligibility only for explicitly supported native evidence. Keep reviewed publication; no automatic sharing. Deduplicate on source/turn digest plus parser/model/policy versions. Corrections create a new reviewed revision and retain the established revision-safe lifecycle behavior. Any scope uncertainty retains the session privately and blocks shared summaries, search, embeddings and MCP exposure.

## Implementation boundaries and acceptance

Work primarily in crates/adapters, crates/agent, the existing private local store, adapter setup documentation and compatibility metadata. Keep vendor parsing out of the server. Avoid a generic event framework or a broad Cursor rewrite.

Before implementation is declared complete:

1. Parser/state tests: mismatched identities, two sequential turns, duplicates, reordered/orphan events, interruption, incomplete records, prefix mutation, oversized input and incompatible schemas.
2. Privacy tests: mixed roots, sibling paths, symlink replacement, unknown tools/attachments, reasoning canaries, revoked membership during inference and before submission, expiry while held.
3. Real client tests on the pinned surfaces: initial and continuation no-tool turns; supported read/edit flow; restart; cancellation; collector outage; repeated callbacks; unsupported context holding. Capture normal hook-trust installation separately from the bypassed diagnostic probe.
4. End-to-end synthetic handoff: visible source -> local draft -> reviewer correction -> publication -> second identity MCP retrieval. Inspect summaries for invented facts and operational instructions.
5. Keep capability labels partial/unverified wherever these tests have not passed. Five-developer load, signing/notarization, other platforms, enterprise deployment and broad client certification remain separate release gates.

## Review checkpoint

Review this design before writing the implementation plan. The plan must sequence independent adapters, evidence/storage changes, privacy tests and live acceptance, with an explicit stop on unobserved vendor formats. No implementation approval is implied by the successful retrieval tests.
