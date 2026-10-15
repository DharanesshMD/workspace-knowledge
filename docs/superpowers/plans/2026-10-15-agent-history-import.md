# Agent History Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `wk agent-history --client claude-code|codex|copilot|cursor list|show|export` exports reviewed past agent sessions for the enrolled folder; `wk-agent import` drafts one summary per session part; memory import also reads Copilot instruction files.

**Architecture:** Three new read-only readers in `wk-adapters` return the existing `cursor_history::{History, Conversation, Message}` types. The CLI shares one list/show/export implementation for every client (`cursor-history` stays as an alias). `chat::import` groups messages into size-bounded parts of whole exchanges with one turn ID per part, so the existing `process` path drafts one summary per part.

**Tech Stack:** Rust (serde_json, rusqlite only where already used), existing CLI (`clap`) and agent crates.

**Spec:** `docs/superpowers/specs/2026-10-15-agent-history-import-design.md`.

**Commands:** always `./scripts/cargo` (or `.superpowers/sdd/2026-10-15-history/run.sh` once created). Gate before each commit: `fmt --all -- --check` and `clippy --workspace --all-targets --locked -- -D warnings`. Commit with `git commit -s` and the `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` trailer.

**Note for implementers:** read the real code first; adapt minimally and report deviations. Fixtures are synthetic and committed; never read a developer's real history in tests.

---

## Storage formats (observed 2026-10-15; undocumented and version-dependent)

**Claude Code** — `~/.claude/projects/<encoded-folder>/<sessionId>.jsonl`, one JSON record per line. Relevant records have `type` `user` or `assistant`, `sessionId`, `uuid`, `timestamp` (RFC 3339), `cwd`, `isSidechain`, `message: {role, content}`.
- User prompt: `message.content` is a string, or an array containing `{"type":"text","text":...}` parts. Skip records with `isMeta: true`, records with a `toolUseResult` field, and arrays whose parts are `tool_result`.
- Assistant reply: `message.content` array; keep `{"type":"text","text":...}` parts; skip `thinking`, `tool_use`. Consecutive assistant records form one reply.
- Skip `isSidechain: true`, and all other `type`s (`queue-operation`, `summary`, …) — count unknown types as unreadable only if they look like messages (have `message`).

**Codex** — `~/.codex/sessions/YYYY/MM/DD/*.jsonl`, records `{timestamp, type, payload}`.
- `session_meta`: `payload.id` (session ID), `payload.cwd` (folder).
- `response_item` with `payload.type == "message"`: `payload.role` `user`/`assistant`, `payload.content[]` parts `{type:"input_text"|"output_text", text}`. Skip role `developer`/`system`. Skip user messages whose text starts with `<environment_context>`, `<user_instructions>` or `<permissions instructions>` (injected context, not prompts).
- Skip `reasoning`, `function_call*`, `custom_tool_call*`, `event_msg`, `turn_context`.

**VS Code Copilot** — `<Code User dir>/workspaceStorage/<hash>/chatSessions/<sessionId>.json` plus `<hash>/workspace.json` `{"folder":"file:///abs/path"}` (multi-root has `workspace` instead: skip).
- Session: `sessionId`, `customTitle` (optional), `creationDate` (ms), `requests[]`.
- Each request: prompt `message.text`; reply = concatenation of `response[]` parts **without** a `kind` (or `kind == "markdownContent"`) using their `value` (string, or object with `value` string). Skip kinds `thinking`, `toolInvocationSerialized`, `prepareToolInvocation`, `textEditGroup`, `codeblockUri`, `inlineReference`, `undoStop`, `progressTaskSerialized`, `mcpServersStarting`, and any unknown kind (count it). `timestamp` per request (ms).
- Code User dir: macOS `~/Library/Application Support/Code/User`, Linux `~/.config/Code/User` (also accept `Code - Insiders` via `--vscode-dir`).

---

## File structure

| File | Responsibility |
|---|---|
| Create `crates/adapters/src/claude_history.rs` | Claude Code reader |
| Create `crates/adapters/src/codex_history.rs` | Codex reader |
| Create `crates/adapters/src/copilot_history.rs` | VS Code Copilot reader |
| Modify `crates/adapters/src/lib.rs` | `pub mod` lines |
| Create `crates/adapters/tests/agent_history.rs` | Reader tests with synthetic fixtures written to temp dirs |
| Modify `crates/cli/src/main.rs` | `AgentHistory` command; shared list/show/export; `cursor-history` alias |
| Modify `crates/cli/tests/cli.rs` (or existing CLI test file) | CLI tests |
| Modify `crates/agent/src/chat.rs` | Part grouping and limits in `import` |
| Modify `crates/agent/src/memory_sources.rs` (+ `memory_split.rs` if needed) | Copilot instruction files |
| Modify docs | `docs/adapters/setup.md`, README, CHANGELOG |

---

### Task 1: Claude Code reader

**Files:** create `crates/adapters/src/claude_history.rs`; `pub mod claude_history;` in `lib.rs`; tests in `crates/adapters/tests/agent_history.rs`.

