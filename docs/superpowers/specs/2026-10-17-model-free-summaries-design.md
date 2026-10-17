# Summaries without a local model

Approved intent (2026-10-17): developers should not have to install or run a model to use Workspace Knowledge. The decision was driven by setup and hardware burden. Summaries come from the developer's own assistant when possible and from deterministic rules otherwise; a local model stays available as an optional mode. Search is a separate project (better keyword, tag and file-link ranking) and is not part of this spec; semantic search is already optional and unchanged.

## Current state

`wk-agent init` and `device connect` take a loopback `--inference` URL (defaulting to an Ollama address) that must pass `wk_summarize::local_url`, and `wk-agent process` calls `wk_summarize::extract` (a local chat model) at five places: `process.rs` journal turns and native turns, `native_process.rs`, `memory_import.rs` `draft_all`, and the CLI `cursor-history cards`. `wk-agent chat` also needs the model. `knowledge.record` (MCP) lets an assistant submit a card, but it posts straight to the team server as a draft, skipping the developer's private review.

## Summarizer setting

`state::Config` gains `summarizer`: `"local"` or `"rules"` (serde default `"local"`, so existing configs behave exactly as before). `wk-agent init` and `wk-agent device connect` accept `--summarizer local|rules` through one function, `state::resolve_summarizer`; with no `--inference` it defaults to `rules` and stores empty `inference`/`model` (`--model` is ignored). With `--inference` it defaults to `local` and validates the URL as today. Contradictory flags are errors (`--summarizer rules` with `--inference`; `--summarizer local` without it). Both commands used to default `--inference` to an Ollama URL, so omitting it now selects `rules`; this is a deliberate behaviour change. `process` uses the setting: `local` calls the model, `rules` builds deterministic drafts (below). `wk-agent chat` requires `local` and fails with a clear message otherwise. `doctor` reports the mode as `summarizer`.

Deviations from the first draft of this section, found while building it:

- **Serialization.** A `local` config is written without the `summarizer` key (`skip_serializing_if`), so older binaries, whose `Config` rejects unknown fields, still read it. A `rules` config carries the key and cannot be read by older binaries. The device receipt hash of a `local` config is unchanged for the same reason.
- **Device policy.** `rules` skips only the destination check (`check_inference`): nothing is sent to a model, so the product's inference restrictions do not apply. A hosted device configuration still needs a well-formed signed policy with a `restrictions` field, as before.
- **Consent binds the summarizer.** Owner-sync consent binds `model`, `inference` and now `summarizer`. Consent files written before the setting existed carry no summarizer and stay valid for `local`; they never match a `rules` setup. Changing the setting requires a fresh `init` in a new state directory (the existing "refusing credential rotation" rule) and new owner-sync consent.

## Assistant-written drafts

`wk-agent draft` reads one JSON object from standard input (limit 64 KiB, strict `deny_unknown_fields`): `title`, `intent`, `rationale`, `decisions[]`, `constraints[]`, `open_questions[]`. It fills the rest of the card itself: `service_ids` from the configured service, `policy_revision` from the server policy, `lifecycle: proposed`, `capture_completeness: partial`, no evidence, and an `idempotency_key` that is the SHA-256 of the six fields, the service, the policy hash and revision and a version tag, so submitting the same card twice does not duplicate it on the server. List fields pass `wk_summarize::tidy`, which drops items under 8 characters or a single word, statements of absence ("none", "n/a") and case-insensitive duplicates, so the prompt asks for full short sentences. `--client` is a label (`[a-z0-9-]{1,32}`, default `other`) stored in the draft as `assistant.client`. The card passes the same schema validation and `wk_summarize::sensitive` screening as every other draft; a sensitive card is refused with a fixed message and never echoed.

The card is stored as a **private local draft** in the encrypted store, with `{"card":…, "assistant":{}, "policy_hash":…, "model":"assistant"}` and a 24-hour retention (the store maximum), so it appears in `wk-agent drafts` and is submitted with the existing `submit --reviewed` path. The first local id of a card is its digest. The store never lets a submitted or expired id be created again, so drafting a card again after submission would otherwise fail: later copies get deterministic counter ids (`draft:` plus the hash of `digest:n`, at most 64 copies), a still-pending copy is reused, and the server dedupes the resubmission by `idempotency_key`. A new `assistant` branch in `deliver` mirrors the memory branch: it rechecks `policy_hash` and `policy_revision` (consuming the draft on mismatch), rechecks sensitivity, posts, and acks on a team submit; owner-only sync leaves it pending (assistant drafts are partial, and owner sync uploads only complete drafts, so they need an explicit `submit`). Nothing reaches the team server until the developer reviews and submits. The existing `knowledge.record` tool is unchanged.

