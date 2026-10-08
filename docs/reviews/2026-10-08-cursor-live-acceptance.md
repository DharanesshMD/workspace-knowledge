# Cursor live acceptance — 2026-10-08

Tested implementation: `c895092`, Cursor 3.23.23, macOS Agents window with This Mac execution. Only the owned synthetic workspace `/private/tmp/wk-cursor-native-live-20261007` and disposable PostgreSQL `wk_test` were used. These results extend the [implementation report](2026-10-06-cursor-native-capture-results.md); they do not certify other client versions, all Cursor surfaces, or unattended deployment.

## Results

| Check | Observed result |
| --- | --- |
| Native read turn | Cursor used Read on `backend/retry.ts`. Five callbacks reached the collector without gaps or scope holds; one complete turn recovered. |
| Local inference | `qwen3.5:2b` produced a private draft containing the idempotency requirement, limit of four attempts, pending-review status, proposed lifecycle and relative file evidence. |
| Review/publication | The test reviewer removed operational tool instructions, an inferred rationale and unnecessary wording before publication. The reviewed card was revision 2. No rejected 0.8B draft was published. |
| Teammate handoff | A second synthetic developer identity retrieved that live-derived card through the actual remote MCP endpoint. The file-overlap API returned its ID with revision 2. This was a second authenticated API identity, not a second person's desktop or live hook-injection demonstration. |
| Collector restart | A real collector process restart preserved the existing draft exactly. Reprocessing did not duplicate it. |
| No-tool continuation | After restart, a further turn in the original Cursor conversation recovered and generated a distinct private candidate. Doctor reported three complete turns across two watching sources. |
| Cancellation | Cancelling an early generation produced an orphan terminal and safely held that source. Cursor returned the UI to a draft. Resuming allocated a fresh source, which captured successfully; the original held source stayed closed. Mid-stream resume within the same source is not established by this case. |
| Expiry across pause | During a roughly five-hour pause, private drafts expired. Existing source checkpoints remained within their 24-hour lifetime, and guards prevented recreating expired candidates. |
| Revocation during inference | A loopback test proxy forwarded the actual request to the real local model, confirmed it was still in flight, and removed the synthetic developer's database membership. The real response returned after about 9.39 seconds. The collector retained no new draft, made only one proxied inference request, held all three sources and cleared all recovered turns. |

The revocation test did not cancel a request already sent to the model; it proved that the returned result was not retained and that subsequent source processing stopped. No pending private drafts remained at that point because of the intervening expiry. Pending-draft submission rejection remains covered by the earlier synthetic integration test, not by a new live pending-draft attempt in this run.

## Model findings

The official [Ollama Qwen3.5 library](https://ollama.com/library/qwen3.5) provides the smaller model sizes used here. Both models ran locally with cloud inference disabled.

- `qwen3.5:0.8b` was fast (about five seconds), but changed “pending review” into “rejected,” invented an open question, and included summarizer instructions in another draft. It failed this quality check and was removed after testing.
- `qwen3.5:2b` preserved the main requirement and uncertainty. A warm summary completed in about 4.2 seconds. Ollama reported about 2.37 GB of loaded model memory; that is neither whole-system RAM consumption nor a peak-memory benchmark. The in-flight revocation case took about 9.3 seconds for the model response.
- The 2B output still required review: operational instructions were treated as project constraints, and a rationale was inferred without explicit source support. This single positive example is not a general quality certification or a reason to enable automatic publication. A broader privacy/meaning-preservation corpus remains necessary.

The 2B model is left installed for future tests. The Ollama process started for this test was stopped and the model is not left running. No production default was changed.

## Safety and cleanup

The test API, collector, revocation proxy and database container were stopped. The owned project's hooks were disabled, temporary OS Keychain items and encrypted collector state were removed, and no global hooks were installed. Published cards and remaining database contents are synthetic and retained only in the stopped disposable database container.

Content-limited local evidence is retained under `.superpowers/sdd/2026-10-06-cursor-native-capture/`: `live-08-generated-card.json`, `live-08-reviewed-card.json`, `live-08-handoff.json`, `live-08-after-continuation*.json`, `live-08-model-memory.json`, `live-08-revocation-result.json`, and `live-08-final-doctor.json`. These are test summaries/diagnostics, not real developer transcripts or credentials.

## Remaining release gates

The controlled live capture → local draft → reviewer correction → publication → teammate MCP/overlap path is now demonstrated. Restart continuity, no-tool continuation and in-flight revocation are also demonstrated within the scope above.

Still required before production rollout: signed/notarized macOS packaging and unattended Keychain behavior; representative summary-quality/privacy evaluation; five-developer load/storage/capture-loss measurements; broader cancellation/resume and unsupported-context coverage; other platforms and client/version certification. Doctor's `live_certified: false` remains intentional. Native Codex/Antigravity adapters and broader Claude Code/Gemini capture were not changed by this acceptance work.
