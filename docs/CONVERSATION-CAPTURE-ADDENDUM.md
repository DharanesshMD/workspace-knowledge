# Conversation capture and graph provenance — revised design

Date: 2026-09-30. Supersedes the earlier assumption that Claude Desktop general chat has only cooperative-summary integration. Research-backed design; no account access or live capture has been tested.

## Decision

Implement multiple capture transports behind one normalized event contract:

1. Native client hooks for coding clients with verified event coverage.
2. An optional Claude Enterprise Compliance API connector for actual Desktop/web chat content.
3. An instrumented workspace chat client for reliable first-party capture without Enterprise dependency, using approved model APIs or local inference.
4. Explicit export import for historical reconciliation.
5. An optional browser extension for best-effort observed web conversations; never certify this as complete native Desktop capture.

MCP remains the retrieval/contribution interface. Calling an MCP tool with a paraphrase does not become a transcript source. Completeness is scoped to what the connector demonstrably observes, not the entire developer's thought process.

## A. Claude Desktop through the official Compliance API

The current chat-content documentation restricts these endpoints to Enterprise organizations. Broader Compliance API eligibility does not imply that every plan has these chat endpoints. Organization-owner setup is required; validate entitlement before enabling the connector. Use a read-only Compliance Access Key, never a delete scope.

Official entry points:

- `GET /v1/compliance/apps/chats`
- `GET /v1/compliance/apps/chats/{claude_chat_id}/messages`

