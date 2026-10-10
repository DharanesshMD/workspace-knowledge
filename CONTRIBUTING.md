# Contributing

Use synthetic fixtures only. Never submit team conversations, credentials, `.env` files, customer data, or captured client histories in issues, tests, screenshots or pull requests. Explain the privacy boundary and relevant negative tests for changes.

## Build and test

Install the pinned Rust toolchain and platform prerequisites from [installation](docs/operations/install.md#install-the-tools). On POSIX, `./scripts/cargo` uses the checkout's optional local toolchain and otherwise system Cargo; on Windows use `cargo` directly.

```sh
./scripts/cargo fmt --all -- --check
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
./scripts/cargo test --workspace --locked
./scripts/cargo build --workspace --locked
python3 -m unittest discover -s scripts/tests -v
python3 scripts/demo.py
```

POSIX installer tests use a disposable prefix and a build fixture; they never install into your home directory. They are skipped on Windows. The demo uses the actual built CLI with synthetic temporary state.

Normal Rust tests skip PostgreSQL cases. To include them, configure `WK_TEST_DATABASE_URL` through your environment for a disposable database named `wk_test`, then run:

```sh
./scripts/cargo test --workspace --locked -- --include-ignored
python3 scripts/integration.py
```

Never point integration scripts at a valued database. OpenBao and native-client acceptance also need explicitly configured disposable instances; report skipped checks honestly.

For the web console, install Node.js 22 and run:

```sh
cd web
npm ci
npx playwright install chromium
npm test
npm run build
```

Linux browser runners may need `npx playwright install --with-deps chromium`. UI fixtures contain synthetic content only.

## Local development API and console

Build `wk-server` and `web/dist`, then create a dedicated PostgreSQL database named `wk_dev`. Development mode seeds a synthetic identity for product `demo` with `frontend`, `backend` and `ai-engine` membership:

```sh
export WK_DATABASE_URL='postgres://USER:PASSWORD@127.0.0.1:5432/wk_dev'
export WK_DEV_TOKEN="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
export WK_PORT=4318
export WK_WEB_DIR="$PWD/web/dist"
export WK_CONSOLE_ORIGIN='http://127.0.0.1:4318'
target/debug/wk-server --dev
```

Run that block from the repository root. Open the local origin and enter the generated development token and product `demo`. This bearer-token console exercises the legacy shared-knowledge views. The full My context/Team context/Devices portal requires configured OIDC browser sign-in. `scripts/portal-identity-smoke.py`, `portal-owner-smoke.py`, `portal-team-smoke.py`, `portal-device-smoke.py` and `portal-admin-smoke.py` exercise it with a disposable local provider/database; they are bounded test harnesses, not a persistent sign-in service.

Production migration and identity provisioning are described in [server setup](docs/operations/install.md#host-a-team-server). MCP configuration is separate; see [remote MCP](docs/architecture/remote-mcp.md).

## Contribution policy

Contributions are Apache-2.0. Sign each commit with `git commit -s`, agreeing to the [Developer Certificate of Origin](https://developercertificate.org/). Do not use another person's identity. Dependency changes must preserve license notices and update lockfiles/SBOM. Public release requires separate maintainer authorization; a successful CI build is not authorization to publish.
