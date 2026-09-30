# Workspace Knowledge

An open-source, on-premises knowledge system for AI-assisted development in explicitly approved product folders.

**Status: working development foundation.** Local consent, offline capture inspection, encrypted outbox, PostgreSQL summary storage, a loopback API, and a read-only MCP bridge are implemented. Live conversation capture and automatic knowledge generation are not implemented yet. Use synthetic data only until the production privacy and identity controls are complete.

## Implemented

- `wk` CLI: local workspace approval, scope checks, pause/revoke, sanitized card validation, offline synthetic JSONL replay.
- Service-folder boundaries: reject personal/sibling paths, sensitive filenames, symlink aliases, unapproved nested repositories, changed scope, and replaced roots.
- Private outbox library: SQLite with XChaCha20-Poly1305 authenticated payload encryption, duplicate protection, bounded payload quota, expiry and acknowledgment.
- Replay correlation: separate parallel agents/turns, flag mixed-session context, reject conflicting duplicates, and report completion without printing conversation text.
- Strict shared-card contracts that reject raw-payload fields and untrusted implementation-status promotions.

- PostgreSQL cards: private drafts, reviewer publication, tenant/service membership enforcement, idempotency, policy revision checks, and full-text retrieval.
- Development API: authenticated loopback HTTP with sanitized errors.
- Standard stdio MCP: `knowledge.search`, `knowledge.get`, and bounded `knowledge.context`.

The outbox accepts a caller-supplied encryption key; production OS credential storage and daemon integration are not implemented. It is deliberately not exposed through the CLI yet. Scope checks classify metadata and are not a secure file-opening primitive or an agent sandbox. Replay eligibility is local inspection only, never authorization to publish.

## Build

Install Rust via [rustup](https://rust-lang.org/tools/install/). The toolchain is pinned in `rust-toolchain.toml`.

```sh
cargo test --workspace --locked
cargo run -p wk-cli -- --help
```

This development checkout also has a project-local Rust installation. On this machine use `./scripts/cargo` instead of `cargo`; the wrapper falls back to system Cargo elsewhere. The `.tooling` directory is ignored and not distributed.

## Safe demonstration

Requires Python 3 and a built `wk` binary. Creates temporary synthetic product folders and removes them afterward. No real chat or project data is read.

```sh
cargo build -p wk-cli
python3 scripts/demo.py
```

On Windows the script automatically selects `target/debug/wk.exe`.

For local consent commands, provide a dedicated private state directory outside the product:

```sh
wk --state-dir /private/state/workspace-knowledge workspace approve /work/product \
  --product example --service backend --service frontend --yes
wk --state-dir /private/state/workspace-knowledge workspace check /work/product /work/product/backend
wk --state-dir /private/state/workspace-knowledge workspace pause /work/product
```

`--yes` approves only the explicitly listed local services. Responses say `team_authorized: false` and `live_capture_enabled: false` until the server and enrollment protocol are implemented. Do not point the state directory at a shared existing folder. Private state writes are atomic; Unix state directories require mode 0700. Windows ACL hardening still needs implementation and native certification.

## Development API and MCP

Use a dedicated PostgreSQL database named `wk_dev` (or `wk_test` for tests). The server creates its development schema and seeds one development identity for product `demo`, covering `frontend`, `backend`, and `ai-engine`. It is not a production authentication service.

```sh
cargo build --workspace --locked
export WK_DATABASE_URL='postgres://USER:PASSWORD@127.0.0.1:5432/wk_dev'
export WK_DEV_TOKEN="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
export WK_PORT=4318
target/debug/wk-server --dev
```

In an MCP client, configure the absolute path to `target/debug/wk-mcp` (Windows: `wk-mcp.exe`) as a stdio server, with these environment variables:

- `WK_SERVER_URL`: `http://127.0.0.1:4318`
- `WK_DEV_TOKEN`: the same generated development token
- `WK_PRODUCT`: `demo`

Registration in individual clients has not been tested. MCP provides retrieval here; it does not observe their conversations. Avoid committing credentials in client configuration.

The API accepts `POST /v1/cards` to create a strict summary draft, `POST /v1/cards/{id}/publish` for authorized review, `POST /v1/search`, and `GET /v1/products/{product}/cards/{id}`. See [the executable smoke test](scripts/integration.py) for a complete synthetic request example. The development identity can review its own draft; production separation of duties is pending. Schema validation rejects raw fields but is not semantic redaction: do not submit real conversations or secrets.

To run database checks against a disposable `wk_test` database:

```sh
export WK_TEST_DATABASE_URL='postgres://USER:PASSWORD@127.0.0.1:5432/wk_test'
cargo test -p wk-knowledge --test access --locked -- --ignored
cargo build --workspace --locked
python3 scripts/integration.py
```

The normal workspace test command skips database tests; the separate PostgreSQL CI job runs them explicitly. The smoke test writes synthetic cards, so never point it at a valued database.

## Development roadmap

The [build package](docs/START-HERE.md) and [capture revision](docs/CONVERSATION-CAPTURE-ADDENDUM.md) define the full product. [Progress and deviations](docs/PROGRESS.md) distinguish implemented foundations from pending plan requirements.

Next: native key management and authenticated daemon IPC; production identity and enrollment; real client capture adapters; local sanitization and summarization with policy-approved publication; Enterprise capture broker and instrumented chat; evidence graph; environment broker; console and release packaging.

macOS arm64 tests run locally. CI is configured for Linux, macOS, and Windows; this is not a claim those remote runs have passed. Live client, provider, Windows ACL, and cross-platform filesystem certification remain pending. Real PostgreSQL 17 integration and HTTP-to-MCP smoke checks have passed locally using a disposable Docker database.

## Privacy

Raw conversations belong only in the private capture boundary. Shared knowledge will contain sanitized summaries and provenance. Secrets belong in a separate vault/broker, never knowledge search. Local consent does not grant team access, and full transcript capture does not prove an implementation exists.

## License

Apache-2.0. See [LICENSE](LICENSE). Dependencies retain their own licenses. No telemetry, hosted service, API credential, or provider account is required to run these foundation tests.
