# Private capture journal

`wk_capture::journal::ingest` connects normalized events to the encrypted local store. This increment is a library integration tested with synthetic events. It neither installs client hooks nor captures actual desktop conversations. The eventual daemon must provide native key storage and authenticated IPC before enabling it with real data.

## Data flow

1. A trusted adapter supplies one schema-v1 event. The journal rejects unknown fields/kinds, invalid identities, and input larger than 256 KiB. Hidden reasoning is not a supported event kind.
2. Root, product and source session identify a single record through SHA-256. Parallel agents in the same source session share its hold state; event deduplication uses agent and event IDs. Source session IDs must remain stable across adapter reconnects and distinct across independent clients.
3. The current binding is checked for every event. An excluded or unknown path, pause, changed policy, explicit capture gap, or conflicting duplicate holds the session. Earlier payloads are removed from the logical record. The rejected event's path and text are not persisted.
4. Otherwise the event is added atomically, with a new AEAD nonce. Exact retries do not add another event. The transaction reads and writes under SQLite's immediate transaction lock, preventing lost updates across connections.
5. Receipts contain only counts and state. They never authorize publication or assert a complete transcript. No raw-data HTTP or MCP endpoint was added.

## Retention and closure

The first write sets a maximum 24-hour raw-payload deadline. Updates never extend it. Cleanup runs during writes and pending reads; a future daemon must schedule it during idle periods. A session is limited to 768 KiB serialized state or 1,000 events. Crossing either limit replaces its payload with a hold, rather than silently truncating it.

Opaque ID guards are retained separately after raw expiry or acknowledgment. An expired/acknowledged session ID cannot be recreated through either write API. A client must begin a genuinely new session; reconnects must not mint new IDs to bypass a hold. Guards contain no paths or conversation text, but are pseudonymous metadata and reveal counts. Their 100,000-record cap is separate from the encrypted payload quota; reaching it rejects new sessions. A safe administrative reset/forget lifecycle is still pending, so do not delete guards while an adapter can replay old sessions.

Logical removal and SQLite secure-delete do not guarantee forensic erasure from filesystem snapshots, backups, swap, or storage-controller copies. Raw text temporarily exists in process memory. This implementation does not claim that all allocations containing text are zeroized.

## Required adapter and daemon behavior

- Authenticate the event source and bind it to enrolled scope. Client-supplied paths alone do not prove what an agent read.
- Observe every relevant context path; pasted personal content needs the later local privacy classifier. A working directory is not proof of transcript privacy.
- Treat parse, quota, database-lock, and storage failures as capture gaps. Do not proceed with automatic publication on errors. Inputs without trustworthy session identity cannot safely update a particular hold.
- Supply trusted time; wall-clock rollback handling and periodic expiry require daemon integration.
- Use a dedicated private database with an OS-protected key and permissions. Neither key-store installation nor authenticated IPC is implemented here.
- Read only explicit, session-scoped transcript sources; never scan global history to discover conversations.
- Revalidate consent and membership before any summarization/publication. A stored session and an offline replay report are not publishing grants.

## Verification

Synthetic tests exercise restart deduplication, conflicting retries, child-agent gaps, sibling-folder rejection, policy/pause holds, oversized sessions, retention plus restart, wrong keys, atomic rollback, independent SQLite connections, and reopening prevention after acknowledgment/expiry. No live-client compatibility certification is implied.
