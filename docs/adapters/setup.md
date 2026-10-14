# Native collector and agent setup

Install the [user tools](../operations/install.md#install-the-tools), then follow the steps below. Live acceptance uses isolated synthetic conversations; these examples do not authorize collecting existing chats.

1. Approve only intended service folders using `wk --state-dir /private/consent workspace approve /work/product --product PRODUCT --service backend --service frontend --yes`. The consent directory must be outside the product and private. Each separate collector state binds one chosen service. A multi-service capture must be explicitly attributed; unknown attribution remains partial.
2. Ask the operator to provision your product/service membership. Copy the `binding_file` returned by approval, then run `wk-agent --state-dir /private/collector device connect --binding /private/consent/APPROVAL.json --service backend --server https://knowledge.example --model LOCAL_MODEL --inference http://127.0.0.1:11434/v1/chat/completions --yes`. Complete the browser sign-in to the configured upstream provider. Connection validates membership and stores credentials in the native keyring; it does not start capture or install hooks. See [team connection](../operations/install.md#join-a-team-server). Legacy `init` and environment tokens are limited to loopback development.
3. Run `wk-agent --state-dir /private/collector serve` in the user's session. Linux requires Secret Service/DBus. Missing secure storage disables persistence. Windows uses native Credential Store plus a private-directory ACL check. Platform certification remains required.
4. Generate hook config with `wk-agent --state-dir /private/collector config --client cursor` (also `claude-code`, `gemini`). Inspect the fragment, then use `install --client CLIENT --file ABS_CONFIG --yes`. Installation merges strict JSON, backs up the prior object privately, and records owned entries. JSONC is refused unchanged. Uninstall removes exact owned entries and preserves user-modified entries. Do not install into real profiles until synthetic client certification is complete.
5. `doctor` separately reports local approval, current membership, key availability, event counters and client capability labels. It does not turn untested fields green. Renew an expired hosted credential with `wk-agent --state-dir /private/collector device login` through upstream browser sign-in.

A hook failure returns normally to the developer's agent and records a content-free capture gap locally. Knowledge generation/submission stops until `reset-capture --yes` closes pending sessions and discards private drafts. Then start a genuinely new client session. Restarts alone must not change client session identity. Hooks do not extract thinking/reasoning. The opt-in Cursor recovery described below validates callback-supplied transcript paths before a background reader may open them; other hooks do not read transcript paths. Product cwd is attribution metadata, not proof that all model context was in scope. Unknown external context therefore prevents automatic publication.

## Private summaries and review

The daemon schedules local inference every 15 seconds; one inference pass runs at a time. Each pass rotates through at most ten sessions using a durable metadata-only cursor, so held/older sessions cannot trap later sessions behind a fixed first page. Model calls may take longer than the scheduling interval. `process` runs it on demand. `drafts` shows only the current user's locally encrypted candidate summaries, up to 100 per page. Pass the returned `next_cursor` as `drafts --after CURSOR` to read subsequent pages. Submitted or expired drafts are not regenerated automatically from unchanged source/model/policy inputs. Changed inputs create a new draft identity; content-free closure guards remain bounded at 100,000 records. After checking one, `submit --id draft:HASH --reviewed` sends that summary as a server draft, with membership, consent, policy and source digest rechecked. Reviewers publish complete approved summaries in the console. Partial hooks stay labeled partial and cannot be automatically published.

Policy-auto remains disabled until the configured real model passes the privacy corpus and the team/developer policy is provisioned. Regex screening and model classification are not proof of perfect privacy. No cloud summarization fallback exists.

## Instrumented chat and explicit exports

`chat --message-file ABS_FILE` starts an explicitly managed local conversation and returns its session ID and visible response. Pass `--session managed:UUID` to continue. Every visible prompt/response is captured; only ordinary visible content is read from the model response. No file tools or agent reasoning are exposed. This is a controlled chat, not interception of an arbitrary existing desktop chat.

An explicit export is `{ "schema_version":1, "workspace":"/work/product", "product":"PRODUCT", "session_id":"SOURCE_SESSION", "messages":[{"id":"m1","role":"user","text":"synthetic request"}] }`. Import only that file using `import --file ABS_FILE --session SOURCE_SESSION --approve-transcript`. Metadata must match the current binding. Imports remain partial and do not scan adjacent files.

## Import existing agent memory

Start with what your coding agents already know. `wk-agent --state-dir /absolute/private/state memory scan` lists the `CLAUDE.md`, `.claude/CLAUDE.md` and `AGENTS.md` files in your approved service folder and this project's Claude Code notes (`~/.claude/projects/<project>/memory/*.md`), with a status for each. It is offline and prints no file content. Workspace-root files are reported as outside the approved service.

Import with `memory import --yes`, adding `--file ABSOLUTE_PATH` for each Claude Code note you choose. The global `~/.claude/CLAUDE.md`, Codex's cross-project memories and other projects' notes are refused. Each note or top-level section becomes one encrypted private record; `wk-agent process` drafts it with your local model and the usual privacy screening, then `wk-agent drafts` shows each draft with its source. Nothing is shared until you submit a reviewed draft.

Re-running the import skips unchanged sections, replaces unsubmitted drafts of changed sections and links a changed, already-submitted section to its earlier summary (`updates`), so a reviewer can supersede it. Deleted sources are reported, not withdrawn. Memory files can contain credentials or personal notes: review every draft before submission, including the source label and heading that `drafts` shows. A submission reports `memory_link: failed` if the summary could not be recorded locally; the next import may then draft that section again, so do not submit it twice.

Notes and limits:

- `scan` status `unchanged` means the content matches the last import; an unsubmitted draft that expired (drafts last one hour) is drafted again on the next `import`.
- Service-folder files that are too large (over 64 KiB), symlinked or not UTF-8 are skipped and counted as `rejected`; an explicit `--file` that fails stops the import before anything is written. A run over 200 sections also stops before writing.
- Claude Code notes are found only on macOS and Linux, only when `HOME` is set, and only when no folder below your home folder on the way to the notes (including `~/.claude`) is a symlink. Without them, `scan` lists no notes and may show earlier note labels under `removed`.
- `wk-agent process` reports `memory_drafting: failed` if the owner-only `memory-import.json` in the state folder cannot be read; other drafting and owner synchronization continue. Restore that file's permissions or contents; deleting it makes the next import start over.

## Claude Desktop Enterprise

On an isolated broker user account only, set `WK_COMPLIANCE_BROKER=1` and a read-only `WK_COMPLIANCE_READ_KEY`. Run `compliance --binding EXACT_GRANT_FILE`. The grant contains `chat_id`, `provider_org`, `provider_user`, `root`, `service`, `approval_hash`, and Unix `expires_at`. It must be issued for an explicitly approved chat by the operator. The client retrieves only that chat, validates returned ownership, follows bounded message pagination, skips reasoning and attachment downloads, and never lists organization-wide chats. Text-only coverage remains partial. Source edits conflict with old event IDs and hold the session. Broad API entitlement is never treated as workspace consent. This pull command has no remotely exposed broker API; broker deployment/entitlement remains a certification prerequisite.

Official protocol references checked during implementation: [Claude Code hooks](https://code.claude.com/docs/en/hooks), [Cursor hooks](https://prod.cursor.com/docs/hooks), [Gemini hooks](https://geminicli.com/docs/hooks/reference/), [Claude Compliance content](https://platform.claude.com/docs/en/manage-claude/compliance-content-data). These document interfaces; they do not certify our adapters.

## Cursor history import (for evaluation sets)

`wk cursor-history` reads your own local Cursor chats from a read-only snapshot of Cursor's undocumented SQLite storage (global `cursorDiskKV` composer/bubble records and per-workspace `ItemTable` entries, including legacy chat tabs). It also reads newer Cursor agent transcripts from `~/.cursor/projects/<path-slug>/agent-transcripts/*.jsonl`; a project is attributed only when its slug resolves to an existing directory, otherwise the chat stays unattributed. Override that location with `--dot-cursor-dir`. It never modifies Cursor's files and works while Cursor is running. Only visible message text is read; thinking and tool payloads are ignored, and missing records are counted rather than guessed.

1. `wk cursor-history list` shows titles and counts per workspace folder, plus unattributed chats (which are never assigned to a workspace).
2. `wk cursor-history show --workspace /work/product` prints that workspace's chats as Markdown with likely credentials masked. Review them.
3. `wk cursor-history export --workspace /work/product --product PRODUCT --out /private/exports --session cursor-composer:ID --yes` writes only the sessions you confirmed, as owner-only files outside the workspace, in the format accepted by `wk-agent import --approve-transcript`.

Use `--cursor-dir` for a non-standard install. Masking is pattern-based and not exhaustive; inspect exports before using them. Cursor can change its storage format in any release; this reader is tested against a synthetic fixture modelled on current layouts, not certified against every Cursor version.

## Team overlap notices and activity

Installed hooks also deliver "who else is working on this" notices (see [overlap notices](../architecture/overlap-notices.md)): Claude Code at prompt submit (locally extracted keywords only) and before file edits; Cursor after edit tools (`postToolUse`; Cursor cannot inject before a prompt or edit). Edits send activity heartbeats with workspace-relative paths only. Use `wk-agent --state-dir DIR activity pause` to stop sharing your activity (and clear it on the server), `activity resume` to restart, and `activity status` to check. MCP clients can call `knowledge.overlaps` directly; add a project rule asking the agent to call it before editing shared code.

To build cards from your own Cursor history for evaluation: `wk cursor-history cards --workspace /work/product --latest 10 --service SERVICE --out /private/dir/cards.jsonl` (local Ollama `gemma4:e2b-mlx` by default; `--endpoint`/`--model` to change). Then `python3 scripts/history-eval.py cards.jsonl --repo /work/product --report /private/dir/report.json` with `WK_TEST_DATABASE_URL` pointing at a disposable `wk_test` database.


## Cursor native recovery (opt-in)

Target: Cursor **3.23.23**, macOS, Agents window, **This Mac** execution. Standard installation does not enable transcript reads. After the existing folder approval, membership and collector connection steps:

```sh
wk-agent --state-dir /private/collector cursor-enroll --yes
wk-agent --state-dir /private/collector config --client cursor
wk-agent --state-dir /private/collector install --client cursor --file /work/product/.cursor/hooks.json --yes
wk-agent --state-dir /private/collector serve
```

Enrollment checks the installed `/Applications/Cursor.app` version and records private consent for the current product/service. A differently located installation or version is unsupported. Keep the collector running in the signed-in user's session. On macOS, rebuilding or replacing a binary can trigger Keychain prompts for its existing credentials; allow only the expected Workspace Knowledge items. This has been observed during live testing; stable signed release packaging remains required.

`doctor` reports `native_cursor`: waiting for binding, watching, stale or held counts, fixed-label hold reasons, recovered complete turns and last-check time. Counts are capped at 100 sessions. A callback must identify the one approved workspace and a session-specific transcript. A missing initial callback is undetectable; the collector never searches global Cursor history. A received callback without a source is **waiting_for_binding**, not complete capture.

Once bound, the 15-second worker reads at most ten sources per pass. Source limits are 4 MiB, 256 KiB per record, 100 turns and 768 KiB retained session data. Raw records expire 24 hours after first receipt; subsequent callbacks cannot extend the deadline. Independent cleanup runs even when the server or inference is unavailable. Cursor's own transcript retention is outside this collector's control.

Only supported visible text, supported terminal success and verified in-service file operations can produce complete candidates. Incomplete/error turns stay partial. Unknown transcript structures, compaction, child agents, attachments, shell/MCP tools and outside-service context hold the session. Conservative text screening can also hold benign discussions containing sensitive keywords. Held, reset and expired sessions cannot resume; start a genuinely new Cursor chat. This is intentionally narrower than arbitrary mixed-context development.

Use `drafts` to review locally, then `submit --id DRAFT_ID --reviewed`. Submission checks the source again under current consent and membership. A separate authorized reviewer publishes the submitted card. Local raw text and transcript paths are never added to shared card metadata. Native captures do not automatically publish or prove implementation/test success. Relative file observations support authorized overlap notices with card revisions.

For rollback: uninstall only the owned project hooks with `uninstall --client cursor --file /work/product/.cursor/hooks.json`; run `reset-capture --yes` to close pending records; remove the private state's `cursor-consent.json` and stop the collector. Keep reviewed shared cards unless their owners separately withdraw them. Re-enrollment is required to enable source reads again.

Verification evidence and remaining gates: [native capture results](../reviews/2026-10-06-cursor-native-capture-results.md). The executable harness uses only an explicitly disposable `/wk_test` database, owned synthetic transcripts, real OS keyring and an explicit local model; it never imports existing developer history.


## Experimental native Codex and Antigravity capture

This opt-in path is restricted to macOS Codex CLI 0.162.0-alpha.2 and Antigravity desktop 2.19.1. Enroll the exact approved service directory as the client workspace. CLI/Desktop/IDE surfaces are not interchangeable.

After the existing workspace approval and agent initialization:

```sh
wk-agent --state-dir /absolute/private/state native-enroll --client codex --yes
wk-agent --state-dir /absolute/private/state config --client codex
wk-agent --state-dir /absolute/private/state install --client codex --file /absolute/approved/service/.codex/hooks.json --yes
```

Review the generated command before installation. Codex must trust the project layer and review the new hooks through its normal `/hooks` UI; do not use the diagnostic hook-trust bypass. Commands verify the actual emitting process ancestry against the pinned installation. An unrecognized launcher or updated client is rejected. Existing hooks are preserved; changed commands require a new vendor trust review.

For Antigravity use `--client antigravity` and the selected workspace's `.agents/hooks.json`. The installer uses one owned `workspace-knowledge` named hook and refuses a conflicting entry. Only the transcript named by a valid callback is read; no conversation history scan runs. **All Antigravity native turns remain held and cannot become shared drafts** until unique terminal attribution is established. Repeated invocation counters, a quiet transcript or `fullyIdle` are insufficient.

Codex currently accepts supported visible no-tool prompt/final-response pairs. Tool activity (including shell reads and patch tools), compaction, subagents, unknown context and ambiguous callbacks hold the session. Cancellation prevents a stale final response from completing that turn. A later clean-looking message does not clear unsupported earlier session context. Start a fresh session after resolving a hold.

```sh
wk-agent --state-dir /absolute/private/state doctor
wk-agent --state-dir /absolute/private/state drafts
wk-agent --state-dir /absolute/private/state native-disable --client codex
wk-agent --state-dir /absolute/private/state uninstall --client codex --file /absolute/approved/service/.codex/hooks.json
```

Disable clears that client's private native evidence/drafts; its remaining hooks acknowledge without capture. Uninstall removes only owned unchanged entries. Raw evidence has a fixed 24-hour deadline; drafts expire within one hour and never outlive their source. Replays do not extend retention. Permission revocation blocks submission and clears native records; unavailable permission services block work. Missing initial callbacks remain undetectable. Ephemeral Codex history cannot be recovered. Collector outage can require an explicit clean-session reset after inspecting the gap.

These adapters are experimental, not production-certified. Claude's partial adapter is unchanged. A synthetic five-developer API/database pilot passed [local operations acceptance](../reviews/2026-10-10-hosted-operations-acceptance.md). Native distribution, other operating systems, broad context coverage and workload-specific deployed-server capacity remain separate validation work.
