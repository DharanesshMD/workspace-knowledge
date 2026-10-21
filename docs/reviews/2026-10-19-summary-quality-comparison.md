# Summary quality: rules, assistant-written and local-model drafts (2026-10-19)

**Status: measured on 20 synthetic turns only. Read the [limits](#limits) before quoting a number.** This is the evidence behind recommending the assistant-written flow ([`/wk-summarize`](../adapters/setup.md#summarize-with-your-assistant-wk-summarize)) as the default, and behind the rule-draft fixes listed below. No real conversation, person, company or repository was used.

## Question

The adoption assessment of 2026-10-19 found that default summaries are rule-based drafts that quote the conversation. How much better are the other two ways of writing a card, and what are the worst failures of the rule drafts?

## Method

- **Fixture.** [`scripts/fixtures/summary-quality.json`](../../scripts/fixtures/summary-quality.json): 20 invented turns of a made-up payments and checkout project, ten for "Developer A" and ten for "Developer B" (the second ten overlap the first ten's files in some cases and not in others). Each turn has a prompt, a reply, the files changed, the stems a reader would need to recognise the topic (`topic`) and the reasons behind the work (`why`). The turns are deliberately messy in ways real transcripts often are: greetings before the request (2), a pasted trace before the request (2), a pasted code fence before the request (2), bare confirmations such as "ok do it" (3), a reply that ends with a courtesy (3, one of them also a greeting turn), a long reply whose summary comes last (2), a reply that is only questions (1) and clean prompts (6).
- **Rules drafts.** The real `rules_draft::from_shape`, exactly as `wk-agent process` calls it, on the turn's first prompt and last reply.
- **Local-model drafts.** The real `wk_summarize::extract` against Ollama `gemma4:e2b-mlx` (native `/api/chat`, schema-enforced, temperature 0), one run, on the same `user_prompt: … / assistant_response: …` source the journal path builds.
- **Assistant-written drafts.** One card per turn, written by Claude in this session following the card shape that `wk-agent prompt` asks for (full-sentence decisions and constraints, no code, no secrets), then passed through the real `assistant_draft::build` (strict fields, tidy, screening, masking). All 20 were accepted.
- **Text measures** ([`scripts/summary-quality-eval.py`](../../scripts/summary-quality-eval.py)): whether the title contains a topic stem, whether the title is junk (greeting, confirmation, pasted trace, code fence), how many reason stems appear in the card, whether it has decisions or constraints, whether courtesies leak into the title, intent or rationale, and card length.
- **Overlap measure.** The unmodified [`scripts/history-eval.py`](../../scripts/history-eval.py): Developer A's cards are loaded and published into a disposable local PostgreSQL `wk_test` database and a local dev server, then Developer B's prompt (and, separately, B's changed files) is checked for notices about A's related cards. Ground truth is A cards sharing at least one changed file with B's turn. Runs were repeated with and without file evidence on the cards.

## Results

### Text measures (20 turns each; share of cards unless noted)

| Measure | Rules **before** the fixes | Rules **after** | Assistant-written | Local model |
|---|---|---|---|---|
| Cards made / held | 20 / 0 | 20 / 0 | 20 / 0 | 20 / 0 |
| Title names the work | 60% | **95%** | 100% | 95% |
| Junk title (greeting, "ok do it", trace, ```) | **45%** | 0% | 0% | 0% |
| Courtesy in title, intent or rationale | 25% | 0% | 0% | 0% |
| Reason stems found anywhere in the card | 97% | 100% | 91% | 95% |
| Reason stems found in the rationale | 96% | 99% | 75% | 42% |
| Has at least one decision or constraint | **0%** | **0%** | 100% | 100% |
| Topic stems in title or intent | 69% | 77% | 89% | 80% |
| Mean card length (characters) | 602 | 588 | 466 | 625 |

How to read it. Rules drafts carry the reasons because they quote the reply, so the reason measure favours quoting (and the stems were taken from the replies, so it flatters them most). What rules drafts cannot do is state a decision or a constraint: those fields are always empty, which is by design. Assistant-written cards were the shortest and the only ones to score at the top on titles and topic. The local model made good short titles but put most of the reasons into the decision list rather than the rationale.

### Overlap notices (`history-eval.py`; 7 of B's 10 turns have a related A card)

| Cards | Prompt-time recall / precision | Edit-time recall / precision |
|---|---|---|
| Rules, before the fixes | 0.81 / 0.27 | 1.00 / 0.49 with file evidence, 0.00 without |
| Rules, after the fixes | 0.81 / 0.32 | same |
| Assistant-written | 0.81 / 0.40 | same |
| Local model | 0.67 / 0.38 | same |
| Existing all-words search on the request (baseline, same data) | 0.00 / 0.00 | n/a |

Prompt-time numbers use only the card text, so they are the same with or without file evidence. Edit-time notices match on changed files, so they work only when the card carries file evidence (today that means Cursor turns) and are identical for every mode. With seven related sessions one session is worth 0.14 of recall, so **the differences between modes here are not distinguishable from noise**. The only clear results are that file evidence decides edit-time notices and that the card text enables prompt-time ones at all.

### Finding: partial cards cannot be published

Rules drafts and assistant-written drafts are `partial`. `history-eval.py` could not publish a single one of them (0 of 10 published for both, against 10 of 10 for the local model's `complete` cards), and the overlap rows above for those modes are therefore a counterfactual in which the same cards are relabelled `complete`. As shipped, they would produce no overlap notices at all, because only published cards are notified about.

This is not a harness artefact. The server's `publish` accepts only cards whose `capture_completeness` is `complete` (`crates/knowledge/src/lib.rs`), the portal's Publish button is disabled for anything else, and "Correct this summary" keeps the card's completeness. So a submitted assistant-written or rule draft reaches a reviewer as a private review copy that nobody can publish into shared team knowledge. [Overlap notices](../architecture/overlap-notices.md) records the same effect for history-derived cards. Whether a developer-reviewed assistant card may be marked `complete` (or a reviewer may promote a partial card) is a product and privacy decision, not something this work changes. It is the main thing standing between "good summaries" and "shared team knowledge", and it is listed under [follow-ups](#follow-ups).

### Worst rule-draft failures and the fixes

All fixes are in `crates/agent/src/rules_draft.rs`, each written as a failing test first (`greetings_before_the_request_…`, `pasted_traces_and_code_fences_…`, `a_bare_confirmation_is_titled_from_the_reply`, `trailing_courtesies_are_dropped_…`, `a_long_reply_keeps_its_summary_…`):

| Failure (share of the fixture) | Before | After |
|---|---|---|
| Greeting first: title was "Hey! Quick one for you." (2) | greeting is the card title and the start of the goal | skipped; the request line is the title and the goal starts there |
| Pasted trace or code fence first: title was "Traceback (most recent call last):" or "```" (4) | junk title | trace, indented code, fenced blocks skipped |
| Bare confirmation: title was "ok do it", "continue" (3) | the confirmation is the title | title and goal come from the reply's first meaningful sentence (a leading "Done." is skipped) |
| Closing courtesy quoted as the outcome: "Hope this helps! Let me know …" (3) | courtesy ends the rationale | trailing courtesy paragraphs dropped (never the whole reply) |
| Summary after 1,500 characters (1) | rationale cut off before the summary | when a reply is too long to quote, it starts at its "In summary" / "TL;DR" / "Summary" paragraph |

Everything else about rule drafts is unchanged: still `[auto-drafted]`, still `partial`, still masked and held when sensitive.

Remaining weak spots seen in the output: a question as the title when the request is a question ("why does login fail …?"); a title like "the mobile build crashes on login with this" when the thing "this" points at was a skipped trace; a skipped code fence loses the diff it pasted; a mid-reply "feel free to" is kept; no decisions or constraints, ever. A held draft is reported as a hold, not as an error: the word "secret" in an ordinary sentence held one of the first fixture drafts for all three modes (the fixture sentence was reworded, but conservative keyword holds will do this in real use).

## Limits

- **Synthetic, small, written by the same author.** The turns, the topic and reason stems, and the assistant-written cards all came from one author. The failure patterns in the fixture were chosen because they are plausible, not sampled from real transcripts, so the table shows that the fixes work on them, not how often they occur. The "after" numbers were measured on the same fixture the failures were found on; a different or larger set could expose other failures.
- **The assistant-written cards are not blind and not typical.** The same session wrote the fixture's stems and the cards, and a person using `/wk-summarize` gets a card from whatever model and context their assistant has. Treat the assistant row as an upper-ish bound on what the flow can do, not an expected average.
- **One local model, one run, a small one.** `gemma4:e2b-mlx` is a small model; the setup guide does not require it. `qwen3.5:2b` was installed but not measured.
- **Stem matching is crude.** A card can contain a stem and still be misleading; no human rated any card. No test checked faithfulness (whether a card says anything the conversation did not).
- **Each turn is one card.** `/wk-summarize` summarises a session and rules and local drafts are per turn, so the unit differs in real use.
- **Overlap sample is tiny** (7 related sessions) and the non-text part is dominated by file evidence. No claim of a recall or precision level for real teams.
- **Cost, speed and effort were not measured.** Rules drafts appear in the background on their own; the assistant flow needs the developer to run `/wk-summarize` and the assistant to spend tokens, and an unrun command produces no card. Nothing here measures how often people run it.
- **Privacy.** All input was invented. The assistant flow sends nothing new to anyone, a local model stays on the machine, and rule drafts quote the conversation (masked, held when sensitive); none of that was re-tested here beyond the existing screening that `draft` and the rules path already apply.

## Follow-ups

1. Decide how a reviewed assistant-written card can become publishable (or confirm that it should not), because as shipped it cannot.
2. Re-run this comparison on a consented, pseudonymised set from the first pilot team, with a human rating for faithfulness and usefulness, and with the assistant flow run by the people who will actually use it.
3. Measure how often developers run `/wk-summarize`; if rarely, consider a reminder rather than automatic summarisation.

## Reproduce

```sh
# 1. Write the three kinds of draft for the synthetic fixture (a throwaway file outside the repository).
WK_EVAL_FIXTURE=$PWD/scripts/fixtures/summary-quality.json WK_EVAL_OUT=/private/dir/drafts.json \
WK_EVAL_INFERENCE=http://127.0.0.1:11434/api/chat WK_EVAL_MODEL=gemma4:e2b-mlx \
  ./scripts/cargo test -p wk-agent dump_drafts_for_summary_quality_eval -- --ignored
# 2. Score them; with a disposable wk_test database this also runs history-eval.py for every mode.
python3 scripts/summary-quality-eval.py --drafts /private/dir/drafts.json --out /private/dir/result.json \
  --database postgres://USER@127.0.0.1:PORT/wk_test
```

Omit `WK_EVAL_INFERENCE` to skip the local model, and omit `--database` to skip the overlap part. The script drops and recreates `wk_test` for every run and refuses any other database name. The same measurement on the unfixed rules code needs the pre-fix `rules_draft.rs`; the "before" column above was produced that way.
