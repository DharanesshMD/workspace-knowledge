# Native capture feasibility investigation — 2026-10-06

## Conclusion

Native integration is feasible, but the current application does not implement a reliable end-to-end capture contract. Existing unsupported labels for Codex and Antigravity describe this implementation, not current vendor capability. No production adapter was changed or certified in this investigation.

## Live Cursor probe

- Installed Cursor: 3.23.23, arm64, commit 2dac2428994fe34f12658d9ecad1541b98db2c00.
- Surface: Cursor desktop Agents window, This Mac, displayed model Grok 4.7 High.
- Isolated synthetic workspace: `/private/tmp/wk-cursor-capture-probe-20261006`.
- Project-only hooks: sessionStart, beforeSubmitPrompt, afterAgentResponse, stop, beforeReadFile, afterFileEdit, postToolUse. No global hook configuration changed.
- The prompt requested reading only sample.txt, appending CAPTURE-PROBE-OK, and reporting a team marker if present in context. It did not supply the marker.
- A postToolUse event for Read was recorded; its hook returned synthetic additional_context containing ORCHID-742.
- Cursor visibly edited sample.txt and replied CAPTURE-PROBE-OK / ORCHID-742. This verifies hook context delivery in this session, not server-backed retrieval or summary quality.
- A stop event with status completed was recorded. Its transcript_path named the exact test conversation. That file contained the visible user prompt, assistant text, read/edit tool references, final response, and a status record.
- Direct prompt/response events were not present in the probe log. A follow-up plain-text turn produced SECOND-TURN-OK in the UI and transcript without an additional event recorded at inspection time.
- A further THIRD-TURN-OK probe added invocation metadata before scope filtering to distinguish missing invocation from scope rejection. Final observations are appended below.
- Missing events are observations, not a diagnosis of a vendor bug. Loading, surface behavior, settings, and delivery reliability need separate certification.
- The probe did not test cancellation, compaction, process restart, external paths, concurrent/subagent work, native keyring, model summarization, publication, or another developer's retrieval.

## Verified implementation blockers

1. crates/adapters/src/lib.rs rejects codex and antigravity explicitly.
2. crates/agent/src/process.rs only marks a turn complete when all events have agent_id managed-chat. Merely installing native hooks cannot change that.
3. crates/agent/src/daemon.rs groups missing turn identifiers under unattributed-turn. Native clients without the expected turn_id require deliberate correlation, not merging unrelated turns.
4. Current native capture does not reconcile the session-specific transcript supplied by a hook.
5. Message completeness does not prove workspace privacy, summary correctness, or implementation correctness.

## Recommended implementation design (not yet implemented)

Use a separate adapter for each client and surface. Receive documented hooks into a fast authenticated local queue. Bind client version, device, workspace, session, agent and turn. Capture only visible supported fields; exclude hidden reasoning.

On completion, reconcile the exact transcript or supported session API for the already-enrolled session. Persist checkpoints and event identities, handle duplicate delivery, partial writes, restarts, truncation and source edits. Never discover chats by scanning all user history. A hook-supplied path is untrusted: validate source location, session identity, file ownership, symlinks and approved-workspace attribution before reading.

An already-bound transcript can be watched to detect missed hooks, subject to consent and retention. This cannot discover an entirely missed session safely; show coverage unavailable until a trustworthy session binding exists.

Represent visible-message completeness, workspace-scope confidence and publication eligibility separately. Replace the managed-chat-only rule with tested adapter evidence, not an unconditional complete flag. Aborted, mixed-scope or ambiguous sessions stay held. Revalidate membership before publication and keep automatic publication disabled until privacy evaluation passes.

Inject small authorized relevant summaries at supported entry points. Keep retrieved content clearly marked as reference data. Use file-aware context where possible, MCP for explicit lookup, and acknowledge that context delivery does not guarantee model compliance.

Acceptance: on each exact supported client version, demonstrate normal turns, tool edits, no-tool turns, cancellation, restart/resume, compaction, duplicate callbacks, concurrent sessions, scope escape, revocation, secret-like synthetic inputs and server outage. Then demonstrate developer A's reviewed card informing developer B's task without exposing A's raw conversation.

## Current official integration paths

- Cursor: hooks include prompt/response, tool, stop, transcript_path and additional_context on supported events: https://cursor.com/docs/hooks
- Claude Code: UserPromptSubmit, Stop/StopFailure, transcript_path and last_assistant_message: https://code.claude.com/docs/en/hooks
- Gemini CLI: BeforeAgent and AfterAgent; AfterAgent includes prompt and prompt_response: https://geminicli.com/docs/hooks/reference/
- Codex: installed CLI 0.160.1 reports hooks stable/enabled. Official UserPromptSubmit/Stop expose turn_id and visible prompt/final text; transcript format is explicitly not stable: https://learn.chatgpt.com/docs/hooks
- Antigravity: official hooks support workspacePaths, conversationId, transcriptPath, Stop and invocation context injection. Desktop/IDE/CLI have distinct data directories: https://antigravity.google/docs/hooks

Documentation support is not live certification. Only the limited Cursor desktop probe above was run.

## Final probe observations

All three synthetic turns completed in the Cursor UI. The named transcript contained all three user prompts and final responses. A restricted visible-text parser recovered those three turns successfully. Only two hook events were recorded overall (Read postToolUse and first-turn stop); the extra invocation-metadata file was not created during the third turn. Therefore no third-turn invocation reached the instrumented probe, rather than merely being rejected by its workspace filter. This is not sufficient to identify why Cursor omitted it.

The project-only hooks configuration was renamed to hooks.probe-disabled.json after the test. Synthetic evidence remains in the temporary probe directory; this report is the durable findings record. No production application code or global client configuration changed.

### Proposed first implementation slice

Start with Cursor desktop 3.23.23 on macOS. Add exact-session transcript binding and incremental reconciliation, correct turn correlation and completeness evidence, and a coverage status that detects stale/missing capture. Reuse the existing transcript parser after extracting a narrow single-source entry point; do not run the broad history importer as a live collector. Keep local raw retention and reviewed publication. Verify a second developer retrieving an approved card and seeing an overlap notice. Fix the previously demonstrated activity-scope and correction-screening defects before a real-data team pilot.

This is an investigation and proposed design, not an implementation approval or a production certification.