`wk-agent prompt --client claude-code|copilot|codex` prints a short instruction text, which the docs tell developers to save where their tool expects commands: a Claude Code slash command (`.claude/commands/wk-summarize.md`), a VS Code Copilot prompt file (`.github/prompts/wk-summarize.prompt.md`) and a Codex instructions snippet. The text asks the assistant to summarise the session into the card fields, to include no secrets, credentials or raw conversation text, and to pipe the JSON to `wk-agent --state-dir … draft`. It is developer-initiated; there are no new hooks and no automatic token spend. The command prints only; it writes no files. It prints JSON `{client, text, save_as}`; the text embeds the absolute path of `wk-agent` and the state directory (shell-quoted), and the Claude Code and Copilot texts carry the frontmatter their tool expects (Copilot prompt files use `agent: agent`). `--raw` prints only the text so it can be redirected to a file. The text shows the card going in through a quoted heredoc and tells the assistant to report a refusal instead of rewording the card; paths are quoted for POSIX shells. The Codex text applies only when the developer asks, so it is safe in `AGENTS.md`.

## Rule-based drafts

With `summarizer: rules`, `process` builds drafts without a model for live captured turns, native turns, history-import parts and memory sections. One function builds the card fields from a small structure (first prompt, last reply, optional files, or a note's heading and body) so all five call sites share it:

- **Exchanges** (captured or imported): `title` is the first prompt's first non-empty line, `intent` the first prompt clipped to 400 characters, `rationale` the last assistant reply clipped to 1,500 characters (clipped text ends in `…`); `decisions`, `constraints` and `open_questions` are empty. A part with several exchanges uses the first prompt and the last reply of the part.
- **Notes and instruction sections:** `title` and `intent` are the heading path (or the note's name when it has no heading) and `rationale` is the section text clipped to 1,500 characters; a note with no text is not drafted. A frontmatter hint (such as a Copilot instruction file's `description`) is put in front of the section text, where the model path would have received it as a hint.
- **Files:** `evidence` lists file paths only from sources that already provide them, which today means Cursor turns (`files_changed`); native Codex capture attaches none even in `local` mode. The Claude Code, Codex and Copilot history readers do not read tool calls, so imported sessions from those clients carry no files; reading file paths from tool-call arguments is a possible follow-up.
- **Marking:** `capture_completeness` is `partial`, and the title is prefixed `[auto-drafted] ` so a reviewer sees it was built from structure and should be edited. The title is clipped by bytes, not characters, so that the marker plus title fit the contract's 256-byte limit for any script.
- **Safety:** every quoted field passes `redact` (history masking). `wk_summarize::sensitive` runs on the unmasked source (prompt plus reply, or heading plus text), so masking cannot hide a hit; a match holds the item exactly as the model path does (counted as held, no draft), and the finished card still goes through the usual validation and hold check. Rule drafts quote the conversation, unlike model summaries, so the docs stress reviewing and editing before submitting.

The existing draft shapes and storage are reused; the change is only where the card text comes from. Source digests, policy checks and `turn_key` handling in `deliver` are untouched.

## Privacy

The assistant path sends nothing to a new party: the assistant's provider already holds the conversation, and the developer reviews the card before anything is shared. The rule path never leaves the machine. Screening, review and publication gates are unchanged. The docs state that the assistant path relies on the assistant following the prompt and that its output is screened, not guaranteed clean.

## Documentation

- Setup guide: install nothing; `init` without `--inference`; the `/wk-summarize` command per client; rule drafts and what they contain; choosing `local` for fully automatic drafts.
- README and quickstart updated so the model is optional; CHANGELOG entry; privacy wording ("raw conversations stay on the machine" still holds; the assistant path is described honestly).

## Testing

- Config: old configs deserialize as `local`; `init --summarizer rules` works with no inference URL; `chat` refuses without `local`.
- `draft`: valid input stores a private draft; strict fields; oversized input; sensitive content refused without echo; idempotent resubmission; `drafts` lists it; `submit` posts it and acks it; a policy change consumes it; owner-only sync leaves it.
- Rule builder: exchange and note structures, clipping at character boundaries, masking, sensitive hold, the `[auto-drafted]` marker, and each of the five call sites producing drafts without a model endpoint (use an endpoint that fails the test if called).
- End to end: capture or import a synthetic session in `rules` mode, run `process`, review, submit against the test server; no model fixture running.