**API:**
```rust
pub fn default_dir() -> Option<PathBuf>;            // ~/.claude/projects
pub fn read(projects_dir: &Path) -> Result<History, String>;
```
`History`/`Conversation`/`Message` come from `crate::cursor_history`. Each `.jsonl` file is one session. `Conversation { id: "claude-code:<sessionId>", title: first prompt clipped to 80 chars, workspace: Some(cwd of the first user/assistant record), created_at_ms: first record timestamp, source: "claude-code", messages, files_changed: vec![], files_read: vec![], outcome: last assistant text, complete: last message is assistant }`. Message IDs: the record `uuid` (assistant replies merged across consecutive records use the first `uuid`). Files over 32 MiB or with more than 5,000 messages are skipped (counted in a new `History.skipped` if you add one, else `unreadable_records`; report the choice). Malformed lines count as unreadable.

- [ ] **Step 1: Failing tests** writing synthetic `.jsonl` fixtures under a tempdir `projects/-work-app/` to cover: string prompt; array text prompt; assistant reply split over two records with `thinking` + `tool_use` + `text` parts (only text kept, merged); `tool_result` user record, `isMeta` record and `toolUseResult` record skipped; `isSidechain` records skipped; malformed line counted; `queue-operation` ignored; `complete` true when ending with assistant; title clipping; `cwd` used as workspace; a second file in another project folder read as a separate conversation; an oversized-file case (write a sparse file > 32 MiB via `set_len`) skipped.
- [ ] **Step 2:** run `./scripts/cargo test -p wk-adapters --locked --test agent_history claude` → compile errors.
- [ ] **Step 3:** implement with `serde_json::Value` parsing line by line (`BufReader::lines`), no unwraps on input.
- [ ] **Step 4:** tests pass; `cursor_history::within(&history, root)` selects only `/work/app` sessions (add a test with sibling `/work/app2`).
- [ ] **Step 5:** gate and commit `feat: read Claude Code session history`.

### Task 2: Codex reader

**API:** `pub fn default_dir() -> Option<PathBuf>` (`~/.codex/sessions`), `pub fn read(sessions_dir: &Path) -> Result<History, String>` walking `**/*.jsonl` (bounded depth 4, no symlink following). `Conversation.id` = `codex:<session_meta.payload.id>` (fallback: file stem), `workspace` = `session_meta.payload.cwd`, `source: "codex"`. Message IDs `u<n>`/`a<n>` in order.

- [ ] **Step 1: Failing tests** with fixtures covering: session_meta cwd; user `input_text` and assistant `output_text`; developer messages skipped; injected `<environment_context>`/`<user_instructions>`/`<permissions instructions>` user messages skipped; reasoning/function_call/custom_tool_call/event_msg skipped; a file without session_meta (workspace None → never selected); malformed line counted; nested date directories; symlinked directory not followed.
- [ ] **Steps 2–5:** as Task 1; commit `feat: read Codex session history`.

### Task 3: VS Code Copilot reader

**API:** `pub fn default_dir() -> Option<PathBuf>` (the Code `User` dir per OS), `pub fn read(user_dir: &Path) -> Result<History, String>` scanning `workspaceStorage/*/chatSessions/*.json` (copy each file to a private temp dir before parsing only if it could be mid-write — a plain read of a JSON file is acceptable; document the choice). `Conversation.id` = `copilot:<sessionId>`, `title` = `customTitle` or first prompt, `workspace` from `workspace.json` `folder` (`file://` URIs only, percent-decoded; anything else → None), `created_at_ms` = `creationDate`, `source: "copilot"`. Message IDs `requestId`-based (`<requestId>:u` / `<requestId>:a`).

- [ ] **Step 1: Failing tests** with fixtures covering: prompt `message.text`; reply from parts without `kind` (string `value` and object `{value}`); skipped kinds (thinking, toolInvocationSerialized, textEditGroup, codeblockUri, inlineReference, undoStop, progressTaskSerialized, mcpServersStarting); unknown kind counted; `workspace.json` with `folder` percent-encoded path (`/work/my%20app`) decoded; `vscode-remote://` and multi-root `workspace` → None; storage folder without `workspace.json` → None; empty requests → counted in `empty_conversations`.
- [ ] **Steps 2–5:** as Task 1; commit `feat: read VS Code Copilot chat history`.

### Task 4: `wk agent-history` command

**Files:** `crates/cli/src/main.rs`, CLI tests.