The documented interface supplies message history and related file metadata. Source retention/deletion limits recovery. Implement pagination and idempotent updates; certify exact content, edit and branch coverage with synthetic live sessions instead of assuming all history variants are present. Sources: [content API](https://platform.claude.com/docs/en/manage-claude/compliance-content-data), [access/setup](https://support.claude.com/en/articles/13015708-access-the-compliance-api).

### Workspace association and privacy

A Desktop general chat has no trustworthy filesystem cwd. Before capture, the developer binds an identified work conversation to an enrolled workspace, or enrolls a dedicated work-only Claude project with clearly disclosed automatic chat inclusion. Persist provider organization/user/chat IDs, local binding, allowed services and consent version. Do not infer authorization from a chat title, open app, last active folder, or matching code terms.

Default to polling explicitly bound chat IDs. Start at a configurable 60-second interval with exponential backoff/rate-limit handling; this is a proposed cadence, not a provider freshness SLA. Bound-chat retrieval avoids listing all organizational conversations. Automatic discovery of new chats inside an approved provider project requires a separately authorized metadata-discovery mode and verification of current filter/update semantics. Never download every conversation body and filter afterward. Unknown or unapproved chats remain untouched.

A provider key can have broader access than our workspace policy. Keep it in an isolated on-premises capture broker, never distribute it to developer machines or ordinary agents. The broker authenticates the developer, validates binding ownership and consent on every request, and retrieves only allowed IDs. Permission enforcement is an application-level narrowing, not a claim that the provider key is folder-scoped.

This route explicitly changes the default data boundary: raw provider content transits the capture broker. Enabling it requires approval of that processing mode. Prefer memory-only broker relay to the authorized developer's local collector for private encrypted storage and local summarization. No shared raw transcript endpoint, payload logs, disk cache, traces, or crash dumps. An optional centrally processed mode requires its own policy; ordinary team users still receive summaries only. Privileged broker administrators remain part of the trusted processing boundary.

Attachment bytes require separate approval. Missing attachments, unavailable branches and deleted source content are represented as gaps. On source deletion or scope withdrawal, retract derived knowledge unless a separately authorized retention policy applies; never fetch deleted content through alternate routes.

## B. Non-Enterprise: instrumented workspace chat

Add `wk chat --workspace <approved-root>` with a local web UI. This is an alternative chat surface, not a plugin that observes arbitrary Claude Desktop chats. It invokes a documented model API or an approved local model using separately configured credentials. Do not reuse Desktop session cookies or assume a consumer subscription grants API credits.

Capture within our own request lifecycle:

- Assign conversation, branch, user-message and request IDs before dispatch.
- Store the exact user-authored message privately; distinguish it from retrieved context, system instructions and transformed provider payloads.
- Record visible assistant text, tool calls/results allowed by scope, stream completion, errors, cancellation and retries.
- Maintain parent/revision IDs for edits and regenerations rather than overwriting messages.
- Mark partial streams partial; persist a completed turn atomically before scheduling summary extraction.
- Exclude hidden reasoning blocks, credentials and out-of-scope attachments.
- Bind workspace before the first message and prevent silent project switching within a conversation.

This gives deterministic capture of eligible messages passing through our client, including on Linux. It does not capture conversations made in another app. External-model egress remains separately approved; local model mode allows a fully internal path.

## C. Export import and browser extension

Claude documents user-requested exports for personal accounts; use these for selected historical conversations, not real-time capture. Parse locally, display only enough metadata for the owner to select work chats, then require workspace binding. Do not upload the whole export. Label imported history as batch capture and record import time separately from message time. [Official export documentation](https://support.claude.com/en/articles/9450526-export-your-claude-data)

A browser extension can observe rendered web content through content scripts after explicit site/session permission. That is a feasibility path for claude.ai in supported browsers, not proof of a complete transcript API. Virtualized/off-screen history, edits, regenerations, attachments and UI changes can all cause gaps. It must mark capture as observed/partial and disable itself on unknown layouts. Never collect other tabs, cookies or authentication tokens. [Chrome content scripts](https://developer.chrome.com/docs/extensions/develop/concepts/content-scripts)

Do not make native Desktop database scraping, accessibility scraping or TLS interception the correctness foundation. No stable complete local Desktop transcript interface was established by this research. These would need separate evidence and consent, and can expose unrelated content. Do not bypass app security mechanisms to force capture.

## D. Make the graph auditable

Full conversation capture alone does not prove an implementation exists. Build a typed evidence graph from validated summaries:

`Product → Service → Feature → Decision → Claim → Evidence`

Allowed typed links include `belongs_to`, `affects`, `implements`, `depends_on`, `supports`, `contradicts`, and `supersedes`. Store this as constrained relational node/edge tables initially; a separate graph database is not required.

Each claim includes source kind, opaque source/message reference, observed revision, coverage boundary, missing-context flags, extraction version, and implementation verification status. Shared references identify provenance without giving colleagues a raw-message retrieval endpoint. Private source-to-message mappings remain within the capture boundary.

Separate fields:

- `capture_mode`: hook, compliance, instrumented_chat, export, browser_observed, cooperative.
- `coverage`: complete_within_boundary, partial, unknown.
- `verification`: reported, code_observed, tests_verified, owner_confirmed.
- `source_boundary`: conversation/branch/message interval actually observed.

Only label coverage complete within a tested boundary after pagination/stream completion and gap checks. Never equate that label with truth. Preserve contradictory decisions and attach code/test evidence. Deletion and re-extraction must remove or supersede derived claims and edges transactionally, with tombstones preventing restoration from resurrecting withdrawn claims.

## E. Additional implementation tasks

### Task 9A — Compliance connector and explicit chat binding

Create `crates/adapters/src/claude_compliance.rs`, `crates/capture_broker/src/{main,bindings,relay}.rs`, binding migrations, and synthetic API fixtures. Add provider capability checks and isolated key storage. Implement authenticated bound-ID retrieval, pagination, checkpoints, memory-only relay, retry and source-deletion propagation.

Tests: unbound chat denied before provider call; wrong owner/organization denied; revoked consent aborts sync; pagination does not duplicate messages; 429 retries within limits; unavailable/deleted content creates a gap; broker logs contain no payload; offline local collector does not cause raw disk spooling. Certify with a permitted Enterprise test organization before claiming Desktop chat capture works.

### Task 9B — Instrumented chat and export import

Create `crates/chat/src/{session,provider,stream}.rs`, `web/src/chat/`, and `crates/capture/src/import.rs`. Implement approved workspace-bound chat and selected export import. Integrate capture with existing local encryption/summarization.

Tests: every submitted user message and completed visible answer is correlated; cancellation is partial; retries/edits/branches remain distinct; out-of-root attachments denied; no API-key or hidden-reasoning persistence; export selection excludes personal chats; mismatched workspace cannot publish. Run with a fake model first, then an approved real provider/local model. Compare transcript counts/content with the controlled test client.

### Task 10A — Provenance graph and reconciliation

Create `crates/knowledge/src/graph.rs` and migrations for claims/nodes/edges with tenant-consistent references. Test mixed-service authorization, contradictory decisions, interrupted capture, missing attachments, changed messages, source deletion, claim correction and backup tombstone replay. Graph traversal uses the same permission checks as search, including intermediate nodes. Demonstrate that a conversation claim never becomes tests_verified without matching code/test evidence.

Browser observation is an optional experimental adapter after these paths; it cannot satisfy the release requirement for full native Desktop capture.
