# Agent history import

Status: implemented on `feat/agent-history-import`; this document was updated to match the implementation. Approved intent (2026-10-15): let a developer bring past AI-agent sessions for the enrolled folder into the knowledge base as context, through the existing review path. Decisions: explicit review-pick-export per session (no bulk or background import); one draft per session, split only when too long for the local model; include VS Code Copilot alongside Claude Code, Codex and Cursor; also add Copilot instruction files to memory import and document VS Code MCP setup. Copilot prompt-time hooks are a separate follow-up that starts with research into what VS Code supports.

## Current state

`wk cursor-history list|show|export` reads Cursor's local storage read-only (`wk_adapters::cursor_history`: `History`, `Conversation`, `Message`, `within`, `redact`) and exports only confirmed sessions as owner-only files in the format accepted by `wk-agent import --approve-transcript` (`schema_version`, `workspace`, `product`, `session_id`, `messages[{id, role, text}]`). `chat::import` records every message under its own turn ID, so `process` drafts each assistant message separately and without its prompt.

## Readers

New read-only modules in `crates/adapters/src` return the existing `History`/`Conversation` types with `source` set to `claude-code`, `codex` or `copilot`. Only visible user prompts and assistant replies become `Message`s; tool calls and results, file contents, attachments, reasoning/thinking blocks and system or developer instructions are skipped. Unrecognised records are counted in `unreadable_records`, never guessed. Files are never modified. The Cursor reader copies its SQLite databases to a private temporary directory first; the Claude Code, Codex and Copilot files are append-only or rewritten whole, so they are read in place through a bounded, symlink-refusing open (a racing write at worst yields a malformed record, which is counted and marks the session incomplete). Session IDs are validated: the agent's ID is used only if it matches `[A-Za-z0-9._-]{1,128}` (not `.` or `..`), else the file stem under the same rule, else a hash of the file name; Claude Code always uses the file stem.

| Client | Location (default, overridable for tests) | Session → folder |
|---|---|---|
| Claude Code | `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`, `*/*.jsonl` | the first absolute `cwd` of the session's user/assistant records |
| Codex | `$CODEX_HOME/sessions` or `~/.codex/sessions`, `YYYY/MM/DD/*.jsonl` | `session_meta.payload.cwd` |
| VS Code Copilot | VS Code `User/workspaceStorage/*/chatSessions/*.json` and `.jsonl` mutation logs, replayed in memory (macOS `~/Library/Application Support/Code/User`, Linux `$XDG_CONFIG_HOME` or `~/.config` + `Code/User`, Windows `%APPDATA%\Code\User`) | that storage folder's `workspace.json` `folder` URI (`file://` only) |
| Cursor | existing reader | existing rules |

A session belongs to the enrolled workspace when its canonical folder is the workspace root or inside it (`within`). Sessions with no folder, remote URIs (`vscode-remote://`, multi-root `workspace` files) or folders outside the root are never listed; `list` reports how many were skipped and why, by fixed reason. Claude Code sidechain (subagent) records are skipped; Codex `event_msg` and non-message `response_item`s are skipped. Each reader caps a single source file at 32 MiB and a session at 5,000 messages (after consecutive same-role records are joined), reporting larger ones as skipped. Only absolute recorded folders count.

**Injected context.** Agents store context they add as user messages. Claude Code and Codex drop, per prompt part, text made only of `<name …>…</name>` blocks at least one of which has a tool-style name containing `_` or `-` (`ide_opened_file`, `environment_context`); this tag rule never drops parts made only of plain-named tags such as `<task>`, which developers write, nor parts with other text. Each reader also drops parts starting with known injected markers (Claude Code: `<command-`, `<local-command-`, `<task-notification`, `<system-reminder`, `<ide_`, `<user-prompt-submit-hook`, `<user-memory-input`, `<bash-`, bash output, interrupt markers; Codex: `<environment_context>`, `<user_instructions>`, `<permissions instructions>`, `# AGENTS.md instructions for `, `<skill>`, `<user_shell_command>`, `<turn_aborted>`, `<subagent_notification>`). Dropped parts are counted in `History::dropped_injected` and reported by `list`. The rule does not apply to Copilot, which keeps attached context outside `message.text`.

**Copilot specifics.** Newer VS Code versions write `<sessionId>.jsonl` logs: a `{"kind":0}` snapshot, then set (`kind` 1) and splice (`kind` 2) operations. The log is replayed with bounds (200,000 lines, paths of at most 16 components), only the top-level `sessionId`, `customTitle`, `creationDate` and `requests` are parsed, and the unsent draft (`inputState`) and attached variables are never read. A session with both files is read from its log, falling back to the `.json` (marked incomplete) only if the log is unreadable; a session copied into several storage folders is listed once, from the newest file. Inline references are read as base names, never paths.

## Command

`wk agent-history --client claude-code|codex|copilot|cursor <action>`, sharing one implementation:

- `list --workspace DIR`: session ID, title (first prompt line, masked, clipped to 80 characters), start time, message count and whether the session looks complete; plus `outside_workspace`, `without_folder`, `unreadable_records`, `empty_conversations`, `skipped` and `dropped_injected`. No message text.
- `show --workspace DIR [--session ID]... [--max-chars N]`: Markdown of the selected sessions with likely credentials masked (`redact`, which also masks short values assigned to credential-like names), for review. An unknown session ID is an error.
- `export --workspace DIR --product PRODUCT --out DIR --session ID... --yes`: one owner-only (0600) file per session in a private directory (0700 when created; an existing one must already be private on Unix) outside the workspace, text masked with `redact`. Hardening: every check runs before anything is written; `--out` must be absolute without `..` and outside the workspace (checked again after creation); only named sessions are exported, each once; file names keep `[A-Za-z0-9._-]` (the first `:` becomes `_`), and any other change or a name over 120 characters adds a 12-character hash of the ID; names that collide case-insensitively are refused; existing files are never overwritten; files written by a failed run are removed.

`--source-dir` overrides the client's storage directory (for Cursor it replaces only the `User` folder).

`wk cursor-history` remains as an alias for `--client cursor`.

## One draft per session

`wk-agent import` groups an export's messages into **parts** of consecutive whole exchanges (a user prompt with the replies that follow it) whose combined text stays under 96 KiB, below the summarizer's 128 KiB input limit, and that hold at most 256 messages. An exchange with more than 256 messages is split into consecutive chunks first. All messages in a part share one turn ID (`part-<n>`), and each part is recorded in its own journal session named `export:<session>:<12-hex content digest>:part-<n>`, which keeps every part inside the journal's per-session bounds, so `process` drafts one summary per part from both prompts and replies. The digest covers the messages: an identical re-import adds nothing, while a continued or edited session gets new part sessions and **all** parts are drafted again (earlier unsubmitted drafts should be discarded; submitted summaries are not linked automatically). A single exchange or chunk larger than the limit becomes its own part, truncated with an explicit " [truncated]" marker; every message keeps a fair share of the limit so prompts and replies all reach the summary. The import output reports `parts`, `truncated_parts` and `process_runs_needed` (parts divided by ten, rounded up: `process` drafts at most ten sessions per run, so this is a minimum). The export limit of 500 messages becomes 5,000 and the 768 KiB file limit becomes 8 MiB. This changes Cursor imports the same way.

## Copilot instruction files

`wk-agent memory scan|import` also reads `.github/copilot-instructions.md` and `.github/instructions/*.instructions.md` in the agent's approved service folder, with the same scope check, size limits, splitting and symlink rules as `CLAUDE.md`. Frontmatter in `.instructions.md` files (`applyTo`, `description`) is removed; `description` becomes a hint for every section. Scan kind is `scoped_instructions`; only files directly in `.github/instructions` are read, and a symlinked folder on the way is never listed. Frontmatter hint values now lose one matching pair of single or double quotes, so single-quoted hints of existing Claude Code notes change once and redraft a single time. Labels are `repo:<service>/.github/...`.

## Documentation

- `docs/adapters/setup.md`: the `wk agent-history` workflow per client, storage locations, what is read and skipped, limits, and the note that formats are undocumented and version-dependent.
- VS Code MCP setup: a `.vscode/mcp.json` example for `wk-mcp` over stdio and for the remote `/mcp/{product}` endpoint, so Copilot agent mode can call `knowledge.search`, `knowledge.context` and `knowledge.overlaps`, noting that `.vscode/mcp.json` is VS Code's format and may change; no automatic prompt-time notices for Copilot yet.
- README client table and CHANGELOG.

## Privacy

Nothing is read without an explicit command; only the enrolled folder's sessions are listed; only named sessions are exported, masked, owner-only, outside the workspace; import still requires `--approve-transcript` and the existing enrollment checks; drafting uses the local model with the existing sensitive-content holds; nothing is shared until a reviewed draft is submitted. Absolute paths do not enter export messages beyond what the conversation text itself contains; developers review `show` output before exporting.

## Testing

- Per reader, synthetic fixture files covering: visible prompts and replies, skipped tool/reasoning/system records, unknown record types counted, sessions in other folders and remote URIs excluded, sidechains skipped, size caps, malformed lines.
- `within` matching including symlinked and sibling-prefix folders (`/work/app` vs `/work/app2`).
- CLI: list shows no text; export refuses without `--yes`, inside the workspace, or for unknown sessions; files are 0600 and masked.
- Chunking: whole exchanges kept together, the 96 KiB and 256-message bounds, oversized single exchanges truncated and marked, turn IDs and digest-named journal sessions per part, identical re-import adds nothing, grown re-import drafts all parts.
- End to end: export a synthetic Claude Code session → `wk-agent import` → `process` against the loopback model fixture → one draft per part containing both prompt and reply text.
- Memory import: Copilot instruction files found, scoped, frontmatter handled.