- [ ] **Step 1:** Add
```rust
/// Review and export past agent sessions for a folder (read-only).
AgentHistory {
    #[arg(long, value_parser = ["claude-code", "codex", "copilot", "cursor"])]
    client: String,
    /// Override the client's storage directory (tests, non-default installs).
    #[arg(long, global = true)]
    source_dir: Option<PathBuf>,
    #[command(subcommand)]
    action: HistoryAction,
},
```
with `HistoryAction::{List { workspace }, Show { workspace, session: Vec<String>, max_chars }, Export { workspace, product, out, session: Vec<String>, yes }}`. Extract the existing Cursor `List`/`Show`/`Export` bodies into shared functions taking `&History` so `cursor-history` and `agent-history` call the same code (Cursor keeps `Diagnose` and `Cards`). `List` for agent-history takes `--workspace` and reports only that folder's sessions (id, title, created_at_ms, messages, complete) plus `outside_workspace` and `without_folder` counts and `unreadable_records`/`empty_conversations`.
- [ ] **Step 2: Failing CLI tests** (run the built `wk` binary against fixture dirs via `--source-dir`): list shows no message text and only in-folder sessions; show masks a synthetic secret (`password=SYNTHETIC_SECRET`); export without `--yes` fails; export into the workspace fails; unknown session fails; export files are 0600 and masked; `cursor-history` still works (existing tests stay green).
- [ ] **Steps 3–5:** implement, pass, gate, commit `feat: add wk agent-history for Claude Code, Codex, Copilot and Cursor`.

### Task 5: One draft per session part

**Files:** `crates/agent/src/chat.rs` (`import`), tests in the same module or `crates/agent/tests`.

- [ ] **Step 1: Failing unit tests** for a pure function
```rust
/// Group messages into parts of whole exchanges (a user prompt and the replies after it)
/// whose combined text stays under `limit` bytes; an oversized exchange is its own part,
/// truncated at a char boundary with " [truncated]".
pub(crate) fn parts(messages: &[Message], limit: usize) -> Vec<Vec<Message>>
```
covering: small session → one part; boundary at whole exchanges; leading assistant messages (no prompt) kept with the first exchange; oversized exchange truncated and marked; total text per part ≤ limit (+ marker); deterministic.
- [ ] **Step 2:** `import` uses `parts(&e.messages, 96 * 1024)` and records each message with `turn: &format!("part-{n}")` (keep event IDs unique: `m.id`); raise limits to 5,000 messages and 8 MiB file size. A truncated part's events still flow through the existing journal; report `parts` and `truncated_parts` in the import output.
- [ ] **Step 3: End-to-end test** (agent crate, existing loopback model fixture pattern from `memory_import.rs` tests / `process.rs` pause test): write a synthetic export with 3 exchanges, run the import path's recording function into a test store (avoid keyring: call the inner recording logic directly or factor `import_with(store, binding, …)`), then `journal_turns` for the session → exactly 1 draft whose model input contains both a prompt and a reply. A 2-part export → 2 drafts.
- [ ] **Steps 4–5:** pass, gate, commit `feat: draft one summary per imported session part`.

### Task 6: Copilot instruction files in memory import

**Files:** `crates/agent/src/memory_sources.rs` (and `memory_split.rs` only if frontmatter handling must change).

- [ ] **Step 1: Failing tests** (`memory_sources` tests): `repository()` lists `backend/.github/copilot-instructions.md` and each `backend/.github/instructions/*.instructions.md` (sorted, non-recursive, `.instructions.md` suffix only) with labels `repo:backend/.github/...`; root-level `.github/...` reported as `outside_approved_service`; symlinked instruction files rejected; `approve_file` accepts them as repository files.
- [ ] **Step 2:** instruction files are `SourceKind::Instructions`, but `.instructions.md` files have YAML frontmatter: strip it before splitting and use its `description` as the section hint for every section of that file (add a test in `memory_split` or handle in `memory_import` — choose the smallest change and report it).
- [ ] **Steps 3–5:** pass (including `memory_import` tests), gate, commit `feat: import Copilot instruction files as agent memory`.

### Task 7: Documentation

- [ ] `docs/adapters/setup.md`: new "Import past agent sessions" section: per-client locations, what is read/skipped, matching rules, limits, the list → show → export → `wk-agent import` workflow, one draft per part, and that formats are undocumented and version-dependent. Mark the Cursor section as covered by `wk agent-history --client cursor` (alias kept).
- [ ] VS Code MCP: a `.vscode/mcp.json` example for `wk-mcp` (stdio) and the remote `/mcp/{product}` endpoint (read `docs/architecture/remote-mcp.md` and the `wk-mcp` CLI for exact flags/env), noting Copilot has no automatic prompt-time notices yet.
- [ ] Memory import docs: Copilot instruction files.
- [ ] README client table row(s) and Features; CHANGELOG bullet under Unreleased.
- [ ] Check relative links; commit `docs: document agent history import and VS Code MCP setup`.

### Task 8: Verification

- [ ] fmt, workspace clippy `-D warnings`, `cargo test --workspace --locked -- --include-ignored` (disposable pgvector DB), build, Python tests, demo, plain-PostgreSQL acceptance, integration, console tests, `cargo audit`.
- [ ] Real-binary smoke test of `wk agent-history --client <each> list --workspace <this repo>` against the developer's real local history, piping the JSON through a filter that prints only counts and field names (titles are the opening words of real prompts and must not be printed): confirm the output has no message-text fields and that every listed session's folder is inside the repository. Do not export, show or print real conversation text.
