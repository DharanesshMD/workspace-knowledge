# Agent memory import

Approved intent: solve the empty-on-day-one problem by turning a developer's existing coding-agent memory into **private draft summaries** that follow the normal review path. Inspired by CortexDB's `import_agent_memory`, adapted to this project's boundaries: nothing is shared without explicit owner submission and reviewer publication, absolute local paths never enter summaries, and no new server API or trust boundary is added. User decisions (2026-10-14): repository files plus per-project notes, one draft per note or heading, skip-unchanged/update-changed re-import, and the existing local drafting pipeline (approach A). Revised the same day after code review: dedicated memory records instead of journal events, a content-free map file, service-folder-only repository files, generation IDs for explicit re-drafting, and linked replacement drafts instead of server corrections.

## Commands and sources

`wk-agent --state-dir DIR memory scan` is read-only and makes no network request. It lists candidate sources with a label, kind, section count and status: `new`, `unchanged`, `changed`, `removed` (imported earlier, now missing) or `rejected` with a fixed reason. It never prints section text.

`wk-agent --state-dir DIR memory import --yes [--file PATH]...` imports. Without `--yes` it fails before reading content.

| Source | Location | Approval |
|---|---|---|
| Repository instructions | `CLAUDE.md`, `.claude/CLAUDE.md` and `AGENTS.md` in the agent's configured service folder | Included by `--yes`; each path must pass the existing enrollment scope check (`Binding::eligible`) |
| Claude Code project notes | `~/.claude/projects/<encoded workspace root>/memory/*.md`, excluding the `MEMORY.md` index | Each file must be named with `--file`; `scan` suggests them |

Workspace-root instruction files are outside every approved service, so `scan` reports them as `rejected: outside_approved_service`. The agent is bound to one service and reads only that service's files, matching the service tag on its drafts.

The encoded folder name replaces every character other than ASCII letters, digits and `-` in the canonical workspace root with `-`, matching Claude Code's convention. The mapping can collide, so the folder is only a suggestion; explicit `--file` approval is the authority. A `--file` must canonicalize into either the workspace (then it must pass scope) or exactly that encoded memory folder; anything else is rejected.

Always excluded: the global `~/.claude/CLAUDE.md`, Codex's cross-project `~/.codex/memories`, other projects' memory folders, symlinks (the path or any component after canonicalization differing from the given path), non-UTF-8 files, files over 64 KiB and runs over 200 sections (the run fails before writing anything).

## Splitting

A Claude project note is one section. YAML frontmatter is removed; its `name` and `description` are kept as title hints. Instruction files split at their highest heading level used more than once (otherwise the highest level present). Text before the first heading is a preamble section; whitespace-only sections are dropped. Fenced code blocks are not scanned for headings.

Each section has a stable key: the source label, the heading path and the occurrence index of a repeated heading. Labels are `repo:<relative path>` or `claude-memory:<file name>`; absolute paths never enter records, drafts or output. The section text given to the model is the label, the heading path, the optional hints and the body.

## Records and processing

Import requires the same current enrollment, server policy check and clean capture state as `wk-agent import`. Each new or changed section is written to the encrypted local store as a dedicated record `memory:<sha256(key, generation)>` (one-day retention, like other private records) holding the label, heading path, hints, body, policy hash and generation. Memory records do not use the conversation journal: its event schema and path holds are built for conversations and would hold external notes.

`process` scans `memory:` records and drafts each one, then acknowledges the record. It reuses `wk_summarize::extract`, the `processing_current` re-checks around inference, the sensitive-content holds and `wk_summarize::card`. A whole note is present, so the card is marked complete. Draft metadata records the label and heading for `drafts` output only; the card body does not contain it. If owner synchronization was already enabled, these drafts follow that existing opt-in like any other draft. Nothing publishes automatically.

## Re-import

The encrypted store keeps records for at most one day and never reopens an expired or acknowledged ID, so the map is a separate owner-only file `memory-import.json` in the agent state folder, written atomically. It holds no section text: per hashed section key, the content hash, generation, current draft ID and, once submitted, the summary ID. An explicit re-run uses a new generation, so a re-drafted section gets new record and draft IDs; nothing regenerates without the developer running `import`.

| State | Action |
|---|---|
| Unchanged, draft pending or already submitted | Skip |
| Unchanged, draft expired without submission | Increment the generation and write a new record |
| Changed, draft not submitted | Acknowledge the old draft and record; increment the generation and write a new record |
| Changed, already submitted | Increment the generation and write a new record whose draft carries `updates: <summary id>` |
| Source removed | Report in `scan`; withdraw nothing |

Device-connected collectors may only send new summaries and owner notes, so an updated section is submitted as a **new** summary. `drafts` shows `updates: <summary id>`, and the reviewer retires the earlier summary with the existing supersede action. Successful submission records the returned summary ID in the map.

## Errors and privacy

Existing gates apply unchanged: capture gaps block, policy is checked before files are read, approval is re-checked around each inference, and model failures or sensitive matches hold content without echoing it. `import` reports counts of written, unchanged, held and rejected sections. Imported memory may contain credentials or personal notes; the existing screening and owner review are the controls, and the docs tell developers to review drafts before submission.

## Testing and documentation

Unit tests: heading selection, preamble, frontmatter, fenced code, duplicate headings, empty sections and size limits; folder encoding including dot and collision cases; global and cross-project exclusion; symlinks and out-of-scope paths; every re-import state, including the `updates` link and map update on submit. An end-to-end test uses the existing loopback model fixture and synthetic files for scan → import → process → drafts, and asserts `scan` makes no network request and no absolute path appears in records or drafts.

Document the workflow in `docs/adapters/setup.md`, add a README feature row and a changelog entry.
