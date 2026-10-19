# Publishable assistant-written cards (design)

**Status: proposal for the owner. Nothing is implemented.** Product and privacy decision; the recommendation is in [Recommendation](#recommendation) and the choices that are the owner's are in [Decisions for the owner](#decisions-for-the-owner).

## Problem

[Setup](../../adapters/setup.md#summarize-with-your-assistant-wk-summarize) recommends `/wk-summarize` (`wk-agent draft`) as the default way to write a card. The [quality comparison](../../reviews/2026-10-19-summary-quality-comparison.md) found these cards are the best of the three modes, but they cannot become team knowledge:

- `assistant_draft::build` sets `capture_completeness: partial` (`crates/agent/src/assistant_draft.rs:91`).
- `Knowledge::publish` only selects cards whose `body->>'capture_completeness' = 'complete'` (`crates/knowledge/src/lib.rs:306`); anything else is `Denied`.
- The portal disables Publish for anything but `complete` (`web/src/App.tsx`, `Detail`), and "Correct summary" resends `...card.body`, so it keeps `partial`.
- Search, semantic retrieval, teammate overlap notices (`activity.rs`, `prompt_context.rs`) and the decisions API read only `published` cards.

So the recommended flow ends in the review queue: a developer writes, screens and reviews a card, a reviewer sees it, and nobody can publish it. No teammate ever gets a notice from it.

## What exists today (facts the design must respect)

- **`partial` means "the collector did not capture the whole session"**: hook gaps, imports, native turns that did not end, rule drafts (`rules_draft::complete` is always false). `truncated` is a stronger version. The portal warns on both.
- **The server does not verify completeness.** `CardSubmission` carries it, `submit_in_transaction` stores it, and any client that posts `complete` is believed. The gate is therefore a *reviewer-facing signal enforced by honest clients*, not a security boundary against the submitter. The real controls on publication are: membership and `can_review`, server-side `sensitive` screening at submit, correct and publish (`validate_shared_body`), and the human who clicks Publish.
- **Nothing records "the author reviewed this".** `submit --reviewed` is a CLI flag; the server never sees it.
- **Assistant cards are screened by refusal, not masking.** `build` rejects a card that `sensitive_reason` flags (credential keywords, email, phone, street address, prompt-injection markers, opaque tokens, home paths) or that the history masker would change. Rule and model drafts are masked instead.
- **Owner sync** uploads only `complete` drafts (`personal_sync::payload`), and a test pins that assistant drafts are therefore never auto-uploaded.
- **Policy-auto** (`publication_mode: policy_auto` in the portal) has no server implementation yet; the docs say it is disabled until the privacy corpus passes, and rule drafts are never eligible.
- `capture_completeness` is not indexed and there is no column for it; it lives in the `cards.body` JSONB. A change to the publish gate needs **no SQL migration**.

## Options

| # | Option | For | Against |
|---|---|---|---|
| A | New `Completeness` value (e.g. `author_reviewed`) | One field, trivial gate | Changes what the field means: it describes capture coverage, and an assistant card was not captured at all. Overloads one enum with two axes (coverage and who wrote it). `truncated`/`partial` semantics get muddier. |
| B | New optional `provenance` field; `partial` keeps its meaning; gate is `complete` OR (`partial` AND provenance `assistant_written`) | `partial` stays literally true (nothing verified the whole session was captured). Rule drafts, hook gaps and imports stay unpublishable with no code touching them. Extensible (later: `person`). | New contract field, schema snapshot, old-server skew. Provenance is self-declared (same trust as `complete` today). |
| C | Reviewer promotion: publish accepts `acknowledge_partial: true`, portal adds a confirm box | No contract change; no provenance to trust | Makes every `partial` card publishable by a click, including rule drafts and real capture gaps, so `partial` stops meaning anything. Click-through fatigue. Cannot keep rule drafts out without knowing the source, which is B's field. |
| D | Keep assistant cards unpublishable; tell users to use My context or `complete` model drafts | Zero change | The recommended default produces no team knowledge; contradicts the setup guide's premise. |
| E | Per-product operator switch ("allow assistant-written cards to be published") on `ProductPolicy` | Gives an operator a privacy lever | New admin API, policy schema and portal UI before anyone has asked. Reviewer role already gates publication. |

## Recommendation

**B, without a blocking acknowledgement and without E (for now).**

1. Add `provenance: Option<Provenance>` to `CardSubmission` with one variant, `assistant_written`, omitted from the wire when absent.
2. `Knowledge::publish` accepts `complete`, or `partial` with `provenance = assistant_written`. `truncated` never publishes. Everything else about publish is unchanged (reviewer, service scope, policy revision, withdrawn, re-screen).
3. Rule drafts, native/hook partials, imports and memory drafts never carry provenance, so they remain unpublishable exactly as today. Rule drafts stay out of team knowledge on purpose: they quote the conversation.
4. The portal shows **who wrote it** and a non-blocking notice, instead of a blocking click-through (see [Portal](#portal)).

Why not A or C: both blur the existing meaning of `partial`. B adds the missing fact ("the author's assistant wrote this and the author reviewed it before submitting") on its own axis and lets `partial` keep saying "capture was not verified complete", which is true of every assistant card.

Why a human reviewer stays in the loop: the author's review cannot be verified by the server, and an assistant can state something the session did not support. A second look at publish time (the existing `can_review` step; the same person may hold both roles on a small team) is the only check that is not self-declared.

## Design

### Contract (`crates/contracts`)

```rust
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Provenance { AssistantWritten }
// CardSubmission
#[serde(default, skip_serializing_if = "Option::is_none")]
pub provenance: Option<Provenance>,
```

- `validate`: reject `provenance` with `capture_completeness = truncated`.
- `schema_version` stays 1: the field is additive and optional, so existing cards serialize byte-for-byte the same. Regenerate `docs/architecture/schemas/card-submission-v1.json` (`schema_snapshot.rs` fails until it is) and add one line to `docs/architecture/schemas/README.md`.
- `CardSubmission` is `deny_unknown_fields`, so an **older server rejects a card that carries `provenance`**. Handled in the collector (next section), not by loosening the contract.

### Collector (`crates/agent`)

- `assistant_draft::build` sets `provenance: Some(AssistantWritten)`, keeps `Completeness::Partial`, and bumps the digest tag `"assistant-v1"` to `"assistant-v2"` so a card resubmitted after an upgrade gets a new idempotency key instead of conflicting with the old row (`submit_in_transaction` returns `Conflict` when the same key has a different body). A card already in the review queue without provenance stays unpublishable; the author withdraws it and runs `/wk-summarize` again.
- Rule, model, import and memory drafts are untouched; a test pins that none of them sets provenance.
- Old-server fallback: `GET /v1/policy` gains `"features":["card_provenance"]` (server, additive). At `submit`, if the server did not advertise it, the collector drops `provenance` from the card (it remains the `partial` card it is today) and says so in the output, rather than failing with a generic "invalid". The check reuses the policy fetch `deliver` already makes.
- `personal_sync::payload` is unchanged and still refuses `partial`, so assistant cards are still never auto-uploaded to the owner store. The comment in `assistant_draft.rs` stays true.

### Server (`crates/knowledge`, `crates/server`)

- `publish`: replace the one predicate with `(completeness = 'complete' OR (completeness = 'partial' AND provenance = 'assistant_written'))` (JSONB `->>`; absent provenance is `NULL`, so old rows behave as before). Keep `validate_shared_body` at publish.
- `correct` (`lifecycle.rs`): the new body's `provenance` must equal the stored one, else `Invalid`. A reviewer correction keeps the origin label and unpublishes the card as it does today; a reviewer cannot add or remove provenance. (A reviewer rewriting a rule draft into a publishable card is a reasonable later feature; it is not part of this change.)
- `decisions.rs` adds `"provenance"` next to `"coverage"` so an agent consuming the decisions API can discount assistant-written cards.
- Graph write at publish already records `source_boundary: "reviewed summary"`, `coverage: "unknown"` and "summary is not proof of implementation"; no change.
- Never eligible for policy-auto. Whenever policy-auto is built, it must require `complete` and not provenance. State this in the policy-auto spec when written.

### Portal (`web/src`)

- `types.ts`: `Body.provenance?: string`.
- Publish enabled when `complete` or provenance `assistant_written` (still disabled for rule drafts and `truncated`).
- Detail: a "Source" row ("Written by the author's assistant; submitted after the author's review") and, instead of the amber "Partial capture" line, nothing for assistant cards whose only gap is that they were never captured. List rows show "Assistant-written" instead of "Partial capture".
- Above Publish on an assistant card, a plain notice (not a checkbox): *"Written by an assistant and not checked against the conversation. Read it for accuracy and for anything the team should not see. Publishing makes it searchable by everyone with access to these services and can appear in teammates' assistant context."* A blocking acknowledgement is an owner choice (see decisions).

### Privacy

- **What publishing adds.** Today a submitted assistant card is visible to reviewers. Published, it is visible to every member of its services, searchable (full-text and, if configured, embedded), and can be surfaced in teammate prompts as overlap notices. That wider audience is the real decision.
- **Free text is the exposure.** Assistant text is stored as written. The existing controls are fail-closed (refuse on any hit) and run three times: at `draft`, at submit, and again at publish. They are pattern-based (documented as not complete PII detection), so the notice above and the reviewer read are part of the control, and the setup guide must keep saying so.
- **No raw conversation reaches the server through this path.** Unchanged: the card is six short strings, 64 KiB at most, no evidence, no code.
- **Provenance is self-declared.** A modified client can already post `complete`; B adds no capability the submitter lacked. It does not make rule drafts publishable (they carry no provenance), and a rule draft relabelled by a modified client still has to pass reviewer, screening and membership checks.
- **Evidence gap.** Assistant cards have no `file_observation` evidence, so their only effect on teammates is prompt-time notices (0.81 recall / 0.40 precision on the synthetic set); edit-time notices need file evidence and stay at 0 for these cards. Both figures come from the comparison's counterfactual run on 7 related sessions; treat them as direction, not a level. Attaching changed-file paths is a separate change with its own path-exposure review; do not bundle it.

### Compatibility and rollout

- Order: server first (accepts the field and advertises `card_provenance`), then collectors. An old collector never sends the field; a new collector against an old server falls back as above.
- No SQL migration, no backfill. Existing `partial` cards stay unpublishable.
- Public wire change is additive; note it under "Unreleased" in `CHANGELOG.md`.

### Docs and wording

- `docs/adapters/setup.md`: rewrite the two statements "assistant drafts are partial … cannot be published" (the `draft` section and "Reviewers publish complete approved summaries"): assistant-written cards are `partial` (never captured) and **can** be published by a reviewer; rule drafts, hook partials and imports cannot. Keep the owner-sync sentence (still true). Keep "Regex screening … is not proof of perfect privacy" next to the publish description.
- README quick start step 5 caveat and the rule-draft bullet: remove "cannot be published" for the assistant flow only.
- Quality comparison, "Finding: partial cards cannot be published" and follow-up 1: link to this spec; update once shipped.

## Test plan

**Contracts:** schema snapshot regenerated and passing; `provenance` omitted from JSON when `None` (old-server byte compatibility); unknown variant rejected; `provenance` + `truncated` rejected by `validate`.

**Knowledge (`crates/knowledge/tests`, Postgres):**
- reviewer can publish `partial` + `assistant_written`; the card then appears in `search`, semantic retrieval and another member's overlap/activity results (not the author's own);
- `partial` without provenance, `truncated` with provenance, a non-reviewer, a stale policy revision, an out-of-scope service, a withdrawn card: all `Denied`/`Invalid`, unchanged;
- text that `sensitive` flags is refused at publish even if it was clean at submit (edited row);
- `correct` keeps provenance and unpublishes; `correct` that adds, removes or changes provenance is `Invalid`;
- existing `complete` publish tests unchanged and green.

**Server (`crates/server/tests`):** `/v1/cards` accepts provenance; `/v1/policy` advertises `card_provenance`; `/publish` end to end; `decisions` shows `provenance`.

**Agent (`crates/agent`):** `build` sets provenance and the v2 digest; rules, model, import and memory drafts never set it; `personal_sync::payload` still refuses assistant cards; with a server that does not advertise the feature the submitted body has no `provenance` and the output says so; old queued v1 cards still submit.

**Portal (Playwright, `web/tests`):** Publish enabled for an assistant card and disabled for a rule draft and a `truncated` card; the notice and Source row render; correction keeps the label.

**Eval:** the overlap numbers in the comparison are a counterfactual (`summary-quality-eval.py` relabels cards `complete` before publishing them, because as shipped none can be published). After this change, run the script with the real cards (provenance set, completeness `partial`, no relabelling) against a disposable `wk_test` database and confirm the prompt-time numbers are reproduced; this is the first end-to-end proof that the default flow produces a teammate notice. Add an `as-assistant` variant to the script and keep `as-shipped` for rule drafts (still 0 published).

**Repo gates:** `cargo fmt`, `clippy -D warnings`, workspace tests, `python3 -m unittest discover` (including `test_release_docs.py` for the changed links).

## Decisions for the owner

1. **Model.** B (provenance field, `partial` unchanged) as recommended, or A (new completeness value) if one field matters more than clean meaning.
2. **Acknowledgement.** Non-blocking notice (recommended) or a blocking "I have read this" checkbox on publish for assistant cards.
3. **Operator switch (E).** Not now (recommended), or add a per-product setting that defaults to off until an operator enables it. If your pilot's privacy owner needs a lever before the first publish, take E in this change.
4. **Rule drafts.** Stay unpublishable (recommended). Revisit only with a reviewer-rewrite path.
5. **Reviewer = author.** Today a member with `can_review` can publish their own card. Allow it for assistant cards (recommended; matches `complete` cards and small teams) or require a different reviewer.

## Out of scope

Changing `Completeness` meaning or values; file evidence for assistant cards; reviewer rewrite of rule drafts; policy-auto; automatic summarisation or reminders to run `/wk-summarize`; any change to owner sync.
