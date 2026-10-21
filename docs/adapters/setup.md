# Native collector and agent setup

Install the [user tools](../operations/install.md#install-the-tools), then follow the steps below. Live acceptance uses isolated synthetic conversations; these examples do not authorize collecting existing chats.

1. Approve only intended service folders using `wk --state-dir /private/consent workspace approve /work/product --product PRODUCT --service backend --service frontend --yes`. The consent directory must be outside the product and private. Each separate collector state binds one chosen service. A multi-service capture must be explicitly attributed; unknown attribution remains partial.
2. Ask the operator to provision your product/service membership. Copy the `binding_file` returned by approval, then run `wk-agent --state-dir /private/collector device connect --binding /private/consent/APPROVAL.json --service backend --server https://knowledge.example --yes`. Add `--model LOCAL_MODEL --inference http://127.0.0.1:11434/v1/chat/completions` only if you want a local model to write drafts; both are optional, and without `--inference` the collector builds drafts from rules (see [Set up without a local model](#set-up-without-a-local-model)). Complete the browser sign-in to the configured upstream provider. Connection validates membership and stores credentials in the native keyring; it does not start capture or install hooks. See [team connection](../operations/install.md#join-a-team-server). Legacy `init` and environment tokens are limited to loopback development; `init` takes the same optional `--inference`, `--summarizer` and `--model` flags.
3. Run `wk-agent --state-dir /private/collector serve` in the user's session. Linux requires Secret Service/DBus. Missing secure storage disables persistence. Windows uses native Credential Store plus a private-directory ACL check. Platform certification remains required.
4. Generate hook config with `wk-agent --state-dir /private/collector config --client cursor` (also `claude-code`, `gemini`). Inspect the fragment, then use `install --client CLIENT --file ABS_CONFIG --yes`. Installation merges strict JSON, backs up the prior object privately, and records owned entries. JSONC is refused unchanged. Uninstall removes exact owned entries and preserves user-modified entries. Do not install into real profiles until synthetic client certification is complete.
5. `doctor` separately reports local approval, current membership, key availability, event counters and client capability labels. It does not turn untested fields green. Renew an expired hosted credential with `wk-agent --state-dir /private/collector device login` through upstream browser sign-in.

A hook failure returns normally to the developer's agent and records a content-free capture gap locally. Knowledge generation/submission stops until `reset-capture --yes` closes pending sessions and discards private drafts. Then start a genuinely new client session. Restarts alone must not change client session identity. Hooks do not extract thinking/reasoning. The opt-in Cursor recovery described below validates callback-supplied transcript paths before a background reader may open them; other hooks do not read transcript paths. Product cwd is attribution metadata, not proof that all model context was in scope. Unknown external context therefore prevents automatic publication.

## Set up without a local model

You do not need to install or run a model. `wk-agent` writes drafts in one of two ways, chosen when the collector is first set up:

| Mode | How to choose it | Where draft text comes from |
|---|---|---|
| `rules` | Omit `--inference` (or pass `--summarizer rules`) | Built from the conversation's structure on your machine, without a model (see [Rule drafts](#rule-drafts-and-their-limits)) |
| `local` | Pass `--inference URL` (or `--summarizer local --inference URL`) | A model on your machine through a loopback OpenAI-compatible endpoint, as before |

`device connect` and `init` take the same flags: `--inference URL` (optional), `--summarizer local|rules` (optional) and `--model NAME` (only used by `local`). For example:

```sh
wk-agent --state-dir /private/collector device connect --binding /private/consent/APPROVAL.json \
  --service backend --server https://knowledge.example --yes
```

- With no flags the mode is `rules` and nothing is stored about a model. Passing `--inference` selects `local` and validates the URL exactly as before. Combinations that contradict each other are refused: `--summarizer rules` with `--inference`, or `--summarizer local` without `--inference`. `--model` is ignored in `rules`.
- Collectors set up before this option existed keep working as `local`; their configuration file is unchanged.
- `wk-agent doctor` reports the current mode as `summarizer`, and `wk-agent drafts` shows `rules` as the model of a rule draft.
- The mode cannot be changed in place. Set up a new private state directory to switch (the existing "refusing credential rotation" rule applies). Owner-sync consent records the mode, so a new setup needs new consent. Turns still in the private journal are drafted again under the new mode, so after switching from `local` to `rules` you may see `[auto-drafted]` drafts next to the model-written ones for the same turns (they have different identities); discard the old drafts by letting them expire or by not submitting them.
- A hosted (`device connect`) collector still needs the product's signed policy, including its inference restrictions field, to be present and well formed. In `rules` mode the allowed inference destinations simply do not apply, because nothing is sent to a model.
- Managed chat (`wk-agent chat`) talks to a local model and refuses to run in `rules` mode. Use the `local` mode for it.

Raw conversations stay on your machine in `rules` mode: no model, local or remote, is called and no conversation text leaves the machine. The collector still talks to the server as in other modes — each processing pass fetches the product's signed policy over an authenticated request that names the product — and the usual review and submission steps are unchanged.

### Rule drafts and their limits

In `rules` mode `process` still runs every 15 seconds (or on demand) and still reads live captured turns, native turns, imported session parts and memory notes. It builds each draft from structure:

- **Conversation turns and imported parts:** the title is the first line of the first prompt, the goal (`intent`) is the first prompt clipped to 400 characters and the outcome (`rationale`) is the last reply clipped to 1,500 characters. A part with several exchanges uses its first prompt and its last reply. `decisions`, `constraints` and `open_questions` are empty.
- **Memory notes and instruction sections:** the title and goal are the heading (the note's name when it has none) and the rationale is the section text, clipped. A frontmatter hint such as `description` is put in front of the section text instead of being passed to a model.
- **Marking:** every title starts with `[auto-drafted] ` (titles are cut to 256 bytes, marker included) and the card is `partial`, so reviewers see that it is built from structure and was not summarised.
- **Safety:** quoted text is masked with the same pattern masking as the history tools (see [Masking](#import-past-agent-sessions)), and the source is held when it looks sensitive. Model-backed summarization also screens common email, formatted telephone and street-address patterns before inference, then checks the output. These are conservative heuristics, not complete personal-data detection; held items are counted and no draft is made.
- **Files:** a draft carries file evidence only where the source already provides it, which today means Cursor turns. Native Codex capture attaches no files even with a model, and imports of Claude Code, Codex and Copilot sessions do not read tool calls, so those drafts have no files.

Rule drafts are `partial`, so owner synchronization, which uploads only complete drafts, never uploads them; they leave your machine only through your own `submit`. A rule draft **quotes your conversation**; it does not summarise it. Masking is pattern-based and not exhaustive, so read every draft with `drafts` and submit only the ones you are happy for the team to see. A draft you do not submit expires. There is no command to edit a stored draft; for a card in your own words, or one with decisions, constraints and open questions, use [`/wk-summarize`](#summarize-with-your-assistant-wk-summarize), write a note under My context in the portal, or choose `--summarizer local` and run a model. Only `/wk-summarize` cards can become team knowledge from a rules-only setup; rule drafts cannot be published.

### Summarize with your assistant (`/wk-summarize`)

Your coding assistant already has the conversation, so it can write the summary. `wk-agent prompt --client claude-code|copilot|codex` prints a JSON object `{"client", "text", "save_as"}`, where `text` is a short instruction that makes the assistant write a card and give it to `wk-agent draft`. With `--raw` it prints only the text, so you can redirect it to a file. The command only prints; it writes no files and starts nothing in the background. Save the text yourself:

| Client | Where to save it | Run it with |
|---|---|---|
| Claude Code | `~/.claude/commands/wk-summarize.md` (all projects; `.claude/commands/wk-summarize.md` in a project also works) | `/wk-summarize` |
| VS Code Copilot Chat | A user prompt: open the Prompts section of VS Code's chat customizations, choose New Prompt, then User, and paste the text. A workspace file `.github/prompts/wk-summarize.prompt.md` also works. | `/wk-summarize` (prompt files are a VS Code feature that changes between versions; check yours) |
| Codex | Under its own heading in `AGENTS.md` | Ask Codex to summarise the session; the text only applies when you ask for it |

```sh
wk-agent --state-dir /private/collector prompt --client claude-code --raw > ~/.claude/commands/wk-summarize.md
```

Notes on the saved file:

- It embeds the absolute path of your `wk-agent` and the state directory you passed, so pass the state directory of the collector that should receive the drafts, **do not commit it to a shared repository**, and generate it again after you upgrade or move the binary or the state directory. That is why a user-level location is preferable to a workspace file.
- Paths are quoted for POSIX shells and the text shows the card going in through a quoted heredoc. Git Bash on Windows works; PowerShell and cmd need the command adjusted by hand.
- The Claude Code and Copilot texts carry the front matter their tool expects. Check and edit the file like any other instruction file.

When you run the command, the assistant is asked to write one JSON card from what happened in the session (`title`, `intent`, `rationale`, and the arrays `decisions`, `constraints` and `open_questions`) with no secrets, personal data, code or pasted conversation text, and to run `wk-agent --state-dir DIR draft --client NAME` with the card on standard input. This relies on the assistant following the instruction. Ask for decisions and constraints as full short sentences: list items under 8 characters or a single word, statements of absence such as "none" and duplicates are dropped by validation, and `draft` reports how many were dropped.

`draft` needs the current server policy (so the server must be reachable and, for a hosted collector, the signed policy must be well formed; in `local` mode the configured inference endpoint must also still be allowed). It accepts only those six fields (anything else is refused) and at most 64 KiB, fills in the rest of the card (your service, the current policy revision, status `proposed`, completeness `partial`, no evidence) and screens the card for credentials and personal data. If a card fails screening, `draft` refuses it with a content-free reason and never echoes it. Remove the flagged content from the card instead of rewording it, because rewording tends to produce the same problem in different words. Screening is pattern-based: it reduces the chance of sharing something sensitive, it does not guarantee a clean card.

An accepted card is a **private, encrypted local draft that lives for 24 hours** (drafts built by rules or a model last one hour). `wk-agent drafts` lists it and `wk-agent submit --id ID --reviewed` sends it. Nothing reaches the team server before you do that. Assistant drafts are `partial` (the session was not captured), so owner synchronization, which uploads only complete drafts, never uploads them; they need your explicit `submit`. They carry an assistant-written origin, so a reviewer can publish them in the portal. Publishing makes the card searchable by everyone with access to its services and can show it in teammates' prompts, so read it again for anything the team should not see; screening is pattern-based and is not proof of perfect privacy. A server that predates this feature cannot publish them: `submit` then sends a plain partial draft and says so, and after upgrading the server you withdraw the card and run `/wk-summarize` again. `reset-capture --yes` discards pending assistant drafts with the others, and a draft made under an older policy revision is discarded when you submit it, so run the command again. Asking again for an identical card while one is pending keeps the pending draft. Asking again after you submitted it creates a new draft that you can review and submit; the server recognises identical cards and does not duplicate them. If you submitted a card and it was later withdrawn, submitting an identical card again is refused; change the card first.

Privacy: the assistant's provider already receives your conversation, and `/wk-summarize` sends nothing new to anyone; the card is a private local draft until you submit it. The instruction is developer-initiated: there are no new hooks and no automatic token spend. `prompt` and `draft` work in either mode.

## Private summaries and review

The daemon schedules drafting every 15 seconds (a local model call in `local` mode, a structural build in `rules` mode); one pass runs at a time. Each pass rotates through at most ten sessions using a durable metadata-only cursor, so held/older sessions cannot trap later sessions behind a fixed first page. Model calls may take longer than the scheduling interval. `process` runs it on demand. `drafts` shows only the current user's locally encrypted candidate summaries, up to 100 per page. Pass the returned `next_cursor` as `drafts --after CURSOR` to read subsequent pages. Submitted or expired drafts are not regenerated automatically from unchanged source/model/policy inputs. Changed inputs create a new draft identity; content-free closure guards remain bounded at 100,000 records. After checking one, `submit --id draft:HASH --reviewed` sends that summary as a server draft, with membership, consent, policy and source digest rechecked. Reviewers publish complete approved summaries, and assistant-written summaries, in the console. Partial hooks and rule drafts stay labeled partial and cannot be published.

Policy-auto remains disabled until the configured real model passes the privacy corpus and the team/developer policy is provisioned; rule drafts are always partial and never eligible. Regex screening and model classification are not proof of perfect privacy. No cloud summarization fallback exists.

## Instrumented chat and explicit exports

`chat --message-file ABS_FILE` (requires the `local` mode and a model; it is refused in `rules` mode) starts an explicitly managed local conversation and returns its session ID and visible response. Pass `--session managed:UUID` to continue. Every visible prompt/response is captured; only ordinary visible content is read from the model response. No file tools or agent reasoning are exposed. This is a controlled chat, not interception of an arbitrary existing desktop chat.

An explicit export is `{ "schema_version":1, "workspace":"/work/product", "product":"PRODUCT", "session_id":"SOURCE_SESSION", "messages":[{"id":"m1","role":"user","text":"synthetic request"}] }`. Import only that file using `import --file ABS_FILE --session SOURCE_SESSION --approve-transcript`. Metadata must match the current binding. Imports remain partial and do not scan adjacent files.

## Import existing agent memory

Start with what your coding agents already know. `wk-agent --state-dir /absolute/private/state memory scan` lists the `CLAUDE.md`, `.claude/CLAUDE.md`, `AGENTS.md` and GitHub Copilot instruction files in your approved service folder and this project's Claude Code notes (`~/.claude/projects/<project>/memory/*.md`), with a status for each. It is offline and prints no file content. Workspace-root files are reported as outside the approved service.

GitHub Copilot instruction files are `.github/copilot-instructions.md` (kind `instructions`) and `.github/instructions/*.instructions.md` (kind `scoped_instructions`; only files directly in that folder, not its subfolders) inside the service folder, labelled `repo:<service>/.github/...`. They follow the same scope, size, UTF-8 and symlink rules as `CLAUDE.md`; a symlinked folder on the way to `.github/instructions` is not listed. In a `*.instructions.md` file the leading `---` frontmatter (`applyTo`, `description`, …) is removed from the imported text, and `description` is passed to the local model as a hint for every section (in `rules` mode it is put in front of each section's text instead). Without a closing `---` line the file has no frontmatter and is imported as written.

Import with `memory import --yes`, adding `--file ABSOLUTE_PATH` for each Claude Code note you choose. The global `~/.claude/CLAUDE.md`, Codex's cross-project memories and other projects' notes are refused. Each note or section (split at the highest heading level used more than once) becomes one encrypted private record; `wk-agent process` drafts it with your local model (or, in `rules` mode, from the heading and text, see [Rule drafts](#rule-drafts-and-their-limits)) and the usual privacy screening, then `wk-agent drafts` shows each draft with its source. Nothing is shared until you submit a reviewed draft. With owner synchronization enabled, memory drafts made by a model upload as owner-only summaries like other drafts (rule drafts are partial and are not uploaded).

Re-running the import skips unchanged sections, replaces unsubmitted drafts of changed sections and links a changed, already-submitted section to its earlier summary (`updates`), so a reviewer can supersede it. Renaming or restructuring headings retires the old unsubmitted drafts and creates new ones without an `updates` link; reviewers supersede earlier summaries manually. Deleted sources are reported, not withdrawn. Memory files can contain credentials or personal notes: review every draft before submission, including the source label and heading that `drafts` shows. A submission reports `memory_link`: `recorded` when the summary was linked for later updates, `not_linked` when a newer import had already replaced or retired that section, or `failed` if the local record could not be written. After `failed`, the next import may draft that section again, so do not submit it twice.

Notes and limits:

- `scan` status `unchanged` means the content matches the last import; an unsubmitted draft that expired (drafts last one hour) is drafted again on the next `import`.
- Service-folder files that are too large (over 64 KiB), symlinked or not UTF-8 are skipped and counted as `rejected`; an explicit `--file` that fails stops the import before anything is written. A run over 200 sections also stops before writing.
- Claude Code note discovery is qualified only on macOS and Linux. Notes are found only when `HOME` is set and only when no folder below your home folder on the way to the notes (including `~/.claude`) is a symlink. Without them, `scan` lists no notes and may show earlier note labels under `removed`.
- `wk-agent process` reports `memory_drafting: failed` if the owner-only `memory-import.json` in the state folder cannot be read; other drafting and owner synchronization continue. Restore that file's permissions or contents; deleting it makes the next import start over.
- Frontmatter hint values now lose one matching pair of surrounding single quotes as well as double quotes (`description: 'Retry policy'` becomes the hint `description: Retry policy`). A Claude Code note whose `name` or `description` was single-quoted therefore counts as changed once on the next import: its unsubmitted draft is replaced, or a submitted one gets an `updates` link.

## Import past agent sessions

`wk agent-history --client claude-code|codex|copilot|cursor` brings sessions you already had with a coding agent in the approved folder into your private drafts, one reviewed session at a time. It only reads: it never modifies the agent's files, never runs in the background and has no bulk import. You list the folder's sessions, review the ones you want, export them by ID and import each export explicitly. The storage formats it reads are undocumented by their vendors and change between versions; the readers are tested against synthetic fixtures modelled on layouts observed on 2026-10-15, not certified against every version. A format change shows up as `unreadable_records` or `empty_conversations` counts rather than guessed text.

```sh
wk agent-history --client claude-code list --workspace /work/product
wk agent-history --client claude-code show --workspace /work/product --session claude-code:SESSION --max-chars 0
wk agent-history --client claude-code export --workspace /work/product --product PRODUCT \
  --out /private/exports --session claude-code:SESSION --yes
wk-agent --state-dir /private/collector import --file /private/exports/claude-code_SESSION.json \
  --session claude-code:SESSION --approve-transcript
wk-agent --state-dir /private/collector process
wk-agent --state-dir /private/collector drafts
```

`list` and `show` accept any `--workspace` folder; only `wk-agent import` enforces the enrolled binding, refusing an export whose `workspace` or `product` differs from the collector's. So use the folder you approved with `wk workspace approve` for exports you mean to import. `--source-dir DIR` (accepted anywhere after `agent-history`) replaces the client's default storage location for non-standard installs. For Claude Code and Codex it is the `projects` or `sessions` folder itself, not `CLAUDE_CONFIG_DIR` or `CODEX_HOME`; for Copilot and Cursor it is the VS Code or Cursor `User` folder.

1. **`list --workspace DIR`** prints JSON without any message bodies: `workspace`, and `sessions` with each session's `id`, `title` (the first line of the first prompt, or Copilot's custom title: likely secrets are masked first, then it is clipped to 80 characters; this one line is the only message text `list` shows), `created_at_ms`, `messages` and `complete` (every record was read and the session ends with a reply). Counts cover the client's whole history, which is read and held in memory before the `--workspace` filter applies, so a very large history needs correspondingly large memory: `outside_workspace` (sessions in other folders), `without_folder`, `unreadable_records`, `empty_conversations`, `skipped` (over the size limits below) and `dropped_injected` (prompt parts dropped as client-injected context).
2. **`show --workspace DIR [--session ID]...`** prints the selected sessions (all of the folder's sessions without `--session`) as Markdown with likely secrets masked. Messages are cut at `--max-chars` characters (default 1500, `0` for no limit). `export` writes whole messages, so review with `--max-chars 0`.
3. **`export --workspace DIR --product PRODUCT --out DIR --session ID... --yes`** writes one `wk-agent import` file per named session (text masked the same way). Every check runs before anything is created, so a refused export writes nothing:
   - `--yes` confirms you reviewed `show` output; each `--session` must be an ID `list` shows for this folder, named once.
   - `--out` must be an absolute path without `..`, outside the workspace (checked before and after the directory is created). A missing directory is created with mode 0700; an existing one must already be private (no group or other permissions), and its permissions are never changed.
   - A file is named after its session ID with the first `:` replaced by `_` (`claude-code_SESSION.json`). An ID with other characters outside `[A-Za-z0-9._-]`, or longer than 120 characters, has those characters replaced by `_` and a 12-character hash suffix so names stay distinct. Two sessions whose names differ only in case are refused.
   - Existing files are never overwritten: if any target file exists, nothing is written. Files are created owner-only (0600); if a write fails, the files this run created are removed.
   - The output lists the `exported` paths, `published: false` and the next command.
4. **`wk-agent import --file FILE --session ID --approve-transcript`** with the session ID from `list` (the file's `session_id`). An export may hold at most 5,000 messages and 8 MiB, with unique, non-empty message IDs. The import groups messages into **parts** of consecutive whole exchanges (a prompt and the replies that follow it) with at most 96 KiB of text and 256 messages each. An exchange with more than 256 messages is split into consecutive chunks; an exchange or chunk over 96 KiB becomes its own part, in which every message keeps a fair share of the limit and longer ones end in ` [truncated]`. Each part is recorded as its own private journal session, so `process` drafts **one summary per part**, from both prompts and replies. The output reports `captured_messages`, `parts`, `truncated_parts`, `process_runs_needed`, `coverage: partial_export` and `published: false`.
5. **`wk-agent process`** drafts at most ten sessions per run (a running collector does this every 15 seconds). `process_runs_needed` is the number of parts divided by ten, rounded up; it is a minimum, since other pending sessions share those runs. Review with `drafts` and submit with `submit --id DRAFT_ID --reviewed`; nothing is shared before that.

**Run `process` promptly.** Imported parts are private journal records that expire 24 hours after they are first written, so parts not yet drafted by then are lost, and the same export cannot be recorded again (the store never reopens an expired session ID). Re-export after the session has changed, or after editing it, to get new part sessions. Importing the same file again while its parts are still live adds nothing: the part sessions are named after a digest of the messages. If you export a session again after it continued, or its text changed, the digest changes and **every** part is recorded and drafted again, including parts drafted before. Submit only the drafts from the newest import and leave the earlier ones unsubmitted (unsubmitted drafts expire within an hour); a summary you already submitted is not updated or linked automatically.

**Folder matching.** A session is listed when its recorded folder, canonicalized, is the `--workspace` folder or a folder inside it (`/work/app2` does not match `/work/app`). Only absolute recorded folders count, and sessions without a folder are never assigned to a workspace.

**Limits.** Claude Code, Codex and Copilot session files over 32 MiB and sessions over 5,000 messages (after consecutive records of the same role are joined) are not read and are counted in `skipped`. Symlinked session files and folders are skipped silently (not counted); only a file swapped for a symlink between the check and the open is refused and counted as unreadable. Malformed lines and records of an unknown shape are counted, never guessed, and mark the session incomplete. Codex and Copilot use the session ID stored in the file only if it matches `[A-Za-z0-9._-]{1,128}` (and is not `.` or `..`); otherwise the file name without its extension under the same rule, otherwise `h-` and a hash of the file's path below the source folder. Claude Code uses the file name under the same rule. A session ID found in two files (two project folders, two Codex files) is listed once, from the most recently modified file, and each dropped copy counts as one `unreadable_records`.

**Injected context (Claude Code and Codex).** Agents store context they add (IDE selections, reminders, environment details, command output) as if you had typed it. A prompt part made only of `<name …>…</name>` blocks, at least one with a tool-style name containing `_` or `-` (`<ide_opened_file>`, `<environment_context>`), is dropped and counted in `dropped_injected`. Parts made only of plain-named tags such as `<task>…</task>`, and any part with other text, are kept. Each reader also drops parts starting with known injected blocks, listed below.

**Masking.** Titles, `show` and `export` mask likely credentials: private-key blocks, tokens with known prefixes (`sk-`, `ghp_`, `github_pat_`, `xoxb-`, `AKIA`, `eyJ`, `AIza`, …), long opaque strings that mix letters and digits, and the value assigned to a credential-like name (`password=…`, `API_KEY: …`, `"token": "…"`, `Authorization: Bearer …`) whatever its length. Masking also covers passwords and bare tokens in URLs (`https://user:PASS@host`, `https://ghp_…@host`), `sig`/`signature` query parameters, `curl -u user:PASS`, the argument of long credential flags (`--password X`, `--api-key X`), `Cookie:` and `Set-Cookie:` header values, and the indented lines below a credential key that ends its line (YAML `password:` or `token: |`). Masking is pattern-based and does **not** find:
- a secret with no credential-like name or known prefix that is shorter than 32 characters or has only letters or only digits, including secrets stated in prose ("my password is X");
- values after short flags (`-p X`, `-pX`);
- multi-line values other than indented blocks under a credential key (heredocs, strings split across lines);
- tokens in unusual positions (inside a URL path or fragment, embedded in other text without a separator);
- personal data such as names, email addresses, IP addresses and file paths, which are kept.

Read the `show` output and do not export a session that still contains a secret, personal data or text you would not share.

**Windows.** Modes 0700 and 0600 are not applied: the output directory and export files inherit the parent folder's ACLs. Choose an `--out` folder that only you can read.

### Claude Code

- **Location:** `$CLAUDE_CONFIG_DIR/projects`, else `~/.claude/projects` (`%USERPROFILE%\.claude\projects` on Windows). The `*.jsonl` files directly inside each project folder are read; subfolders are not.
- **Session:** ID `claude-code:<file name without .jsonl>`; folder: the first `cwd` on a prompt or reply record that is absolute and has no `.` or `..` component.
- **Read:** prompt text (string content or `text` parts) and reply `text` parts; consecutive records of the same role form one message.
- **Skipped:** thinking, tool calls and results, meta, compact-summary and transcript-only records, API-error and synthetic replies, interrupt markers, subagent sidechains, and injected prompt parts: those starting with `<command-`, `<local-command-`, `<task-notification`, `<system-reminder`, `<ide_`, `<user-prompt-submit-hook`, `<user-memory-input` or `<bash-`, those containing `<bash-stdout>` or `<bash-stderr>`, and tool-style tag blocks.

### Codex

- **Location:** `$CODEX_HOME/sessions`, else `~/.codex/sessions`; `*.jsonl` files up to `YYYY/MM/DD/`. `archived_sessions` is not read.
- **Session:** ID `codex:<session_meta id>`; folder: the first `session_meta` record's `cwd`, only when absolute and without `.` or `..` components.
- **Read:** `response_item` messages from the user and the assistant: `input_text` and `output_text` parts.
- **Skipped:** developer and system messages, reasoning, tool calls and their output, events, turn context, compaction records, attached images, and injected user parts: those starting (in any case) with `<environment_context>`, `<user_instructions>`, `<permissions instructions>`, `# AGENTS.md instructions for `, `<skill>` (with or without attributes), `<user_shell_command>`, `<turn_aborted>` or `<subagent_notification>`, and tool-style tag blocks. A new injected wrapper with a plain name needs its own marker: the tag rule only sees names containing `_` or `-`. Unknown roles and content part types are counted.
- **Known limits:** sub-agent threads are stored, and listed, as their own sessions; a forked session copies its parent's history, so the same exchanges can appear in two sessions.

### VS Code GitHub Copilot Chat

- **Location:** VS Code's `User` folder: macOS `~/Library/Application Support/Code/User`; Linux `$XDG_CONFIG_HOME/Code/User` when that variable is an absolute path, else `~/.config/Code/User`; Windows `%APPDATA%\Code\User`. For VS Code Insiders pass its `User` folder (for example `--source-dir "$HOME/Library/Application Support/Code - Insiders/User"`).
- **Session files:** `workspaceStorage/<hash>/chatSessions/<id>.json`, and the `.jsonl` change logs newer versions write, which are replayed in memory. When both exist the log is read; the `.json` is used only if the log cannot be read at all, and the session is then marked incomplete. A session copied into several storage folders is listed once, from the most recently modified copy. Logs over 200,000 lines are skipped.
- **Session:** ID `copilot:<sessionId>`; folder: the storage folder's `workspace.json` `folder`, only when it is a local `file://` folder. Remote, multi-root and empty-window chats have no folder.
- **Read:** the prompt's `message.text` and markdown reply parts. Inline file references appear as the file's base name, never its path. Prompts are kept whole: Copilot keeps attached context outside the prompt text, so the tag rule does not apply.
- **Never read:** the unsent draft in the chat input (`inputState`), attached variables and files (`variableData`, prompt `parts`), thinking, tool invocations, edits, code-block URIs, progress, confirmations, elicitations, commands, MCP notices, vulnerability details and error messages. Unknown reply part kinds are counted. A failed or cancelled request keeps only the reply text it produced.

### Cursor

`--client cursor` uses the Cursor reader described in [Cursor history import](#cursor-history-import-for-evaluation-sets), with the same `list`, `show` and `export` steps as above; the 32 MiB file and 5,000-message reader limits and the injected-context rule do not apply (the import's own 5,000-message and 8 MiB limits still do). `--source-dir` replaces only the Cursor `User` folder; agent transcripts are always read from `.cursor` in your home folder (`wk cursor-history --dot-cursor-dir` reads another location).

## VS Code and GitHub Copilot MCP

Copilot agent mode in VS Code can call the Workspace Knowledge MCP tools (`knowledge.search`, `knowledge.context`, `knowledge.overlaps` and the others listed in the [README](../../README.md#coding-agent-integration)). Add the server to the workspace's `.vscode/mcp.json` (or to your user MCP configuration). The file format is VS Code's, not this project's, and may change; check VS Code's MCP documentation for your version. VS Code uses a top-level `servers` object; each server has `"type": "stdio"` or `"type": "http"`.

**Stdio bridge.** `wk-mcp` takes no arguments and reads three environment variables: `WK_SERVER_URL`, an `https://` origin or a literal loopback `http://127.0.0.1:PORT` / `http://[::1]:PORT`, without path, query or credentials; `WK_ACCESS_TOKEN`, an access token for the server (at least 32 characters; `WK_DEV_TOKEN` is the loopback development fallback); and `WK_PRODUCT`, the product ID. Keep the token out of the file with a VS Code input prompt:

```json
{
  "inputs": [
    {
      "type": "promptString",
      "id": "wk-access-token",
      "description": "Workspace Knowledge access token",
      "password": true
    }
  ],
  "servers": {
    "workspace-knowledge": {
      "type": "stdio",
      "command": "wk-mcp",
      "env": {
        "WK_SERVER_URL": "https://knowledge.example",
        "WK_PRODUCT": "PRODUCT",
        "WK_ACCESS_TOKEN": "${input:wk-access-token}"
      }
    }
  }
}
```

Use the absolute path of `wk-mcp` as `command` if it is not on the `PATH` VS Code sees.

**Remote endpoint.** Point an HTTP server at the product's route:

```json
{
  "servers": {
    "workspace-knowledge": {
      "type": "http",
      "url": "https://knowledge.example/mcp/PRODUCT"
    }
  }
}
```

Each request authenticates with an OAuth access token from your identity provider, discovered through the server's protected-resource metadata; see [remote MCP](../architecture/remote-mcp.md) for the server configuration. Sign-in from VS Code has not been certified against a real identity provider; if it does not work with yours, use the stdio bridge.

MCP gives Copilot retrieval only. Copilot has no capture hooks and gets **no automatic prompt-time overlap notices** yet; add a project rule (for example in `.github/copilot-instructions.md`) asking the agent to call `knowledge.context` and `knowledge.overlaps` before editing shared code.

## Claude Desktop Enterprise

On an isolated broker user account only, set `WK_COMPLIANCE_BROKER=1` and a read-only `WK_COMPLIANCE_READ_KEY`. Run `compliance --binding EXACT_GRANT_FILE`. The grant contains `chat_id`, `provider_org`, `provider_user`, `root`, `service`, `approval_hash`, and Unix `expires_at`. It must be issued for an explicitly approved chat by the operator. The client retrieves only that chat, validates returned ownership, follows bounded message pagination, skips reasoning and attachment downloads, and never lists organization-wide chats. Text-only coverage remains partial. Source edits conflict with old event IDs and hold the session. Broad API entitlement is never treated as workspace consent. This pull command has no remotely exposed broker API; broker deployment/entitlement remains a certification prerequisite.

Official protocol references checked during implementation: [Claude Code hooks](https://code.claude.com/docs/en/hooks), [Cursor hooks](https://prod.cursor.com/docs/hooks), [Gemini hooks](https://geminicli.com/docs/hooks/reference/), [Claude Compliance content](https://platform.claude.com/docs/en/manage-claude/compliance-content-data). These document interfaces; they do not certify our adapters.

## Cursor history import (for evaluation sets)

`wk agent-history --client cursor list|show|export` now covers the review and export workflow for Cursor, with the same options as the other clients (see [Import past agent sessions](#import-past-agent-sessions)); `wk cursor-history` is kept as an alias for it and additionally offers `diagnose` and `cards`. The notes below describe the Cursor reader.

`wk cursor-history` reads your own local Cursor chats from a read-only snapshot of Cursor's undocumented SQLite storage (global `cursorDiskKV` composer/bubble records and per-workspace `ItemTable` entries, including legacy chat tabs). It also reads newer Cursor agent transcripts from `~/.cursor/projects/<path-slug>/agent-transcripts/*.jsonl`; a project is attributed only when its slug resolves to an existing directory, otherwise the chat stays unattributed. Override that location with `--dot-cursor-dir`. It never modifies Cursor's files and works while Cursor is running. Only visible message text is read; thinking and tool payloads are ignored, and missing records are counted rather than guessed.

1. `wk cursor-history list` (or `wk agent-history --client cursor list --workspace DIR`) shows titles and counts per workspace folder, plus unattributed chats (which are never assigned to a workspace).
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
