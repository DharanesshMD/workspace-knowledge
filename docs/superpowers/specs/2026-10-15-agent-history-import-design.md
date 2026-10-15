# Agent history import

Approved intent (2026-10-15): let a developer bring past AI-agent sessions for the enrolled folder into the knowledge base as context, through the existing review path. Decisions: explicit review-pick-export per session (no bulk or background import); one draft per session, split only when too long for the local model; include VS Code Copilot alongside Claude Code, Codex and Cursor; also add Copilot instruction files to memory import and document VS Code MCP setup. Copilot prompt-time hooks are a separate follow-up that starts with research into what VS Code supports.

## Current state

`wk cursor-history list|show|export` reads Cursor's local storage read-only (`wk_adapters::cursor_history`: `History`, `Conversation`, `Message`, `within`, `redact`) and exports only confirmed sessions as owner-only files in the format accepted by `wk-agent import --approve-transcript` (`schema_version`, `workspace`, `product`, `session_id`, `messages[{id, role, text}]`). `chat::import` records every message under its own turn ID, so `process` drafts each assistant message separately and without its prompt.

## Readers

New read-only modules in `crates/adapters/src` return the existing `History`/`Conversation` types with `source` set to `claude-code`, `codex` or `copilot`. Only visible user prompts and assistant replies become `Message`s; tool calls and results, file contents, attachments, reasoning/thinking blocks and system or developer instructions are skipped. Unrecognised records are counted in `unreadable_records`, never guessed. Files are never modified; SQLite or JSON sources that another process may be writing are copied to a private temporary directory first, as the Cursor reader does.

| Client | Location (default, overridable for tests) | Session → folder |
|---|---|---|
| Claude Code | `~/.claude/projects/*/*.jsonl` | the `cwd` of the session's user/assistant records |
| Codex | `~/.codex/sessions/**/*.jsonl` | `session_meta.payload.cwd` |
| VS Code Copilot | VS Code `User/workspaceStorage/*/chatSessions/*.json` (macOS `~/Library/Application Support/Code/User`, Linux `~/.config/Code/User`) | that storage folder's `workspace.json` `folder` URI (`file://` only) |
| Cursor | existing reader | existing rules |

A session belongs to the enrolled workspace when its canonical folder is the workspace root or inside it (`within`). Sessions with no folder, remote URIs (`vscode-remote://`, multi-root `workspace` files) or folders outside the root are never listed; `list` reports how many were skipped and why, by fixed reason. Claude Code sidechain (subagent) records are skipped; Codex `event_msg` and non-message `response_item`s are skipped. Each reader caps a single source file at 32 MiB and a session at 5,000 messages, reporting larger ones as skipped.

## Command

`wk agent-history --client claude-code|codex|copilot|cursor <action>`, sharing one implementation:

- `list --workspace DIR`: session ID, title (first prompt, clipped), start time, message count and whether the session looks complete; plus skipped counts. No message text.
- `show --workspace DIR [--session ID]...`: Markdown of the selected sessions with likely credentials masked (`redact`), for review.
- `export --workspace DIR --product PRODUCT --out DIR --session ID... --yes`: one owner-only (0600) file per session in a private directory outside the workspace, text masked with `redact`. Refuses without `--yes`, refuses an output directory inside the workspace, and exports only the named sessions.

`wk cursor-history` remains as an alias for `--client cursor`.

## One draft per session

`wk-agent import` groups an export's messages into **parts** of consecutive whole exchanges (a user prompt with the replies that follow it) whose combined text stays under 96 KiB, below the summarizer's 128 KiB input limit. All messages in a part share one turn ID (`part-<n>`), so `process` drafts one summary per part from both prompts and replies. A single exchange larger than the limit becomes its own part and is truncated at a character boundary with an explicit "[truncated]" marker, so the draft is marked partial. The existing 500-message export limit becomes 5,000 to fit long sessions; the 768 KiB export file limit becomes 8 MiB. This changes Cursor imports the same way.

## Copilot instruction files

`wk-agent memory scan|import` also reads `.github/copilot-instructions.md` and `.github/instructions/*.instructions.md` in the agent's approved service folder, with the same scope check, size limits, splitting and symlink rules as `CLAUDE.md`. Frontmatter in `.instructions.md` files (`applyTo`, `description`) is removed; `description` becomes a hint. Labels are `repo:<service>/.github/...`.

## Documentation

- `docs/adapters/setup.md`: the `wk agent-history` workflow per client, storage locations, what is read and skipped, limits, and the note that formats are undocumented and version-dependent.
- VS Code MCP setup: a `.vscode/mcp.json` example for `wk-mcp` over stdio and for the remote `/mcp/{product}` endpoint, so Copilot agent mode can call `knowledge.search`, `knowledge.context` and `knowledge.overlaps`; no automatic prompt-time notices for Copilot yet.
- README client table and CHANGELOG.

## Privacy

Nothing is read without an explicit command; only the enrolled folder's sessions are listed; only named sessions are exported, masked, owner-only, outside the workspace; import still requires `--approve-transcript` and the existing enrollment checks; drafting uses the local model with the existing sensitive-content holds; nothing is shared until a reviewed draft is submitted. Absolute paths do not enter export messages beyond what the conversation text itself contains; developers review `show` output before exporting.

## Testing

- Per reader, synthetic fixture files covering: visible prompts and replies, skipped tool/reasoning/system records, unknown record types counted, sessions in other folders and remote URIs excluded, sidechains skipped, size caps, malformed lines.
- `within` matching including symlinked and sibling-prefix folders (`/work/app` vs `/work/app2`).
- CLI: list shows no text; export refuses without `--yes`, inside the workspace, or for unknown sessions; files are 0600 and masked.
- Chunking: whole exchanges kept together, the 96 KiB bound, oversized single exchanges truncated and marked, turn IDs per part.
- End to end: export a synthetic Claude Code session → `wk-agent import` → `process` against the loopback model fixture → one draft per part containing both prompt and reply text.
- Memory import: Copilot instruction files found, scoped, frontmatter handled.
