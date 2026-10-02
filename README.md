# Workspace Knowledge

Open-source, on-premises knowledge sharing for developers working in explicitly approved product folders. Raw conversations remain inside a private capture boundary; agents and teammates retrieve scoped summaries.

**Status: development implementation, not a certified production release.** The implementation spans collector, summaries, shared API, MCP, console and environment tooling. The [completion matrix](docs/architecture/completion-matrix.md) records remaining implementation and live-certification gaps. In particular, universal native capture, policy-auto, live-client remote MCP OAuth, a configured Jev endpoint and signed releases are not claimed complete.

## Components

| Command | Purpose |
|---|---|
| `wk` | Local workspace approval, scope checks, pause/revoke, card validation, replay, search/context and companion commands |
| `wk-agent` | Native-keyring collector, authenticated local receiver, hook setup, explicit imports, instrumented local chat, private summaries, doctor |
| `wk-server` | PostgreSQL knowledge API, OIDC verification, provisioning, migrations, revisions, withdrawal, feedback, provenance graph and console hosting |
| `wk-mcp` | Standard stdio tools: knowledge search/get/context/record/feedback, workspace status and environment schema |
| `wk-env` | Separate OpenBao schema/import/run/export broker; no secret-value MCP tool |

The native collector uses encrypted SQLite records, fixed raw retention, durable closed-session guards, duplicate detection and persistent mixed-context holds. Capture occurs only after local approval and a current server membership check. Missing keyring, consent, or membership fails closed. A hook failure does not block the developer's AI agent; it stops knowledge submission until a clean-session reset.

Cursor, Claude Code and Gemini hook adapters collect documented visible fields with **partial, unverified coverage**. Codex and Antigravity native capture remain unsupported; standard MCP retrieval and the managed chat alternative are separate capabilities. Claude Desktop general chat has an explicit Enterprise bound-chat text pull for an isolated broker, requiring entitlement and chat consent. MCP alone does not capture conversations.

Summaries are generated through an explicitly configured loopback OpenAI-compatible model endpoint. Candidate summaries stay encrypted locally; `drafts` and `submit --reviewed` support owner review. Submitted summaries become server drafts before reviewer publication. No cloud fallback, hidden-reasoning collection, global transcript scan or automatic `.env` collection exists. Automatic policy publication requires further privacy evaluation and remains disabled.

## Build and development

Install the pinned Rust toolchain through rustup, Node.js 22 and Python 3. Linux collector builds require `libdbus-1-dev` and `pkg-config`, and runtime Secret Service. Native keyring availability is required for persistent collector use.

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cd web
npm ci
npm run build
```

This checkout has an ignored local toolchain; `./scripts/cargo` uses it and otherwise falls back to system Cargo. On Windows binaries have `.exe` suffixes. CI describes Linux/macOS/Windows builds; configured CI is not proof that those platforms passed.

The synthetic scope demo does not access real chats:

```sh
python3 scripts/demo.py
```

Use a disposable PostgreSQL database named `wk_test` for integration checks:

```sh
export WK_TEST_DATABASE_URL='postgres://USER:PASSWORD@127.0.0.1:5432/wk_test'
cargo test -p wk-knowledge --test access --locked -- --ignored
python3 scripts/integration.py
```

The normal workspace test command skips the database tests; run them explicitly. The smoke script writes synthetic cards. Never use a valued database.

## Start a local development service

Create a dedicated PostgreSQL database named `wk_dev`. Development mode seeds one identity for product `demo` with `frontend`, `backend`, and `ai-engine` membership. Use synthetic data only.

```sh
export WK_DATABASE_URL='postgres://USER:PASSWORD@127.0.0.1:5432/wk_dev'
export WK_DEV_TOKEN="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
export WK_PORT=4318
export WK_WEB_DIR="$PWD/web/dist"
export WK_CONSOLE_ORIGIN='http://127.0.0.1:4318'
target/debug/wk-server --dev
```

Open the console at the local origin, connect to `demo` using the generated token, and use approved summary cards. Development identity can review its own drafts. Production identity provisioning and migration commands are described in [backend operations](docs/architecture/backend.md).

Configure an MCP client with the absolute `wk-mcp` executable, `WK_SERVER_URL`, `WK_PRODUCT`, and `WK_ACCESS_TOKEN` (or `WK_DEV_TOKEN` for development). It supports HTTPS origins or literal loopback HTTP. No client profiles have been modified by development. Clients may send returned summaries to their model provider; deployment egress policy must account for that.

## Setup and operations

- [Collector, hooks, managed chat and Desktop capture](docs/adapters/setup.md)
- [Private journal and retention boundaries](docs/architecture/private-capture-journal.md)
- [Shared service and identity configuration](docs/architecture/backend.md)
- [Console](docs/architecture/console.md)
- [Remote MCP](docs/architecture/remote-mcp.md)
- [Signed evidence connectors](docs/architecture/evidence-connectors.md)
- [Jev decision layer](docs/architecture/jev-decisions.md)
- [Operations](docs/operations/install.md)
- [Implementation ledger](docs/PROGRESS.md)
- [Original full plan](docs/docs/superpowers/plans/2026-09-30-workspace-knowledge.md)

Keep the secret broker independently authorized. Capture keys, identity tokens and OpenBao values never belong in source control, shared knowledge cards, logs or MCP output. Source scanners, classifiers and operating-system isolation have limits; see [security reporting](SECURITY.md) and the documented certification gates.

## License

Apache-2.0. See [LICENSE](LICENSE), [NOTICE](NOTICE), and [contribution policy](CONTRIBUTING.md). No telemetry or hosted account is required for synthetic foundation checks. Deployment, account access, real conversation capture, signing and publication require explicit operator setup.
