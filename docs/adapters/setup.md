# Native collector and agent setup

Build the workspace. Commands below are operator instructions, not actions performed on this machine. No real conversations were collected during development.

1. Approve only intended service folders using `wk --state-dir /private/consent workspace approve /work/product --product PRODUCT --service backend --service frontend --yes`. The consent directory must be outside the product and private. Each separate collector state binds one chosen service. A multi-service capture must be explicitly attributed; unknown attribution remains partial.
2. Provision server product membership, set a short-lived `WK_ACCESS_TOKEN`, then run `wk-agent --state-dir /private/collector init --binding /private/consent/HASH.json --service backend --server https://knowledge.example --model LOCAL_MODEL --yes`. `init` validates membership before creating OS keyring entries. It does not start capture or install hooks. The inference default is a literal loopback OpenAI-compatible endpoint. Tokens and capture keys go to native keyring storage, never a key file.
3. Run `wk-agent --state-dir /private/collector serve` in the user's session. Linux requires Secret Service/DBus. Missing secure storage disables persistence. Windows uses native Credential Store plus a private-directory ACL check. Platform certification remains required.
4. Generate hook config with `wk-agent --state-dir /private/collector config --client cursor` (also `claude-code`, `gemini`). Inspect the fragment, then use `install --client CLIENT --file ABS_CONFIG --yes`. Installation merges strict JSON, backs up the prior object privately, and records owned entries. JSONC is refused unchanged. Uninstall removes exact owned entries and preserves user-modified entries. Do not install into real profiles until synthetic client certification is complete.
5. `doctor` separately reports local approval, current membership, key availability, event counters and client capability labels. It does not turn untested fields green. Refresh an expired server token with `WK_ACCESS_TOKEN=... wk-agent --state-dir ... auth` through your normal secret-entry mechanism.

A hook failure returns normally to the developer's agent and records a content-free capture gap locally. Knowledge generation/submission stops until `reset-capture --yes` closes pending sessions and discards private drafts. Then start a genuinely new client session. Restarts alone must not change client session identity. Hooks do not extract thinking/reasoning or read transcript paths supplied by clients. Product cwd is attribution metadata, not proof that all model context was in scope. Unknown external context therefore prevents automatic publication.

## Private summaries and review

The daemon schedules local inference every 15 seconds; one inference pass runs at a time. Each pass rotates through at most ten sessions using a durable metadata-only cursor, so held/older sessions cannot trap later sessions behind a fixed first page. Model calls may take longer than the scheduling interval. `process` runs it on demand. `drafts` shows only the current user's locally encrypted candidate summaries, up to 100 per page. Pass the returned `next_cursor` as `drafts --after CURSOR` to read subsequent pages. Submitted or expired drafts are not regenerated automatically from unchanged source/model/policy inputs. Changed inputs create a new draft identity; content-free closure guards remain bounded at 100,000 records. After checking one, `submit --id draft:HASH --reviewed` sends that summary as a server draft, with membership, consent, policy and source digest rechecked. Reviewers publish complete approved summaries in the console. Partial hooks stay labeled partial and cannot be automatically published.

Policy-auto remains disabled until the configured real model passes the privacy corpus and the team/developer policy is provisioned. Regex screening and model classification are not proof of perfect privacy. No cloud summarization fallback exists.

## Instrumented chat and explicit exports

`chat --message-file ABS_FILE` starts an explicitly managed local conversation and returns its session ID and visible response. Pass `--session managed:UUID` to continue. Every visible prompt/response is captured; only ordinary visible content is read from the model response. No file tools or agent reasoning are exposed. This is a controlled chat, not interception of an arbitrary existing desktop chat.

An explicit export is `{ "schema_version":1, "workspace":"/work/product", "product":"PRODUCT", "session_id":"SOURCE_SESSION", "messages":[{"id":"m1","role":"user","text":"synthetic request"}] }`. Import only that file using `import --file ABS_FILE --session SOURCE_SESSION --approve-transcript`. Metadata must match the current binding. Imports remain partial and do not scan adjacent files.

## Claude Desktop Enterprise

On an isolated broker user account only, set `WK_COMPLIANCE_BROKER=1` and a read-only `WK_COMPLIANCE_READ_KEY`. Run `compliance --binding EXACT_GRANT_FILE`. The grant contains `chat_id`, `provider_org`, `provider_user`, `root`, `service`, `approval_hash`, and Unix `expires_at`. It must be issued for an explicitly approved chat by the operator. The client retrieves only that chat, validates returned ownership, follows bounded message pagination, skips reasoning and attachment downloads, and never lists organization-wide chats. Text-only coverage remains partial. Source edits conflict with old event IDs and hold the session. Broad API entitlement is never treated as workspace consent. This pull command has no remotely exposed broker API; broker deployment/entitlement remains a certification prerequisite.

Official protocol references checked during implementation: [Claude Code hooks](https://code.claude.com/docs/en/hooks), [Cursor hooks](https://prod.cursor.com/docs/hooks), [Gemini hooks](https://geminicli.com/docs/hooks/reference/), [Claude Compliance content](https://platform.claude.com/docs/en/manage-claude/compliance-content-data). These document interfaces; they do not certify our adapters.

## Cursor history import (for evaluation sets)

`wk cursor-history` reads your own local Cursor chats from a read-only snapshot of Cursor's undocumented SQLite storage (global `cursorDiskKV` composer/bubble records and per-workspace `ItemTable` entries, including legacy chat tabs). It never modifies Cursor's files and works while Cursor is running. Only visible message text is read; thinking and tool payloads are ignored, and missing records are counted rather than guessed.

1. `wk cursor-history list` shows titles and counts per workspace folder, plus unattributed chats (which are never assigned to a workspace).
2. `wk cursor-history show --workspace /work/product` prints that workspace's chats as Markdown with likely credentials masked. Review them.
3. `wk cursor-history export --workspace /work/product --product PRODUCT --out /private/exports --session cursor-composer:ID --yes` writes only the sessions you confirmed, as owner-only files outside the workspace, in the format accepted by `wk-agent import --approve-transcript`.

Use `--cursor-dir` for a non-standard install. Masking is pattern-based and not exhaustive; inspect exports before using them. Cursor can change its storage format in any release; this reader is tested against a synthetic fixture modelled on current layouts, not certified against every Cursor version.
